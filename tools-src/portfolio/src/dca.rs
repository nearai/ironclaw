//! Dollar-cost averaging (DCA) for the Intents Trading Agent.
//!
//! Two surfaces:
//!
//! - `backtest_dca`: deterministic replay of a fixed-cadence,
//!   fixed-notional buy schedule over a candle series. Reports lots,
//!   average cost basis, breakeven price, mark-to-market, alpha vs
//!   lump-sum buy-and-hold, and the underlying drawdown the schedule
//!   had to survive. Optional symmetric price band turns the schedule
//!   into a "skip when stretched, double-up when discounted" variant
//!   without breaking determinism.
//!
//! - `plan_dca_schedule`: turn a user-friendly description (pair,
//!   chains, per-period USD, cadence, total periods) into a recurring
//!   intent schedule. Emits a `build_intent` template the caller uses
//!   per period plus deterministic risk gates and a cron expression.
//!   Quotes are unsigned; signing remains a wallet action outside the
//!   agent.
//!
//! Both are pure functions — no live HTTP, no signing, no key access.

use serde::{Deserialize, Serialize};

use crate::backtest::{Candle, EquityPoint};

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

fn default_max_slippage_bps() -> f64 {
    50.0
}

fn default_period_candles() -> usize {
    1
}

fn default_mode() -> String {
    "paper".to_string()
}

fn default_assumed_price_usd() -> Option<f64> {
    None
}

fn default_currency() -> String {
    "USDC".to_string()
}

// -------------------- backtest_dca --------------------

#[derive(Debug, Deserialize)]
pub struct DcaBacktestInput {
    pub candles: Vec<Candle>,
    /// Notional USD committed per buy.
    pub notional_per_period_usd: f64,
    /// Buy every N candles. 1 = every candle.
    #[serde(default = "default_period_candles")]
    pub period_candles: usize,
    /// Skip the first `offset` candles before the first buy.
    #[serde(default)]
    pub offset: usize,
    /// If set, skip a buy when the candidate close is more than
    /// `skip_above_premium_bps` above the running average cost basis.
    #[serde(default)]
    pub skip_above_premium_bps: Option<f64>,
    /// If set, double the buy when the candidate close is more than
    /// `opportunistic_below_discount_bps` below the running average
    /// cost basis.
    #[serde(default)]
    pub opportunistic_below_discount_bps: Option<f64>,
    #[serde(default = "default_fee_bps")]
    pub fee_bps: f64,
    #[serde(default = "default_slippage_bps")]
    pub slippage_bps: f64,
    #[serde(default = "default_initial_cash_usd")]
    pub initial_cash_usd: f64,
}

