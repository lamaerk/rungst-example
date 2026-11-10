use std::{collections::HashMap, time::Duration};

use crate::collector::Callout;

pub struct Position {
    pub mint: String,
    pub amount: u64,
    pub entry_mcap: f64,
    pub starting_sol: u64,
    pub deadline: tokio::time::Instant,
}

pub enum Action {
    Buy { mint: String, mcap: f64 },
    Sell(&'static str),
}

pub struct Strategy {
    threshold: usize,
    max_mcap: f64,
    counts: HashMap<String, usize>,
    pub position: Option<Position>,
}

impl Strategy {
    /// Create an empty strategy with its entry thresholds.
    pub fn new(threshold: usize, max_mcap: f64) -> Self {
        Self {
            threshold,
            max_mcap,
            counts: HashMap::new(),
            position: None,
        }
    }

    /// Convert a callout into a buy, sell, or no-op decision.
    pub fn on_callout(&mut self, callout: Callout) -> Option<Action> {
        if self.position.is_some() {
            return self.exit_action(&callout);
        }

        self.entry_action(callout)
    }

    /// Apply position exit rules to a callout for the open mint.
    fn exit_action(&self, callout: &Callout) -> Option<Action> {
        let position = self.position.as_ref()?;
        if callout.mint != position.mint {
            return None;
        }

        let current = valid_market_cap(callout.call_at_mcap_usd)?;
        let change = current / position.entry_mcap - 1.0;
        println!(
            "{} | callout mcap ${current:.2} | market cap change {:+.1}%",
            position.mint,
            change * 100.0
        );

        // Exit thresholds are measured against the callout that triggered entry.
        if change >= 0.20 {
            Some(Action::Sell("take profit"))
        } else if change <= -0.60 {
            Some(Action::Sell("stop loss"))
        } else {
            None
        }
    }

    /// Apply entry rules and count callouts for an eligible mint.
    fn entry_action(&mut self, callout: Callout) -> Option<Action> {
        if !callout.mint.ends_with("pump") {
            return None;
        }

        let count = self.counts.entry(callout.mint.clone()).or_default();
        *count = count.saturating_add(1);
        if *count < self.threshold {
            return None;
        }

        // A token must pass both the recurrence and market-cap gates before entry.
        let mcap = valid_market_cap(callout.call_at_mcap_usd)?;
        if mcap > self.max_mcap {
            return None;
        }

        Some(Action::Buy {
            mint: callout.mint,
            mcap,
        })
    }

    /// Sleep until the open position's deadline, or wait forever without one.
    pub async fn wait_for_timeout(&self) {
        match &self.position {
            Some(position) => tokio::time::sleep_until(position.deadline).await,
            None => std::future::pending().await,
        }
    }
}

/// Keep only finite, positive market-cap values.
fn valid_market_cap(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

impl Position {
    /// Record an opened position and set its ten-minute deadline.
    pub fn new(mint: String, amount: u64, entry_mcap: f64, starting_sol: u64) -> Self {
        Self {
            mint,
            amount,
            entry_mcap,
            starting_sol,
            deadline: tokio::time::Instant::now() + Duration::from_secs(600),
        }
    }
}
