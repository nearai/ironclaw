//! Strategy lab primitives for the Intents Trading Agent.
//!
//! Wraps `backtest::run` and `backtest::run_suite` with three
//! deterministic research workflows:
//!
//! - `backtest_walkforward`: contiguous, non-overlapping fold windows;
//!   each fold splits its window into in-sample and out-of-sample
//!   ranges, runs the same strategy on both, and reports the IS/OOS
//!   gap. The strategy parameters are not refit — this is a regime
//!   robustness check, not a hyperparameter search.
//! - `backtest_montecarlo`: trade-order resampling on a caller-provided
//!   per-trade return series, producing return / drawdown / terminal
//!   equity distributions. Uses an xorshift64* PRNG seeded from the
//!   request so runs are reproducible.
//! - `backtest_grid`: cartesian sweep over strategy parameter axes
//!   plus optional sizing/stop/take-profit options. Materializes
//!   candidates, hands them to `backtest::run_suite`, and returns the
//!   ranked grid plus a flat cell table for heatmaps.
//!
//! All inputs are pure functions of caller-provided candles or trade
//! logs. No network, no exchange adapters, no signing, no randomness
//! that isn't seeded.

use serde::{Deserialize, Serialize};

use crate::backtest::{
    self, BacktestCandidate, BacktestInput, BacktestMetrics, BacktestSuiteInput,
    BacktestSuiteResult, Candle, StrategyConfig, StrategySummary,
};

// -------------------- shared defaults --------------------

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

// -------------------- walk-forward --------------------