#[derive(Debug, Serialize)]
pub struct DcaBacktestOutput {
    pub schema_version: &'static str,
    pub periods_planned: usize,
    pub periods_executed: usize,
    pub periods_skipped_band: usize,
    pub periods_skipped_cash: usize,
    pub periods_doubled: usize,
    pub initial_cash_usd: String,
    pub total_invested_usd: String,
    pub fees_paid_usd: String,
    pub units_acquired: String,
    pub average_cost_basis_usd: String,
    pub breakeven_price_usd: String,
    pub final_close_usd: String,
    pub mark_to_market_usd: String,
    pub total_return_pct: f64,
    pub vs_lumpsum_buy_hold_pct: f64,
    pub max_underlying_drawdown_pct: f64,
    pub lots: Vec<DcaLot>,
    pub equity_curve: Vec<EquityPoint>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DcaLot {
    pub index: usize,
    pub ts: String,
    pub price_usd: String,
    pub notional_usd: String,
    pub units: String,
    pub fee_usd: String,
    /// "regular" | "doubled" | "skipped-band" | "skipped-cash"
    pub kind: String,
}

pub fn run_backtest(input: DcaBacktestInput) -> Result<DcaBacktestOutput, String> {
    if input.candles.len() < 2 {
        return Err("dca backtest requires at least 2 candles".to_string());
    }
    if !input.notional_per_period_usd.is_finite() || input.notional_per_period_usd <= 0.0 {
        return Err("notional_per_period_usd must be > 0".to_string());
    }
    if input.period_candles == 0 {
        return Err("period_candles must be >= 1".to_string());
    }
    if input.offset >= input.candles.len() {
        return Err("offset must be less than the number of candles".to_string());
    }
    if !input.fee_bps.is_finite() || input.fee_bps < 0.0 {
        return Err("fee_bps must be >= 0".to_string());
    }
    if !input.slippage_bps.is_finite() || input.slippage_bps < 0.0 {
        return Err("slippage_bps must be >= 0".to_string());
    }
    for (idx, c) in input.candles.iter().enumerate() {
        for (name, value) in [
            ("open", c.open),
            ("high", c.high),
            ("low", c.low),
            ("close", c.close),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("candle {idx} {name} invalid"));
            }
        }
        if c.high < c.low || c.high < c.open || c.high < c.close {
            return Err(format!("candle {idx} high inconsistent"));
        }
        if c.low > c.open || c.low > c.close {
            return Err(format!("candle {idx} low inconsistent"));
        }
    }

    let fee_rate = input.fee_bps / 10_000.0;
    let slip_rate = input.slippage_bps / 10_000.0;
    let mut cash = input.initial_cash_usd;
    let mut units = 0.0_f64;
    let mut invested = 0.0_f64;
    let mut fees = 0.0_f64;
    let mut planned = 0usize;
    let mut executed = 0usize;
    let mut skipped_band = 0usize;
    let mut skipped_cash = 0usize;
    let mut doubled = 0usize;
    let mut lots = Vec::new();
    let mut equity_curve = Vec::with_capacity(input.candles.len());
    let mut peak_close = 0.0_f64;
    let mut max_dd = 0.0_f64;

    let skip_premium = input
        .skip_above_premium_bps
        .map(|bps| bps.max(0.0) / 10_000.0);
    let double_discount = input
        .opportunistic_below_discount_bps
        .map(|bps| bps.max(0.0) / 10_000.0);

    for (idx, candle) in input.candles.iter().enumerate() {
        if candle.close > peak_close {
            peak_close = candle.close;
        }
        if peak_close > 0.0 {
            let dd = (peak_close - candle.close) / peak_close * 100.0;
            if dd > max_dd {
                max_dd = dd;
            }
        }

        let fires =
            idx >= input.offset && (idx - input.offset).is_multiple_of(input.period_candles);
        if fires {
            planned += 1;
            let avg_basis = if units > 0.0 { invested / units } else { 0.0 };
            let mut commit_usd = input.notional_per_period_usd;
            let mut kind = "regular";

            if let (Some(bps), true) = (skip_premium, units > 0.0) {
                let limit = avg_basis * (1.0 + bps);
                if candle.close > limit {
                    skipped_band += 1;
                    lots.push(DcaLot {
                        index: idx,
                        ts: candle.ts.clone(),
                        price_usd: format!("{:.8}", candle.close),
                        notional_usd: "0.00".to_string(),
                        units: "0".to_string(),
                        fee_usd: "0.00".to_string(),
                        kind: "skipped-band".to_string(),
                    });
                    push_curve(&mut equity_curve, idx, candle, cash, units);
                    continue;
                }
            }

            if let (Some(bps), true) = (double_discount, units > 0.0) {
                let limit = avg_basis * (1.0 - bps);
                if candle.close < limit {
                    commit_usd *= 2.0;
                    kind = "doubled";
                    doubled += 1;
                }
            }

            if cash <= 0.0 {
                skipped_cash += 1;
                lots.push(DcaLot {
                    index: idx,
                    ts: candle.ts.clone(),
                    price_usd: format!("{:.8}", candle.close),
                    notional_usd: "0.00".to_string(),
                    units: "0".to_string(),
                    fee_usd: "0.00".to_string(),
                    kind: "skipped-cash".to_string(),
                });
                push_curve(&mut equity_curve, idx, candle, cash, units);
                continue;
            }

            let buy_usd = commit_usd.min(cash);
            let execution_price = candle.close * (1.0 + slip_rate);
            let principal = buy_usd / (1.0 + fee_rate);
            let fee = principal * fee_rate;
            let unit_buy = principal / execution_price;
            cash -= principal + fee;
            units += unit_buy;
            invested += principal;
            fees += fee;
            executed += 1;
            lots.push(DcaLot {
                index: idx,
                ts: candle.ts.clone(),
                price_usd: format!("{execution_price:.8}"),
                notional_usd: format!("{principal:.2}"),
                units: format!("{unit_buy:.8}"),
                fee_usd: format!("{fee:.2}"),
                kind: kind.to_string(),
            });
        }

        push_curve(&mut equity_curve, idx, candle, cash, units);
    }

    let final_close = input.candles.last().map(|c| c.close).unwrap_or(0.0);
    let mtm = cash + units * final_close;
    let total_return_pct = if input.initial_cash_usd > 0.0 {
        (mtm / input.initial_cash_usd - 1.0) * 100.0
    } else {
        0.0
    };
    let lumpsum_pct =
        if let (Some(first), Some(last)) = (input.candles.first(), input.candles.last()) {
            let buy = first.open * (1.0 + slip_rate);
            let sell = last.close * (1.0 - slip_rate);
            if buy > 0.0 {
                (sell / buy - 1.0) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        };
    let avg_basis = if units > 0.0 { invested / units } else { 0.0 };
    let breakeven = if units > 0.0 {
        (invested + fees) / units
    } else {
        0.0
    };

    let mut warnings = vec![
        "DCA backtest commits at candle close with explicit slippage; signals are not lookahead."
            .to_string(),
    ];
    if executed == 0 {
        warnings.push(
            "no buys executed; check offset, period_candles, and notional vs initial cash"
                .to_string(),
        );
    }
    if input.candles.len() < 30 {
        warnings.push("small sample: fewer than 30 candles".to_string());
    }
    if cash < 0.0 {
        warnings.push(
            "cash went negative; treat the over-commit as a financing assumption, not a real fill"
                .to_string(),
        );
    }

    Ok(DcaBacktestOutput {
        schema_version: "intents-dca-backtest/1",
        periods_planned: planned,
        periods_executed: executed,
        periods_skipped_band: skipped_band,
        periods_skipped_cash: skipped_cash,
        periods_doubled: doubled,
        initial_cash_usd: format!("{:.2}", input.initial_cash_usd),
        total_invested_usd: format!("{invested:.2}"),
        fees_paid_usd: format!("{fees:.2}"),
        units_acquired: format!("{units:.8}"),
        average_cost_basis_usd: format!("{avg_basis:.8}"),
        breakeven_price_usd: format!("{breakeven:.8}"),
        final_close_usd: format!("{final_close:.8}"),
        mark_to_market_usd: format!("{mtm:.2}"),
        total_return_pct,
        vs_lumpsum_buy_hold_pct: total_return_pct - lumpsum_pct,
        max_underlying_drawdown_pct: max_dd,
        lots,
        equity_curve,
        warnings,
    })
}

