//! Shared episode format for the Intents Trading Agent.
//!
//! An episode bundles the three replay surfaces the agent currently
//! treats as separate inputs:
//!
//! - candle series for `backtest` / `backtest_suite`,
//! - solver fixture for `build_intent` (`solver: "fixture"`),
//! - news/catalyst snippets for the news/sentiment analyst.
//!
//! It is the substrate the research notes call out as missing
//! ("solver-route replay and market-data replay are separate; they
//! need a shared episode format"). Two actions are exposed:
//!
//! - `validate_episode`: structural checks plus a deterministic
//!   summary, no side effects.
//! - `replay_episode`: validate, then run a `backtest_suite` against
//!   the embedded candles. The solver fixture and news context are
//!   surfaced verbatim so the caller can route them into
//!   `build_intent` and the analyst memos.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backtest::{self, BacktestCandidate, BacktestSuiteInput, BacktestSuiteOutput, Candle};

#[derive(Debug, Deserialize, Clone)]
pub struct Episode {
    pub id: String,
    pub pair: String,
    #[serde(default)]
    pub timeframe: Option<String>,
    pub candles: Vec<Candle>,
    #[serde(default)]
    pub solver_fixture: Option<Value>,
    #[serde(default)]
    pub news_context: Vec<NewsItem>,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct NewsItem {
    pub ts: String,
    pub headline: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ValidateEpisodeInput {
    pub episode: Episode,
}

#[derive(Debug, Serialize)]
pub struct EpisodeSummary {
    pub schema_version: &'static str,
    pub id: String,
    pub pair: String,
    pub timeframe: Option<String>,
    pub generated_at: Option<String>,
    pub source: Option<String>,
    pub candles: usize,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
    pub min_close: f64,
    pub max_close: f64,
    pub buy_hold_return_pct: f64,
    pub has_solver_fixture: bool,
    pub solver_fixture_kind: Option<String>,
    pub news_items: usize,
    pub warnings: Vec<String>,
}

pub fn validate(input: ValidateEpisodeInput) -> Result<EpisodeSummary, String> {
    summarize(&input.episode)
}

#[derive(Debug, Deserialize)]
pub struct ReplayEpisodeInput {
    pub episode: Episode,
    pub candidates: Vec<BacktestCandidate>,
    #[serde(default = "default_initial_cash_usd")]
    pub initial_cash_usd: f64,
    #[serde(default = "default_fee_bps")]
    pub fee_bps: f64,
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: f64,
}

fn default_initial_cash_usd() -> f64 {
    10_000.0
}

fn default_fee_bps() -> f64 {
    10.0
}

fn default_slippage_bps() -> f64 {
    5.0
}

#[derive(Debug, Serialize)]
pub struct ReplayEpisodeOutput {
    pub schema_version: &'static str,
    pub episode: EpisodeSummary,
    pub backtest_suite: BacktestSuiteOutput,
    pub solver_fixture: Option<Value>,
    pub news_context: Vec<NewsItem>,
}

pub fn replay(input: ReplayEpisodeInput) -> Result<ReplayEpisodeOutput, String> {
    let summary = summarize(&input.episode)?;
    if input.candidates.is_empty() {
        return Err("replay_episode requires at least one candidate".to_string());
    }
    let suite = backtest::run_suite(BacktestSuiteInput {
        candles: input.episode.candles.clone(),
        candidates: input.candidates,
        initial_cash_usd: input.initial_cash_usd,
        fee_bps: input.fee_bps,
        slippage_bps: input.slippage_bps,
    })?;

    Ok(ReplayEpisodeOutput {
        schema_version: "intents-episode-replay/1",
        episode: summary,
        backtest_suite: suite,
        solver_fixture: input.episode.solver_fixture,
        news_context: input.episode.news_context,
    })
}

fn summarize(episode: &Episode) -> Result<EpisodeSummary, String> {
    if episode.id.trim().is_empty() {
        return Err("episode id must be non-empty".to_string());
    }
    if episode.pair.trim().is_empty() {
        return Err("episode pair must be non-empty".to_string());
    }
    if episode.candles.len() < 2 {
        return Err("episode requires at least 2 candles".to_string());
    }
    let mut warnings = Vec::new();
    let mut prior_ts: Option<&str> = None;
    let mut min_close = f64::INFINITY;
    let mut max_close = f64::NEG_INFINITY;
    for (idx, c) in episode.candles.iter().enumerate() {
        if !c.open.is_finite() || c.open <= 0.0 {
            return Err(format!("candle {idx} open invalid"));
        }
        if !c.high.is_finite() || c.high <= 0.0 {
            return Err(format!("candle {idx} high invalid"));
        }
        if !c.low.is_finite() || c.low <= 0.0 {
            return Err(format!("candle {idx} low invalid"));
        }
        if !c.close.is_finite() || c.close <= 0.0 {
            return Err(format!("candle {idx} close invalid"));
        }
        if c.high < c.low || c.high < c.open || c.high < c.close {
            return Err(format!("candle {idx} high inconsistent"));
        }
        if c.low > c.open || c.low > c.close {
            return Err(format!("candle {idx} low inconsistent"));
        }
        if let Some(prev) = prior_ts {
            if c.ts.as_str() < prev {
                return Err(format!(
                    "candles must be oldest-to-newest; candle {idx} ts '{}' precedes prior '{prev}'",
                    c.ts
                ));
            }
            if c.ts.as_str() == prev {
                warnings.push(format!("candle {idx} ts equals prior candle"));
            }
        }
        prior_ts = Some(&c.ts);
        if c.close < min_close {
            min_close = c.close;
        }
        if c.close > max_close {
            max_close = c.close;
        }
    }
    let first_ts = episode.candles.first().map(|c| c.ts.clone());
    let last_ts = episode.candles.last().map(|c| c.ts.clone());
    let bh_return =
        if let (Some(first), Some(last)) = (episode.candles.first(), episode.candles.last()) {
            (last.close / first.open - 1.0) * 100.0
        } else {
            0.0
        };

    let solver_fixture_kind = episode
        .solver_fixture
        .as_ref()
        .and_then(|v| v.get("kind"))
        .and_then(|k| k.as_str())
        .map(|s| s.to_string());

    if episode.candles.len() < 30 {
        warnings.push("small episode: fewer than 30 candles".to_string());
    }
    if episode.solver_fixture.is_none() {
        warnings
            .push("episode has no solver_fixture; intent build replay not available".to_string());
    }
    if episode.news_context.is_empty() {
        warnings.push("episode has no news_context; analyst memo replay not available".to_string());
    }

    Ok(EpisodeSummary {
        schema_version: "intents-episode/1",
        id: episode.id.clone(),
        pair: episode.pair.clone(),
        timeframe: episode.timeframe.clone(),
        generated_at: episode.generated_at.clone(),
        source: episode.source.clone(),
        candles: episode.candles.len(),
        first_ts,
        last_ts,
        min_close,
        max_close,
        buy_hold_return_pct: bh_return,
        has_solver_fixture: episode.solver_fixture.is_some(),
        solver_fixture_kind,
        news_items: episode.news_context.len(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::StrategyConfig;

    fn cs(closes: &[f64]) -> Vec<Candle> {
        closes
            .iter()
            .enumerate()
            .map(|(i, c)| Candle {
                ts: format!("2026-04-{:02}T00:00:00Z", i + 1),
                open: *c,
                high: c * 1.01,
                low: c * 0.99,
                close: *c,
                volume: 1_000.0,
            })
            .collect()
    }

    #[test]
    fn validate_episode_returns_summary() {
        let ep = Episode {
            id: "near-usdc-2026q1".to_string(),
            pair: "NEAR/USDC".to_string(),
            timeframe: Some("1d".to_string()),
            candles: cs(&[3.0, 3.1, 3.2, 3.4]),
            solver_fixture: None,
            news_context: vec![],
            generated_at: Some("2026-05-02".to_string()),
            source: Some("manual".to_string()),
            notes: None,
        };
        let summary = validate(ValidateEpisodeInput { episode: ep }).unwrap();
        assert_eq!(summary.candles, 4);
        assert_eq!(summary.pair, "NEAR/USDC");
        assert!(summary.buy_hold_return_pct > 12.0);
    }

    #[test]
    fn replay_runs_suite_over_episode() {
        let ep = Episode {
            id: "ep-1".to_string(),
            pair: "BTC/USDC".to_string(),
            timeframe: Some("1d".to_string()),
            candles: cs(&[
                100.0, 100.5, 101.0, 102.5, 103.0, 104.0, 105.0, 106.0, 107.0, 108.0,
            ]),
            solver_fixture: Some(serde_json::json!({"kind": "swap"})),
            news_context: vec![NewsItem {
                ts: "2026-04-02".to_string(),
                headline: "Test".to_string(),
                url: None,
                source: None,
                kind: None,
            }],
            generated_at: None,
            source: None,
            notes: None,
        };
        let out = replay(ReplayEpisodeInput {
            episode: ep,
            candidates: vec![BacktestCandidate {
                id: "buy_hold".to_string(),
                strategy: StrategyConfig {
                    kind: "buy-hold".to_string(),
                    fast_window: None,
                    slow_window: None,
                    lookback_window: None,
                    threshold_bps: None,
                    entry_threshold: None,
                    exit_threshold: None,
                },
                max_position_pct: None,
                stop_loss_bps: None,
                take_profit_bps: None,
            }],
            initial_cash_usd: 1_000.0,
            fee_bps: 0.0,
            slippage_bps: 0.0,
        })
        .unwrap();
        assert_eq!(out.episode.solver_fixture_kind.as_deref(), Some("swap"));
        assert_eq!(out.backtest_suite.ranked.len(), 1);
    }

    #[test]
    fn rejects_descending_timestamps() {
        let mut candles = cs(&[100.0, 101.0, 102.0]);
        candles[0].ts = "2026-05-10T00:00:00Z".to_string();
        candles[1].ts = "2026-05-01T00:00:00Z".to_string();
        let err = validate(ValidateEpisodeInput {
            episode: Episode {
                id: "x".to_string(),
                pair: "x/y".to_string(),
                timeframe: None,
                candles,
                solver_fixture: None,
                news_context: vec![],
                generated_at: None,
                source: None,
                notes: None,
            },
        })
        .unwrap_err();
        assert!(err.contains("oldest-to-newest"));
    }
}
