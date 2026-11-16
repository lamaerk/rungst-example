//! Client-side build/sign/submit example. Wallet bytes never leave this process.
//! Run only when you intend to trade: see docs/trading.md.
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use solana_address::Address;
use solana_transaction::versioned::VersionedTransaction;
use std::str::FromStr;
use std::{env, error::Error, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();
    let args: Vec<_> = env::args().collect();
    let submit = args.last().is_some_and(|arg| arg == "--submit");
    let positional = &args[1..args.len() - usize::from(submit)];
    if !(positional.len() == 3 || (positional.len() == 2 && positional[0] == "sell"))
        || !matches!(positional.first().map(String::as_str), Some("buy" | "sell"))
    {
        return Err(
            "Usage: cargo run -- buy MINT AMOUNT [--submit] | sell MINT [AMOUNT] [--submit]".into(),
        );
    }
    let origin = "https://api.rungst.com";
    let api_key = env::var("RUNGST_API_KEY")?;
    let rpc = env::var("RUNGST_SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet.solana.com".into());
    let rpc_url = reqwest::Url::parse(&rpc).map_err(|_| "Invalid RUNGST_SOLANA_RPC_URL")?;
    if rpc_url.scheme() != "https"
        && !(rpc_url.scheme() == "http"
            && matches!(rpc_url.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
    {
        return Err("RPC URL must use HTTPS or loopback HTTP".into());
    }
    // Read a local Solana CLI JSON keypair. This example is a client, not the server.
    let bytes: Vec<u8> = serde_json::from_slice(&std::fs::read(env::var("RUNGST_WALLET_FILE")?)?)?;
    let bytes: [u8; 64] = bytes
        .try_into()
        .map_err(|_| "Expected a 64-byte local keypair")?;
    let signer = SigningKey::from_keypair_bytes(&bytes)?;
    let wallet = bs58::encode(signer.verifying_key().to_bytes()).into_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let amount = if positional[0] == "sell" && positional.len() == 2 {
        sell_balance(&client, &rpc_url, &wallet, &positional[1]).await?
    } else {
        positional[2].clone()
    };
    let response = client.post(format!("{}/v1/trade/build", origin.trim_end_matches('/')))
        .bearer_auth(&api_key)
        .json(&json!({"side":positional[0],"mint":positional[1],"amount":amount,"wallet":wallet,"slippageBps":100}))
        .send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await?;
        let detail = serde_json::from_str::<Value>(&body).ok().and_then(|value| {
            value["message"]
                .as_str()
                .or_else(|| value["error"].as_str())
                .map(str::to_owned)
        });
        return Err(match detail {
            Some(detail) => format!("Trade build failed (HTTP {status}): {detail}"),
            None => format!(
                "Trade build failed (HTTP {status}); the server did not provide an error message"
            ),
        }
        .into());
    }
    let build: Value = response.json().await?;
    println!(
        "Build ID: {}. Commission: {} USDC ({}). Last valid block height: {}.",
        build["transactionId"],
        build["fee"]["amount"],
        build["fee"]["basis"],
        build["lastValidBlockHeight"]
    );
    if !submit {
        println!(
            "Build only. Add --submit to a new invocation to build, sign locally and broadcast."
        );
        return Ok(());
    }
    let mut tx: VersionedTransaction = bincode::deserialize(
        &STANDARD.decode(build["transaction"].as_str().ok_or("Missing transaction")?)?,
    )?;
    tx.sanitize()?;
    if tx.message.header().num_required_signatures != 1
        || tx.message.static_account_keys()[0].to_string() != wallet
        || !matches!(tx.message, solana_message::VersionedMessage::V0(_))
    {
        return Err("Unexpected transaction payer, signers or version".into());
    }
    tx.signatures[0] = signer.sign(&tx.message.serialize()).to_bytes().into();
    let payload = json!({"transactionId":build["transactionId"],"signedTransaction":STANDARD.encode(bincode::serialize(&tx)?)});
    // Every retry sends the same payload; never rebuild after an ambiguous submit.
    let mut last_error = None;
    for _ in 0..3 {
        match client
            .post(format!("{}/v1/trade/submit", origin.trim_end_matches('/')))
            .bearer_auth(&api_key)
            .json(&payload)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                let submitted: Value = response.json().await?;
                println!("{}", submitted);
                let signature = submitted["signature"].as_str().ok_or("Missing signature")?;
                return confirm(&client, &rpc_url, signature).await;
            }
            Ok(response) if !response.status().is_server_error() => {
                let rejected: Value = response.json().await?;
                return Err(rejected["message"]
                    .as_str()
                    .or_else(|| rejected["error"].as_str())
                    .unwrap_or("Trade could not be completed")
                    .to_owned()
                    .into());
            }
            Ok(response) => last_error = Some(format!("HTTP {}", response.status())),
            Err(_) => last_error = Some("Network timeout or connection failure".into()),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(format!(
        "Submission uncertain: {}. Check signature {} on chain before creating another trade.",
        last_error.unwrap_or_default(),
        tx.signatures[0]
    )
    .into())
}

async fn confirm(
    client: &reqwest::Client,
    url: &reqwest::Url,
    signature: &str,
) -> Result<(), Box<dyn Error>> {
    println!("Waiting for confirmation of {signature}...");
    for _ in 0..60 {
        let result: Value = match async {
            let response = client.post(url.clone()).json(&json!({"jsonrpc":"2.0","id":1,"method":"getSignatureStatuses","params":[[signature], {"searchTransactionHistory":true}]})).send().await?.error_for_status()?;
            response.json().await
        }.await {
            Ok(result) => result,
            Err(_) => return Err(format!("Could not check confirmation. Check signature {signature} on chain before creating another trade.").into()),
        };
        if result.get("error").is_some() || result.pointer("/result/value/0").is_none() {
            return Err(format!("Invalid RPC response while checking confirmation. Check signature {signature} on chain before creating another trade.").into());
        }
        let status = &result["result"]["value"][0];
        if matches!(
            status["confirmationStatus"].as_str(),
            Some("confirmed" | "finalized")
        ) {
            let error = status.get("err").ok_or("Invalid RPC response")?;
            if error.is_null() {
                println!("Trade confirmed successfully: {signature}");
                return Ok(());
            }
            let reason = error
                .as_str()
                .or_else(|| {
                    error
                        .as_object()
                        .and_then(|o| o.keys().next().map(String::as_str))
                })
                .unwrap_or("");
            if matches!(
                reason,
                "InsufficientFundsForFee" | "InsufficientFundsForRent"
            ) {
                return Err("Your trading wallet needs more SOL for transaction fees. Fund it with SOL and try again.".into());
            }
            return Err("The trade failed. Check your wallet balances and request a new quote before trying again.".into());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(format!(
        "Confirmation is still pending. Check signature {signature} before creating another trade."
    )
    .into())
}

async fn sell_balance(
    client: &reqwest::Client,
    url: &reqwest::Url,
    wallet: &str,
    mint: &str,
) -> Result<String, Box<dyn Error>> {
    let result: Value = client.post(url.clone()).json(&json!({"jsonrpc":"2.0","id":1,"method":"getTokenAccountsByOwner","params":[wallet,{"mint":mint},{"encoding":"jsonParsed","commitment":"confirmed"}]})).send().await?.error_for_status()?.json().await?;
    let accounts = result
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or("Failed to fetch token balance")?;
    let owner = Address::from_str(wallet)?;
    let mint = Address::from_str(mint)?;
    let ata_program = Address::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")?;
    for account in accounts {
        let program = account
            .pointer("/account/owner")
            .and_then(Value::as_str)
            .ok_or("Invalid token account")?;
        if !matches!(
            program,
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
                | "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
        ) {
            continue;
        }
        let program = Address::from_str(program)?;
        let ata = Address::find_program_address(
            &[owner.as_ref(), program.as_ref(), mint.as_ref()],
            &ata_program,
        )
        .0;
        if account["pubkey"].as_str() != Some(&ata.to_string()) {
            continue;
        }
        let token = account
            .pointer("/account/data/parsed/info/tokenAmount")
            .ok_or("Invalid token balance")?;
        let raw = token["amount"].as_str().ok_or("Invalid token amount")?;
        let decimals = token["decimals"]
            .as_u64()
            .filter(|n| *n <= 255)
            .ok_or("Invalid token decimals")? as usize;
        if raw.parse::<u64>()? == 0 {
            return Err("No tokens available to sell".into());
        }
        let padded = format!("{:0>width$}", raw, width = decimals + 1);
        let split = padded.len() - decimals;
        return Ok(if decimals == 0 {
            padded
        } else {
            format!("{}.{}", &padded[..split], &padded[split..])
        });
    }
    Err("No associated token account found for this mint".into())
}
