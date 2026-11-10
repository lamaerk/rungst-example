use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer};
use std::{error::Error, str::FromStr};

const BUY_LAMPORTS: u64 = 10_000; // 0.01 SOL
const RPC: &str = "https://api.mainnet-beta.solana.com";
type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Load the Solana keypair configured for this example.
pub fn load_wallet() -> Result<Keypair> {
    let path = std::env::var("MOMENTUM_KEYPAIR")
        .map_err(|_| "set MOMENTUM_KEYPAIR to a Solana JSON keypair path")?;
    let bytes: Vec<u8> = serde_json::from_slice(&std::fs::read(&path)?)?;
    let bytes: [u8; 64] = bytes
        .try_into()
        .map_err(|_| "keypair must contain 64 bytes")?;
    Ok(Keypair::try_from(bytes.as_slice())?)
}

/// Create an RPC client for Solana mainnet.
fn rpc() -> RpcClient {
    RpcClient::new(RPC.to_owned())
}

/// Build and submit a buy transaction.
pub async fn buy(_http: &reqwest::Client, wallet: &Keypair, mint: &str) -> Result<String> {
    let (client, tx) = crate::instructions::trade(wallet, mint, true, BUY_LAMPORTS).await?;
    let signature = tx.signatures[0].to_string();
    eprintln!("Submitting transaction {signature}; check its status if the outcome is unknown.");
    client.send_transaction(&tx).await?;
    Ok(signature)
}

/// Build and submit a sell transaction.
pub async fn sell(
    _http: &reqwest::Client,
    wallet: &Keypair,
    mint: &str,
    amount: u64,
) -> Result<String> {
    let (client, tx) = crate::instructions::trade(wallet, mint, false, amount).await?;
    let signature = tx.signatures[0].to_string();
    eprintln!("Submitting transaction {signature}; check its status if the outcome is unknown.");
    client.send_transaction(&tx).await?;
    Ok(signature)
}

/// Simulate a buy and print its logs without submitting it.
pub async fn simulate_buy(_http: &reqwest::Client, wallet: &Keypair, mint: &str) -> Result<()> {
    let (client, tx) = crate::instructions::trade(wallet, mint, true, BUY_LAMPORTS).await?;
    let result = client.simulate_transaction(&tx).await?.value;
    println!(
        "Simulation | mint {mint} | compute units {}",
        result
            .units_consumed
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unknown".into())
    );
    if let Some(logs) = result.logs {
        for log in logs {
            println!("{log}");
        }
    }
    if let Some(error) = result.err {
        return Err(format!("simulation failed: {error}").into());
    }
    println!("Simulation succeeded; transaction was not submitted.");
    Ok(())
}

/// Return the wallet's token balance for a mint.
pub async fn token_balance(_http: &reqwest::Client, wallet: &Keypair, mint: &str) -> Result<u64> {
    let client = rpc();
    let accounts = client
        .get_token_accounts_by_owner(
            &wallet.pubkey(),
            solana_client::rpc_request::TokenAccountsFilter::Mint(Pubkey::from_str(mint)?),
        )
        .await?;
    if accounts.len() > 1 {
        return Err("multiple token accounts; manage this position manually".into());
    }
    match accounts.first() {
        None => Ok(0),
        Some(account) => {
            let key = Pubkey::from_str(&account.pubkey)?;
            Ok(client
                .get_token_account_balance(&key)
                .await?
                .amount
                .parse()?)
        }
    }
}

/// Wait for a transaction to confirm or return its failure or timeout.
pub async fn confirm(_http: &reqwest::Client, signature: &str) -> Result<()> {
    let rpc = rpc();
    let signature = solana_sdk::signature::Signature::from_str(signature)?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let statuses = rpc.get_signature_statuses(&[signature]).await?;
        if let Some(status) = statuses.value[0].as_ref() {
            if let Some(error) = &status.err {
                return Err(format!("transaction failed: {error}").into());
            }
            if status.confirmations.is_none() || status.confirmations.unwrap_or(0) > 0 {
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(
                format!("confirmation unknown for {signature}; check it before retrying").into(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Buy a token and record the confirmed amount as an open position.
pub async fn open_position(
    client: &reqwest::Client,
    wallet: &Keypair,
    mint: &str,
    mcap: f64,
) -> Result<crate::strategies::Position> {
    let starting_sol = rpc().get_balance(&wallet.pubkey()).await?;
    let existing_balance = token_balance(client, wallet, mint).await?;
    if existing_balance != 0 {
        return Err("wallet already holds this token; refusing to mix positions".into());
    }
    let signature = buy(client, wallet, mint).await?;
    confirm(client, &signature).await?;
    let amount = token_balance(client, wallet, mint).await?;
    if amount == 0 {
        return Err("confirmed buy but token balance is zero; investigate manually".into());
    }
    println!("Buy confirmed {signature}; monitoring {mint}, {amount} raw units. Entry market cap is the triggering callout snapshot.");
    Ok(crate::strategies::Position::new(
        mint.to_owned(),
        amount,
        mcap,
        starting_sol,
    ))
}

/// Sell the recorded position and report the wallet's SOL balance change.
pub async fn close_position(
    client: &reqwest::Client,
    wallet: &Keypair,
    position: crate::strategies::Position,
    reason: &str,
) -> Result<()> {
    eprintln!("Selling {} because of {reason}", position.mint);
    let available = token_balance(client, wallet, &position.mint).await?;
    if available < position.amount {
        return Err("token balance is lower than the recorded position; manage it manually".into());
    }
    let signature = sell(client, wallet, &position.mint, position.amount).await?;
    println!("Sell submitted {signature}; waiting for confirmation");
    confirm(client, &signature).await?;
    let final_sol = rpc().get_balance(&wallet.pubkey()).await?;
    let change = (final_sol as i128 - position.starting_sol as i128) as f64 / 1_000_000_000.0;
    println!(
        "Result | {} | {reason} | SOL balance change {change:+.9} | sell {signature}",
        position.mint
    );
    Ok(())
}
