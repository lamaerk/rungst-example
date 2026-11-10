use std::{error::Error, time::Duration};

use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const URL: &str = "wss://api.rungst.com/v1/";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Trade {
    block_time: i64,
    terminal: String,
    trader: String,
    handle: Option<String>,
    side: String,
    token_in: Token,
    token_out: Token,
    amount_usd: Option<f64>,
}

#[derive(Deserialize)]
struct Token {
    mint: String,
}

/// Shorten a wallet address for readable stream output.
fn short_address(address: &str) -> String {
    let chars: Vec<char> = address.chars().collect();
    if chars.len() <= 12 {
        return address.to_owned();
    }
    format!(
        "{}…{}",
        chars[..4].iter().collect::<String>(),
        chars[chars.len() - 4..].iter().collect::<String>()
    )
}

/// Format one trade as a concise terminal row.
fn print_trade(trade: Trade) {
    let time = DateTime::<Utc>::from_timestamp(trade.block_time, 0)
        .map(|time| time.to_rfc3339())
        .unwrap_or_else(|| trade.block_time.to_string());
    let author = trade
        .handle
        .filter(|handle| !handle.is_empty())
        .map(|handle| format!("@{}", handle.trim_start_matches('@')))
        .unwrap_or_else(|| short_address(&trade.trader));
    // The traded token is received on a buy and sent on a sell.
    let mint = if trade.side == "buy" {
        &trade.token_out.mint
    } else {
        &trade.token_in.mint
    };
    let amount = trade
        .amount_usd
        .map(|usd| format!("${usd:.2}"))
        .unwrap_or_else(|| "USD unavailable".to_owned());
    println!(
        "{time} | {} | {author} | {} | {mint} | {amount}",
        trade.terminal, trade.side
    );
}

/// Subscribe to Solana trades, optionally filtered by mint or trader.
async fn subscribe(filter: Option<(&str, &str)>) -> Result<(), Box<dyn Error>> {
    let (mut socket, _) = connect_async(URL).await?;
    let mut command = json!({"action": "subscribe", "chain": "solana"});
    if let Some((kind, address)) = filter {
        command[kind] = json!(address);
    }
    socket
        .send(Message::Text(command.to_string().into()))
        .await?;
    while let Some(message) = socket.next().await {
        match message? {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text)?;
                match value.get("type").and_then(|value| value.as_str()) {
                    Some("subscribed") => eprintln!("Subscribed; waiting for trades…"),
                    Some("error") => return Err(format!("Rungst error: {value}").into()),
                    Some(_) => {}
                    None => match serde_json::from_value::<Trade>(value) {
                        Ok(trade) => print_trade(trade),
                        Err(error) => eprintln!("Ignoring an invalid trade: {error}"),
                    },
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

/// Parse stream filters and reconnect until interrupted.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let filter = match args.as_slice() {
        [] => None,
        [flag, address] if flag == "--mint" => Some(("mint", address.as_str())),
        [flag, address] if flag == "--trader" => Some(("trader", address.as_str())),
        [flag] if flag == "--help" => {
            println!("Usage: cargo run -p trades -- [--mint ADDRESS | --trader ADDRESS]");
            return Ok(());
        }
        _ => {
            return Err("Usage: cargo run -p trades -- [--mint ADDRESS | --trader ADDRESS]".into())
        }
    };
    loop {
        tokio::select! {
            result = subscribe(filter) => match result {
                Ok(()) => eprintln!("Disconnected; reconnecting in 5 seconds…"),
                Err(error) => {
                    eprintln!("Connection ended: {error}");
                    if error.to_string().starts_with("Rungst error:") { return Err(error); }
                    eprintln!("Reconnecting in 5 seconds…");
                }
            },
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {},
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}
