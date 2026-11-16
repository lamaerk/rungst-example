mod collector;
mod executor;
mod logging;
mod strategies;

use collector::{Callout, Collector, Trade};
use std::{
    collections::HashMap,
    error::Error,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use strategies::{Action, Config, Strategy};
use tokio::time::Instant;

/// Start in observation mode unless live trading is explicitly enabled.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();
    let args: Vec<_> = std::env::args().skip(1).collect();
    let live = match args.as_slice() {
        [] => false,
        [flag] if flag == "--buy" => true,
        _ => return Err("Usage: cargo run -p momentum -- [--buy]".into()),
    };
    let max_buys_per_run: Option<u64> = match std::env::var("MAX_BUYS_PER_RUN") {
        Ok(value) => {
            let count: u64 = value
                .parse()
                .map_err(|_| "MAX_BUYS_PER_RUN must be a positive integer")?;
            if count == 0 {
                return Err("MAX_BUYS_PER_RUN must be a positive integer".into());
            }
            Some(count)
        }
        Err(std::env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let mut buys_this_run = 0_u64;
    let config = read_config()?;
    let executor = if live {
        Some(executor::Executor::from_env()?)
    } else {
        None
    };
    let max_buys: u64 = std::env::var("MAX_BUYS_PER_MINT")
        .unwrap_or_else(|_| "1".into())
        .parse()?;
    if max_buys == 0 {
        return Err("MAX_BUYS_PER_MINT must be positive".into());
    }
    let history_path = PathBuf::from(
        std::env::var("BUY_HISTORY_FILE").unwrap_or_else(|_| "momentum-buy-history.json".into()),
    );
    let mut history: HashMap<String, u64> = if executor.is_some() && history_path.exists() {
        serde_json::from_slice(&std::fs::read(&history_path)?)?
    } else {
        HashMap::new()
    };
    let mut strategy = Strategy::new(config);
    let mut trades = Collector::connect("subscribe").await?;
    let mut callouts = Collector::connect("subscribeCallouts").await?;
    if let Some(executor) = &executor {
        logging::info(&format!(
            "Live trading enabled | strategy=momentum | size={} USDC",
            executor.buy_amount()
        ));
    } else {
        logging::info("Observation only | strategy=momentum");
    }
    logging::info("Waiting for momentum…");
    let mut opened_at = 0_u128;
    let timeout_secs: u64 = std::env::var("SELL_TIMEOUT_SECS")
        .unwrap_or_else(|_| "90".into())
        .parse()?;
    if timeout_secs == 0 {
        return Err("SELL_TIMEOUT_SECS must be positive".into());
    }
    let mut deadline: Option<Instant> = None;
    loop {
        let action = tokio::select! {
            _ = async { match deadline { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => Some(Action::Sell("timeout")),
            _ = tokio::signal::ctrl_c() => {
                if let (Some(position), Some(executor)) = (strategy.position.as_ref(), executor.as_ref()) {
                    logging::info("Ctrl+C received; selling the open position before exiting.");
                    executor.close(position, "shutdown").await.map_err(|error| format!("Shutdown sell failed or is uncertain: {error}; check the position and transaction manually"))?;
                }
                return Ok(());
            }
            event = trades.next::<Trade>() => match event {
                Ok(Some(trade)) => strategy.on_trade(trade, Instant::now()),
                Ok(None) => return Err("Trade stream disconnected; check any open position manually".into()),
                Err(e) => return Err(format!("Trade stream error: {e}; check any open position manually").into()),
            },
            event = callouts.next::<Callout>() => match event {
                Ok(Some(callout)) => {
                    // Ignore queued callouts from before the buy finished confirming.
                    if strategy.position.is_some() && callout.created_at.is_none_or(|t| t < 0 || (t as u128) <= opened_at) { None }
                    else { strategy.on_callout(callout, Instant::now()) }
                },
                Ok(None) => return Err("Callout stream disconnected; check any open position manually".into()),
                Err(e) => return Err(format!("Callout stream error: {e}; check any open position manually").into()),
            },
        };
        match (action, &executor) {
            (
                Some(Action::Buy {
                    mint,
                    mcap,
                    callouts,
                    buyers,
                    ratio,
                    netflow,
                }),
                Some(executor),
            ) => {
                if history.get(&mint).copied().unwrap_or(0) >= max_buys {
                    continue;
                }
                logging::info(&format!(
                    "SIGNAL  | {} | entry callout mcap ${mcap:.2} | callouts={callouts} | buyers={buyers} | buys={:.1}% | netflow ${netflow:.2}",
                    logging::short(&mint),
                    ratio * 100.0
                ));
                // Record the attempt before placing the order, including uncertain outcomes.
                *history.entry(mint.clone()).or_default() += 1;
                let temporary = history_path.with_extension("tmp");
                std::fs::write(&temporary, serde_json::to_vec(&history)?)?;
                std::fs::rename(&temporary, &history_path)?;
                strategy.position = Some(executor.open(&mint, mcap).await?);
                buys_this_run += 1;
                opened_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
                deadline = Some(Instant::now() + Duration::from_secs(timeout_secs));
                logging::info(&format!(
                    "OPEN    | {} | entry callout mcap ${mcap:.2} | timeout {timeout_secs}s",
                    logging::short(&mint)
                ));
            }
            (Some(Action::Sell(reason)), Some(executor)) => {
                let position = strategy
                    .position
                    .as_ref()
                    .expect("sell requires a position");
                executor.close(position, reason).await?;
                if max_buys_per_run.is_some_and(|limit| buys_this_run >= limit) {
                    logging::info(&format!(
                        "Buy limit reached ({buys_this_run}); exiting after confirmed sale."
                    ));
                    return Ok(());
                }
                strategy.position = None;
                deadline = None;
                // Discard activity buffered while the order was in flight.
                strategy = Strategy::new(read_config()?);
                trades = Collector::connect("subscribe").await?;
                callouts = Collector::connect("subscribeCallouts").await?;
            }
            (Some(Action::Buy { mint, mcap, .. }), None) => {
                println!("candidate | {mint} | callout mcap ${mcap:.2}")
            }
            _ => {}
        }
    }
}

/// Read entry thresholds; ratios are fractions between zero and one.
fn read_config() -> Result<Config, Box<dyn Error>> {
    fn get<T: std::str::FromStr>(name: &str, default: &str) -> Result<T, Box<dyn Error>> {
        std::env::var(name)
            .unwrap_or_else(|_| default.into())
            .parse()
            .map_err(|_| format!("Invalid {name}").into())
    }
    let seconds: u64 = get("WINDOW_SECS", "30")?;
    let min_buyers = get("MIN_BUYERS", "5")?;
    let min_buy_ratio: f64 = get("MIN_BUY_RATIO", "0.6")?;
    let min_netflow: f64 = get("MIN_NETFLOW_USD", "100")?;
    let min_callouts = get("MIN_CALLOUTS", "1")?;
    if seconds == 0
        || min_buyers == 0
        || min_callouts == 0
        || !min_buy_ratio.is_finite()
        || !(0.0..=1.0).contains(&min_buy_ratio)
        || !min_netflow.is_finite()
        || min_netflow < 0.0
    {
        return Err("Window, buyers and callouts must be positive; ratio must be 0–1 and netflow nonnegative".into());
    }
    Ok(Config {
        window: Duration::from_secs(seconds),
        min_buyers,
        min_buy_ratio,
        min_netflow,
        min_callouts,
    })
}