fn push_curve(curve: &mut Vec<EquityPoint>, idx: usize, candle: &Candle, cash: f64, units: f64) {
    let equity = cash + units * candle.close;
    curve.push(EquityPoint {
        index: idx,
        ts: candle.ts.clone(),
        equity_usd: format!("{equity:.2}"),
        in_position: units > 0.0,
    });
}

// -------------------- plan_dca_schedule --------------------

#[derive(Debug, Deserialize)]
pub struct DcaScheduleInput {
    pub pair: String,
    pub source_asset: String,
    pub destination_asset: String,
    pub source_chain: String,
    pub destination_chain: String,
    pub notional_per_period_usd: f64,
    /// One of: "daily", "weekly", "biweekly", "monthly", or a raw cron
    /// string (5 or 6 fields).
    pub cadence: String,
    pub total_periods: usize,
    #[serde(default = "default_max_slippage_bps")]
    pub max_slippage_bps: f64,
    #[serde(default)]
    pub start_at: Option<String>,
    #[serde(default = "default_assumed_price_usd")]
    pub assumed_price_usd: Option<f64>,
    /// "paper" (fixture solver, default) or "quote" (live solver).
    /// Always unsigned regardless.
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Defaults to USDC. The notional currency the user denominates the
    /// schedule in.
    #[serde(default = "default_currency")]
    pub notional_currency: String,
    /// Optional solver override (e.g. `near-intents`). Defaults to
    /// `fixture` in paper mode and `near-intents` in quote mode.
    #[serde(default)]
    pub solver: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DcaScheduleOutput {
    pub schema_version: &'static str,
    pub pair: String,
    pub mode: String,
    pub cadence: String,
    pub cron: String,
    pub total_periods: usize,
    pub source_asset: String,
    pub destination_asset: String,
    pub source_chain: String,
    pub destination_chain: String,
    pub notional_per_period_usd: String,
    pub total_notional_usd: String,
    pub assumed_price_usd: Option<f64>,
    pub estimated_total_units: Option<String>,
    pub max_slippage_bps: f64,
    pub solver: String,
    pub safe_to_quote: bool,
    pub risk_gates: Vec<DcaGate>,
    pub schedule: Vec<DcaPeriodPlan>,
    pub build_intent_request_template: serde_json::Value,
    pub schedule_summary: String,
    pub guardrails: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DcaGate {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct DcaPeriodPlan {
    pub index: usize,
    pub fires_at: String,
    pub notional_usd: String,
    pub estimated_units: Option<String>,
    pub note: String,
}

pub fn plan_schedule(input: DcaScheduleInput) -> Result<DcaScheduleOutput, String> {
    if input.pair.trim().is_empty() {
        return Err("pair must be non-empty".to_string());
    }
    if input.source_asset.trim().is_empty() || input.destination_asset.trim().is_empty() {
        return Err("source_asset and destination_asset must be non-empty".to_string());
    }
    if input.source_chain.trim().is_empty() || input.destination_chain.trim().is_empty() {
        return Err("source_chain and destination_chain must be non-empty".to_string());
    }
    if !input.notional_per_period_usd.is_finite() || input.notional_per_period_usd <= 0.0 {
        return Err("notional_per_period_usd must be > 0".to_string());
    }
    if input.total_periods == 0 {
        return Err("total_periods must be >= 1".to_string());
    }
    if !input.max_slippage_bps.is_finite() || input.max_slippage_bps < 0.0 {
        return Err("max_slippage_bps must be >= 0".to_string());
    }
    let mode = match input.mode.as_str() {
        "paper" | "quote" => input.mode.clone(),
        other => return Err(format!("mode must be paper or quote (got {other})")),
    };
    let solver = input.solver.clone().unwrap_or_else(|| match mode.as_str() {
        "paper" => "fixture".to_string(),
        _ => "near-intents".to_string(),
    });

    let (cron, fires_at) = cadence_to_cron(&input.cadence)?;

    let total_notional = input.notional_per_period_usd * input.total_periods as f64;
    let estimated_total_units = input
        .assumed_price_usd
        .filter(|p| *p > 0.0)
        .map(|p| total_notional / p);
    let est_units_per_period = input
        .assumed_price_usd
        .filter(|p| *p > 0.0)
        .map(|p| input.notional_per_period_usd / p);

    let schedule: Vec<DcaPeriodPlan> = (0..input.total_periods)
        .map(|i| {
            let note = if i == 0 {
                "First lot. Re-quote price before signing; the agent never signs.".to_string()
            } else {
                format!(
                    "Lot {} of {}. Verify slippage and price drift before signing.",
                    i + 1,
                    input.total_periods
                )
            };
            DcaPeriodPlan {
                index: i,
                fires_at: relative_fire_time(&fires_at, i),
                notional_usd: format!("{:.2}", input.notional_per_period_usd),
                estimated_units: est_units_per_period.map(|u| format!("{u:.8}")),
                note,
            }
        })
        .collect();

    let mut gates: Vec<DcaGate> = Vec::new();
    let big_total = total_notional > 10_000.0;
    gates.push(DcaGate {
        name: "max_slippage".to_string(),
        status: if input.max_slippage_bps <= 100.0 {
            "pass".to_string()
        } else {
            "warn".to_string()
        },
        detail: format!(
            "max_slippage_bps={} (recommend <= 100 for spot DCA)",
            input.max_slippage_bps
        ),
    });
    gates.push(DcaGate {
        name: "total_notional".to_string(),
        status: if big_total {
            "warn".to_string()
        } else {
            "pass".to_string()
        },
        detail: format!(
            "Total commit ${total_notional:.2} across {} periods",
            input.total_periods
        ),
    });
    let supported_assets = ["NEAR", "USDC", "USDT", "BTC", "ETH", "WETH", "WBTC"];
    let src_supported = supported_assets
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&input.source_asset));
    let dst_supported = supported_assets
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&input.destination_asset));
    gates.push(DcaGate {
        name: "asset_allowlist".to_string(),
        status: if src_supported && dst_supported {
            "pass".to_string()
        } else {
            "warn".to_string()
        },
        detail: format!(
            "{} → {} (intents-supported: {} → {})",
            input.source_asset, input.destination_asset, src_supported, dst_supported
        ),
    });
    gates.push(DcaGate {
        name: "unsigned_only".to_string(),
        status: "pass".to_string(),
        detail: "Wallet signature is required outside the agent.".to_string(),
    });

    let safe_to_quote = gates
        .iter()
        .all(|g| g.status == "pass" || g.status == "warn")
        && mode == "paper"
        || (mode == "quote"
            && gates
                .iter()
                .all(|g| g.status == "pass" || g.status == "warn")
            && src_supported
            && dst_supported);

    let template = serde_json::json!({
        "action": "build_intent",
        "solver": solver,
        "plan": {
            "proposal_id": format!("dca-{}-{}", slug(&input.pair), input.cadence),
            "legs": [
                {
                    "kind": "swap",
                    "chain": input.source_chain,
                    "from_token": {
                        "symbol": input.source_asset,
                        "address": null,
                        "chain": input.source_chain,
                        "amount": format!("{:.6}", input.notional_per_period_usd),
                        "value_usd": format!("{:.2}", input.notional_per_period_usd)
                    },
                    "to_token": {
                        "symbol": input.destination_asset,
                        "address": null,
                        "chain": input.destination_chain,
                        "amount": est_units_per_period
                            .map(|u| format!("{u:.6}"))
                            .unwrap_or_else(|| "TBD".to_string()),
                        "value_usd": format!("{:.2}", input.notional_per_period_usd)
                    },
                    "description": format!(
                        "DCA lot for {}: swap {} {} -> {} via NEAR Intents",
                        input.pair, input.notional_per_period_usd, input.source_asset, input.destination_asset
                    )
                }
            ],
            "expected_out": {
                "symbol": input.destination_asset,
                "address": null,
                "chain": input.destination_chain,
                "amount": est_units_per_period
                    .map(|u| format!("{u:.6}"))
                    .unwrap_or_else(|| "TBD".to_string()),
                "value_usd": format!("{:.2}", input.notional_per_period_usd)
            },
            "expected_cost_usd": format!("{:.2}", input.notional_per_period_usd)
        }
    });

    let summary = format!(
        "DCA ${:.2} {} -> {} every {} for {} periods (~${:.2}); cron `{}`",
        input.notional_per_period_usd,
        input.source_asset,
        input.destination_asset,
        input.cadence,
        input.total_periods,
        total_notional,
        cron
    );

    let guardrails = vec![
        "Per-period intent must be re-quoted before user signs; agent never signs.".to_string(),
        "If solver returns empty quote, skip the period and journal the miss.".to_string(),
        "If the live close drifts more than max_slippage_bps from the assumed price, pause and re-plan.".to_string(),
        "Honor `cooldown_minutes` from project config; do not chain quotes back-to-back without operator confirmation.".to_string(),
        "Cron is advisory metadata, not a daemon — IronClaw schedules each tick separately.".to_string(),
    ];

    let mut warnings: Vec<String> = Vec::new();
    if !src_supported {
        warnings.push(format!(
            "source asset '{}' is not in the default NEAR Intents allowlist; verify route support",
            input.source_asset
        ));
    }
    if !dst_supported {
        warnings.push(format!(
            "destination asset '{}' is not in the default NEAR Intents allowlist; verify route support",
            input.destination_asset
        ));
    }
    if input.assumed_price_usd.is_none() && mode == "quote" {
        warnings.push(
            "live quote mode without assumed_price_usd; estimated_units will be empty until the solver replies"
                .to_string(),
        );
    }

    Ok(DcaScheduleOutput {
        schema_version: "intents-dca-schedule/1",
        pair: input.pair,
        mode,
        cadence: input.cadence.clone(),
        cron: cron.clone(),
        total_periods: input.total_periods,
        source_asset: input.source_asset,
        destination_asset: input.destination_asset,
        source_chain: input.source_chain,
        destination_chain: input.destination_chain,
        notional_per_period_usd: format!("{:.2}", input.notional_per_period_usd),
        total_notional_usd: format!("{total_notional:.2}"),
        assumed_price_usd: input.assumed_price_usd,
        estimated_total_units: estimated_total_units.map(|u| format!("{u:.8}")),
        max_slippage_bps: input.max_slippage_bps,
        solver,
        safe_to_quote,
        risk_gates: gates,
        schedule,
        build_intent_request_template: template,
        schedule_summary: summary,
        guardrails,
        warnings,
    })
}

