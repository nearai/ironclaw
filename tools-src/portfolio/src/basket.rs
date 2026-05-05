//! Multi-asset (basket) dollar-cost averaging.
//!
//! Extends the single-asset `plan_dca_schedule` to a fixed-proportion
//! accumulation across multiple NEAR-Intents-supported destination
//! assets in one cadence. Per-period total notional splits into N
//! legs by weight; each leg becomes one swap in the per-period
//! `build_intent` template.
//!
//! Pure deterministic compute — no HTTP, no signing, no key access.
//! All output is unsigned. Signing remains a wallet action outside
//! the agent.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub struct BasketDcaScheduleInput {
    pub allocations: Vec<BasketAllocation>,
    pub source_asset: String,
    pub source_chain: String,
    /// Total notional USD committed per period across all legs.
    pub notional_per_period_usd: f64,
    /// One of: "daily", "weekly", "biweekly", "monthly", or a 5-or-6
    /// field cron string.
    pub cadence: String,
    pub total_periods: usize,
    #[serde(default = "default_max_slippage_bps")]
    pub max_slippage_bps: f64,
    /// Optional per-asset assumed price (USD) for sizing previews.
    /// Map of `destination_asset` → price.
    #[serde(default)]
    pub assumed_prices_usd: std::collections::BTreeMap<String, f64>,
    /// "paper" (fixture solver, default) or "quote" (live solver).
    /// Always unsigned regardless.
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_currency")]
    pub notional_currency: String,
    #[serde(default)]
    pub solver: Option<String>,
    /// Optional ISO-8601 start timestamp for the schedule.
    #[serde(default)]
    pub start_at: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BasketAllocation {
    pub destination_asset: String,
    pub destination_chain: String,
    /// Weight as a percentage, 0..=100. The full allocation array must
    /// sum to 100 (with a small tolerance) or the request is rejected.
    pub weight_pct: f64,
}

fn default_max_slippage_bps() -> f64 {
    50.0
}

fn default_mode() -> String {
    "paper".to_string()
}

fn default_currency() -> String {
    "USDC".to_string()
}