#[derive(Debug, Deserialize)]
pub struct WalkForwardInput {
    pub candles: Vec<Candle>,
    pub strategy: StrategyConfig,
    /// Number of non-overlapping fold windows to split the candle
    /// series into.
    pub n_folds: usize,
    /// Fraction of each fold window used for the in-sample (training)
    /// run. The remaining tail (minus the embargo) is the
    /// out-of-sample (test) run.
    #[serde(default = "default_train_pct")]
    pub train_pct: f64,
    /// Number of candles to drop between train and test windows. Used
    /// to prevent leakage from indicator state.
    #[serde(default)]
    pub embargo: usize,
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

fn default_train_pct() -> f64 {
    0.7
}

#[derive(Debug, Serialize)]
pub struct WalkForwardOutput {
    pub schema_version: &'static str,
    pub strategy: StrategySummary,
    pub n_folds: usize,
    pub train_pct: f64,
    pub embargo: usize,
    pub folds: Vec<WalkForwardFold>,
    pub aggregate: WalkForwardAggregate,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct WalkForwardFold {
    pub index: usize,
    pub train_start_index: usize,
    pub train_end_index: usize,
    pub test_start_index: usize,
    pub test_end_index: usize,
    pub train_metrics: BacktestMetrics,
    pub test_metrics: BacktestMetrics,
    pub is_oos_gap_pct: f64,
    pub passes_test_gate: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct WalkForwardAggregate {
    pub mean_train_return_pct: f64,
    pub mean_test_return_pct: f64,
    pub mean_is_oos_gap_pct: f64,
    pub worst_test_drawdown_pct: f64,
    pub fold_pass_rate: f64,
    pub robustness: String,
}

pub fn run_walkforward(input: WalkForwardInput) -> Result<WalkForwardOutput, String> {
    if input.n_folds < 2 {
        return Err("walkforward requires n_folds >= 2".to_string());
    }
    if !input.train_pct.is_finite() || input.train_pct <= 0.0 || input.train_pct >= 1.0 {
        return Err("walkforward train_pct must be in (0, 1)".to_string());
    }
    if input.candles.len() < input.n_folds * 4 {
        return Err(format!(
            "walkforward requires at least {} candles (n_folds * 4)",
            input.n_folds * 4
        ));
    }

    let total = input.candles.len();
    let window = total / input.n_folds;
    let mut folds = Vec::with_capacity(input.n_folds);
    let mut warnings = Vec::new();

    let mut sum_train = 0.0;
    let mut sum_test = 0.0;
    let mut sum_gap = 0.0;
    let mut worst_dd = 0.0_f64;
    let mut passes = 0usize;

    for k in 0..input.n_folds {
        let win_start = k * window;
        let win_end = if k + 1 == input.n_folds {
            total
        } else {
            (k + 1) * window
        };
        let win_len = win_end - win_start;
        let train_len = ((win_len as f64) * input.train_pct).floor() as usize;
        if train_len < 2 {
            return Err(format!(
                "walkforward fold {k}: train window has fewer than 2 candles; reduce n_folds or train_pct"
            ));
        }
        let train_start = win_start;
        let train_end = win_start + train_len;
        let test_start = train_end + input.embargo;
        if test_start + 2 > win_end {
            return Err(format!(
                "walkforward fold {k}: test window has fewer than 2 candles after embargo"
            ));
        }
        let test_end = win_end;

        let train_candles = input.candles[train_start..train_end].to_vec();
        let test_candles = input.candles[test_start..test_end].to_vec();

        let train_out = backtest::run(BacktestInput {
            candles: train_candles,
            strategy: input.strategy.clone(),
            initial_cash_usd: input.initial_cash_usd,
            fee_bps: input.fee_bps,
            slippage_bps: input.slippage_bps,
            max_position_pct: input.max_position_pct,
            stop_loss_bps: input.stop_loss_bps,
            take_profit_bps: input.take_profit_bps,
        })?;

        let test_out = backtest::run(BacktestInput {
            candles: test_candles,
            strategy: input.strategy.clone(),
            initial_cash_usd: input.initial_cash_usd,
            fee_bps: input.fee_bps,
            slippage_bps: input.slippage_bps,
            max_position_pct: input.max_position_pct,
            stop_loss_bps: input.stop_loss_bps,
            take_profit_bps: input.take_profit_bps,
        })?;

        let train_ret = train_out.metrics.total_return_pct;
        let test_ret = test_out.metrics.total_return_pct;
        let gap = train_ret - test_ret;
        sum_train += train_ret;
        sum_test += test_ret;
        sum_gap += gap;
        if test_out.metrics.max_drawdown_pct > worst_dd {
            worst_dd = test_out.metrics.max_drawdown_pct;
        }
        let passes_test_gate = test_ret > 0.0
            && test_out.metrics.max_drawdown_pct <= 35.0
            && test_out.metrics.profit_factor >= 1.0
            && test_out.metrics.trades > 0;
        if passes_test_gate {
            passes += 1;
        }

        let mut fold_warnings = Vec::new();
        if test_out.metrics.trades == 0 {
            fold_warnings.push(format!(
                "fold {k}: zero out-of-sample trades; strategy did not signal in test window"
            ));
        }
        if gap > 25.0 {
            fold_warnings.push(format!(
                "fold {k}: IS/OOS gap > 25pp; possible regime overfit"
            ));
        }

        folds.push(WalkForwardFold {
            index: k,
            train_start_index: train_start,
            train_end_index: train_end,
            test_start_index: test_start,
            test_end_index: test_end,
            train_metrics: train_out.metrics,
            test_metrics: test_out.metrics,
            is_oos_gap_pct: gap,
            passes_test_gate,
            warnings: fold_warnings,
        });
    }

    let n = input.n_folds as f64;
    let mean_train = sum_train / n;
    let mean_test = sum_test / n;
    let mean_gap = sum_gap / n;
    let pass_rate = passes as f64 / n;

    let robustness = if mean_test <= 0.0 {
        "overfit".to_string()
    } else if mean_gap.abs() > 25.0 || pass_rate < 0.5 {
        "fragile".to_string()
    } else {
        "robust".to_string()
    };

    if mean_test <= 0.0 {
        warnings.push(
            "mean out-of-sample return is non-positive; do not use this strategy for live quotes"
                .to_string(),
        );
    }
    if mean_gap > 25.0 {
        warnings.push("mean IS/OOS gap > 25pp; strategy likely fits training regimes".to_string());
    }
    if pass_rate < 0.5 {
        warnings.push(format!(
            "only {} of {} folds passed the basic test gate; widen the data window or simplify the strategy",
            passes, input.n_folds
        ));
    }

    let strategy_summary = StrategySummary {
        kind: input.strategy.kind.clone(),
        fast_window: input.strategy.fast_window,
        slow_window: input.strategy.slow_window,
        lookback_window: input.strategy.lookback_window,
        threshold_bps: input.strategy.threshold_bps,
        entry_threshold: input.strategy.entry_threshold,
        exit_threshold: input.strategy.exit_threshold,
    };

    Ok(WalkForwardOutput {
        schema_version: "intents-walkforward/1",
        strategy: strategy_summary,
        n_folds: input.n_folds,
        train_pct: input.train_pct,
        embargo: input.embargo,
        folds,
        aggregate: WalkForwardAggregate {
            mean_train_return_pct: mean_train,
            mean_test_return_pct: mean_test,
            mean_is_oos_gap_pct: mean_gap,
            worst_test_drawdown_pct: worst_dd,
            fold_pass_rate: pass_rate,
            robustness,
        },
        warnings,
    })
}

// -------------------- monte carlo --------------------

#[derive(Debug, Deserialize)]
pub struct MonteCarloInput {
    /// Per-trade percentage returns (e.g. 1.5 = +1.5%). Typically the
    /// `trades[].return_pct` array from a prior `backtest` response.
    pub trade_returns_pct: Vec<f64>,
    #[serde(default = "default_initial_cash_usd")]
    pub initial_cash_usd: f64,
    #[serde(default = "default_iterations")]
    pub iterations: usize,
    /// Seed for the deterministic xorshift64* PRNG. Fixed default so
    /// scenarios are reproducible without an explicit seed.
    #[serde(default = "default_mc_seed")]
    pub seed: u64,
    /// "shuffle" (sample with replacement) or "permute" (rearrange the
    /// existing trade order without replacement). Permutation
    /// preserves the empirical mean; shuffling exposes tail risk.
    #[serde(default = "default_mc_method")]
    pub method: String,
}

fn default_iterations() -> usize {
    1_000
}

fn default_mc_seed() -> u64 {
    0xC0FFEE_DEADBEEF
}

fn default_mc_method() -> String {
    "shuffle".to_string()
}

#[derive(Debug, Serialize)]
pub struct MonteCarloOutput {
    pub schema_version: &'static str,
    pub iterations: usize,
    pub seed: u64,
    pub method: String,
    pub sample_size: usize,
    pub initial_cash_usd: f64,
    pub return_distribution: Distribution,
    pub drawdown_distribution: Distribution,
    pub terminal_equity_distribution: Distribution,
    pub probability_of_loss: f64,
    pub probability_of_drawdown_above_25pct: f64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, Default)]
pub struct Distribution {
    pub mean: f64,
    pub stddev: f64,
    pub min: f64,
    pub p05: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
    pub max: f64,
}

pub fn run_montecarlo(input: MonteCarloInput) -> Result<MonteCarloOutput, String> {
    if input.trade_returns_pct.is_empty() {
        return Err("montecarlo requires at least 1 trade return".to_string());
    }
    if input.iterations == 0 {
        return Err("montecarlo requires iterations >= 1".to_string());
    }
    if !input.initial_cash_usd.is_finite() || input.initial_cash_usd <= 0.0 {
        return Err("initial_cash_usd must be > 0".to_string());
    }
    let method = match input.method.as_str() {
        "shuffle" => Method::Shuffle,
        "permute" => Method::Permute,
        other => return Err(format!("unknown montecarlo method: {other}")),
    };
    for (idx, r) in input.trade_returns_pct.iter().enumerate() {
        if !r.is_finite() {
            return Err(format!("trade_returns_pct[{idx}] is not finite"));
        }
    }

    let mut warnings = Vec::new();
    if input.trade_returns_pct.len() < 5 {
        warnings.push(format!(
            "small sample: {} trades; resampled distributions are fragile",
            input.trade_returns_pct.len()
        ));
    }

    let mut rng = XorShift64Star::new(input.seed);
    let n = input.trade_returns_pct.len();
    let iterations = input.iterations;

    let mut total_returns = Vec::with_capacity(iterations);
    let mut drawdowns = Vec::with_capacity(iterations);
    let mut terminals = Vec::with_capacity(iterations);
    let mut loss_count = 0usize;
    let mut dd_above_25_count = 0usize;

    let mut perm_buffer: Vec<f64> = input.trade_returns_pct.clone();

    for _ in 0..iterations {
        let path: Vec<f64> = match method {
            Method::Shuffle => (0..n)
                .map(|_| {
                    let idx = (rng.next() as usize) % n;
                    input.trade_returns_pct[idx]
                })
                .collect(),
            Method::Permute => {
                fisher_yates(&mut perm_buffer, &mut rng);
                perm_buffer.clone()
            }
        };

        let mut equity = input.initial_cash_usd;
        let mut peak = equity;
        let mut max_dd = 0.0_f64;
        for r in &path {
            equity *= 1.0 + r / 100.0;
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
        let total_return_pct = (equity / input.initial_cash_usd - 1.0) * 100.0;
        total_returns.push(total_return_pct);
        drawdowns.push(max_dd);
        terminals.push(equity);
        if total_return_pct <= 0.0 {
            loss_count += 1;
        }
        if max_dd >= 25.0 {
            dd_above_25_count += 1;
        }
    }

    let return_distribution = describe(&mut total_returns);
    let drawdown_distribution = describe(&mut drawdowns);
    let terminal_equity_distribution = describe(&mut terminals);

    Ok(MonteCarloOutput {
        schema_version: "intents-montecarlo/1",
        iterations,
        seed: input.seed,
        method: input.method,
        sample_size: n,
        initial_cash_usd: input.initial_cash_usd,
        return_distribution,
        drawdown_distribution,
        terminal_equity_distribution,
        probability_of_loss: loss_count as f64 / iterations as f64,
        probability_of_drawdown_above_25pct: dd_above_25_count as f64 / iterations as f64,
        warnings,
    })
}

#[derive(Debug, Clone, Copy)]
enum Method {
    Shuffle,
    Permute,
}

struct XorShift64Star {
    state: u64,
}

impl XorShift64Star {
    fn new(seed: u64) -> Self {
        // Avoid the all-zero degenerate state by mixing a constant.
        let s = if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        };
        Self { state: s }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

fn fisher_yates(buf: &mut [f64], rng: &mut XorShift64Star) {
    if buf.len() < 2 {
        return;
    }
    for i in (1..buf.len()).rev() {
        let j = (rng.next() as usize) % (i + 1);
        buf.swap(i, j);
    }
}

fn describe(values: &mut [f64]) -> Distribution {
    if values.is_empty() {
        return Distribution::default();
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = if values.len() > 1 {
        values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)
    } else {
        0.0
    };
    let stddev = variance.sqrt();

    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = *values.first().unwrap_or(&0.0);
    let max = *values.last().unwrap_or(&0.0);
    let pct = |q: f64| -> f64 {
        let pos = (q * (values.len().saturating_sub(1)) as f64).round() as usize;
        values[pos.min(values.len() - 1)]
    };

    Distribution {
        mean,
        stddev,
        min,
        p05: pct(0.05),
        p25: pct(0.25),
        p50: pct(0.50),
        p75: pct(0.75),
        p95: pct(0.95),
        max,
    }
}

// -------------------- grid sweep --------------------

#[derive(Debug, Deserialize)]
pub struct GridInput {
    pub candles: Vec<Candle>,
    pub base: StrategyConfig,
    /// Strategy-parameter axes. The cartesian product of these axes
    /// (combined with the optional sizing/stop/take-profit options
    /// below) defines the grid.
    #[serde(default)]
    pub axes: Vec<GridAxis>,
    #[serde(default)]
    pub max_position_pct_options: Vec<f64>,
    #[serde(default)]
    pub stop_loss_bps_options: Vec<f64>,
    #[serde(default)]
    pub take_profit_bps_options: Vec<f64>,
    #[serde(default = "default_initial_cash_usd")]
    pub initial_cash_usd: f64,
    #[serde(default = "default_fee_bps")]
    pub fee_bps: f64,
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: f64,
    /// Hard cap on materialized cells. Default is enough for a 6-axis
    /// 4-value-each sweep without DOSing the WASM sandbox.
    #[serde(default = "default_max_cells")]
    pub max_cells: usize,
}

fn default_max_cells() -> usize {
    1024
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct GridAxis {
    /// One of: "fast_window", "slow_window", "lookback_window",
    /// "threshold_bps", "entry_threshold", "exit_threshold".
    pub param: String,
    pub values: Vec<f64>,
}

#[derive(Debug, Serialize)]
pub struct GridOutput {
    pub schema_version: &'static str,
    pub base: StrategySummary,
    pub axes: Vec<GridAxis>,
    pub cells: Vec<GridCell>,
    pub ranked: Vec<BacktestSuiteResult>,
    pub warnings: Vec<String>,
    pub total_cells: usize,
}

#[derive(Debug, Serialize)]
pub struct GridCell {
    pub id: String,
    pub params: serde_json::Value,
    pub max_position_pct: f64,
    pub stop_loss_bps: Option<f64>,
    pub take_profit_bps: Option<f64>,
}

pub fn run_grid(input: GridInput) -> Result<GridOutput, String> {
    if !input.initial_cash_usd.is_finite() || input.initial_cash_usd <= 0.0 {
        return Err("initial_cash_usd must be > 0".to_string());
    }
    for axis in &input.axes {
        if axis.values.is_empty() {
            return Err(format!("grid axis '{}' has no values", axis.param));
        }
        match axis.param.as_str() {
            "fast_window" | "slow_window" | "lookback_window" | "threshold_bps"
            | "entry_threshold" | "exit_threshold" => {}
            other => return Err(format!("unknown grid axis param: {other}")),
        }
    }

    let position_options = if input.max_position_pct_options.is_empty() {
        vec![1.0]
    } else {
        input.max_position_pct_options.clone()
    };
    let stop_options: Vec<Option<f64>> = if input.stop_loss_bps_options.is_empty() {
        vec![None]
    } else {
        input
            .stop_loss_bps_options
            .iter()
            .map(|v| Some(*v))
            .collect()
    };
    let tp_options: Vec<Option<f64>> = if input.take_profit_bps_options.is_empty() {
        vec![None]
    } else {
        input
            .take_profit_bps_options
            .iter()
            .map(|v| Some(*v))
            .collect()
    };

    let mut total: usize = 1;
    for axis in &input.axes {
        total = total
            .checked_mul(axis.values.len())
            .ok_or_else(|| "grid cell count overflow".to_string())?;
    }
    total = total
        .checked_mul(position_options.len())
        .and_then(|t| t.checked_mul(stop_options.len()))
        .and_then(|t| t.checked_mul(tp_options.len()))
        .ok_or_else(|| "grid cell count overflow".to_string())?;

    if total > input.max_cells {
        return Err(format!(
            "grid would generate {total} cells (>{max}); narrow the axes or raise max_cells",
            max = input.max_cells
        ));
    }

    let mut cells: Vec<GridCell> = Vec::with_capacity(total);
    let mut candidates: Vec<BacktestCandidate> = Vec::with_capacity(total);

    enumerate_axes(
        &input.axes,
        0,
        &mut Vec::new(),
        &position_options,
        &stop_options,
        &tp_options,
        &input.base,
        &mut cells,
        &mut candidates,
    )?;

    let suite = backtest::run_suite(BacktestSuiteInput {
        candles: input.candles,
        candidates,
        initial_cash_usd: input.initial_cash_usd,
        fee_bps: input.fee_bps,
        slippage_bps: input.slippage_bps,
    })?;

    let mut warnings = suite.warnings;
    if suite.ranked.iter().all(|r| !r.passes_basic_gate) {
        warnings
            .push("no grid cell passed the basic gate; widen axes or change strategy".to_string());
    }

    let base = StrategySummary {
        kind: input.base.kind.clone(),
        fast_window: input.base.fast_window,
        slow_window: input.base.slow_window,
        lookback_window: input.base.lookback_window,
        threshold_bps: input.base.threshold_bps,
        entry_threshold: input.base.entry_threshold,
        exit_threshold: input.base.exit_threshold,
    };

    Ok(GridOutput {
        schema_version: "intents-grid/1",
        base,
        axes: input.axes,
        total_cells: cells.len(),
        cells,
        ranked: suite.ranked,
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
fn enumerate_axes(
    axes: &[GridAxis],
    depth: usize,
    selected: &mut Vec<(String, f64)>,
    position_options: &[f64],
    stop_options: &[Option<f64>],
    tp_options: &[Option<f64>],
    base: &StrategyConfig,
    cells: &mut Vec<GridCell>,
    candidates: &mut Vec<BacktestCandidate>,
) -> Result<(), String> {
    if depth == axes.len() {
        for &pos in position_options {
            for &stop in stop_options {
                for &tp in tp_options {
                    let mut strategy = base.clone();
                    let mut params = serde_json::Map::new();
                    for (name, value) in selected.iter() {
                        apply_param(&mut strategy, name, *value)?;
                        params.insert(name.clone(), serde_json::json!(*value));
                    }
                    let id = make_cell_id(selected, pos, stop, tp);
                    cells.push(GridCell {
                        id: id.clone(),
                        params: serde_json::Value::Object(params),
                        max_position_pct: pos,
                        stop_loss_bps: stop,
                        take_profit_bps: tp,
                    });
                    candidates.push(BacktestCandidate {
                        id,
                        strategy,
                        max_position_pct: Some(pos),
                        stop_loss_bps: stop,
                        take_profit_bps: tp,
                    });
                }
            }
        }
        return Ok(());
    }
    let axis = &axes[depth];
    for value in &axis.values {
        selected.push((axis.param.clone(), *value));
        enumerate_axes(
            axes,
            depth + 1,
            selected,
            position_options,
            stop_options,
            tp_options,
            base,
            cells,
            candidates,
        )?;
        selected.pop();
    }
    Ok(())
}

fn apply_param(strategy: &mut StrategyConfig, name: &str, value: f64) -> Result<(), String> {
    match name {
        "fast_window" => strategy.fast_window = Some(value as usize),
        "slow_window" => strategy.slow_window = Some(value as usize),
        "lookback_window" => strategy.lookback_window = Some(value as usize),
        "threshold_bps" => strategy.threshold_bps = Some(value),
        "entry_threshold" => strategy.entry_threshold = Some(value),
        "exit_threshold" => strategy.exit_threshold = Some(value),
        other => return Err(format!("unknown grid param: {other}")),
    }
    Ok(())
}

fn make_cell_id(
    selected: &[(String, f64)],
    pos: f64,
    stop: Option<f64>,
    tp: Option<f64>,
) -> String {
    let mut parts: Vec<String> = selected
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    parts.push(format!("pos={pos}"));
    if let Some(s) = stop {
        parts.push(format!("stop={s}"));
    }
    if let Some(t) = tp {
        parts.push(format!("tp={t}"));
    }
    parts.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(idx: usize, close: f64) -> Candle {
        Candle {
            ts: format!("2026-01-{idx:02}T00:00:00Z"),
            open: close,
            high: close * 1.01,
            low: close * 0.99,
            close,
            volume: 1_000.0,
        }
    }

    fn series(closes: &[f64]) -> Vec<Candle> {
        closes
            .iter()
            .enumerate()
            .map(|(idx, c)| candle(idx + 1, *c))
            .collect()
    }

    #[test]
    fn walkforward_computes_per_fold_and_aggregate() {
        // 40 candles, gentle uptrend so buy-hold has a non-trivial return
        let mut closes = Vec::new();
        for i in 0..40 {
            closes.push(100.0 + i as f64);
        }
        let out = run_walkforward(WalkForwardInput {
            candles: series(&closes),
            strategy: StrategyConfig {
                kind: "buy-hold".to_string(),
                fast_window: None,
                slow_window: None,
                lookback_window: None,
                threshold_bps: None,
                entry_threshold: None,
                exit_threshold: None,
            },
            n_folds: 4,
            train_pct: 0.7,
            embargo: 1,
            initial_cash_usd: 1_000.0,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            max_position_pct: 1.0,
            stop_loss_bps: None,
            take_profit_bps: None,
        })
        .unwrap();
        assert_eq!(out.schema_version, "intents-walkforward/1");
        assert_eq!(out.folds.len(), 4);
        assert!(out.aggregate.mean_test_return_pct > 0.0);
    }

    #[test]
    fn walkforward_rejects_too_few_candles() {
        let closes = [100.0, 101.0, 102.0, 103.0];
        let err = run_walkforward(WalkForwardInput {
            candles: series(&closes),
            strategy: StrategyConfig {
                kind: "buy-hold".to_string(),
                fast_window: None,
                slow_window: None,
                lookback_window: None,
                threshold_bps: None,
                entry_threshold: None,
                exit_threshold: None,
            },
            n_folds: 5,
            train_pct: 0.7,
            embargo: 0,
            initial_cash_usd: 1_000.0,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            max_position_pct: 1.0,
            stop_loss_bps: None,
            take_profit_bps: None,
        })
        .unwrap_err();
        assert!(err.contains("at least"));
    }

    #[test]
    fn montecarlo_reproduces_with_same_seed() {
        let returns = vec![1.5, -0.5, 2.0, -1.0, 0.8, 0.3, -0.2, 1.1];
        let a = run_montecarlo(MonteCarloInput {
            trade_returns_pct: returns.clone(),
            initial_cash_usd: 1_000.0,
            iterations: 200,
            seed: 42,
            method: "shuffle".to_string(),
        })
        .unwrap();
        let b = run_montecarlo(MonteCarloInput {
            trade_returns_pct: returns,
            initial_cash_usd: 1_000.0,
            iterations: 200,
            seed: 42,
            method: "shuffle".to_string(),
        })
        .unwrap();
        assert_eq!(a.return_distribution.p50, b.return_distribution.p50);
        assert_eq!(a.drawdown_distribution.p95, b.drawdown_distribution.p95);
    }

    #[test]
    fn montecarlo_permute_preserves_mean() {
        let returns = vec![1.0, -0.5, 1.5, -0.2, 0.7];
        let out = run_montecarlo(MonteCarloInput {
            trade_returns_pct: returns.clone(),
            initial_cash_usd: 1_000.0,
            iterations: 500,
            seed: 1,
            method: "permute".to_string(),
        })
        .unwrap();
        // Permutation re-orders the empirical sequence, so each path's
        // compounded return is a permutation invariant of returns.
        let mut equity = 1_000.0;
        for r in &returns {
            equity *= 1.0 + r / 100.0;
        }
        let observed = (equity / 1_000.0 - 1.0) * 100.0;
        assert!((out.return_distribution.mean - observed).abs() < 1e-6);
        assert!((out.return_distribution.p50 - observed).abs() < 1e-6);
    }

    #[test]
    fn grid_enumerates_cartesian_product() {
        let mut closes = Vec::new();
        for i in 0..30 {
            closes.push(100.0 + i as f64);
        }
        let out = run_grid(GridInput {
            candles: series(&closes),
            base: StrategyConfig {
                kind: "sma-cross".to_string(),
                fast_window: Some(5),
                slow_window: Some(20),
                lookback_window: None,
                threshold_bps: None,
                entry_threshold: None,
                exit_threshold: None,
            },
            axes: vec![
                GridAxis {
                    param: "fast_window".to_string(),
                    values: vec![3.0, 5.0],
                },
                GridAxis {
                    param: "slow_window".to_string(),
                    values: vec![10.0, 20.0],
                },
            ],
            max_position_pct_options: vec![1.0],
            stop_loss_bps_options: vec![],
            take_profit_bps_options: vec![],
            initial_cash_usd: 1_000.0,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            max_cells: 64,
        })
        .unwrap();
        assert_eq!(out.cells.len(), 4);
        assert_eq!(out.ranked.len(), 4);
        assert_eq!(out.schema_version, "intents-grid/1");
    }

    #[test]
    fn grid_enforces_cell_cap() {
        let closes = [100.0, 101.0, 102.0, 103.0, 104.0];
        let err = run_grid(GridInput {
            candles: series(&closes),
            base: StrategyConfig {
                kind: "sma-cross".to_string(),
                fast_window: Some(2),
                slow_window: Some(3),
                lookback_window: None,
                threshold_bps: None,
                entry_threshold: None,
                exit_threshold: None,
            },
            axes: vec![GridAxis {
                param: "fast_window".to_string(),
                values: (1..=10).map(|i| i as f64).collect(),
            }],
            max_position_pct_options: vec![],
            stop_loss_bps_options: vec![],
            take_profit_bps_options: vec![],
            initial_cash_usd: 1_000.0,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            max_cells: 5,
        })
        .unwrap_err();
        assert!(err.contains("max_cells"));
    }
}
