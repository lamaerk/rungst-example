//! Build trades through Rungst and sign locally, following the swaps example.
use crate::logging;
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use solana_address::Address;
use solana_transaction::versioned::VersionedTransaction;
use std::cell::Cell;
use std::io::IsTerminal;
use std::str::FromStr;
use std::{env, error::Error, time::Duration};

/// A confirmed on-chain failure is safe to retry with a new transaction.
#[derive(Debug)]
struct ConfirmedFailure(String);
impl std::fmt::Display for ConfirmedFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Error for ConfirmedFailure {}

const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

pub struct Executor {
    client: reqwest::Client,
    rpc_url: reqwest::Url,
    api_key: String,
    signer: SigningKey,
    wallet: String,
    buy_amount: String,
    slippage_bps: u64,
    trades: Cell<u64>,
    wins: Cell<u64>,
    losses: Cell<u64>,
    pnl: Cell<f64>,
    unknown_pnl: Cell<u64>,
}

impl Executor {
    /// Load trading credentials and validate the RPC endpoint.
    pub fn from_env() -> Result<Self, Box<dyn Error>> {
        let api_key = env::var("RUNGST_API_KEY")
            .map_err(|_| "Set RUNGST_API_KEY in your environment or .env file")?;
        let rpc = env::var("RUNGST_SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet.solana.com".into());
        let rpc_url = reqwest::Url::parse(&rpc)?;
        if rpc_url.scheme() != "https"
            && !(rpc_url.scheme() == "http"
                && matches!(rpc_url.host_str(), Some("localhost" | "127.0.0.1" | "::1")))
        {
            return Err("RPC URL must use HTTPS or loopback HTTP".into());
        }
        let bytes: Vec<u8> = serde_json::from_slice(&std::fs::read(
            env::var("RUNGST_WALLET_FILE")
                .map_err(|_| "Set RUNGST_WALLET_FILE to your Solana keypair path")?,
        )?)?;
        let bytes: [u8; 64] = bytes
            .try_into()
            .map_err(|_| "Expected a 64-byte local keypair")?;
        let signer = SigningKey::from_keypair_bytes(&bytes)?;
        let wallet = bs58::encode(signer.verifying_key().to_bytes()).into_string();
        let buy_amount = env::var("BUY_AMOUNT_USDC")
            .map_err(|_| "Set BUY_AMOUNT_USDC to the amount to spend")?;
        let amount: f64 = buy_amount.parse()?;
        if !amount.is_finite() || amount <= 0.0 {
            return Err("BUY_AMOUNT_USDC must be positive and finite".into());
        }
        let slippage_bps: u64 = env::var("SLIPPAGE_BPS")
            .unwrap_or_else(|_| "100".into())
            .parse()?;
        if slippage_bps == 0 || slippage_bps > 10_000 {
            return Err("SLIPPAGE_BPS must be between 1 and 10000".into());
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            client,
            rpc_url,
            api_key,
            signer,
            wallet,
            buy_amount,
            slippage_bps,
            trades: Cell::new(0),
            wins: Cell::new(0),
            losses: Cell::new(0),
            pnl: Cell::new(0.0),
            unknown_pnl: Cell::new(0),
        })
    }

    /// Return the configured USDC order size for startup output.
    pub fn buy_amount(&self) -> &str {
        &self.buy_amount
    }

    /// Confirm a buy and record the resulting associated-token-account balance.
    pub async fn open(
        &self,
        mint: &str,
        mcap: f64,
    ) -> Result<crate::strategies::Position, Box<dyn Error>> {
        if token_balance(&self.client, &self.rpc_url, &self.wallet, mint)
            .await?
            .is_some()
        {
            return Err("Wallet already holds this token; refusing to mix positions".into());
        }
        let starting_usdc = self.usdc_balance().await?;
        self.trade("buy", mint, &self.buy_amount).await?;
        let amount = token_balance(&self.client, &self.rpc_url, &self.wallet, mint)
            .await?
            .ok_or("Confirmed buy but token balance is zero; investigate manually")?;
        logging::info(&format!(
            "BUY     | {} | confirmed | requested {} USDC | received {amount} tokens",
            logging::short(mint),
            self.buy_amount
        ));
        Ok(crate::strategies::Position {
            mint: mint.to_owned(),
            entry_mcap: mcap,
            starting_usdc,
            opened_at: tokio::time::Instant::now(),
            last_mcap: None,
            amount,
        })
    }

    /// Sell the recorded amount after verifying the wallet still has that balance.
    pub async fn close(
        &self,
        position: &crate::strategies::Position,
        reason: &str,
    ) -> Result<(), Box<dyn Error>> {
        let snapshot = match position.last_mcap {
            Some((at, mcap)) => format!(
                "last callout mcap ${mcap:.2} ({}s ago)",
                at.elapsed().as_secs()
            ),
            None => "last callout mcap n/a".to_owned(),
        };
        logging::info(&format!(
            "EXIT    | {} | reason={reason} | held {}s | {snapshot}",
            logging::short(&position.mint),
            position.opened_at.elapsed().as_secs()
        ));
        let current = token_balance(&self.client, &self.rpc_url, &self.wallet, &position.mint)
            .await?
            .ok_or("Position balance is missing; manage it manually")?;
        if decimal_less(&current, &position.amount) {
            return Err("Token balance is below the recorded position; manage it manually".into());
        }
        self.trade("sell", &position.mint, &position.amount).await?;
        logging::info(&format!(
            "SELL    | {} | confirmed | sold {} tokens",
            logging::short(&position.mint),
            position.amount
        ));
        self.trades.set(self.trades.get() + 1);
        match self.usdc_balance().await {
            Ok(final_usdc) => {
                let pnl = final_usdc - position.starting_usdc;
                let color = if pnl >= 0.0 { "\x1b[32m" } else { "\x1b[31m" };
                let (start, end) = if std::io::stdout().is_terminal() {
                    (color, "\x1b[0m")
                } else {
                    ("", "")
                };
                self.pnl.set(self.pnl.get() + pnl);
                if pnl > 0.0 {
                    self.wins.set(self.wins.get() + 1);
                } else if pnl < 0.0 {
                    self.losses.set(self.losses.get() + 1);
                }
                logging::info(&format!(
                    "{start}P&L     | {} | USDC balance change {pnl:+.6} USDC | held {}s | excludes SOL fees{end}",
                    logging::short(&position.mint),
                    position.opened_at.elapsed().as_secs()
                ));
            }
            Err(error) => {
                self.unknown_pnl.set(self.unknown_pnl.get() + 1);
                eprintln!("Could not calculate P&L after confirmed sale: {error}");
            }
        }
        logging::info(&format!(
            "SESSION | trades={} | wins={} | losses={} | unknown={} | USDC balance change {:+.6}",
            self.trades.get(),
            self.wins.get(),
            self.losses.get(),
            self.unknown_pnl.get(),
            self.pnl.get()
        ));
        Ok(())
    }

    /// Read the wallet's USDC associated-token-account balance.
    async fn usdc_balance(&self) -> Result<f64, Box<dyn Error>> {
        let amount = token_balance(&self.client, &self.rpc_url, &self.wallet, USDC_MINT).await?;
        Ok(amount.as_deref().unwrap_or("0").parse()?)
    }

    /// Build, sign, submit and confirm a trade without rebuilding ambiguous submissions.
    async fn trade(&self, side: &str, mint: &str, amount: &str) -> Result<(), Box<dyn Error>> {
        for attempt in 1..=3 {
            match self.trade_once(side, mint, amount).await {
                Err(error) if error.is::<ConfirmedFailure>() && attempt < 3 => {
                    eprintln!(
                        "Trade attempt {attempt} failed: {error}; building a new transaction"
                    );
                }
                result => return result,
            }
        }
        unreachable!()
    }

    /// Build and submit one transaction, preserving its outcome for retry decisions.
    async fn trade_once(&self, side: &str, mint: &str, amount: &str) -> Result<(), Box<dyn Error>> {
        let Self {
            client,
            rpc_url,
            api_key,
            signer,
            wallet,
            slippage_bps,
            ..
        } = self;
        let origin = "https://api.rungst.com";
        let response = client.post(format!("{}/v1/trade/build", origin.trim_end_matches('/')))
        .bearer_auth(&api_key)
        .json(&json!({"side":side,"mint":mint,"amount":amount,"wallet":wallet,"slippageBps":slippage_bps}))
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
        let mut tx: VersionedTransaction = bincode::deserialize(
            &STANDARD.decode(build["transaction"].as_str().ok_or("Missing transaction")?)?,
        )?;
        tx.sanitize()?;
        if tx.message.header().num_required_signatures != 1
            || tx.message.static_account_keys()[0].to_string() != *wallet
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
                    let signature = submitted["signature"].as_str().ok_or("Missing signature")?;
                    logging::info(&format!(
                        "{:<7} | {} | submitted | sig {} | https://solscan.io/tx/{signature}",
                        side.to_uppercase(),
                        logging::short(mint),
                        logging::short(signature)
                    ));
                    let started = std::time::Instant::now();
                    confirm(client, rpc_url, signature).await?;
                    logging::info(&format!(
                        "{:<7} | {} | transaction confirmed in {:.1}s",
                        side.to_uppercase(),
                        logging::short(mint),
                        started.elapsed().as_secs_f64()
                    ));
                    return Ok(());
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
}

