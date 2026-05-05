//! Deterministic spot backtesting for the Intents Trading Agent.
//!
//! This is intentionally small and pure: no exchange adapters, no
//! external market data fetches, and no live orders. The skill passes
//! candle data in, this module evaluates long-only strategy rules, and
//! returns metrics plus a trade log. Execution uses the next candle's
//! open after a signal is formed on a closed candle, which keeps replay
//! tests honest and avoids lookahead by construction.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct BacktestInput {
    pub candles: Vec<Candle>,
    pub strategy: StrategyConfig,
    #[serde(default = "default_initial_cash_usd")]
    pub initial_cash_usd: f64,
    #[serde(default = "default_fee_bps")]
    pub fee_bps: f64,
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: f64,
    #[serde(default = "default_max_position_pct")]
    pub max_position_pct: f64,
    #[serde(default)]
    pub stop_loss_bps: Option<f64>,
    #[serde(default)]
    pub take_profit_bps: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct BacktestSuiteInput {
    pub candles: Vec<Candle>,
    pub candidates: Vec<BacktestCandidate>,
    #[serde(default = "default_initial_cash_usd")]
    pub initial_cash_usd: f64,
    #[serde(default = "default_fee_bps")]
    pub fee_bps: f64,
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: f64,
}

#[derive(Debug, Deserialize)]
pub struct BacktestCandidate {
    pub id: String,
    pub strategy: StrategyConfig,
    #[serde(default)]
    pub max_position_pct: Option<f64>,
    #[serde(default)]
    pub stop_loss_bps: Option<f64>,
    #[serde(default)]
    pub take_profit_bps: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Candle {
    pub ts: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    #[serde(default)]
    pub volume: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StrategyConfig {
    /// Supported values: "buy-hold", "sma-cross", "breakout",
    /// "mean-reversion", "momentum", "rsi-mean-reversion".
    pub kind: String,
    #[serde(default)]
    pub fast_window: Option<usize>,
    #[serde(default)]
    pub slow_window: Option<usize>,
    #[serde(default)]
    pub lookback_window: Option<usize>,
    #[serde(default)]
    pub threshold_bps: Option<f64>,
    #[serde(default)]
    pub entry_threshold: Option<f64>,
    #[serde(default)]
    pub exit_threshold: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct BacktestOutput {
    pub schema_version: &'static str,
    pub strategy: StrategySummary,
    pub metrics: BacktestMetrics,
    pub trades: Vec<BacktestTrade>,
    pub equity_curve: Vec<EquityPoint>,
    pub warnings: Vec<String>,
    pub lookahead_safe: bool,
}

#[derive(Debug, Serialize)]
pub struct BacktestSuiteOutput {
    pub schema_version: &'static str,
    pub ranked: Vec<BacktestSuiteResult>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BacktestSuiteResult {
    pub rank: usize,
    pub id: String,
    pub strategy: StrategySummary,
    pub selection_score: f64,
    pub passes_basic_gate: bool,
    pub metrics: BacktestMetrics,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct StrategySummary {
    pub kind: String,
    pub fast_window: Option<usize>,
    pub slow_window: Option<usize>,
    pub lookback_window: Option<usize>,
    pub threshold_bps: Option<f64>,
    pub entry_threshold: Option<f64>,
    pub exit_threshold: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct BacktestMetrics {
    pub candles: usize,
    pub trades: usize,
    pub start_equity_usd: String,
    pub end_equity_usd: String,
    pub total_return_pct: f64,
    pub buy_hold_return_pct: f64,
    pub alpha_vs_buy_hold_pct: f64,
    pub max_drawdown_pct: f64,
    pub win_rate_pct: f64,
    pub exposure_pct: f64,
    pub profit_factor: f64,
    pub average_trade_return_pct: f64,
    pub return_stability: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestTrade {
    pub entry_index: usize,
    pub exit_index: usize,
    pub entry_ts: String,
    pub exit_ts: String,
    pub entry_price: String,
    pub exit_price: String,
    pub units: String,
    pub gross_pnl_usd: String,
    pub net_pnl_usd: String,
    pub return_pct: f64,
    pub exit_reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquityPoint {
    pub index: usize,
    pub ts: String,
    pub equity_usd: String,
    pub in_position: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    Enter,
    Exit,
    Hold,
}

#[derive(Debug, Clone)]
struct OpenTrade {
    entry_index: usize,
    entry_ts: String,
    entry_price: f64,
    units: f64,
    entry_notional: f64,
    entry_fee: f64,
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

fn default_max_position_pct() -> f64 {
    1.0
}

pub fn run(input: BacktestInput) -> Result<BacktestOutput, String> {
    validate_input(&input)?;

    let mut warnings = vec![
        "Backtest is long-only spot and does not model funding, borrow, MEV, partial fills, solver expiry, or route refusal.".to_string(),
        "Signals are generated on closed candles and executed at the next candle open to avoid lookahead.".to_string(),
    ];

    if input.candles.len() < 30 {
        warnings.push("Small sample: fewer than 30 candles; metrics are fragile.".to_string());
    }

    let fee_rate = input.fee_bps / 10_000.0;
    let slip_rate = input.slippage_bps / 10_000.0;
    let max_position_pct = input.max_position_pct.clamp(0.0, 1.0);

    let mut cash = input.initial_cash_usd;
    let mut open: Option<OpenTrade> = None;
    let mut trades = Vec::new();
    let mut equity_curve = Vec::with_capacity(input.candles.len());
    let mut periodic_returns = Vec::new();
    let mut prior_equity: Option<f64> = None;
    let mut exposure_count = 0usize;
    let mut pending = if input.strategy.kind == "buy-hold" {
        Signal::Enter
    } else {
        Signal::Hold
    };

    for idx in 0..input.candles.len() {
        let candle = &input.candles[idx];

        match pending {
            Signal::Enter if open.is_none() => {
                open = enter_trade(
                    idx,
                    candle,
                    &mut cash,
                    max_position_pct,
                    fee_rate,
                    slip_rate,
                );
            }
            Signal::Exit if open.is_some() => {
                exit_trade(
                    idx,
                    candle,
                    candle.open * (1.0 - slip_rate),
                    "signal",
                    fee_rate,
                    &mut cash,
                    &mut open,
                    &mut trades,
                );
            }
            _ => {}
        }
        pending = Signal::Hold;

        if let Some(active) = &open {
            if let Some(stop_bps) = input.stop_loss_bps {
                let stop_price = active.entry_price * (1.0 - stop_bps / 10_000.0);
                if candle.low <= stop_price {
                    exit_trade(
                        idx,
                        candle,
                        stop_price * (1.0 - slip_rate),
                        "stop-loss",
                        fee_rate,
                        &mut cash,
                        &mut open,
                        &mut trades,
                    );
                }
            }
        }

        if let Some(active) = &open {
            if let Some(tp_bps) = input.take_profit_bps {
                let take_profit_price = active.entry_price * (1.0 + tp_bps / 10_000.0);
                if candle.high >= take_profit_price {
                    exit_trade(
                        idx,
                        candle,
                        take_profit_price * (1.0 - slip_rate),
                        "take-profit",
                        fee_rate,
                        &mut cash,
                        &mut open,
                        &mut trades,
                    );
                }
            }
        }

        let equity = mark_equity(cash, open.as_ref(), candle.close);
        if let Some(prev) = prior_equity {
            if prev > 0.0 {
                periodic_returns.push((equity / prev) - 1.0);
            }
        }
        prior_equity = Some(equity);
        if open.is_some() {
            exposure_count += 1;
        }
        equity_curve.push(EquityPoint {
            index: idx,
            ts: candle.ts.clone(),
            equity_usd: format!("{equity:.2}"),
            in_position: open.is_some(),
        });

        if idx + 1 < input.candles.len() {
            pending = signal_at(&input.candles, idx, &input.strategy, open.is_some())?;
        }
    }

    if let Some(last) = input.candles.last() {
        if open.is_some() {
            let idx = input.candles.len() - 1;
            exit_trade(
                idx,
                last,
                last.close * (1.0 - slip_rate),
                "end-of-test",
                fee_rate,
                &mut cash,
                &mut open,
                &mut trades,
            );
            if let Some(point) = equity_curve.last_mut() {
                point.equity_usd = format!("{cash:.2}");
                point.in_position = false;
            }
        }
    }

    if trades.is_empty() {
        warnings.push("No completed trades; inspect strategy windows and thresholds.".to_string());
    }

    let metrics = compute_metrics(
        input.initial_cash_usd,
        cash,
        &input.candles,
        &trades,
        &equity_curve,
        &periodic_returns,
        exposure_count,
        slip_rate,
    );

    Ok(BacktestOutput {
        schema_version: "intents-backtest/1",
        strategy: StrategySummary {
            kind: input.strategy.kind,
            fast_window: input.strategy.fast_window,
            slow_window: input.strategy.slow_window,
            lookback_window: input.strategy.lookback_window,
            threshold_bps: input.strategy.threshold_bps,
            entry_threshold: input.strategy.entry_threshold,
            exit_threshold: input.strategy.exit_threshold,
        },
        metrics,
        trades,
        equity_curve,
        warnings,
        lookahead_safe: true,
    })
}

pub fn run_suite(input: BacktestSuiteInput) -> Result<BacktestSuiteOutput, String> {
    validate_suite_input(&input)?;

    let mut ranked = Vec::with_capacity(input.candidates.len());
    for candidate in input.candidates {
        let output = run(BacktestInput {
            candles: input.candles.clone(),
            strategy: candidate.strategy,
            initial_cash_usd: input.initial_cash_usd,
            fee_bps: input.fee_bps,
            slippage_bps: input.slippage_bps,
            max_position_pct: candidate
                .max_position_pct
                .unwrap_or_else(default_max_position_pct),
            stop_loss_bps: candidate.stop_loss_bps,
            take_profit_bps: candidate.take_profit_bps,
        })?;

        let selection_score = selection_score(&output.metrics);
        let passes_basic_gate = passes_basic_gate(&output.metrics);

        ranked.push(BacktestSuiteResult {
            rank: 0,
            id: candidate.id,
            strategy: output.strategy,
            selection_score,
            passes_basic_gate,
            metrics: output.metrics,
            warnings: output.warnings,
        });
    }

    ranked.sort_by(|a, b| {
        b.selection_score
            .partial_cmp(&a.selection_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    for (idx, result) in ranked.iter_mut().enumerate() {
        result.rank = idx + 1;
    }

    Ok(BacktestSuiteOutput {
        schema_version: "intents-backtest-suite/1",
        ranked,
        warnings: vec![
            "Selection score is a deterministic paper-trading heuristic, not expected return."
                .to_string(),
            "Use out-of-sample, walk-forward, market-data replay, and solver-route replay before live wallet signing."
                .to_string(),
        ],
    })
}

fn validate_suite_input(input: &BacktestSuiteInput) -> Result<(), String> {
    if input.candidates.is_empty() {
        return Err("backtest_suite requires at least one candidate".to_string());
    }
    if !input.initial_cash_usd.is_finite() || input.initial_cash_usd <= 0.0 {
        return Err("initial_cash_usd must be > 0".to_string());
    }
    if !input.fee_bps.is_finite() || input.fee_bps < 0.0 {
        return Err("fee_bps must be >= 0".to_string());
    }
    if !input.slippage_bps.is_finite() || input.slippage_bps < 0.0 {
        return Err("slippage_bps must be >= 0".to_string());
    }

    let mut ids = std::collections::BTreeSet::new();
    for candidate in &input.candidates {
        if candidate.id.trim().is_empty() {
            return Err("backtest_suite candidate id must be non-empty".to_string());
        }
        if !ids.insert(candidate.id.clone()) {
            return Err(format!(
                "backtest_suite candidate id '{}' is duplicated",
                candidate.id
            ));
        }
        let max_position_pct = candidate
            .max_position_pct
            .unwrap_or_else(default_max_position_pct);
        validate_input(&BacktestInput {
            candles: input.candles.clone(),
            strategy: candidate.strategy.clone(),
            initial_cash_usd: input.initial_cash_usd,
            fee_bps: input.fee_bps,
            slippage_bps: input.slippage_bps,
            max_position_pct,
            stop_loss_bps: candidate.stop_loss_bps,
            take_profit_bps: candidate.take_profit_bps,
        })?;
    }

    Ok(())
}

fn selection_score(metrics: &BacktestMetrics) -> f64 {
    let trade_penalty = if metrics.trades == 0 { 25.0 } else { 0.0 };
    let drawdown_penalty = metrics.max_drawdown_pct * 0.75;
    let exposure_penalty = (metrics.exposure_pct - 85.0).max(0.0) * 0.05;
    let profit_factor_bonus = metrics.profit_factor.clamp(0.0, 5.0) * 2.0;
    let stability_bonus = metrics.return_stability.clamp(-5.0, 5.0) * 2.0;
    let score = metrics.total_return_pct + metrics.alpha_vs_buy_hold_pct
        - drawdown_penalty
        - exposure_penalty
        - trade_penalty
        + (metrics.win_rate_pct * 0.05)
        + profit_factor_bonus
        + stability_bonus;

    if score.is_finite() {
        score
    } else {
        -1_000_000.0
    }
}

fn passes_basic_gate(metrics: &BacktestMetrics) -> bool {
    metrics.trades > 0
        && metrics.total_return_pct > 0.0
        && metrics.alpha_vs_buy_hold_pct >= -5.0
        && metrics.max_drawdown_pct <= 35.0
        && metrics.profit_factor >= 1.0
}

fn validate_input(input: &BacktestInput) -> Result<(), String> {
    if input.candles.len() < 2 {
        return Err("backtest requires at least 2 candles".to_string());
    }
    if !input.initial_cash_usd.is_finite() || input.initial_cash_usd <= 0.0 {
        return Err("initial_cash_usd must be > 0".to_string());
    }
    if !input.fee_bps.is_finite() || input.fee_bps < 0.0 {
        return Err("fee_bps must be >= 0".to_string());
    }
    if !input.slippage_bps.is_finite() || input.slippage_bps < 0.0 {
        return Err("slippage_bps must be >= 0".to_string());
    }
    if !input.max_position_pct.is_finite()
        || input.max_position_pct <= 0.0
        || input.max_position_pct > 1.0
    {
        return Err("max_position_pct must be in (0, 1]".to_string());
    }
    for (idx, c) in input.candles.iter().enumerate() {
        for (name, value) in [
            ("open", c.open),
            ("high", c.high),
            ("low", c.low),
            ("close", c.close),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("candle {idx} {name} must be > 0"));
            }
        }
        if c.high < c.low || c.high < c.open || c.high < c.close {
            return Err(format!("candle {idx} high is inconsistent"));
        }
        if c.low > c.open || c.low > c.close {
            return Err(format!("candle {idx} low is inconsistent"));
        }
    }

    match input.strategy.kind.as_str() {
        "buy-hold" => {}
        "sma-cross" => {
            let fast = input.strategy.fast_window.unwrap_or(5);
            let slow = input.strategy.slow_window.unwrap_or(20);
            if fast == 0 || slow == 0 || fast >= slow {
                return Err("sma-cross requires 0 < fast_window < slow_window".to_string());
            }
        }
        "breakout" => {
            let lookback = input.strategy.lookback_window.unwrap_or(20);
            if lookback == 0 {
                return Err("breakout requires lookback_window > 0".to_string());
            }
        }
        "mean-reversion" => {
            let lookback = input.strategy.lookback_window.unwrap_or(20);
            if lookback == 0 {
                return Err("mean-reversion requires lookback_window > 0".to_string());
            }
        }
        "momentum" => {
            let lookback = input.strategy.lookback_window.unwrap_or(20);
            if lookback == 0 {
                return Err("momentum requires lookback_window > 0".to_string());
            }
        }
        "rsi-mean-reversion" => {
            let lookback = input.strategy.lookback_window.unwrap_or(14);
            if lookback == 0 {
                return Err("rsi-mean-reversion requires lookback_window > 0".to_string());
            }
        }
        other => return Err(format!("unknown backtest strategy kind: {other}")),
    }

    Ok(())
}

fn enter_trade(
    idx: usize,
    candle: &Candle,
    cash: &mut f64,
    max_position_pct: f64,
    fee_rate: f64,
    slip_rate: f64,
) -> Option<OpenTrade> {
    if *cash <= 0.0 {
        return None;
    }
    let execution_price = candle.open * (1.0 + slip_rate);
    let notional = (*cash * max_position_pct) / (1.0 + fee_rate);
    if notional <= 0.0 {
        return None;
    }
    let fee = notional * fee_rate;
    let units = notional / execution_price;
    *cash -= notional + fee;
    Some(OpenTrade {
        entry_index: idx,
        entry_ts: candle.ts.clone(),
        entry_price: execution_price,
        units,
        entry_notional: notional,
        entry_fee: fee,
    })
}

#[allow(clippy::too_many_arguments)]
fn exit_trade(
    idx: usize,
    candle: &Candle,
    execution_price: f64,
    reason: &str,
    fee_rate: f64,
    cash: &mut f64,
    open: &mut Option<OpenTrade>,
    trades: &mut Vec<BacktestTrade>,
) {
    let Some(active) = open.take() else {
        return;
    };
    let exit_notional = active.units * execution_price;
    let exit_fee = exit_notional * fee_rate;
    *cash += exit_notional - exit_fee;

    let gross_pnl = exit_notional - active.entry_notional;
    let net_pnl = gross_pnl - active.entry_fee - exit_fee;
    let denom = active.entry_notional + active.entry_fee;
    let return_pct = if denom > 0.0 {
        (net_pnl / denom) * 100.0
    } else {
        0.0
    };

    trades.push(BacktestTrade {
        entry_index: active.entry_index,
        exit_index: idx,
        entry_ts: active.entry_ts,
        exit_ts: candle.ts.clone(),
        entry_price: format!("{:.8}", active.entry_price),
        exit_price: format!("{execution_price:.8}"),
        units: format!("{:.8}", active.units),
        gross_pnl_usd: format!("{gross_pnl:.2}"),
        net_pnl_usd: format!("{net_pnl:.2}"),
        return_pct,
        exit_reason: reason.to_string(),
    });
}

fn mark_equity(cash: f64, open: Option<&OpenTrade>, close: f64) -> f64 {
    cash + open.map(|t| t.units * close).unwrap_or(0.0)
}

fn signal_at(
    candles: &[Candle],
    idx: usize,
    strategy: &StrategyConfig,
    in_position: bool,
) -> Result<Signal, String> {
    match strategy.kind.as_str() {
        "buy-hold" => Ok(Signal::Hold),
        "sma-cross" => sma_cross_signal(candles, idx, strategy, in_position),
        "breakout" => breakout_signal(candles, idx, strategy, in_position),
        "mean-reversion" => mean_reversion_signal(candles, idx, strategy, in_position),
        "momentum" => momentum_signal(candles, idx, strategy, in_position),
        "rsi-mean-reversion" => rsi_signal(candles, idx, strategy, in_position),
        other => Err(format!("unknown backtest strategy kind: {other}")),
    }
}

fn sma_cross_signal(
    candles: &[Candle],
    idx: usize,
    strategy: &StrategyConfig,
    in_position: bool,
) -> Result<Signal, String> {
    let fast = strategy.fast_window.unwrap_or(5);
    let slow = strategy.slow_window.unwrap_or(20);
    if idx < slow {
        return Ok(Signal::Hold);
    }
    let Some(fast_prev) = sma(candles, idx - 1, fast) else {
        return Ok(Signal::Hold);
    };
    let Some(slow_prev) = sma(candles, idx - 1, slow) else {
        return Ok(Signal::Hold);
    };
    let Some(fast_now) = sma(candles, idx, fast) else {
        return Ok(Signal::Hold);
    };
    let Some(slow_now) = sma(candles, idx, slow) else {
        return Ok(Signal::Hold);
    };

    if !in_position && fast_prev <= slow_prev && fast_now > slow_now {
        Ok(Signal::Enter)
    } else if in_position && fast_prev >= slow_prev && fast_now < slow_now {
        Ok(Signal::Exit)
    } else {
        Ok(Signal::Hold)
    }
}

fn breakout_signal(
    candles: &[Candle],
    idx: usize,
    strategy: &StrategyConfig,
    in_position: bool,
) -> Result<Signal, String> {
    let lookback = strategy.lookback_window.unwrap_or(20);
    if idx < lookback {
        return Ok(Signal::Hold);
    }
    let threshold = strategy.threshold_bps.unwrap_or(0.0) / 10_000.0;
    let window = &candles[idx - lookback..idx];
    let prior_high = window
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let prior_low = window.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let close = candles[idx].close;

    if !in_position && close > prior_high * (1.0 + threshold) {
        Ok(Signal::Enter)
    } else if in_position && close < prior_low * (1.0 - threshold) {
        Ok(Signal::Exit)
    } else {
        Ok(Signal::Hold)
    }
}

fn mean_reversion_signal(
    candles: &[Candle],
    idx: usize,
    strategy: &StrategyConfig,
    in_position: bool,
) -> Result<Signal, String> {
    let lookback = strategy.lookback_window.unwrap_or(20);
    let threshold = strategy.threshold_bps.unwrap_or(200.0) / 10_000.0;
    let Some(avg) = sma(candles, idx, lookback) else {
        return Ok(Signal::Hold);
    };
    let close = candles[idx].close;
    if !in_position && close <= avg * (1.0 - threshold) {
        Ok(Signal::Enter)
    } else if in_position && close >= avg {
        Ok(Signal::Exit)
    } else {
        Ok(Signal::Hold)
    }
}

fn momentum_signal(
    candles: &[Candle],
    idx: usize,
    strategy: &StrategyConfig,
    in_position: bool,
) -> Result<Signal, String> {
    let lookback = strategy.lookback_window.unwrap_or(20);
    if idx < lookback {
        return Ok(Signal::Hold);
    }
    let threshold = strategy.threshold_bps.unwrap_or(200.0) / 10_000.0;
    let prior = candles[idx - lookback].close;
    let momentum = (candles[idx].close / prior) - 1.0;
    if !in_position && momentum >= threshold {
        Ok(Signal::Enter)
    } else if in_position && momentum <= 0.0 {
        Ok(Signal::Exit)
    } else {
        Ok(Signal::Hold)
    }
}

fn rsi_signal(
    candles: &[Candle],
    idx: usize,
    strategy: &StrategyConfig,
    in_position: bool,
) -> Result<Signal, String> {
    let lookback = strategy.lookback_window.unwrap_or(14);
    let Some(rsi) = rsi(candles, idx, lookback) else {
        return Ok(Signal::Hold);
    };
    let entry = strategy.entry_threshold.unwrap_or(30.0);
    let exit = strategy.exit_threshold.unwrap_or(50.0);
    if !in_position && rsi <= entry {
        Ok(Signal::Enter)
    } else if in_position && rsi >= exit {
        Ok(Signal::Exit)
    } else {
        Ok(Signal::Hold)
    }
}

fn sma(candles: &[Candle], end_idx: usize, window: usize) -> Option<f64> {
    if window == 0 || end_idx + 1 < window {
        return None;
    }
    let start = end_idx + 1 - window;
    let sum: f64 = candles[start..=end_idx].iter().map(|c| c.close).sum();
    Some(sum / window as f64)
}

fn rsi(candles: &[Candle], end_idx: usize, window: usize) -> Option<f64> {
    if window == 0 || end_idx < window {
        return None;
    }
    let start = end_idx + 1 - window;
    let mut gains = 0.0;
    let mut losses = 0.0;
    for i in start..=end_idx {
        let delta = candles[i].close - candles[i - 1].close;
        if delta >= 0.0 {
            gains += delta;
        } else {
            losses += delta.abs();
        }
    }
    if losses == 0.0 {
        Some(100.0)
    } else {
        let rs = gains / losses;
        Some(100.0 - (100.0 / (1.0 + rs)))
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_metrics(
    initial_cash: f64,
    end_cash: f64,
    candles: &[Candle],
    trades: &[BacktestTrade],
    equity_curve: &[EquityPoint],
    periodic_returns: &[f64],
    exposure_count: usize,
    slip_rate: f64,
) -> BacktestMetrics {
    let total_return_pct = pct_return(initial_cash, end_cash);
    let first_buy = candles
        .first()
        .map(|c| c.open * (1.0 + slip_rate))
        .unwrap_or(0.0);
    let last_sell = candles
        .last()
        .map(|c| c.close * (1.0 - slip_rate))
        .unwrap_or(0.0);
    let buy_hold_return_pct = pct_return(first_buy, last_sell);

    let wins = trades.iter().filter(|t| t.return_pct > 0.0).count();
    let win_rate_pct = if trades.is_empty() {
        0.0
    } else {
        wins as f64 / trades.len() as f64 * 100.0
    };
    let average_trade_return_pct = if trades.is_empty() {
        0.0
    } else {
        trades.iter().map(|t| t.return_pct).sum::<f64>() / trades.len() as f64
    };

    let mut gross_wins = 0.0;
    let mut gross_losses = 0.0;
    for t in trades {
        let pnl = parse_money(&t.net_pnl_usd);
        if pnl >= 0.0 {
            gross_wins += pnl;
        } else {
            gross_losses += pnl.abs();
        }
    }
    let profit_factor = if gross_losses > 0.0 {
        gross_wins / gross_losses
    } else if gross_wins > 0.0 {
        gross_wins
    } else {
        0.0
    };

    BacktestMetrics {
        candles: candles.len(),
        trades: trades.len(),
        start_equity_usd: format!("{initial_cash:.2}"),
        end_equity_usd: format!("{end_cash:.2}"),
        total_return_pct,
        buy_hold_return_pct,
        alpha_vs_buy_hold_pct: total_return_pct - buy_hold_return_pct,
        max_drawdown_pct: max_drawdown_pct(equity_curve),
        win_rate_pct,
        exposure_pct: exposure_count as f64 / candles.len() as f64 * 100.0,
        profit_factor,
        average_trade_return_pct,
        return_stability: return_stability(periodic_returns),
    }
}

fn pct_return(start: f64, end: f64) -> f64 {
    if start > 0.0 {
        ((end / start) - 1.0) * 100.0
    } else {
        0.0
    }
}

fn max_drawdown_pct(equity_curve: &[EquityPoint]) -> f64 {
    let mut peak = 0.0;
    let mut max_dd = 0.0;
    for point in equity_curve {
        let equity = parse_money(&point.equity_usd);
        if equity > peak {
            peak = equity;
        }
        if peak > 0.0 {
            let dd = (peak - equity) / peak * 100.0;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd
}

fn return_stability(returns: &[f64]) -> f64 {
    if returns.len() < 2 {
        return 0.0;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance =
        returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
    let stddev = variance.sqrt();
    if stddev > 0.0 {
        mean / stddev
    } else {
        0.0
    }
}

fn parse_money(s: &str) -> f64 {
    s.parse::<f64>().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(idx: usize, close: f64) -> Candle {
        Candle {
            ts: format!("2026-05-{idx:02}T00:00:00Z"),
            open: close,
            high: close * 1.01,
            low: close * 0.99,
            close,
            volume: 1_000.0,
        }
    }

    fn run_with(kind: &str, closes: &[f64], strategy: StrategyConfig) -> BacktestOutput {
        let candles = closes
            .iter()
            .enumerate()
            .map(|(idx, close)| candle(idx + 1, *close))
            .collect();
        run(BacktestInput {
            candles,
            strategy: StrategyConfig {
                kind: kind.to_string(),
                ..strategy
            },
            initial_cash_usd: 1_000.0,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            max_position_pct: 1.0,
            stop_loss_bps: None,
            take_profit_bps: None,
        })
        .unwrap()
    }

    #[test]
    fn buy_hold_enters_and_exits() {
        let out = run_with(
            "buy-hold",
            &[100.0, 110.0, 120.0],
            StrategyConfig {
                kind: "buy-hold".to_string(),
                fast_window: None,
                slow_window: None,
                lookback_window: None,
                threshold_bps: None,
                entry_threshold: None,
                exit_threshold: None,
            },
        );
        assert_eq!(out.schema_version, "intents-backtest/1");
        assert_eq!(out.metrics.trades, 1);
        assert!(out.metrics.total_return_pct > 19.0);
        assert!(out.lookahead_safe);
    }

    #[test]
    fn suite_ranks_candidate_menu() {
        let closes = [
            10.0, 10.0, 10.0, 10.0, 10.5, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 16.0, 15.0,
            14.0, 13.0, 12.0,
        ];
        let candles = closes
            .iter()
            .enumerate()
            .map(|(idx, close)| candle(idx + 1, *close))
            .collect();

        let out = run_suite(BacktestSuiteInput {
            candles,
            candidates: vec![
                BacktestCandidate {
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
                },
                BacktestCandidate {
                    id: "sma".to_string(),
                    strategy: StrategyConfig {
                        kind: "sma-cross".to_string(),
                        fast_window: Some(2),
                        slow_window: Some(4),
                        lookback_window: None,
                        threshold_bps: None,
                        entry_threshold: None,
                        exit_threshold: None,
                    },
                    max_position_pct: Some(1.0),
                    stop_loss_bps: Some(1200.0),
                    take_profit_bps: None,
                },
            ],
            initial_cash_usd: 1_000.0,
            fee_bps: 0.0,
            slippage_bps: 0.0,
        })
        .unwrap();

        assert_eq!(out.schema_version, "intents-backtest-suite/1");
        assert_eq!(out.ranked.len(), 2);
        assert_eq!(out.ranked[0].rank, 1);
        assert!(out.ranked.iter().any(|result| result.passes_basic_gate));
    }

    #[test]
    fn sma_cross_produces_a_trade() {
        let closes = [
            10.0, 10.0, 10.0, 10.0, 10.0, 10.5, 11.0, 12.0, 13.0, 14.0, 13.0, 12.0, 11.0, 10.0, 9.0,
        ];
        let out = run_with(
            "sma-cross",
            &closes,
            StrategyConfig {
                kind: "sma-cross".to_string(),
                fast_window: Some(2),
                slow_window: Some(4),
                lookback_window: None,
                threshold_bps: None,
                entry_threshold: None,
                exit_threshold: None,
            },
        );
        assert!(out.metrics.trades >= 1);
    }

    #[test]
    fn breakout_produces_a_trade() {
        let closes = [10.0, 10.1, 10.0, 10.2, 10.1, 11.0, 11.5, 12.0, 12.5, 13.0];
        let out = run_with(
            "breakout",
            &closes,
            StrategyConfig {
                kind: "breakout".to_string(),
                fast_window: None,
                slow_window: None,
                lookback_window: Some(3),
                threshold_bps: Some(25.0),
                entry_threshold: None,
                exit_threshold: None,
            },
        );
        assert!(out.metrics.trades >= 1);
    }

    #[test]
    fn mean_reversion_produces_a_trade() {
        let closes = [100.0, 100.0, 100.0, 94.0, 93.0, 95.0, 98.0, 101.0, 100.0];
        let out = run_with(
            "mean-reversion",
            &closes,
            StrategyConfig {
                kind: "mean-reversion".to_string(),
                fast_window: None,
                slow_window: None,
                lookback_window: Some(3),
                threshold_bps: Some(300.0),
                entry_threshold: None,
                exit_threshold: None,
            },
        );
        assert!(out.metrics.trades >= 1);
    }

    #[test]
    fn momentum_produces_a_trade() {
        let closes = [10.0, 10.2, 10.4, 10.6, 11.2, 11.8, 12.4, 12.0, 11.5];
        let out = run_with(
            "momentum",
            &closes,
            StrategyConfig {
                kind: "momentum".to_string(),
                fast_window: None,
                slow_window: None,
                lookback_window: Some(3),
                threshold_bps: Some(500.0),
                entry_threshold: None,
                exit_threshold: None,
            },
        );
        assert!(out.metrics.trades >= 1);
    }

    #[test]
    fn rsi_mean_reversion_produces_a_trade() {
        let closes = [
            100.0, 98.0, 96.0, 94.0, 92.0, 90.0, 91.0, 93.0, 95.0, 98.0, 101.0,
        ];
        let out = run_with(
            "rsi-mean-reversion",
            &closes,
            StrategyConfig {
                kind: "rsi-mean-reversion".to_string(),
                fast_window: None,
                slow_window: None,
                lookback_window: Some(5),
                threshold_bps: None,
                entry_threshold: Some(25.0),
                exit_threshold: Some(55.0),
            },
        );
        assert!(out.metrics.trades >= 1);
    }

    #[test]
    fn invalid_ohlc_errors() {
        let result = run(BacktestInput {
            candles: vec![
                Candle {
                    ts: "a".to_string(),
                    open: 10.0,
                    high: 9.0,
                    low: 8.0,
                    close: 10.0,
                    volume: 0.0,
                },
                candle(2, 10.0),
            ],
            strategy: StrategyConfig {
                kind: "buy-hold".to_string(),
                fast_window: None,
                slow_window: None,
                lookback_window: None,
                threshold_bps: None,
                entry_threshold: None,
                exit_threshold: None,
            },
            initial_cash_usd: 1_000.0,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            max_position_pct: 1.0,
            stop_loss_bps: None,
            take_profit_bps: None,
        });
        assert!(result.unwrap_err().contains("high"));
    }
}
