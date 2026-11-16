use crate::collector::{Callout, Trade};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};
use tokio::time::Instant;

pub struct Config {
    pub window: Duration,
    pub min_buyers: usize,
    pub min_buy_ratio: f64,
    pub min_netflow: f64,
    pub min_callouts: usize,
}

struct Event {
    at: Instant,
    trader: String,
    buy: bool,
    usd: f64,
}
#[derive(Default)]
struct Activity {
    trades: VecDeque<Event>,
    callouts: VecDeque<Instant>,
    mcap: Option<(Instant, f64)>,
}

pub struct Position {
    pub mint: String,
    pub entry_mcap: f64,
    pub starting_usdc: f64,
    pub opened_at: Instant,
    pub last_mcap: Option<(Instant, f64)>,
    pub amount: String,
}
pub enum Action {
    Buy {
        mint: String,
        mcap: f64,
        callouts: usize,
        buyers: usize,
        ratio: f64,
        netflow: f64,
    },
    Sell(&'static str),
}
pub struct Strategy {
    config: Config,
    activity: HashMap<String, Activity>,
    pub position: Option<Position>,
}

impl Strategy {
    /// Create a strategy that measures activity over a rolling window.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            activity: HashMap::new(),
            position: None,
        }
    }

    /// Record a callout and use its market cap for entry or exit decisions.
    pub fn on_callout(&mut self, callout: Callout, now: Instant) -> Option<Action> {
        let mcap = callout
            .call_at_mcap_usd
            .filter(|v| v.is_finite() && *v > 0.0);
        if let Some(position) = &mut self.position {
            if position.mint == callout.mint {
                let current = mcap?;
                position.last_mcap = Some((now, current));
                let change = current / position.entry_mcap - 1.0;

                if change >= 0.05 {
                    return Some(Action::Sell("take profit"));
                }
                if change <= -0.40 {
                    return Some(Action::Sell("stop loss"));
                }
            }
            return None;
        }
        self.prune(now);
        let mint = callout.mint;
        let activity = self.activity.entry(mint.clone()).or_default();
        activity.callouts.push_back(now);
        if let Some(mcap) = mcap {
            activity.mcap = Some((now, mcap));
        }
        self.candidate(&mint)
    }

    /// Record a priced trade, then check whether all entry gates are met.
    pub fn on_trade(&mut self, trade: Trade, now: Instant) -> Option<Action> {
        if self.position.is_some() {
            return None;
        }
        self.prune(now);
        let buy = match trade.side.as_str() {
            "buy" => true,
            "sell" => false,
            _ => return None,
        };
        let usd = trade.amount_usd.filter(|v| v.is_finite() && *v > 0.0)?;
        if trade.trader.is_empty() {
            return None;
        }
        let mint = if buy {
            trade.token_out.mint
        } else {
            trade.token_in.mint
        };
        if mint.is_empty() {
            return None;
        }
        self.activity
            .entry(mint.clone())
            .or_default()
            .trades
            .push_back(Event {
                at: now,
                trader: trade.trader,
                buy,
                usd,
            });
        self.candidate(&mint)
    }

    /// Expire old observations and remove inactive mints to bound retained state.
    fn prune(&mut self, now: Instant) {
        let window = self.config.window;
        self.activity.retain(|_, a| {
            while a
                .trades
                .front()
                .is_some_and(|e| now.duration_since(e.at) >= window)
            {
                a.trades.pop_front();
            }
            while a
                .callouts
                .front()
                .is_some_and(|t| now.duration_since(*t) >= window)
            {
                a.callouts.pop_front();
            }
            if a.mcap
                .is_some_and(|(at, _)| now.duration_since(at) >= window)
            {
                a.mcap = None;
            }
            !a.trades.is_empty() || !a.callouts.is_empty()
        });
    }

    /// Require unique buyers, buy transaction ratio, USD netflow, and callouts.
    fn candidate(&self, mint: &str) -> Option<Action> {
        let a = self.activity.get(mint)?;
        let buyers: HashSet<_> = a
            .trades
            .iter()
            .filter(|e| e.buy)
            .map(|e| &e.trader)
            .collect();
        let buys = a.trades.iter().filter(|e| e.buy).count();
        let ratio = buys as f64 / a.trades.len().max(1) as f64;
        let netflow: f64 = a
            .trades
            .iter()
            .map(|e| if e.buy { e.usd } else { -e.usd })
            .sum();
        if buyers.len() >= self.config.min_buyers
            && ratio >= self.config.min_buy_ratio
            && netflow >= self.config.min_netflow
            && a.callouts.len() >= self.config.min_callouts
        {
            let (_, mcap) = a.mcap?;
            return Some(Action::Buy {
                mint: mint.to_owned(),
                mcap,
                callouts: a.callouts.len(),
                buyers: buyers.len(),
                ratio,
                netflow,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::Token;

    fn trade(trader: &str, side: &str, usd: f64) -> Trade {
        Trade {
            trader: trader.into(),
            side: side.into(),
            token_in: Token {
                mint: if side == "sell" { "mint" } else { "usdc" }.into(),
            },
            token_out: Token {
                mint: if side == "buy" { "mint" } else { "usdc" }.into(),
            },
            amount_usd: Some(usd),
        }
    }
    fn callout(mcap: f64) -> Callout {
        Callout {
            mint: "mint".into(),
            created_at: Some(1),
            call_at_mcap_usd: Some(mcap),
        }
    }
    fn strategy() -> Strategy {
        Strategy::new(Config {
            window: Duration::from_secs(30),
            min_buyers: 2,
            min_buy_ratio: 0.6,
            min_netflow: 100.0,
            min_callouts: 1,
        })
    }

    #[test]
    fn entry_requires_all_gates_and_distinct_buyers() {
        let mut s = strategy();
        let now = Instant::now();
        assert!(s.on_callout(callout(1000.0), now).is_none());
        assert!(s.on_trade(trade("a", "buy", 80.0), now).is_none());
        assert!(s.on_trade(trade("a", "buy", 30.0), now).is_none());
        assert!(s.on_trade(trade("b", "sell", 20.0), now).is_none());
        assert!(matches!(
            s.on_trade(trade("b", "buy", 20.0), now),
            Some(Action::Buy { .. })
        ));
    }

    #[test]
    fn expired_observations_cannot_trigger_entry() {
        let mut s = strategy();
        let now = Instant::now();
        s.on_callout(callout(1000.0), now);
        s.on_trade(trade("a", "buy", 100.0), now);
        assert!(
            s.on_trade(trade("b", "buy", 100.0), now + Duration::from_secs(30))
                .is_none()
        );
    }

    #[test]
    fn exit_uses_matching_callout_market_cap() {
        let mut s = strategy();
        let now = Instant::now();
        s.position = Some(Position {
            mint: "mint".into(),
            entry_mcap: 1000.0,
            starting_usdc: 100.0,
            opened_at: now,
            last_mcap: None,
            amount: "1".into(),
        });
        assert!(matches!(
            s.on_callout(callout(1060.0), now),
            Some(Action::Sell("take profit"))
        ));
        assert!(matches!(
            s.on_callout(callout(590.0), now),
            Some(Action::Sell("stop loss"))
        ));
        assert!(s.on_callout(callout(900.0), now).is_none());
    }
}
