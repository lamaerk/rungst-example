mod curve;
mod pool;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    account::Account,
    hash::Hash,
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    transaction::Transaction,
};
use std::{error::Error, str::FromStr};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;
const RPC: &str = "https://api.mainnet-beta.solana.com";
const PUMP: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const AMM: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const WSOL: &str = "So11111111111111111111111111111111111111112";
const SPL: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const ATA: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
const FEE_PROGRAM: &str = "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ";
const BUYBACK: &str = "5YxQFdt3Tr9zJLvkFccqXVUwhdTWJQc1fFg2YPbxvxeD";

/// Parse a program address that is known at compile time.
fn key(s: &str) -> Pubkey {
    Pubkey::from_str(s).expect("known program address")
}
/// Derive a program address from its seeds.
fn pda(program: Pubkey, seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &program).0
}
/// Derive the associated token account for an owner and mint.
fn ata(owner: Pubkey, mint: Pubkey, token_program: Pubkey) -> Pubkey {
    pda(
        key(ATA),
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
    )
}
/// Build an idempotent associated-token-account creation instruction.
fn create_ata(owner: Pubkey, mint: Pubkey, token_program: Pubkey) -> Instruction {
    Instruction {
        program_id: key(ATA),
        accounts: vec![
            AccountMeta::new(owner, true),
            AccountMeta::new(ata(owner, mint, token_program), false),
            AccountMeta::new_readonly(owner, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![1], // CreateIdempotent
    }
}

/// Read a public key from an account's packed byte data.
fn account_key(data: &[u8], at: usize) -> Result<Pubkey> {
    Ok(Pubkey::new_from_array(
        data.get(at..at + 32)
            .ok_or("short account data")?
            .try_into()?,
    ))
}
/// Fetch an account when it exists, without treating absence as an RPC error.
async fn optional(rpc: &RpcClient, address: &Pubkey) -> Result<Option<Account>> {
    Ok(rpc
        .get_account_with_commitment(address, rpc.commitment())
        .await?
        .value)
}

/// Simulate a signed transaction and return the requested account snapshots.
async fn simulate(tx: &Transaction, addresses: &[Pubkey]) -> Result<Value> {
    let encoded = STANDARD.encode(bincode::serialize(tx)?);
    let response: Value = reqwest::Client::new().post(RPC)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"simulateTransaction","params":[encoded,{
            "encoding":"base64","sigVerify":true,"commitment":"confirmed",
            "accounts":{"encoding":"jsonParsed","addresses":addresses.iter().map(ToString::to_string).collect::<Vec<_>>()}
        }]})).send().await?.error_for_status()?.json().await?;
    if let Some(err) = response.get("error") {
        return Err(format!("simulation RPC: {err}").into());
    }
    let result = response
        .pointer("/result/value")
        .ok_or("missing simulation result")?;
    if !result["err"].is_null() {
        let logs = result["logs"]
            .as_array()
            .map(|logs| {
                logs.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        return Err(format!("quote simulation failed: {}\n{logs}", result["err"]).into());
    }
    Ok(result.clone())
}

/// Sign instructions with the wallet and its public key as fee payer.
fn signed(wallet: &Keypair, ixs: &[Instruction], hash: Hash) -> Transaction {
    Transaction::new_signed_with_payer(ixs, Some(&wallet.pubkey()), &[wallet], hash)
}

/// Quote a direct SOL pair and return a transaction with 5% output protection.
pub async fn trade(
    wallet: &Keypair,
    mint: &str,
    is_buy: bool,
    amount: u64,
) -> Result<(RpcClient, Transaction)> {
    validate_trade(mint, amount)?;
    let mint = Pubkey::from_str(mint)?;
    let rpc = RpcClient::new(RPC.to_owned());
    let pump = key(PUMP);
    let (complete, cashback) = curve_state(&rpc, pump, mint).await?;
    let token_program = rpc.get_account(&mint).await?.owner;
    validate_token_program(token_program)?;
    let base_ata = ata(wallet.pubkey(), mint, token_program);
    ensure_empty_token_account(&rpc, base_ata, is_buy).await?;
    let (venue, setup, wsol_ata) = prepare_route(
        &rpc,
        wallet,
        pump,
        mint,
        token_program,
        complete,
        cashback,
        is_buy,
        amount,
    )
    .await?;
    let hash = rpc.get_latest_blockhash().await?;
    let first = signed(
        wallet,
        &build_instructions(
            &venue,
            &setup,
            mint,
            wallet.pubkey(),
            token_program,
            is_buy,
            amount,
            1,
            wsol_ata,
        ),
        hash,
    );
    let before_sol = if is_buy {
        0
    } else {
        rpc.get_balance(&wallet.pubkey()).await?
    };
    let quoted = simulate(&first, &[base_ata, wallet.pubkey()]).await?;
    let out = quoted_output(&quoted, is_buy, before_sol)?;
    let min_out = (out as u128 * 95 / 100) as u64;
    if min_out == 0 {
        return Err("quoted output is too small".into());
    }
    eprintln!("Quoted output {out} raw units; minimum {min_out}");
    let instructions = build_instructions(
        &venue,
        &setup,
        mint,
        wallet.pubkey(),
        token_program,
        is_buy,
        amount,
        min_out,
        wsol_ata,
    );
    Ok((rpc, signed(wallet, &instructions, hash)))
}

/// Reject unsupported mints and empty trades before making RPC calls.
fn validate_trade(mint: &str, amount: u64) -> Result<()> {
    if !mint.ends_with("pump") {
        return Err("this example trades pump.fun mints only".into());
    }
    if amount == 0 {
        return Err("trade amount must be positive".into());
    }
    Ok(())
}

/// Read route flags from the mint's bonding-curve account.
async fn curve_state(rpc: &RpcClient, pump: Pubkey, mint: Pubkey) -> Result<(bool, bool)> {
    let address = pda(pump, &[b"bonding-curve", mint.as_ref()]);
    let account = rpc.get_account(&address).await?;
    if account.owner != pump || account.data.get(..8) != Some(&[23, 183, 248, 55, 96, 216, 172, 96])
    {
        return Err("mint has no valid Pump bonding curve".into());
    }

    let data = &account.data;
    let complete = *data.get(48).ok_or("short bonding curve")? != 0;
    let cashback = data.get(82).copied().unwrap_or(0) != 0;
    let quote = if data.len() >= 115 {
        account_key(data, 83)?
    } else {
        Pubkey::default()
    };
    if quote != Pubkey::default() && quote != key(WSOL) {
        return Err("only direct SOL/token pairs are supported".into());
    }
    Ok((complete, cashback))
}

/// Check that the mint is managed by one of the supported token programs.
fn validate_token_program(token_program: Pubkey) -> Result<()> {
    if token_program != key(SPL) && token_program != key(TOKEN_2022) {
        return Err("unsupported token program".into());
    }
    Ok(())
}

/// Prevent a new buy from being mixed with an existing token position.
async fn ensure_empty_token_account(rpc: &RpcClient, address: Pubkey, is_buy: bool) -> Result<()> {
    if !is_buy {
        return Ok(());
    }
    if let Some(existing) = optional(rpc, &address).await? {
        let balance = existing.data.get(64..72).ok_or("invalid token account")?;
        if u64::from_le_bytes(balance.try_into()?) != 0 {
            return Err("wallet already holds this token; refusing to mix positions".into());
        }
    }
    Ok(())
}

/// Select the bonding curve or PumpSwap pool and prepare any needed accounts.
async fn prepare_route(
    rpc: &RpcClient,
    wallet: &Keypair,
    pump: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
    complete: bool,
    cashback: bool,
    is_buy: bool,
    amount: u64,
) -> Result<(Venue, Vec<Instruction>, Pubkey)> {
    let wsol_ata = ata(wallet.pubkey(), key(WSOL), key(SPL));
    let mut setup = if is_buy {
        vec![create_ata(wallet.pubkey(), mint, token_program)]
    } else {
        Vec::new()
    };
    if !complete {
        if cashback {
            return Err("cashback curves require a different instruction; skipping token".into());
        }
        eprintln!("Pump route: bonding curve");
        return Ok((Venue::Curve, setup, wsol_ata));
    }

    let state = load_pool(rpc, pump, mint).await?;
    ensure_empty_wsol_account(rpc, wsol_ata).await?;
    setup.push(create_ata(wallet.pubkey(), key(WSOL), key(SPL)));
    if is_buy {
        setup.extend(wsol_deposit_instructions(wallet.pubkey(), wsol_ata, amount));
    }
    eprintln!("Pump route: PumpSwap pool");
    Ok((Venue::Pool(state), setup, wsol_ata))
}

/// Fetch and parse the canonical migrated PumpSwap pool.
async fn load_pool(rpc: &RpcClient, pump: Pubkey, mint: Pubkey) -> Result<pool::Pool> {
    let authority = pda(pump, &[b"pool-authority", mint.as_ref()]);
    let address = pda(
        key(AMM),
        &[
            b"pool",
            &0u16.to_le_bytes(),
            authority.as_ref(),
            mint.as_ref(),
            key(WSOL).as_ref(),
        ],
    );
    let account = optional(rpc, &address)
        .await?
        .ok_or("curve is complete but canonical PumpSwap pool is not available yet")?;
    pool::Pool::parse(address, &account, mint, key(WSOL))
}

/// Ensure the wrapped-SOL account is empty before this example uses it.
async fn ensure_empty_wsol_account(rpc: &RpcClient, address: Pubkey) -> Result<()> {
    if let Some(existing) = optional(rpc, &address).await? {
        let balance = existing
            .data
            .get(64..72)
            .map(|bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
            .unwrap_or(0);
        if balance != 0 {
            return Err(
                "wallet already has wrapped SOL; unwrap it before this example trades".into(),
            );
        }
    }
    Ok(())
}

/// Create and initialize wrapped SOL for a pool buy.
fn wsol_deposit_instructions(user: Pubkey, wsol_ata: Pubkey, amount: u64) -> Vec<Instruction> {
    vec![
        Instruction {
            program_id: solana_sdk::system_program::id(),
            accounts: vec![
                AccountMeta::new(user, true),
                AccountMeta::new(wsol_ata, false),
            ],
            data: [
                2u32.to_le_bytes().as_slice(),
                amount.to_le_bytes().as_slice(),
            ]
            .concat(),
        },
        Instruction {
            program_id: key(SPL),
            accounts: vec![AccountMeta::new(wsol_ata, false)],
            data: vec![17],
        },
    ]
}

/// Assemble compute, setup, swap, and optional wrapped-SOL cleanup instructions.
fn build_instructions(
    venue: &Venue,
    setup: &[Instruction],
    mint: Pubkey,
    user: Pubkey,
    token_program: Pubkey,
    is_buy: bool,
    amount: u64,
    min_out: u64,
    wsol_ata: Pubkey,
) -> Vec<Instruction> {
    let swap = match venue {
        Venue::Curve => curve::instruction(mint, user, token_program, is_buy, amount, min_out),
        Venue::Pool(state) => {
            pool::instruction(state, user, token_program, is_buy, amount, min_out)
        }
    };
    let mut instructions = vec![compute_budget_instruction()];
    instructions.extend_from_slice(setup);
    instructions.push(swap);
    if matches!(venue, Venue::Pool(_)) {
        instructions.push(close_wsol_instruction(wsol_ata, user));
    }
    instructions
}

/// Build the compute-unit limit instruction used by both venue routes.
fn compute_budget_instruction() -> Instruction {
    Instruction {
        program_id: key("ComputeBudget111111111111111111111111111111"),
        accounts: vec![],
        data: [vec![2], 400_000u32.to_le_bytes().to_vec()].concat(),
    }
}

/// Close the wrapped-SOL account and return its rent to the signer.
fn close_wsol_instruction(wsol_ata: Pubkey, user: Pubkey) -> Instruction {
    Instruction {
        program_id: key(SPL),
        accounts: vec![
            AccountMeta::new(wsol_ata, false),
            AccountMeta::new(user, false),
            AccountMeta::new_readonly(user, true),
        ],
        data: vec![9],
    }
}

/// Extract quoted token or SOL proceeds from the simulation response.
fn quoted_output(quoted: &Value, is_buy: bool, before_sol: u64) -> Result<u64> {
    if is_buy {
        return Ok(quoted
            .pointer("/accounts/0/data/parsed/info/tokenAmount/amount")
            .and_then(Value::as_str)
            .ok_or("simulation did not return token amount")?
            .parse::<u64>()?);
    }
    let after = quoted
        .pointer("/accounts/1/lamports")
        .and_then(Value::as_u64)
        .ok_or("simulation did not return SOL balance")?;
    after
        .checked_sub(before_sol)
        .ok_or_else(|| "simulation produced no SOL proceeds".into())
}

enum Venue {
    Curve,
    Pool(pool::Pool),
}