/// Wait for an on-chain confirmation without resubmitting an uncertain trade.
async fn confirm(
    client: &reqwest::Client,
    url: &reqwest::Url,
    signature: &str,
) -> Result<(), Box<dyn Error>> {
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
                return Ok(());
            }
            return Err(ConfirmedFailure(format!(
                "Transaction {signature} failed on chain: {error}"
            ))
            .into());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(format!(
        "Confirmation is still pending. Check signature {signature} before creating another trade."
    )
    .into())
}

/// Read the associated token account as a decimal amount, or None for zero balance.
async fn token_balance(
    client: &reqwest::Client,
    url: &reqwest::Url,
    wallet: &str,
    mint: &str,
) -> Result<Option<String>, Box<dyn Error>> {
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
            return Ok(None);
        }
        let padded = format!("{:0>width$}", raw, width = decimals + 1);
        let split = padded.len() - decimals;
        return Ok(Some(if decimals == 0 {
            padded
        } else {
            format!("{}.{}", &padded[..split], &padded[split..])
        }));
    }
    Ok(None)
}

/// Compare nonnegative decimal token amounts without floating-point rounding.
fn decimal_less(a: &str, b: &str) -> bool {
    fn parts(s: &str) -> (&str, &str) {
        let (i, f) = s.split_once('.').unwrap_or((s, ""));
        (i.trim_start_matches('0'), f)
    }
    let (ai, af) = parts(a);
    let (bi, bf) = parts(b);
    if ai.len() != bi.len() {
        return ai.len() < bi.len();
    }
    if ai != bi {
        return ai < bi;
    }
    let n = af.len().max(bf.len());
    format!("{af:0<n$}") < format!("{bf:0<n$}")
}
