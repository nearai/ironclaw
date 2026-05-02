//! Trial-mode planning for users who want to rehearse NEAR Intents
//! trading with a nominal amount of NEAR.
//!
//! This module does not sign, wrap, deposit, quote, or execute. It turns
//! a small user budget into a concrete paper/quote workflow: strategy
//! candidates to backtest, funding caveats, risk gates, and a
//! `build_intent` request the skill can run in fixture or live-quote
//! mode after explicit user approval.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::types::{MovementLeg, MovementPlan, ProjectConfig, TokenAmount};

#[derive(Debug, Clone, Deserialize)]
pub struct NearTrialPlanInput {
    #[serde(default)]
    pub near_account_id: Option<String>,
    #[serde(default = "default_trial_mode")]
    pub mode: String,
    #[serde(default = "default_pair")]
    pub pair: String,
    #[serde(default = "default_nominal_near")]
    pub nominal_near: f64,
    #[serde(default)]
    pub max_trade_near: Option<f64>,
    #[serde(default = "default_assumed_near_usd")]
    pub assumed_near_usd: f64,
    #[serde(default = "default_max_slippage_bps")]
    pub max_slippage_bps: u16,
    #[serde(default)]
    pub selected_strategy_id: Option<String>,
    #[serde(default)]
    pub backtest_suite: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct NearTrialPlan {
    pub schema_version: &'static str,
    pub mode: String,
    pub pair: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub near_account_id: Option<String>,
    pub nominal_near: f64,
    pub max_trade_near: f64,
    pub assumed_near_usd: f64,
    pub max_trade_usd: f64,
    pub safe_to_quote: bool,
    pub safe_to_execute: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended_strategy_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_strategy_id: Option<String>,
    pub strategy_menu: Vec<TrialStrategyCandidate>,
    pub setup_steps: Vec<TrialSetupStep>,
    pub risk_gates: Vec<TrialRiskGate>,
    pub movement_plan: MovementPlan,
    pub build_intent_request: serde_json::Value,
    pub docs: Vec<TrialDocRef>,
    pub warnings: Vec<String>,
    pub next_action: String,
}

#[derive(Debug, Serialize)]
pub struct TrialStrategyCandidate {
    pub id: String,
    pub kind: String,
    pub position_size_pct: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss_bps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit_bps: Option<f64>,
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct TrialSetupStep {
    pub order: usize,
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct TrialRiskGate {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct TrialDocRef {
    pub label: String,
    pub url: String,
}

pub fn plan(input: NearTrialPlanInput) -> Result<NearTrialPlan, String> {
    let mode = normalize_mode(&input.mode)?;
    let pair = normalize_pair(&input.pair);
    let nominal_near = clean_positive(input.nominal_near, "nominal_near")?;
    let assumed_near_usd = clean_positive(input.assumed_near_usd, "assumed_near_usd")?;
    let max_trade_near = input
        .max_trade_near
        .unwrap_or_else(|| default_max_trade_near(nominal_near));
    let max_trade_near = clean_positive(max_trade_near, "max_trade_near")?;
    let max_slippage_bps = input.max_slippage_bps.min(1_000);
    let max_trade_usd = round4(max_trade_near * assumed_near_usd);
    let min_out_usd = round4(max_trade_usd * (1.0 - max_slippage_bps as f64 / 10_000.0));

    let strategy_menu = default_strategy_menu();
    let (recommended_strategy_id, strategy_gate) = strategy_gate(
        input.selected_strategy_id.as_deref(),
        input.backtest_suite.as_ref(),
        &strategy_menu,
    );

    let mut risk_gates = vec![
        TrialRiskGate {
            name: "nominal-budget".to_string(),
            status: if nominal_near <= 1.0 { "pass" } else { "warn" }.to_string(),
            detail: format!(
                "Trial wallet budget is {:.4} NEAR. Keep this wallet small and separate from long-term holdings.",
                nominal_near
            ),
        },
        TrialRiskGate {
            name: "trade-cap".to_string(),
            status: if max_trade_near <= nominal_near {
                "pass"
            } else {
                "fail"
            }
            .to_string(),
            detail: format!(
                "Single trial trade is capped at {:.4} NEAR, estimated at ${:.4}.",
                max_trade_near, max_trade_usd
            ),
        },
        TrialRiskGate {
            name: "small-route-liquidity".to_string(),
            status: if max_trade_usd >= 1.0 { "pass" } else { "warn" }.to_string(),
            detail:
                "Very small quote sizes may be refused or dominated by route/bridge minimums; paper mode still works."
                    .to_string(),
        },
        TrialRiskGate {
            name: "wallet-account".to_string(),
            status: if input.near_account_id.is_some() {
                "pass"
            } else {
                "warn"
            }
            .to_string(),
            detail: input
                .near_account_id
                .as_ref()
                .map(|account| format!("Use {account} only after you have reviewed the unsigned payload."))
                .unwrap_or_else(|| {
                    "No NEAR account id supplied; paper mode can run, live quote setup should record the signer account."
                        .to_string()
                }),
        },
        TrialRiskGate {
            name: "native-near-wrap".to_string(),
            status: "warn".to_string(),
            detail:
                "The verifier does not accept raw native NEAR deposits; wrap NEAR to wNEAR before verifier funding."
                    .to_string(),
        },
        strategy_gate,
        TrialRiskGate {
            name: "signing-boundary".to_string(),
            status: if mode == "execution" { "blocked" } else { "pass" }.to_string(),
            detail: if mode == "execution" {
                "Execution cannot be autonomous here: IronClaw builds unsigned payloads, then your wallet must sign outside the agent."
                    .to_string()
            } else {
                "This action returns only plans and unsigned build requests; it cannot spend funds."
                    .to_string()
            },
        },
    ];

    if max_slippage_bps > 100 {
        risk_gates.push(TrialRiskGate {
            name: "slippage-cap".to_string(),
            status: "warn".to_string(),
            detail: format!(
                "Max slippage is {} bps. For a nominal trial, prefer <= 100 bps unless route depth is poor.",
                max_slippage_bps
            ),
        });
    }

    let movement_plan = movement_plan(&pair, max_trade_near, max_trade_usd, min_out_usd);
    let solver = if mode == "paper" {
        "fixture"
    } else {
        "near-intents"
    };
    let build_intent_request = json!({
        "action": "build_intent",
        "solver": solver,
        "plan": movement_plan,
        "config": {
            "max_slippage_bps": max_slippage_bps,
            "auto_intent_ceiling_usd": max_trade_usd + 1.0,
            "allowed_chains": ["near"]
        }
    });

    let safe_to_quote = mode != "execution"
        && risk_gates
            .iter()
            .all(|gate| gate.status != "fail" && gate.status != "blocked");
    let safe_to_execute = false;
    let next_action = match mode.as_str() {
        "paper" => "Run backtest_suite with fresh candles, select a passing strategy, then run build_intent with solver=fixture.".to_string(),
        "quote" => "After paper gates pass, run build_intent with solver=near-intents to request an unsigned live quote; do not sign yet.".to_string(),
        _ => "Execution remains manual-wallet only: inspect the signed payload request, then decide in your wallet outside IronClaw.".to_string(),
    };

    let warnings = vec![
        "This is not financial advice and it is not an execution bot. Treat every quote as experimental."
            .to_string(),
        "Prices are estimates unless supplied from a fresh market data source; re-run before any live quote."
            .to_string(),
    ];

    Ok(NearTrialPlan {
        schema_version: "near-intents-trial-plan/1",
        mode,
        pair,
        near_account_id: input.near_account_id,
        nominal_near: round4(nominal_near),
        max_trade_near: round4(max_trade_near),
        assumed_near_usd: round4(assumed_near_usd),
        max_trade_usd,
        safe_to_quote,
        safe_to_execute,
        recommended_strategy_id,
        selected_strategy_id: input.selected_strategy_id,
        strategy_menu,
        setup_steps: setup_steps(),
        risk_gates,
        movement_plan,
        build_intent_request,
        docs: doc_refs(),
        warnings,
        next_action,
    })
}

fn movement_plan(
    pair: &str,
    max_trade_near: f64,
    max_trade_usd: f64,
    min_out_usd: f64,
) -> MovementPlan {
    let target_symbol = pair
        .split('/')
        .nth(1)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("USDC")
        .to_uppercase();
    let target_address = if target_symbol == "USDC" {
        Some("usdc.near".to_string())
    } else {
        None
    };
    MovementPlan {
        legs: vec![MovementLeg {
            kind: "swap".to_string(),
            chain: "near".to_string(),
            from_token: Some(TokenAmount {
                symbol: "wNEAR".to_string(),
                address: Some("wrap.near".to_string()),
                chain: "near".to_string(),
                amount: format!("{max_trade_near:.6}"),
                value_usd: format!("{max_trade_usd:.4}"),
            }),
            to_token: Some(TokenAmount {
                symbol: target_symbol.clone(),
                address: target_address.clone(),
                chain: "near".to_string(),
                amount: format!("{min_out_usd:.6}"),
                value_usd: format!("{min_out_usd:.4}"),
            }),
            description: format!(
                "Nominal NEAR Intents trial swap from wNEAR into {target_symbol}; user wallet signs only after reviewing the quote."
            ),
        }],
        expected_out: TokenAmount {
            symbol: target_symbol.clone(),
            address: target_address,
            chain: "near".to_string(),
            amount: format!("{min_out_usd:.6}"),
            value_usd: format!("{min_out_usd:.4}"),
        },
        expected_cost_usd: "0.00".to_string(),
        proposal_id: format!("trial-near-{}", target_symbol.to_lowercase()),
    }
}

fn strategy_gate(
    selected_strategy_id: Option<&str>,
    suite: Option<&serde_json::Value>,
    menu: &[TrialStrategyCandidate],
) -> (Option<String>, TrialRiskGate) {
    let fallback = menu.first().map(|candidate| candidate.id.clone());
    let Some(suite) = suite else {
        return (
            fallback,
            TrialRiskGate {
                name: "strategy-backtest".to_string(),
                status: "warn".to_string(),
                detail: "No backtest_suite output supplied; run the menu before a live quote."
                    .to_string(),
            },
        );
    };
    let ranked = suite
        .get("ranked")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let recommended = ranked
        .iter()
        .find(|candidate| {
            candidate
                .get("passes_basic_gate")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        })
        .or_else(|| ranked.first())
        .and_then(|candidate| candidate.get("id"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or(fallback);

    if let Some(selected) = selected_strategy_id {
        let maybe_selected = ranked.iter().find(|candidate| {
            candidate.get("id").and_then(|value| value.as_str()) == Some(selected)
        });
        let Some(candidate) = maybe_selected else {
            return (
                recommended,
                TrialRiskGate {
                    name: "strategy-backtest".to_string(),
                    status: "fail".to_string(),
                    detail: format!(
                        "Selected strategy '{selected}' was not present in the backtest suite."
                    ),
                },
            );
        };
        let passes = candidate
            .get("passes_basic_gate")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        return (
            recommended,
            TrialRiskGate {
                name: "strategy-backtest".to_string(),
                status: if passes { "pass" } else { "fail" }.to_string(),
                detail: format!(
                    "Selected strategy '{selected}' {} the basic paper gate.",
                    if passes { "passed" } else { "failed" }
                ),
            },
        );
    }

    (
        recommended.clone(),
        TrialRiskGate {
            name: "strategy-backtest".to_string(),
            status: if recommended.is_some() {
                "pass"
            } else {
                "warn"
            }
            .to_string(),
            detail: recommended
                .map(|id| format!("Recommended paper strategy: {id}."))
                .unwrap_or_else(|| "Backtest suite had no ranked candidates.".to_string()),
        },
    )
}

fn setup_steps() -> Vec<TrialSetupStep> {
    vec![
        TrialSetupStep {
            order: 1,
            name: "Use a small wallet".to_string(),
            status: "manual".to_string(),
            detail: "Fund a separate NEAR account with only the amount you are willing to test."
                .to_string(),
        },
        TrialSetupStep {
            order: 2,
            name: "Wrap NEAR".to_string(),
            status: "manual".to_string(),
            detail:
                "Convert the trial amount from native NEAR into wNEAR before depositing to the verifier."
                    .to_string(),
        },
        TrialSetupStep {
            order: 3,
            name: "Deposit to verifier".to_string(),
            status: "manual".to_string(),
            detail:
                "Deposit wNEAR into intents.near using the official verifier deposit flow; do not transfer raw NEAR directly."
                    .to_string(),
        },
        TrialSetupStep {
            order: 4,
            name: "Paper first".to_string(),
            status: "required".to_string(),
            detail:
                "Run strategy backtests and fixture intent construction before requesting a live quote."
                    .to_string(),
        },
        TrialSetupStep {
            order: 5,
            name: "Wallet review".to_string(),
            status: "required".to_string(),
            detail: "Any live quote still needs your wallet signature outside the agent.".to_string(),
        },
    ]
}

fn default_strategy_menu() -> Vec<TrialStrategyCandidate> {
    vec![
        TrialStrategyCandidate {
            id: "buy_hold_baseline".to_string(),
            kind: "buy-hold".to_string(),
            position_size_pct: 1.0,
            stop_loss_bps: None,
            take_profit_bps: None,
            note: "Baseline for comparing active signals.".to_string(),
        },
        TrialStrategyCandidate {
            id: "sma_cross_fast".to_string(),
            kind: "sma-cross".to_string(),
            position_size_pct: 0.8,
            stop_loss_bps: Some(1_200.0),
            take_profit_bps: None,
            note: "Trend-following candidate for clean spot rotations.".to_string(),
        },
        TrialStrategyCandidate {
            id: "breakout_short".to_string(),
            kind: "breakout".to_string(),
            position_size_pct: 0.8,
            stop_loss_bps: Some(1_000.0),
            take_profit_bps: None,
            note: "Looks for range breaks; sensitive to false moves.".to_string(),
        },
        TrialStrategyCandidate {
            id: "momentum_rotation".to_string(),
            kind: "momentum".to_string(),
            position_size_pct: 0.8,
            stop_loss_bps: Some(1_000.0),
            take_profit_bps: None,
            note: "Simple lookback momentum for token rotation tests.".to_string(),
        },
        TrialStrategyCandidate {
            id: "mean_reversion_range".to_string(),
            kind: "mean-reversion".to_string(),
            position_size_pct: 0.5,
            stop_loss_bps: Some(700.0),
            take_profit_bps: None,
            note: "Range strategy for pullback entries.".to_string(),
        },
        TrialStrategyCandidate {
            id: "rsi_reversion".to_string(),
            kind: "rsi-mean-reversion".to_string(),
            position_size_pct: 0.5,
            stop_loss_bps: Some(700.0),
            take_profit_bps: None,
            note: "Oversold/exit threshold candidate.".to_string(),
        },
    ]
}

fn doc_refs() -> Vec<TrialDocRef> {
    vec![
        TrialDocRef {
            label: "NEAR Intents deposit/withdrawal service".to_string(),
            url: "https://docs.near-intents.org/integration/market-makers/deposit-withdrawal-service"
                .to_string(),
        },
        TrialDocRef {
            label: "NEAR Intents verifier deposits".to_string(),
            url: "https://docs.near-intents.org/near-intents/market-makers/verifier/deposits-and-withdrawals/deposits"
                .to_string(),
        },
        TrialDocRef {
            label: "NEAR Intents solver relay API".to_string(),
            url: "https://docs.near-intents.org/near-intents/market-makers/bus/solver-relay"
                .to_string(),
        },
    ]
}

fn normalize_mode(mode: &str) -> Result<String, String> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "" | "paper" => Ok("paper".to_string()),
        "quote" | "live-quote" => Ok("quote".to_string()),
        "execution" | "execute" => Ok("execution".to_string()),
        other => Err(format!(
            "unknown near trial mode '{other}'; expected paper, quote, or execution"
        )),
    }
}

fn normalize_pair(pair: &str) -> String {
    let trimmed = pair.trim();
    if trimmed.is_empty() {
        default_pair()
    } else if trimmed.contains('/') {
        trimmed.to_uppercase()
    } else {
        format!("NEAR/{}", trimmed.to_uppercase())
    }
}

fn clean_positive(value: f64, name: &str) -> Result<f64, String> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(format!("{name} must be a positive finite number"))
    }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn default_trial_mode() -> String {
    "paper".to_string()
}

fn default_pair() -> String {
    "NEAR/USDC".to_string()
}

fn default_nominal_near() -> f64 {
    0.25
}

fn default_max_trade_near(nominal_near: f64) -> f64 {
    if nominal_near <= 0.01 {
        nominal_near
    } else {
        (nominal_near * 0.2).max(0.01).min(nominal_near)
    }
}

fn default_assumed_near_usd() -> f64 {
    3.0
}

fn default_max_slippage_bps() -> u16 {
    ProjectConfig::default().max_slippage_bps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trial_plan_defaults_to_paper_fixture_request() {
        let plan = plan(NearTrialPlanInput {
            near_account_id: Some("alice.near".to_string()),
            mode: "paper".to_string(),
            pair: "NEAR/USDC".to_string(),
            nominal_near: 0.25,
            max_trade_near: Some(0.05),
            assumed_near_usd: 3.0,
            max_slippage_bps: 50,
            selected_strategy_id: None,
            backtest_suite: None,
        })
        .unwrap();
        assert_eq!(plan.schema_version, "near-intents-trial-plan/1");
        assert_eq!(plan.build_intent_request["solver"], "fixture");
        assert!(!plan.safe_to_execute);
        assert_eq!(plan.movement_plan.expected_out.symbol, "USDC");
    }

    #[test]
    fn quote_mode_uses_near_intents_solver() {
        let plan = plan(NearTrialPlanInput {
            mode: "quote".to_string(),
            ..NearTrialPlanInput {
                near_account_id: None,
                mode: "paper".to_string(),
                pair: "USDC".to_string(),
                nominal_near: 1.0,
                max_trade_near: Some(0.1),
                assumed_near_usd: 3.0,
                max_slippage_bps: 50,
                selected_strategy_id: None,
                backtest_suite: None,
            }
        })
        .unwrap();
        assert_eq!(plan.mode, "quote");
        assert_eq!(plan.build_intent_request["solver"], "near-intents");
    }

    #[test]
    fn over_budget_trade_fails_gate() {
        let plan = plan(NearTrialPlanInput {
            nominal_near: 0.1,
            max_trade_near: Some(0.2),
            ..NearTrialPlanInput {
                near_account_id: None,
                mode: "paper".to_string(),
                pair: "NEAR/USDC".to_string(),
                nominal_near: 0.25,
                max_trade_near: None,
                assumed_near_usd: 3.0,
                max_slippage_bps: 50,
                selected_strategy_id: None,
                backtest_suite: None,
            }
        })
        .unwrap();
        assert!(plan
            .risk_gates
            .iter()
            .any(|gate| { gate.name == "trade-cap" && gate.status == "fail" }));
        assert!(!plan.safe_to_quote);
    }
}