#[derive(Debug, Serialize)]
pub struct BasketDcaScheduleOutput {
    pub schema_version: &'static str,
    pub source_asset: String,
    pub source_chain: String,
    pub mode: String,
    pub cadence: String,
    pub cron: String,
    pub total_periods: usize,
    pub notional_per_period_usd: String,
    pub total_notional_usd: String,
    pub max_slippage_bps: f64,
    pub solver: String,
    pub legs: Vec<BasketLegPlan>,
    pub schedule_summary: String,
    pub risk_gates: Vec<BasketGate>,
    pub safe_to_quote: bool,
    pub build_intent_request_template: Value,
    pub guardrails: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BasketLegPlan {
    pub destination_asset: String,
    pub destination_chain: String,
    pub weight_pct: f64,
    pub notional_usd_per_period: String,
    pub assumed_price_usd: Option<f64>,
    pub estimated_units_per_period: Option<String>,
    pub estimated_total_units: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BasketGate {
    pub name: String,
    pub status: String,
    pub detail: String,
}

const INTENTS_SUPPORTED: &[&str] = &["NEAR", "USDC", "USDT", "BTC", "WBTC", "ETH", "WETH"];

pub fn plan(input: BasketDcaScheduleInput) -> Result<BasketDcaScheduleOutput, String> {
    if input.allocations.is_empty() {
        return Err("basket schedule requires at least one allocation".to_string());
    }
    if input.allocations.len() > 12 {
        return Err("basket schedule supports at most 12 allocations".to_string());
    }
    if !input.notional_per_period_usd.is_finite() || input.notional_per_period_usd <= 0.0 {
        return Err("notional_per_period_usd must be > 0".to_string());
    }
    if input.total_periods == 0 {
        return Err("total_periods must be >= 1".to_string());
    }
    let mode = match input.mode.as_str() {
        "paper" | "quote" => input.mode.clone(),
        other => return Err(format!("mode must be paper or quote (got {other})")),
    };
    let solver = input.solver.clone().unwrap_or_else(|| match mode.as_str() {
        "paper" => "fixture".to_string(),
        _ => "near-intents".to_string(),
    });

    let mut weight_sum = 0.0_f64;
    let mut seen = std::collections::BTreeSet::new();
    for a in &input.allocations {
        if a.destination_asset.trim().is_empty() {
            return Err("allocation destination_asset must be non-empty".to_string());
        }
        if a.destination_chain.trim().is_empty() {
            return Err("allocation destination_chain must be non-empty".to_string());
        }
        if !a.weight_pct.is_finite() || a.weight_pct <= 0.0 || a.weight_pct > 100.0 {
            return Err("allocation weight_pct must be in (0, 100]".to_string());
        }
        if !seen.insert(a.destination_asset.to_uppercase()) {
            return Err(format!(
                "allocation '{}' duplicated; merge weights before submitting",
                a.destination_asset
            ));
        }
        weight_sum += a.weight_pct;
    }
    if (weight_sum - 100.0).abs() > 0.5 {
        return Err(format!(
            "allocation weights must sum to 100 (got {weight_sum:.2})"
        ));
    }

    let (cron, _fires) = cadence_to_cron(&input.cadence)?;

    let total_notional = input.notional_per_period_usd * input.total_periods as f64;
    let mut legs = Vec::with_capacity(input.allocations.len());
    let mut warnings = Vec::new();
    let mut all_supported = true;

    for a in &input.allocations {
        let leg_notional = input.notional_per_period_usd * a.weight_pct / 100.0;
        let assumed_price = input
            .assumed_prices_usd
            .get(&a.destination_asset)
            .copied()
            .filter(|p| p.is_finite() && *p > 0.0);
        let est_units = assumed_price.map(|p| leg_notional / p);
        let est_total = est_units.map(|u| u * input.total_periods as f64);
        if !INTENTS_SUPPORTED
            .iter()
            .any(|s| s.eq_ignore_ascii_case(&a.destination_asset))
        {
            all_supported = false;
            warnings.push(format!(
                "destination '{}' is outside the default NEAR Intents allowlist; verify route",
                a.destination_asset
            ));
        }
        legs.push(BasketLegPlan {
            destination_asset: a.destination_asset.clone(),
            destination_chain: a.destination_chain.clone(),
            weight_pct: a.weight_pct,
            notional_usd_per_period: format!("{leg_notional:.2}"),
            assumed_price_usd: assumed_price,
            estimated_units_per_period: est_units.map(|u| format!("{u:.8}")),
            estimated_total_units: est_total.map(|u| format!("{u:.8}")),
        });
    }

    if !INTENTS_SUPPORTED
        .iter()
        .any(|s| s.eq_ignore_ascii_case(&input.source_asset))
    {
        warnings.push(format!(
            "source asset '{}' is outside the default NEAR Intents allowlist",
            input.source_asset
        ));
        all_supported = false;
    }

    let mut gates = vec![
        BasketGate {
            name: "weights_sum_to_100".to_string(),
            status: "pass".to_string(),
            detail: format!("sum={weight_sum:.2}"),
        },
        BasketGate {
            name: "max_slippage".to_string(),
            status: if input.max_slippage_bps <= 100.0 {
                "pass".to_string()
            } else {
                "warn".to_string()
            },
            detail: format!("max_slippage_bps={}", input.max_slippage_bps),
        },
        BasketGate {
            name: "asset_allowlist".to_string(),
            status: if all_supported { "pass" } else { "warn" }.to_string(),
            detail: format!("all assets in NEAR Intents allowlist: {all_supported}"),
        },
        BasketGate {
            name: "unsigned_only".to_string(),
            status: "pass".to_string(),
            detail: "Wallet signature required outside the agent.".to_string(),
        },
    ];

    let big_total = total_notional > 50_000.0;
    gates.push(BasketGate {
        name: "total_notional".to_string(),
        status: if big_total { "warn" } else { "pass" }.to_string(),
        detail: format!(
            "${total_notional:.2} across {} periods",
            input.total_periods
        ),
    });

    let safe_to_quote =
        gates.iter().all(|g| g.status != "fail") && (mode == "paper" || all_supported);

    let template_legs: Vec<Value> = legs
        .iter()
        .map(|leg| {
            json!({
                "kind": "swap",
                "chain": input.source_chain,
                "from_token": {
                    "symbol": input.source_asset,
                    "address": null,
                    "chain": input.source_chain,
                    "amount": leg.notional_usd_per_period,
                    "value_usd": leg.notional_usd_per_period
                },
                "to_token": {
                    "symbol": leg.destination_asset,
                    "address": null,
                    "chain": leg.destination_chain,
                    "amount": leg.estimated_units_per_period.clone().unwrap_or_else(|| "TBD".to_string()),
                    "value_usd": leg.notional_usd_per_period
                },
                "description": format!(
                    "Basket DCA leg ({:.0}%): {} {} -> {} via NEAR Intents",
                    leg.weight_pct,
                    leg.notional_usd_per_period,
                    input.source_asset,
                    leg.destination_asset
                )
            })
        })
        .collect();

    let template = json!({
        "action": "build_intent",
        "solver": solver,
        "plan": {
            "proposal_id": format!(
                "basket-dca-{}-{}",
                slug(&input.source_asset),
                input.cadence
            ),
            "legs": template_legs,
            "expected_out": null,
            "expected_cost_usd": format!("{:.2}", input.notional_per_period_usd)
        }
    });

    let summary = format!(
        "Basket DCA ${:.2} from {} into {} legs every {} for {} periods (~${:.2}); cron `{cron}`",
        input.notional_per_period_usd,
        input.source_asset,
        legs.len(),
        input.cadence,
        input.total_periods,
        total_notional,
    );

    let guardrails = vec![
        "Per-period bundle contains all legs; the engine should re-quote each leg independently.".to_string(),
        "Skip a leg if its solver returns empty quote or slippage exceeds max_slippage_bps; journal the miss and proceed with the rest.".to_string(),
        "If any leg fails, the period is partial — re-run risk gates against the remaining cash before the next tick.".to_string(),
        "Do not auto-rebalance the basket weights mid-run; treat weight changes as a new schedule.".to_string(),
        "Cron is advisory metadata; IronClaw schedules each tick independently.".to_string(),
    ];

    let _ = input.start_at;
    Ok(BasketDcaScheduleOutput {
        schema_version: "intents-basket-dca-schedule/1",
        source_asset: input.source_asset,
        source_chain: input.source_chain,
        mode,
        cadence: input.cadence.clone(),
        cron,
        total_periods: input.total_periods,
        notional_per_period_usd: format!("{:.2}", input.notional_per_period_usd),
        total_notional_usd: format!("{total_notional:.2}"),
        max_slippage_bps: input.max_slippage_bps,
        solver,
        legs,
        schedule_summary: summary,
        risk_gates: gates,
        safe_to_quote,
        build_intent_request_template: template,
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

    #[test]
    fn basket_plan_three_assets_weekly() {
        let mut prices = std::collections::BTreeMap::new();
        prices.insert("NEAR".to_string(), 3.0);
        prices.insert("BTC".to_string(), 110_000.0);
        prices.insert("ETH".to_string(), 3_000.0);
        let out = plan(BasketDcaScheduleInput {
            allocations: vec![
                BasketAllocation {
                    destination_asset: "NEAR".to_string(),
                    destination_chain: "near".to_string(),
                    weight_pct: 60.0,
                },
                BasketAllocation {
                    destination_asset: "BTC".to_string(),
                    destination_chain: "bitcoin".to_string(),
                    weight_pct: 30.0,
                },
                BasketAllocation {
                    destination_asset: "ETH".to_string(),
                    destination_chain: "ethereum".to_string(),
                    weight_pct: 10.0,
                },
            ],
            source_asset: "USDC".to_string(),
            source_chain: "near".to_string(),
            notional_per_period_usd: 300.0,
            cadence: "weekly".to_string(),
            total_periods: 26,
            max_slippage_bps: 50.0,
            assumed_prices_usd: prices,
            mode: "paper".to_string(),
            notional_currency: "USDC".to_string(),
            solver: None,
            start_at: None,
        })
        .unwrap();

        assert_eq!(out.schema_version, "intents-basket-dca-schedule/1");
        assert_eq!(out.cron, "0 12 * * 1");
        assert_eq!(out.legs.len(), 3);
        assert_eq!(out.legs[0].weight_pct, 60.0);
        assert_eq!(out.legs[0].notional_usd_per_period, "180.00");
        assert_eq!(out.legs[1].notional_usd_per_period, "90.00");
        assert_eq!(out.legs[2].notional_usd_per_period, "30.00");
        assert!(out.safe_to_quote);
        assert_eq!(out.solver, "fixture");
        let template_legs = out.build_intent_request_template["plan"]["legs"]
            .as_array()
            .unwrap();
        assert_eq!(template_legs.len(), 3);
    }

    #[test]
    fn basket_rejects_weights_off_100() {
        let err = plan(BasketDcaScheduleInput {
            allocations: vec![
                BasketAllocation {
                    destination_asset: "NEAR".to_string(),
                    destination_chain: "near".to_string(),
                    weight_pct: 70.0,
                },
                BasketAllocation {
                    destination_asset: "BTC".to_string(),
                    destination_chain: "bitcoin".to_string(),
                    weight_pct: 20.0,
                },
            ],
            source_asset: "USDC".to_string(),
            source_chain: "near".to_string(),
            notional_per_period_usd: 100.0,
            cadence: "weekly".to_string(),
            total_periods: 12,
            max_slippage_bps: 50.0,
            assumed_prices_usd: Default::default(),
            mode: "paper".to_string(),
            notional_currency: "USDC".to_string(),
            solver: None,
            start_at: None,
        })
        .unwrap_err();
        assert!(err.contains("100"));
    }

    #[test]
    fn basket_rejects_duplicates() {
        let err = plan(BasketDcaScheduleInput {
            allocations: vec![
                BasketAllocation {
                    destination_asset: "NEAR".to_string(),
                    destination_chain: "near".to_string(),
                    weight_pct: 50.0,
                },
                BasketAllocation {
                    destination_asset: "near".to_string(),
                    destination_chain: "near".to_string(),
                    weight_pct: 50.0,
                },
            ],
            source_asset: "USDC".to_string(),
            source_chain: "near".to_string(),
            notional_per_period_usd: 100.0,
            cadence: "weekly".to_string(),
            total_periods: 12,
            max_slippage_bps: 50.0,
            assumed_prices_usd: Default::default(),
            mode: "paper".to_string(),
            notional_currency: "USDC".to_string(),
            solver: None,
            start_at: None,
        })
        .unwrap_err();
        assert!(err.contains("duplicat"));
    }

    #[test]
    fn basket_warns_on_off_allowlist_asset() {
        let out = plan(BasketDcaScheduleInput {
            allocations: vec![
                BasketAllocation {
                    destination_asset: "NEAR".to_string(),
                    destination_chain: "near".to_string(),
                    weight_pct: 50.0,
                },
                BasketAllocation {
                    destination_asset: "DOGE".to_string(),
                    destination_chain: "dogechain".to_string(),
                    weight_pct: 50.0,
                },
            ],
            source_asset: "USDC".to_string(),
            source_chain: "near".to_string(),
            notional_per_period_usd: 100.0,
            cadence: "weekly".to_string(),
            total_periods: 12,
            max_slippage_bps: 50.0,
            assumed_prices_usd: Default::default(),
            mode: "paper".to_string(),
            notional_currency: "USDC".to_string(),
            solver: None,
            start_at: None,
        })
        .unwrap();
        assert!(out.warnings.iter().any(|w| w.contains("DOGE")));
        // paper mode still allows safe_to_quote even with off-allowlist assets
        assert!(out.safe_to_quote);
    }
}
