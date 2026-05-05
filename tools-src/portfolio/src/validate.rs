//! Composed strategy validation + journal-ready markdown report.
//!
//! Two new actions:
//!
//! - `validate_strategy`: runs `backtest::run` + `lab::run_walkforward` +
//!   `lab::run_montecarlo` (when the base backtest produced trades) on a
//!   single candidate over a single candle series. Returns the union of
//!   their outputs plus an eligibility verdict (`approved`, `watch`,
//!   `rejected`) and a per-gate pass/fail table the risk manager
//!   consumes verbatim.
//!
//! - `format_strategy_doc`: deterministic Markdown formatter for any of
//!   the new backtest-family schemas (base backtest, walk-forward, Monte
//!   Carlo, grid, DCA backtest, DCA schedule, validation). Returns a
//!   journal entry the agent persists under
//!   `projects/intents-trading-agent/journal/` after a run.
//!
//! Both are pure compositions — no new external dependencies, no live
//! HTTP, no signing, no key access.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backtest::{self, BacktestInput, BacktestOutput, Candle, StrategyConfig};
use crate::lab::{self, MonteCarloInput, MonteCarloOutput, WalkForwardInput, WalkForwardOutput};

// -------------------- defaults --------------------

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
fn default_n_folds() -> usize {
    4
}
fn default_train_pct() -> f64 {
    0.7
}
fn default_mc_iterations() -> usize {
    1_000
}
fn default_mc_seed() -> u64 {
    0xC0FFEE_DEADBEEF
}
fn default_mc_method() -> String {
    "shuffle".to_string()
}
fn default_min_trades() -> usize {
    5
}
fn default_min_total_return_pct() -> f64 {
    0.0
}
fn default_max_drawdown_pct() -> f64 {
    35.0
}
fn default_min_profit_factor() -> f64 {
    1.0
}
fn default_max_loss_probability() -> f64 {
    0.4
}
fn default_max_is_oos_gap_pct() -> f64 {
    25.0
}

// -------------------- validate_strategy --------------------

#[derive(Debug, Deserialize)]
pub struct ValidateStrategyInput {
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

    /// Walk-forward configuration. Set `n_folds=0` to skip the
    /// walk-forward stage entirely.
    #[serde(default = "default_n_folds")]
    pub walkforward_n_folds: usize,
    #[serde(default = "default_train_pct")]
    pub walkforward_train_pct: f64,
    #[serde(default)]
    pub walkforward_embargo: usize,

    /// Monte Carlo configuration. Set `iterations=0` to skip Monte Carlo.
    #[serde(default = "default_mc_iterations")]
    pub montecarlo_iterations: usize,
    #[serde(default = "default_mc_seed")]
    pub montecarlo_seed: u64,
    #[serde(default = "default_mc_method")]
    pub montecarlo_method: String,

    /// Eligibility thresholds the validation report measures against.
    #[serde(default = "default_min_trades")]
    pub min_trades: usize,
    #[serde(default = "default_min_total_return_pct")]
    pub min_total_return_pct: f64,
    #[serde(default = "default_max_drawdown_pct")]
    pub max_drawdown_pct: f64,
    #[serde(default = "default_min_profit_factor")]
    pub min_profit_factor: f64,
    #[serde(default = "default_max_loss_probability")]
    pub max_loss_probability: f64,
    #[serde(default = "default_max_is_oos_gap_pct")]
    pub max_is_oos_gap_pct: f64,
}

