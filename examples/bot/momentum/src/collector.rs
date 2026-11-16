use std::error::Error;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    pub trader: String,
    pub side: String,
    pub token_in: Token,
    pub token_out: Token,
    pub amount_usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct Token {
    pub mint: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Callout {
    pub created_at: Option<i64>,
    pub mint: String,
    pub call_at_mcap_usd: Option<f64>,
}

pub struct Collector(WebSocketStream<MaybeTlsStream<TcpStream>>);

impl Collector {
    /// Subscribe to either the unfiltered Solana trades or callouts stream.
    pub async fn connect(action: &str) -> Result<Self, Box<dyn Error>> {
        let (mut socket, _) = connect_async("wss://api.rungst.com/v1/").await?;
        socket
            .send(Message::Text(
                json!({"action": action, "chain": "solana"})
                    .to_string()
                    .into(),
            ))
            .await?;
        Ok(Self(socket))
    }

    /// Read the next data message and surface subscription errors.
    pub async fn next<T: serde::de::DeserializeOwned>(
        &mut self,
    ) -> Result<Option<T>, Box<dyn Error>> {
        loop {
            match self.0.next().await {
                Some(Ok(Message::Text(text))) => {
                    let value: serde_json::Value = serde_json::from_str(&text)?;
                    match value.get("type").and_then(|v| v.as_str()) {
                        Some("error") => return Err(format!("Rungst error: {value}").into()),
                        Some(_) => continue,
                        None => match serde_json::from_value(value) {
                            Ok(event) => return Ok(Some(event)),
                            Err(error) => eprintln!("Ignoring invalid stream event: {error}"),
                        },
                    }
                }
                Some(Ok(Message::Close(_))) | None => return Ok(None),
                Some(Err(error)) => return Err(error.into()),
                _ => {}
            }
        }
    }
}
