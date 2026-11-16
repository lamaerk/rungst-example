use std::error::Error;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Callout {
    pub mint: String,
    pub call_at_mcap_usd: Option<f64>,
}

pub struct Collector(WebSocketStream<MaybeTlsStream<TcpStream>>);

impl Collector {
    /// Connect to Rungst and subscribe to Solana callouts.
    pub async fn connect() -> Result<Self, Box<dyn Error>> {
        let (mut socket, _) = match connect_async("wss://api.rungst.com/v1/").await {
            Err(tokio_tungstenite::tungstenite::Error::Http(response))
                if response.status().as_u16() == 403 =>
            {
                return Err("Rungst rejected the connection (HTTP 403): register this machine's public IP address in your Rungst account.".into());
            }
            result => result?,
        };
        socket
            .send(Message::Text(
                json!({"action":"subscribeCallouts", "chain":"solana"})
                    .to_string()
                    .into(),
            ))
            .await?;
        Ok(Self(socket))
    }

    /// Read the next callout, skipping non-callout messages.
    pub async fn next(&mut self) -> Result<Option<Callout>, Box<dyn Error>> {
        loop {
            match self.0.next().await {
                Some(Ok(Message::Text(text))) => {
                    let value: serde_json::Value = serde_json::from_str(&text)?;
                    match value.get("type").and_then(|value| value.as_str()) {
                        Some("error") => return Err(format!("Rungst error: {value}").into()),
                        Some(_) => continue,
                        None => {
                            if let Ok(callout) = serde_json::from_value(value) {
                                return Ok(Some(callout));
                            }
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => return Ok(None),
                Some(Err(error)) => return Err(error.into()),
                _ => {}
            }
        }
    }
}