#[derive(Debug, Serialize)]
pub struct ValidateStrategyOutput {
    pub schema_version: &'static str,
    pub strategy_kind: String,
    pub base: BacktestOutput,
    pub walkforward: Option<WalkForwardOutput>,
    pub montecarlo: Option<MonteCarloOutput>,
    pub gates: Vec<ValidationGate>,
    pub eligibility: String,
    pub recommendation: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ValidationGate {
    pub name: String,
    pub status: String,
    pub observed: String,
    pub threshold: String,
    pub detail: String,
}

pub fn run(input: ValidateStrategyInput) -> Result<ValidateStrategyOutput, String> {
    let strategy_kind = input.strategy.kind.clone();
    let base = backtest::run(BacktestInput {
        candles: input.candles.clone(),
        strategy: input.strategy.clone(),
        initial_cash_usd: input.initial_cash_usd,
        fee_bps: input.fee_bps,
        slippage_bps: input.slippage_bps,
        max_position_pct: input.max_position_pct,
        stop_loss_bps: input.stop_loss_bps,
        take_profit_bps: input.take_profit_bps,
    })?;

    let walkforward = if input.walkforward_n_folds >= 2 {
        match lab::run_walkforward(WalkForwardInput {
            candles: input.candles.clone(),
            strategy: input.strategy.clone(),
            n_folds: input.walkforward_n_folds,
            train_pct: input.walkforward_train_pct,
            embargo: input.walkforward_embargo,
            initial_cash_usd: input.initial_cash_usd,
            fee_bps: input.fee_bps,
            slippage_bps: input.slippage_bps,
            max_position_pct: input.max_position_pct,
            stop_loss_bps: input.stop_loss_bps,
            take_profit_bps: input.take_profit_bps,
        }) {
            Ok(out) => Some(out),
            Err(e) => {
                return Err(format!("walkforward stage failed: {e}"));
            }
        }
    } else {
        None
    };

    let trade_returns: Vec<f64> = base.trades.iter().map(|t| t.return_pct).collect();
    let montecarlo = if input.montecarlo_iterations > 0 && !trade_returns.is_empty() {
        Some(lab::run_montecarlo(MonteCarloInput {
            trade_returns_pct: trade_returns,
            initial_cash_usd: input.initial_cash_usd,
            iterations: input.montecarlo_iterations,
            seed: input.montecarlo_seed,
            method: input.montecarlo_method.clone(),
        })?)
    } else {
        None
    };

    let mut gates = Vec::new();
    let mut warnings = Vec::new();

    gates.push(gate_min(
        "min_trades",
        base.metrics.trades as f64,
        input.min_trades as f64,
        "trades",
    ));
    gates.push(gate_min(
        "min_total_return",
        base.metrics.total_return_pct,
        input.min_total_return_pct,
        "%",
    ));
    gates.push(gate_max(
        "max_drawdown",
        base.metrics.max_drawdown_pct,
        input.max_drawdown_pct,
        "%",
    ));
    gates.push(gate_min(
        "min_profit_factor",
        base.metrics.profit_factor,
        input.min_profit_factor,
        "",
    ));

    if let Some(wf) = &walkforward {
        gates.push(gate_min(
            "walkforward_mean_oos_return",
            wf.aggregate.mean_test_return_pct,
            input.min_total_return_pct,
            "%",
        ));
        gates.push(gate_max(
            "walkforward_is_oos_gap",
            wf.aggregate.mean_is_oos_gap_pct.abs(),
            input.max_is_oos_gap_pct,
            "%",
        ));
        gates.push(ValidationGate {
            name: "walkforward_robustness".to_string(),
            status: match wf.aggregate.robustness.as_str() {
                "robust" => "pass".to_string(),
                "fragile" => "warn".to_string(),
                _ => "fail".to_string(),
            },
            observed: wf.aggregate.robustness.clone(),
            threshold: "robust|fragile".to_string(),
            detail: format!("fold_pass_rate={:.2}", wf.aggregate.fold_pass_rate),
        });
    } else if input.walkforward_n_folds >= 2 {
        warnings.push(
            "walkforward stage skipped despite n_folds>=2 (input may be inconsistent)".to_string(),
        );
    }

    if let Some(mc) = &montecarlo {
        gates.push(gate_max(
            "montecarlo_loss_probability",
            mc.probability_of_loss,
            input.max_loss_probability,
            "",
        ));
        gates.push(gate_max(
            "montecarlo_p05_drawdown",
            mc.drawdown_distribution.p95,
            input.max_drawdown_pct,
            "%",
        ));
    } else if input.montecarlo_iterations > 0 {
        warnings.push("montecarlo skipped: base backtest produced no trades".to_string());
    }

    let any_fail = gates.iter().any(|g| g.status == "fail");
    let any_warn = gates.iter().any(|g| g.status == "warn");
    let eligibility = if any_fail {
        "rejected".to_string()
    } else if any_warn {
        "watch".to_string()
    } else {
        "approved".to_string()
    };
    let recommendation = match eligibility.as_str() {
        "approved" => format!(
            "Strategy '{strategy_kind}' cleared all gates. Eligible for paper-intent construction. Live quoting still requires explicit operator approval."
        ),
        "watch" => format!(
            "Strategy '{strategy_kind}' has warnings; downgrade to watch mode and gather more data before paper intents."
        ),
        _ => format!(
            "Strategy '{strategy_kind}' failed at least one hard gate; do not use for paper or quote intents until thresholds are met."
        ),
    };

    Ok(ValidateStrategyOutput {
        schema_version: "intents-strategy-validation/1",
        strategy_kind,
        base,
        walkforward,
        montecarlo,
        gates,
        eligibility,
        recommendation,
        warnings,
    })
}

fn gate_min(name: &str, observed: f64, threshold: f64, unit: &str) -> ValidationGate {
    let status = if observed >= threshold {
        "pass"
    } else {
        "fail"
    };
    ValidationGate {
        name: name.to_string(),
        status: status.to_string(),
        observed: format!("{observed:.4}{unit}"),
        threshold: format!(">= {threshold:.4}{unit}"),
        detail: String::new(),
    }
}

fn gate_max(name: &str, observed: f64, threshold: f64, unit: &str) -> ValidationGate {
    let status = if observed <= threshold {
        "pass"
    } else {
        "fail"
    };
    ValidationGate {
        name: name.to_string(),
        status: status.to_string(),
        observed: format!("{observed:.4}{unit}"),
        threshold: format!("<= {threshold:.4}{unit}"),
        detail: String::new(),
    }
}

// -------------------- format_strategy_doc --------------------

#[derive(Debug, Deserialize)]
pub struct FormatStrategyDocInput {
    /// Any of the new backtest-family responses, passed through
    /// verbatim. The formatter inspects `schema_version` and dispatches.
    pub report: Value,
    /// Optional human title for the journal entry.
    #[serde(default)]
    pub title: Option<String>,
    /// Optional pair label for the heading.
    #[serde(default)]
    pub pair: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FormatStrategyDocOutput {
    pub schema_version: &'static str,
    pub kind: String,
    pub markdown: String,
}

pub fn format(input: FormatStrategyDocInput) -> Result<FormatStrategyDocOutput, String> {
    let kind = input
        .report
        .get("schema_version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "report.schema_version is required".to_string())?
        .to_string();
    let title = input
        .title
        .clone()
        .unwrap_or_else(|| format!("Strategy report ({kind})"));
    let pair = input
        .pair
        .clone()
        .unwrap_or_else(|| "(pair unspecified)".to_string());
    let mut md = String::new();
    md.push_str(&format!("# {title}\n\n"));
    md.push_str(&format!("- **Pair**: {pair}\n"));
    md.push_str(&format!("- **Schema**: `{kind}`\n\n"));

    match kind.as_str() {
        "intents-backtest/1" => render_backtest(&input.report, &mut md),
        "intents-walkforward/1" => render_walkforward(&input.report, &mut md),
        "intents-montecarlo/1" => render_montecarlo(&input.report, &mut md),
        "intents-grid/1" => render_grid(&input.report, &mut md),
        "intents-dca-backtest/1" => render_dca_backtest(&input.report, &mut md),
        "intents-dca-schedule/1" => render_dca_schedule(&input.report, &mut md),
        "intents-strategy-validation/1" => render_validation(&input.report, &mut md),
        other => {
            md.push_str(&format!(
                "_Formatter does not have a renderer for `{other}`; raw JSON below._\n\n"
            ));
            md.push_str("```json\n");
            md.push_str(
                &serde_json::to_string_pretty(&input.report)
                    .unwrap_or_else(|_| "(unrenderable)".to_string()),
            );
            md.push_str("\n```\n");
        }
    }

    md.push_str("\n---\n\n");
    md.push_str("_Generated by `portfolio.format_strategy_doc`. The trading agent never signs._\n");

    Ok(FormatStrategyDocOutput {
        schema_version: "intents-strategy-doc/1",
        kind,
        markdown: md,
    })
}

fn render_backtest(report: &Value, md: &mut String) {
    let metrics = &report["metrics"];
    md.push_str("## Backtest metrics\n\n");
    md.push_str(
        "| candles | trades | total return | buy-hold | alpha | max DD | win rate | profit factor |\n",
    );
    md.push_str("|---:|---:|---:|---:|---:|---:|---:|---:|\n");
    md.push_str(&format!(
        "| {} | {} | {:.2}% | {:.2}% | {:+.2}% | {:.2}% | {:.1}% | {:.2} |\n\n",
        metrics["candles"].as_u64().unwrap_or(0),
        metrics["trades"].as_u64().unwrap_or(0),
        metrics["total_return_pct"].as_f64().unwrap_or(0.0),
        metrics["buy_hold_return_pct"].as_f64().unwrap_or(0.0),
        metrics["alpha_vs_buy_hold_pct"].as_f64().unwrap_or(0.0),
        metrics["max_drawdown_pct"].as_f64().unwrap_or(0.0),
        metrics["win_rate_pct"].as_f64().unwrap_or(0.0),
        metrics["profit_factor"].as_f64().unwrap_or(0.0),
    ));
    if let Some(strat) = report.get("strategy") {
        md.push_str(&format!(
            "Strategy: `{}` (windows fast={:?}, slow={:?}, lookback={:?})\n\n",
            strat["kind"].as_str().unwrap_or("?"),
            strat.get("fast_window"),
            strat.get("slow_window"),
            strat.get("lookback_window"),
        ));
    }
}

fn render_walkforward(report: &Value, md: &mut String) {
    md.push_str("## Walk-forward\n\n");
    if let Some(agg) = report.get("aggregate") {
        md.push_str(&format!(
            "- mean train return: {:.2}%\n- mean test return: {:.2}%\n- mean IS/OOS gap: {:.2}pp\n- worst test drawdown: {:.2}%\n- fold pass rate: {:.0}%\n- robustness: **{}**\n\n",
            agg["mean_train_return_pct"].as_f64().unwrap_or(0.0),
            agg["mean_test_return_pct"].as_f64().unwrap_or(0.0),
            agg["mean_is_oos_gap_pct"].as_f64().unwrap_or(0.0),
            agg["worst_test_drawdown_pct"].as_f64().unwrap_or(0.0),
            agg["fold_pass_rate"].as_f64().unwrap_or(0.0) * 100.0,
            agg["robustness"].as_str().unwrap_or("?"),
        ));
    }
    if let Some(folds) = report.get("folds").and_then(|v| v.as_array()) {
        md.push_str(
            "| fold | train | test | gap | OOS DD | passes |\n|---:|---:|---:|---:|---:|---|\n",
        );
        for f in folds {
            md.push_str(&format!(
                "| {} | {:.2}% | {:.2}% | {:.2}pp | {:.2}% | {} |\n",
                f["index"].as_u64().unwrap_or(0),
                f["train_metrics"]["total_return_pct"]
                    .as_f64()
                    .unwrap_or(0.0),
                f["test_metrics"]["total_return_pct"]
                    .as_f64()
                    .unwrap_or(0.0),
                f["is_oos_gap_pct"].as_f64().unwrap_or(0.0),
                f["test_metrics"]["max_drawdown_pct"]
                    .as_f64()
                    .unwrap_or(0.0),
                if f["passes_test_gate"].as_bool().unwrap_or(false) {
                    "✓"
                } else {
                    "✗"
                }
            ));
        }
        md.push('\n');
    }
}

fn render_montecarlo(report: &Value, md: &mut String) {
    md.push_str("## Monte Carlo\n\n");
    md.push_str(&format!(
        "- iterations: {}\n- method: `{}`\n- seed: {}\n- sample size: {}\n- P(loss): {:.1}%\n- P(DD ≥ 25%): {:.1}%\n\n",
        report["iterations"].as_u64().unwrap_or(0),
        report["method"].as_str().unwrap_or("?"),
        report["seed"].as_u64().unwrap_or(0),
        report["sample_size"].as_u64().unwrap_or(0),
        report["probability_of_loss"].as_f64().unwrap_or(0.0) * 100.0,
        report["probability_of_drawdown_above_25pct"]
            .as_f64()
            .unwrap_or(0.0)
            * 100.0,
    ));
    if let Some(rd) = report.get("return_distribution") {
        md.push_str("| distribution | mean | p05 | p50 | p95 |\n|---|---:|---:|---:|---:|\n");
        md.push_str(&format!(
            "| return % | {:.2} | {:.2} | {:.2} | {:.2} |\n",
            rd["mean"].as_f64().unwrap_or(0.0),
            rd["p05"].as_f64().unwrap_or(0.0),
            rd["p50"].as_f64().unwrap_or(0.0),
            rd["p95"].as_f64().unwrap_or(0.0),
        ));
        if let Some(dd) = report.get("drawdown_distribution") {
            md.push_str(&format!(
                "| drawdown % | {:.2} | {:.2} | {:.2} | {:.2} |\n",
                dd["mean"].as_f64().unwrap_or(0.0),
                dd["p05"].as_f64().unwrap_or(0.0),
                dd["p50"].as_f64().unwrap_or(0.0),
                dd["p95"].as_f64().unwrap_or(0.0),
            ));
        }
        md.push('\n');
    }
}

fn render_grid(report: &Value, md: &mut String) {
    md.push_str("## Parameter grid\n\n");
    let total = report["total_cells"].as_u64().unwrap_or(0);
    md.push_str(&format!("Total cells materialized: {total}\n\n"));
    if let Some(top) = report.get("ranked").and_then(|v| v.as_array()) {
        md.push_str("Top 5 candidates:\n\n");
        md.push_str(
            "| rank | id | return | alpha | DD | profit factor | passes basic gate |\n|---:|---|---:|---:|---:|---:|---|\n",
        );
        for cand in top.iter().take(5) {
            let m = &cand["metrics"];
            md.push_str(&format!(
                "| {} | {} | {:.2}% | {:+.2}% | {:.2}% | {:.2} | {} |\n",
                cand["rank"].as_u64().unwrap_or(0),
                cand["id"].as_str().unwrap_or("?"),
                m["total_return_pct"].as_f64().unwrap_or(0.0),
                m["alpha_vs_buy_hold_pct"].as_f64().unwrap_or(0.0),
                m["max_drawdown_pct"].as_f64().unwrap_or(0.0),
                m["profit_factor"].as_f64().unwrap_or(0.0),
                if cand["passes_basic_gate"].as_bool().unwrap_or(false) {
                    "✓"
                } else {
                    "✗"
                }
            ));
        }
        md.push('\n');
    }
}

fn render_dca_backtest(report: &Value, md: &mut String) {
    md.push_str("## DCA backtest\n\n");
    md.push_str(&format!(
        "- periods planned: {}\n- executed: {}\n- skipped (band): {}\n- doubled: {}\n- total invested: ${}\n- units acquired: {}\n- average basis: ${}\n- breakeven price: ${}\n- final close: ${}\n- mark-to-market: ${}\n- total return: {:.2}%\n- vs lump-sum buy-and-hold: {:+.2}%\n- max underlying DD: {:.2}%\n\n",
        report["periods_planned"].as_u64().unwrap_or(0),
        report["periods_executed"].as_u64().unwrap_or(0),
        report["periods_skipped_band"].as_u64().unwrap_or(0),
        report["periods_doubled"].as_u64().unwrap_or(0),
        report["total_invested_usd"].as_str().unwrap_or("0"),
        report["units_acquired"].as_str().unwrap_or("0"),
        report["average_cost_basis_usd"].as_str().unwrap_or("0"),
        report["breakeven_price_usd"].as_str().unwrap_or("0"),
        report["final_close_usd"].as_str().unwrap_or("0"),
        report["mark_to_market_usd"].as_str().unwrap_or("0"),
        report["total_return_pct"].as_f64().unwrap_or(0.0),
        report["vs_lumpsum_buy_hold_pct"].as_f64().unwrap_or(0.0),
        report["max_underlying_drawdown_pct"].as_f64().unwrap_or(0.0),
    ));
}

fn render_dca_schedule(report: &Value, md: &mut String) {
    md.push_str("## DCA schedule\n\n");
    md.push_str(&format!(
        "- pair: {}\n- mode: {}\n- cadence: {} (`{}`)\n- total periods: {}\n- per-period: ${}\n- total notional: ${}\n- max slippage: {} bps\n- solver: {}\n- safe to quote: {}\n\n",
        report["pair"].as_str().unwrap_or("?"),
        report["mode"].as_str().unwrap_or("?"),
        report["cadence"].as_str().unwrap_or("?"),
        report["cron"].as_str().unwrap_or("?"),
        report["total_periods"].as_u64().unwrap_or(0),
        report["notional_per_period_usd"].as_str().unwrap_or("0"),
        report["total_notional_usd"].as_str().unwrap_or("0"),
        report["max_slippage_bps"].as_f64().unwrap_or(0.0),
        report["solver"].as_str().unwrap_or("?"),
        report["safe_to_quote"].as_bool().unwrap_or(false),
    ));
    if let Some(gates) = report.get("risk_gates").and_then(|v| v.as_array()) {
        md.push_str("Risk gates:\n\n");
        for g in gates {
            md.push_str(&format!(
                "- **{}** ({}) — {}\n",
                g["name"].as_str().unwrap_or("?"),
                g["status"].as_str().unwrap_or("?"),
                g["detail"].as_str().unwrap_or(""),
            ));
        }
        md.push('\n');
    }
}

fn render_validation(report: &Value, md: &mut String) {
    md.push_str(&format!(
        "## Validation: {}\n\n",
        report["eligibility"].as_str().unwrap_or("?")
    ));
    md.push_str(&format!(
        "**Recommendation**: {}\n\n",
        report["recommendation"].as_str().unwrap_or("(none)")
    ));
    if let Some(base) = report.get("base") {
        render_backtest(base, md);
    }
    if let Some(wf) = report.get("walkforward") {
        if !wf.is_null() {
            render_walkforward(wf, md);
        }
    }
    if let Some(mc) = report.get("montecarlo") {
        if !mc.is_null() {
            render_montecarlo(mc, md);
        }
    }
    if let Some(gates) = report.get("gates").and_then(|v| v.as_array()) {
        md.push_str("## Gates\n\n");
        md.push_str("| gate | status | observed | threshold |\n|---|---|---|---|\n");
        for g in gates {
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                g["name"].as_str().unwrap_or("?"),
                g["status"].as_str().unwrap_or("?"),
                g["observed"].as_str().unwrap_or("?"),
                g["threshold"].as_str().unwrap_or("?"),
            ));
        }
        md.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs(closes: &[f64]) -> Vec<Candle> {
        closes
            .iter()
            .enumerate()
            .map(|(i, c)| Candle {
                ts: format!("2026-02-{:02}T00:00:00Z", i + 1),
                open: *c,
                high: c * 1.01,
                low: c * 0.99,
                close: *c,
                volume: 1_000.0,
            })
            .collect()
    }

    fn uptrend(n: usize) -> Vec<Candle> {
        let closes: Vec<f64> = (0..n).map(|i| 100.0 + i as f64).collect();
        cs(&closes)
    }

    #[test]
    fn validate_buy_hold_uptrend_approves() {
        let out = run(ValidateStrategyInput {
            candles: uptrend(40),
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
            walkforward_n_folds: 4,
            walkforward_train_pct: 0.7,
            walkforward_embargo: 0,
            montecarlo_iterations: 100,
            montecarlo_seed: 1,
            montecarlo_method: "shuffle".to_string(),
            min_trades: 1,
            min_total_return_pct: 0.0,
            max_drawdown_pct: 35.0,
            min_profit_factor: 1.0,
            max_loss_probability: 0.4,
            max_is_oos_gap_pct: 50.0,
        })
        .unwrap();
        assert_eq!(out.schema_version, "intents-strategy-validation/1");
        assert_eq!(out.eligibility, "approved");
        assert!(out.walkforward.is_some());
        assert!(out.montecarlo.is_some());
    }

    #[test]
    fn validate_skips_montecarlo_when_no_trades() {
        // sma-cross with windows that require more candles than provided
        // returns 0 trades; montecarlo should be skipped.
        let out = run(ValidateStrategyInput {
            candles: uptrend(40),
            strategy: StrategyConfig {
                kind: "sma-cross".to_string(),
                fast_window: Some(45),
                slow_window: Some(50),
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
            walkforward_n_folds: 0,
            walkforward_train_pct: 0.7,
            walkforward_embargo: 0,
            montecarlo_iterations: 100,
            montecarlo_seed: 1,
            montecarlo_method: "shuffle".to_string(),
            min_trades: 1,
            min_total_return_pct: 0.0,
            max_drawdown_pct: 35.0,
            min_profit_factor: 1.0,
            max_loss_probability: 0.4,
            max_is_oos_gap_pct: 50.0,
        })
        .unwrap();
        assert!(out.montecarlo.is_none());
        assert!(out.warnings.iter().any(|w| w.contains("montecarlo")));
        assert_eq!(out.eligibility, "rejected");
    }

    #[test]
    fn format_dispatches_by_schema() {
        let report = serde_json::json!({
            "schema_version": "intents-backtest/1",
            "strategy": {"kind": "buy-hold"},
            "metrics": {
                "candles": 40,
                "trades": 1,
                "total_return_pct": 19.0,
                "buy_hold_return_pct": 19.0,
                "alpha_vs_buy_hold_pct": 0.0,
                "max_drawdown_pct": 0.5,
                "win_rate_pct": 100.0,
                "profit_factor": 1.5,
                "exposure_pct": 100.0,
                "average_trade_return_pct": 19.0,
                "return_stability": 0.0,
                "start_equity_usd": "1000",
                "end_equity_usd": "1190"
            },
            "trades": [],
            "equity_curve": [],
            "warnings": [],
            "lookahead_safe": true
        });
        let out = format(FormatStrategyDocInput {
            report,
            title: Some("Test buy-hold report".to_string()),
            pair: Some("NEAR/USDC".to_string()),
        })
        .unwrap();
        assert_eq!(out.schema_version, "intents-strategy-doc/1");
        assert_eq!(out.kind, "intents-backtest/1");
        assert!(out.markdown.contains("Test buy-hold report"));
        assert!(out.markdown.contains("NEAR/USDC"));
        assert!(out.markdown.contains("19.00%"));
    }

    #[test]
    fn format_handles_unknown_schema_with_raw_json() {
        let report = serde_json::json!({"schema_version": "totally-new/9", "x": 1});
        let out = format(FormatStrategyDocInput {
            report,
            title: None,
            pair: None,
        })
        .unwrap();
        assert!(out.markdown.contains("totally-new/9"));
        assert!(out.markdown.contains("```json"));
    }

    #[test]
    fn format_requires_schema_version() {
        let err = format(FormatStrategyDocInput {
            report: serde_json::json!({"foo": "bar"}),
            title: None,
            pair: None,
        })
        .unwrap_err();
        assert!(err.contains("schema_version"));
    }
}