fn cadence_to_cron(cadence: &str) -> Result<(String, String), String> {
    match cadence {
        "daily" => Ok(("0 12 * * *".to_string(), "+1 day".to_string())),
        "weekly" => Ok(("0 12 * * 1".to_string(), "+1 week".to_string())),
        "biweekly" => Ok(("0 12 1,15 * *".to_string(), "+2 weeks".to_string())),
        "monthly" => Ok(("0 12 1 * *".to_string(), "+1 month".to_string())),
        other => {
            let fields: Vec<&str> = other.split_whitespace().collect();
            if fields.len() == 5 || fields.len() == 6 {
                Ok((other.to_string(), "per-cron".to_string()))
            } else {
                Err(format!(
                    "cadence must be one of daily/weekly/biweekly/monthly or a 5-or-6-field cron (got '{other}')"
                ))
            }
        }
    }
}

fn relative_fire_time(unit: &str, idx: usize) -> String {
    if idx == 0 {
        return "first cron tick after start".to_string();
    }
    format!("{idx} ticks after start ({unit} cadence)")
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs(closes: &[f64]) -> Vec<Candle> {
        closes
            .iter()
            .enumerate()
            .map(|(i, c)| Candle {
                ts: format!("2026-01-{:02}T00:00:00Z", i + 1),
                open: *c,
                high: c * 1.01,
                low: c * 0.99,
                close: *c,
                volume: 1_000.0,
            })
            .collect()
    }

    #[test]
    fn dca_buys_at_each_period() {
        let out = run_backtest(DcaBacktestInput {
            candles: cs(&[10.0, 11.0, 9.0, 12.0, 10.5, 11.5, 13.0, 14.0, 12.5, 11.0]),
            notional_per_period_usd: 100.0,
            period_candles: 1,
            offset: 0,
            skip_above_premium_bps: None,
            opportunistic_below_discount_bps: None,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            initial_cash_usd: 2_000.0,
        })
        .unwrap();
        assert_eq!(out.periods_planned, 10);
        assert_eq!(out.periods_executed, 10);
        assert_eq!(out.lots.len(), 10);
        assert_eq!(out.schema_version, "intents-dca-backtest/1");
        let total_invested: f64 = out.total_invested_usd.parse().unwrap();
        assert!((total_invested - 1_000.0).abs() < 0.01);
    }

    #[test]
    fn dca_band_skips_above_premium() {
        let out = run_backtest(DcaBacktestInput {
            candles: cs(&[10.0, 10.0, 10.0, 50.0, 10.0, 10.0, 10.0, 50.0, 10.0, 10.0]),
            notional_per_period_usd: 100.0,
            period_candles: 1,
            offset: 0,
            // 100% above avg = skip
            skip_above_premium_bps: Some(10_000.0),
            opportunistic_below_discount_bps: None,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            initial_cash_usd: 2_000.0,
        })
        .unwrap();
        assert!(out.periods_skipped_band >= 1);
        assert!(out.periods_executed < 10);
    }

    #[test]
    fn dca_doubles_at_discount() {
        let out = run_backtest(DcaBacktestInput {
            candles: cs(&[10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 5.0, 5.0, 5.0]),
            notional_per_period_usd: 100.0,
            period_candles: 1,
            offset: 0,
            skip_above_premium_bps: None,
            // 30% below avg triggers double-up
            opportunistic_below_discount_bps: Some(3_000.0),
            fee_bps: 0.0,
            slippage_bps: 0.0,
            initial_cash_usd: 2_000.0,
        })
        .unwrap();
        assert!(out.periods_doubled >= 1);
    }

    #[test]
    fn dca_period_skips_until_offset() {
        let out = run_backtest(DcaBacktestInput {
            candles: cs(&[10.0; 12]),
            notional_per_period_usd: 100.0,
            period_candles: 3,
            offset: 1,
            skip_above_premium_bps: None,
            opportunistic_below_discount_bps: None,
            fee_bps: 0.0,
            slippage_bps: 0.0,
            initial_cash_usd: 2_000.0,
        })
        .unwrap();
        // candles fired at idx 1, 4, 7, 10 → 4 lots
        assert_eq!(out.periods_executed, 4);
    }

    #[test]
    fn schedule_weekly_emits_cron_and_template() {
        let out = plan_schedule(DcaScheduleInput {
            pair: "NEAR/USDC".to_string(),
            source_asset: "USDC".to_string(),
            destination_asset: "NEAR".to_string(),
            source_chain: "near".to_string(),
            destination_chain: "near".to_string(),
            notional_per_period_usd: 100.0,
            cadence: "weekly".to_string(),
            total_periods: 26,
            max_slippage_bps: 50.0,
            start_at: None,
            assumed_price_usd: Some(3.0),
            mode: "paper".to_string(),
            notional_currency: "USDC".to_string(),
            solver: None,
        })
        .unwrap();
        assert_eq!(out.schema_version, "intents-dca-schedule/1");
        assert_eq!(out.cron, "0 12 * * 1");
        assert_eq!(out.solver, "fixture");
        assert_eq!(out.schedule.len(), 26);
        assert!(out.safe_to_quote);
        assert!(out.estimated_total_units.is_some());
        assert!(
            out.build_intent_request_template
                .pointer("/plan/legs/0/kind")
                .and_then(|v| v.as_str())
                == Some("swap")
        );
    }

    #[test]
    fn schedule_rejects_bad_cadence() {
        let err = plan_schedule(DcaScheduleInput {
            pair: "NEAR/USDC".to_string(),
            source_asset: "USDC".to_string(),
            destination_asset: "NEAR".to_string(),
            source_chain: "near".to_string(),
            destination_chain: "near".to_string(),
            notional_per_period_usd: 100.0,
            cadence: "fortnightly".to_string(),
            total_periods: 12,
            max_slippage_bps: 50.0,
            start_at: None,
            assumed_price_usd: Some(3.0),
            mode: "paper".to_string(),
            notional_currency: "USDC".to_string(),
            solver: None,
        })
        .unwrap_err();
        assert!(err.contains("cadence"));
    }

    #[test]
    fn schedule_quote_mode_picks_near_intents() {
        let out = plan_schedule(DcaScheduleInput {
            pair: "NEAR/USDC".to_string(),
            source_asset: "USDC".to_string(),
            destination_asset: "NEAR".to_string(),
            source_chain: "near".to_string(),
            destination_chain: "near".to_string(),
            notional_per_period_usd: 50.0,
            cadence: "daily".to_string(),
            total_periods: 7,
            max_slippage_bps: 50.0,
            start_at: None,
            assumed_price_usd: Some(3.0),
            mode: "quote".to_string(),
            notional_currency: "USDC".to_string(),
            solver: None,
        })
        .unwrap();
        assert_eq!(out.solver, "near-intents");
        assert_eq!(out.cron, "0 12 * * *");
    }
}
