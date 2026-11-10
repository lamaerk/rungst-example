mod collector;
mod executor;
mod instructions;
mod strategies;

use std::error::Error;

use collector::Collector;
use solana_sdk::signer::Signer;
use strategies::{Action, Strategy};

/// Start the callout strategy in the selected run mode.
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (threshold, max_mcap) = read_strategy_limits()?;
    let (simulation, live) = read_run_mode()?;
    let wallet = executor::load_wallet()?;
    eprintln!("Wallet: {}", wallet.pubkey());
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "failed to install the Rustls crypto provider")?;

    let client = reqwest::Client::new();
    let mut strategy = Strategy::new(threshold, max_mcap);
    let mut collector = Collector::connect().await?;
    print_startup_message(simulation, live);

    run(
        &client,
        &wallet,
        &mut strategy,
        &mut collector,
        simulation,
        live,
    )
    .await
}

/// Read and validate the callout-count and market-cap limits.
fn read_strategy_limits() -> Result<(usize, f64), Box<dyn Error>> {
    let threshold: usize = std::env::var("THESIS_COUNT")
        .unwrap_or_else(|_| "3".into())
        .parse()?;
    let max_mcap: f64 = std::env::var("MAX_MCAP_USD")
        .unwrap_or_else(|_| "40000".into())
        .parse()?;
    if threshold == 0 || !max_mcap.is_finite() || max_mcap <= 0.0 {
        return Err("THESIS_COUNT and MAX_MCAP_USD must be positive".into());
    }

    Ok((threshold, max_mcap))
}

/// Parse the observation, simulation, or live-buy command-line mode.
fn read_run_mode() -> Result<(bool, bool), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let simulation = args.iter().any(|arg| arg == "--simulation");
    let live = args.iter().any(|arg| arg == "--buy");
    if (simulation && live)
        || args
            .iter()
            .any(|arg| arg != "--simulation" && arg != "--buy")
    {
        return Err("usage: thesis-momentum [--simulation | --buy]".into());
    }

    Ok((simulation, live))
}

/// Print the startup message for the selected run mode.
fn print_startup_message(simulation: bool, live: bool) {
    eprintln!(
        "{}",
        if simulation {
            "Simulation mode; waiting for a qualifying callout…"
        } else if live {
            "Live trading enabled; waiting for callouts…"
        } else {
            "Observation only: no trades will be placed. Waiting for callouts…"
        }
    );
}

/// Read callouts and timer events until the strategy finishes.
async fn run(
    client: &reqwest::Client,
    wallet: &solana_sdk::signature::Keypair,
    strategy: &mut Strategy,
    collector: &mut Collector,
    simulation: bool,
    live: bool,
) -> Result<(), Box<dyn Error>> {
    loop {
        let action = tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                if strategy.position.is_some() { eprintln!("Position remains open; manage it manually."); }
                return Ok(());
            },
            _ = strategy.wait_for_timeout() => Some(Action::Sell("timeout")),
            callout = collector.next() => match callout? {
                Some(callout) => strategy.on_callout(callout),
                None => {
                    if strategy.position.is_some() { return Err("stream disconnected with an open position; manage it manually".into()); }
                    return Ok(());
                }
            },
        };
        if handle_action(client, wallet, strategy, action, simulation, live).await? {
            return Ok(());
        }
    }
}

/// Run one strategy action. Return true when the program should exit.
async fn handle_action(
    client: &reqwest::Client,
    wallet: &solana_sdk::signature::Keypair,
    strategy: &mut Strategy,
    action: Option<Action>,
    simulation: bool,
    live: bool,
) -> Result<bool, Box<dyn Error>> {
    match action {
        Some(Action::Buy { mint, mcap }) => {
            println!("candidate | {mint} | callout mcap ${mcap:.2}");
            if simulation {
                executor::simulate_buy(client, wallet, &mint).await?;
                return Ok(true);
            }
            if live {
                strategy.position =
                    Some(executor::open_position(client, wallet, &mint, mcap).await?);
            }
            Ok(false)
        }
        Some(Action::Sell(reason)) => {
            let position = strategy.position.take().expect("sell requires a position");
            executor::close_position(client, wallet, position, reason).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}
