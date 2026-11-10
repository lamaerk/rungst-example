use std::{error::Error, time::Duration};

use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const URL: &str = "wss://api.rungst.com/v1/";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Callout {
    created_at: i64,
    handle: Option<String>,
    trader: Option<String>,
    terminal: String,
    symbol: Option<String>,
    thesis: String,
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

/// Format one callout as a concise terminal row.
fn print_callout(callout: Callout) {
    let time = DateTime::<Utc>::from_timestamp_millis(callout.created_at)
        .map(|time| time.to_rfc3339())
        .unwrap_or_else(|| callout.created_at.to_string());
    let author = callout
        .handle
        .filter(|handle| !handle.is_empty())
        .map(|handle| format!("@{}", handle.trim_start_matches('@')))
        .or_else(|| callout.trader.as_deref().map(short_address))
        .unwrap_or_else(|| "unknown".to_owned());
    println!(
        "{} | {} | {} | {} | {}",
        time,
        callout.terminal,
        author,
        callout
            .symbol
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("?"),
        callout.thesis
    );
}

/// Subscribe to callouts, optionally filtered by mint.
async fn subscribe(mint: Option<&str>) -> Result<(), Box<dyn Error>> {
    let (mut socket, _) = connect_async(URL).await?;
    let mut command = json!({"action": "subscribeCallouts", "chain": "solana"});
    if let Some(mint) = mint {
        command["mint"] = json!(mint);
    }
    socket
        .send(Message::Text(command.to_string().into()))
        .await?;

    while let Some(message) = socket.next().await {
        match message? {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text)?;
                match value.get("type").and_then(|value| value.as_str()) {
                    Some("subscribed") => eprintln!("Subscribed; waiting for callouts…"),
                    Some("error") => return Err(format!("Rungst error: {}", value).into()),
                    Some(_) => {}
                    None => match serde_json::from_value::<Callout>(value) {
                        Ok(callout) => print_callout(callout),
                        Err(error) => eprintln!("Ignoring an invalid callout: {error}"),
                    },
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

/// Parse the optional mint filter and keep the stream connected.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let mint = args.next();
    if mint.as_deref() == Some("--help") || args.next().is_some() {
        eprintln!("Usage: cargo run -p callouts -- [MINT]");
        return Ok(());
    }

    loop {
        tokio::select! {
            result = subscribe(mint.as_deref()) => match result {
                Ok(()) => eprintln!("Disconnected; reconnecting in 5 seconds…"),
                Err(error) => {
                    eprintln!("Connection ended: {error}");
                    // Subscription errors require user action; don't retry them indefinitely.
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
