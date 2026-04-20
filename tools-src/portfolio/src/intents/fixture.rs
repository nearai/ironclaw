//! Fixture intent builder.
//!
//! Synthesizes a deterministic single-leg `IntentBundle` from a
//! `MovementPlan`. Used in M1 smoke tests and as the default for any
//! replay scenario that doesn't need a real solver quote.
//!
//! Output is intentionally schema-stable so M4's real solver path can
//! drop in without breaking downstream consumers (the widget, the
//! suggestion markdown, etc.).

use serde_json::json;

use crate::types::{
    parse_decimal, BoundedChecks, IntentBundle, IntentLeg, MovementPlan, ProjectConfig, TokenAmount,
};

pub fn build(plan: &MovementPlan, config: &ProjectConfig) -> Result<IntentBundle, String> {
    if plan.legs.is_empty() {
        return Err("MovementPlan has no legs".to_string());
    }

    let expected_out_value = parse_decimal(&plan.expected_out.value_usd);
    let expected_out_amount = parse_decimal(&plan.expected_out.amount);
    let slippage_factor = 1.0 - (config.max_slippage_bps as f64 / 10_000.0);
    let min_out_value = expected_out_value * slippage_factor;
    let min_out_token_amount = expected_out_amount * slippage_factor;

    let min_out = TokenAmount {
        symbol: plan.expected_out.symbol.clone(),
        address: plan.expected_out.address.clone(),
        chain: plan.expected_out.chain.clone(),
        amount: format!("{min_out_token_amount:.6}"),
        value_usd: format!("{min_out_value:.2}"),
    };

    let leg_id = format!("{}-leg-0", plan.proposal_id);
    let terminal_kind = plan
        .legs
        .last()
        .ok_or_else(|| "MovementPlan has no legs".to_string())?
        .kind
        .clone();
    let leg = IntentLeg {
        id: leg_id.clone(),
        kind: terminal_kind,
        chain: plan.expected_out.chain.clone(),
        near_intent_payload: json!({
            "kind": "fixture",
            "proposal_id": plan.proposal_id,
            "expected_out": plan.expected_out,
            "min_out": min_out,
            "schema_version": "portfolio-intent/1",
        }),
        depends_on: None,
        min_out: min_out.clone(),
        quoted_by: "fixture-solver".to_string(),
    };

    Ok(IntentBundle {
        id: format!("bundle-{}", plan.proposal_id),
        legs: vec![leg],
        total_cost_usd: plan.expected_cost_usd.clone(),
        bounded_checks: BoundedChecks {
            min_out_per_leg: vec![min_out],
            max_slippage_bps: config.max_slippage_bps,
            solver_quote_version: "fixture/1".to_string(),
        },
        // Fixture bundles use 0 (no expiry). Real solver paths set
        // expiry from the quote response. Host-side expiry enforcement
        // treats 0 as "never expires" for fixture/test bundles.
        expires_at: 0,
        signer_placeholder: "<signed-by-user>".to_string(),
        schema_version: "portfolio-intent/1".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MovementLeg;

    fn make_plan(expected_value: &str, cost: &str, proposal_id: &str) -> MovementPlan {
        MovementPlan {
            legs: vec![MovementLeg {
                kind: "deposit".to_string(),
                chain: "base".to_string(),
                from_token: None,
                to_token: None,
                description: "test deposit".to_string(),
            }],
            expected_out: TokenAmount {
                symbol: "USDC".to_string(),
                address: None,
                chain: "base".to_string(),
                amount: expected_value.to_string(),
                value_usd: expected_value.to_string(),
            },
            expected_cost_usd: cost.to_string(),
            proposal_id: proposal_id.to_string(),
        }
    }

    fn cfg(slippage_bps: u16) -> ProjectConfig {
        ProjectConfig {
            max_slippage_bps: slippage_bps,
            ..ProjectConfig::default()
        }
    }

    #[test]
    fn empty_plan_error() {
        let plan = MovementPlan {
            legs: vec![],
            expected_out: TokenAmount {
                symbol: "USDC".into(),
                address: None,
                chain: "base".into(),
                amount: "1000".into(),
                value_usd: "1000".into(),
            },
            expected_cost_usd: "0.50".into(),
            proposal_id: "p1".into(),
        };
        let result = build(&plan, &cfg(50));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no legs"));
    }

    #[test]
    fn bundle_id_format() {
        let plan = make_plan("1000.00", "0.50", "my-proposal-123");
        let bundle = build(&plan, &cfg(50)).unwrap();
        assert_eq!(bundle.id, "bundle-my-proposal-123");
    }

    #[test]
    fn leg_id_format() {
        let plan = make_plan("1000.00", "0.50", "my-proposal");
        let bundle = build(&plan, &cfg(50)).unwrap();
        assert_eq!(bundle.legs[0].id, "my-proposal-leg-0");
    }

    #[test]
    fn schema_version_is_portfolio_intent_1() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(50)).unwrap();
        assert_eq!(bundle.schema_version, "portfolio-intent/1");
    }

    #[test]
    fn slippage_50_bps_min_out() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(50)).unwrap();
        let min_out: f64 = bundle.legs[0].min_out.value_usd.parse().unwrap();
        // 1000 * 0.995 = 995.00
        assert!((min_out - 995.0).abs() < 0.01);
    }

    #[test]
    fn slippage_zero_min_out_equals_expected() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(0)).unwrap();
        let min_out: f64 = bundle.legs[0].min_out.value_usd.parse().unwrap();
        assert!((min_out - 1000.0).abs() < 0.01);
    }

    #[test]
    fn slippage_10000_min_out_zero() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(10000)).unwrap();
        let min_out: f64 = bundle.legs[0].min_out.value_usd.parse().unwrap();
        assert!(min_out.abs() < 0.01);
    }

    #[test]
    fn total_cost_matches_plan() {
        let plan = make_plan("1000.00", "3.50", "p1");
        let bundle = build(&plan, &cfg(50)).unwrap();
        assert_eq!(bundle.total_cost_usd, "3.50");
    }

    #[test]
    fn single_leg_output() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(50)).unwrap();
        assert_eq!(bundle.legs.len(), 1);
    }

    #[test]
    fn leg_inherits_chain_from_expected_out() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(50)).unwrap();
        assert_eq!(bundle.legs[0].chain, "base");
    }

    #[test]
    fn leg_quoted_by_fixture_solver() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(50)).unwrap();
        assert_eq!(bundle.legs[0].quoted_by, "fixture-solver");
    }

    #[test]
    fn bounded_checks_populated() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(75)).unwrap();
        assert_eq!(bundle.bounded_checks.max_slippage_bps, 75);
        assert_eq!(bundle.bounded_checks.solver_quote_version, "fixture/1");
        assert_eq!(bundle.bounded_checks.min_out_per_leg.len(), 1);
    }

    #[test]
    fn min_out_amount_has_six_decimal_places() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(50)).unwrap();
        let parts: Vec<&str> = bundle.legs[0].min_out.amount.split('.').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].len(), 6);
    }

    #[test]
    fn near_intent_payload_has_required_fields() {
        let plan = make_plan("1000.00", "0.50", "p1");
        let bundle = build(&plan, &cfg(50)).unwrap();
        let payload = &bundle.legs[0].near_intent_payload;
        assert_eq!(
            payload.get("kind").and_then(|v| v.as_str()),
            Some("fixture")
        );
        assert_eq!(
            payload.get("proposal_id").and_then(|v| v.as_str()),
            Some("p1")
        );
        assert_eq!(
            payload.get("schema_version").and_then(|v| v.as_str()),
            Some("portfolio-intent/1")
        );
    }

    #[test]
    fn amount_and_value_usd_computed_independently() {
        // 3.5 stETH worth $12250 — amount and value_usd differ
        let plan = MovementPlan {
            legs: vec![MovementLeg {
                kind: "deposit".to_string(),
                chain: "ethereum".to_string(),
                from_token: None,
                to_token: None,
                description: "test".to_string(),
            }],
            expected_out: TokenAmount {
                symbol: "stETH".to_string(),
                address: None,
                chain: "ethereum".to_string(),
                amount: "3.500000".to_string(),
                value_usd: "12250.00".to_string(),
            },
            expected_cost_usd: "3.00".to_string(),
            proposal_id: "p-steth".to_string(),
        };
        let bundle = build(&plan, &cfg(50)).unwrap();
        let leg = &bundle.legs[0];
        // amount: 3.5 * 0.995 = 3.4825
        let min_amount: f64 = leg.min_out.amount.parse().unwrap();
        assert!((min_amount - 3.4825).abs() < 0.001, "amount={min_amount}");
        // value_usd: 12250 * 0.995 = 12188.75
        let min_value: f64 = leg.min_out.value_usd.parse().unwrap();
        assert!((min_value - 12188.75).abs() < 0.01, "value_usd={min_value}");
    }
}
