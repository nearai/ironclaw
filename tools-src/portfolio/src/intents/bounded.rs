//! Bounded checks for intent bundles.
//!
//! Run for every backend (fixture, near-intents). If any check fails
//! the bundle is refused — the proposal's status downgrades to
//! `unmet-route` in the skill playbook and is logged for the next
//! mission run. We never write a bundle to disk that we wouldn't
//! sign ourselves.

use crate::types::{parse_decimal, IntentBundle, MovementPlan, ProjectConfig};

pub fn check(
    bundle: &IntentBundle,
    plan: &MovementPlan,
    config: &ProjectConfig,
) -> Result<(), String> {
    if bundle.legs.is_empty() {
        return Err("IntentBundle has no legs".to_string());
    }

    // 1. min_out per leg must be at least expected_out * (1 - slippage).
    if plan.expected_out.value_usd.is_empty() {
        return Err("plan expected_out.value_usd is empty".to_string());
    }
    let expected_out = parse_decimal(&plan.expected_out.value_usd);
    // Reject zero/negative/NaN/infinite — a zero anchor would make
    // min_required = 0 and every leg would pass vacuously; NaN would
    // poison the comparison below (NaN comparisons are always false).
    if !expected_out.is_finite() || expected_out <= 0.0 {
        return Err(format!(
            "plan expected_out.value_usd must be > 0, got {}",
            plan.expected_out.value_usd
        ));
    }
    let slippage_factor = 1.0 - (config.max_slippage_bps as f64 / 10_000.0);
    let min_required = expected_out * slippage_factor;

    // For multi-leg bundles the terminal leg is the one whose chain
    // matches plan.expected_out.chain — that's where the USD value
    // ultimately materializes. Intermediate legs may legitimately
    // carry an empty value_usd (they're hops in the solver route).
    let single_leg = bundle.legs.len() == 1;
    let terminal_chain = &plan.expected_out.chain;
    let mut terminal_checked = false;
    for leg in &bundle.legs {
        let is_terminal = single_leg || &leg.chain == terminal_chain;
        if !is_terminal {
            continue;
        }
        if leg.min_out.value_usd.is_empty() {
            return Err("terminal leg min_out.value_usd is empty".to_string());
        }
        let leg_min = parse_decimal(&leg.min_out.value_usd);
        // Tolerate rounding from 2-decimal-place formatting (±0.005)
        if leg_min + 0.005 < min_required {
            return Err(format!(
                "min_out {} below required {} ({} bps slippage)",
                leg_min, min_required, config.max_slippage_bps
            ));
        }
        terminal_checked = true;
    }
    if !terminal_checked {
        return Err(format!(
            "no leg on terminal chain '{}' (plan.expected_out.chain)",
            terminal_chain
        ));
    }

    // 2. total_cost_usd must not exceed the plan's expected cost.
    let bundle_cost = parse_decimal(&bundle.total_cost_usd);
    let plan_cost = parse_decimal(&plan.expected_cost_usd);
    if bundle_cost > plan_cost + 1e-6 {
        return Err(format!(
            "bundle cost {} exceeds plan expected {}",
            bundle_cost, plan_cost
        ));
    }

    // 3. Chain allowlist (only enforced when the user has explicitly
    // narrowed it).
    if !config.allowed_chains.is_empty() {
        for leg in &bundle.legs {
            if !config.allowed_chains.contains(&leg.chain) {
                return Err(format!(
                    "leg on chain '{}' not in allowed_chains",
                    leg.chain
                ));
            }
        }
    }

    // 4. Expiry headroom is checked when the bundle is materialized,
    // not here — the host's clock is the source of truth and we don't
    // want to read it from the constraint stage.

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BoundedChecks, IntentLeg, TokenAmount};

    fn token(value_usd: &str) -> TokenAmount {
        TokenAmount {
            symbol: "USDC".to_string(),
            address: None,
            chain: "base".to_string(),
            amount: value_usd.to_string(),
            value_usd: value_usd.to_string(),
        }
    }

    fn leg(chain: &str, min_out_usd: &str) -> IntentLeg {
        IntentLeg {
            id: "leg-0".to_string(),
            kind: "deposit".to_string(),
            chain: chain.to_string(),
            near_intent_payload: serde_json::Value::Null,
            depends_on: None,
            min_out: token(min_out_usd),
            quoted_by: "fixture".to_string(),
        }
    }

    fn bundle(legs: Vec<IntentLeg>, cost: &str) -> IntentBundle {
        IntentBundle {
            id: "bundle-1".to_string(),
            legs,
            total_cost_usd: cost.to_string(),
            bounded_checks: BoundedChecks::default(),
            expires_at: 0,
            signer_placeholder: "<signed>".to_string(),
            schema_version: "portfolio-intent/1".to_string(),
        }
    }

    fn plan(expected_out_usd: &str, cost: &str) -> MovementPlan {
        use crate::types::MovementLeg;
        MovementPlan {
            legs: vec![MovementLeg {
                kind: "deposit".to_string(),
                chain: "base".to_string(),
                from_token: None,
                to_token: None,
                description: "test".to_string(),
            }],
            expected_out: token(expected_out_usd),
            expected_cost_usd: cost.to_string(),
            proposal_id: "prop-1".to_string(),
        }
    }

    fn cfg(slippage_bps: u16, allowed_chains: Vec<String>) -> ProjectConfig {
        ProjectConfig {
            max_slippage_bps: slippage_bps,
            allowed_chains,
            ..ProjectConfig::default()
        }
    }

    // ---- empty legs ----

    #[test]
    fn empty_legs_error() {
        let b = bundle(vec![], "0.50");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(50, vec![]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no legs"));
    }

    // ---- slippage checks ----

    #[test]
    fn single_leg_min_out_at_slippage_limit_passes() {
        // 50 bps = 0.5% slippage. 1000 * 0.995 = 995
        let b = bundle(vec![leg("base", "995.00")], "0.50");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(50, vec![])).is_ok());
    }

    #[test]
    fn single_leg_min_out_above_limit_passes() {
        let b = bundle(vec![leg("base", "999.00")], "0.50");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(50, vec![])).is_ok());
    }

    #[test]
    fn single_leg_min_out_below_limit_fails() {
        let b = bundle(vec![leg("base", "990.00")], "0.50");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(50, vec![]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("min_out"));
    }

    #[test]
    fn zero_slippage_min_out_must_equal_expected() {
        // 0 bps slippage = no slippage allowed
        let b = bundle(vec![leg("base", "999.99")], "0.50");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(0, vec![]));
        assert!(result.is_err());
    }

    #[test]
    fn zero_slippage_exact_match_passes() {
        let b = bundle(vec![leg("base", "1000.00")], "0.50");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(0, vec![])).is_ok());
    }

    #[test]
    fn multi_leg_intermediate_ignored_terminal_checked() {
        // Intermediate leg (on a different chain) is skipped; the
        // terminal leg on plan.expected_out.chain must still clear
        // slippage.
        let l1 = leg("ethereum", "");
        let mut l2 = leg("base", "995.00");
        l2.id = "leg-1".to_string();
        let b = bundle(vec![l1, l2], "0.50");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(50, vec![])).is_ok());
    }

    #[test]
    fn multi_leg_terminal_below_slippage_fails() {
        // Regression: previously only single-leg bundles enforced
        // slippage, so a terminal leg with min_out == "0" would pass
        // for any multi-leg bundle. Now the terminal leg (matching
        // plan.expected_out.chain) must clear min_required.
        let l1 = leg("ethereum", "");
        let mut l2 = leg("base", "0");
        l2.id = "leg-1".to_string();
        let b = bundle(vec![l1, l2], "0.50");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(50, vec![]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("min_out"));
    }

    #[test]
    fn multi_leg_terminal_empty_value_usd_fails() {
        // If no leg carries the terminal USD, we can't verify slippage.
        let l1 = leg("ethereum", "");
        let mut l2 = leg("base", "");
        l2.id = "leg-1".to_string();
        let b = bundle(vec![l1, l2], "0.50");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(50, vec![]));
        assert!(result.is_err());
    }

    #[test]
    fn multi_leg_no_terminal_chain_fails() {
        let l1 = leg("ethereum", "500.00");
        let mut l2 = leg("optimism", "500.00");
        l2.id = "leg-1".to_string();
        let b = bundle(vec![l1, l2], "0.50");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(50, vec![]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("terminal chain"));
    }

    #[test]
    fn zero_expected_out_rejected() {
        // Regression: "0" expected_out would make min_required == 0
        // and any leg value would pass vacuously.
        let b = bundle(vec![leg("base", "0")], "0.50");
        let p = plan("0", "0.50");
        let result = check(&b, &p, &cfg(50, vec![]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected_out"));
    }

    #[test]
    fn unparseable_expected_out_rejected() {
        let b = bundle(vec![leg("base", "1000")], "0.50");
        let p = plan("not-a-number", "0.50");
        let result = check(&b, &p, &cfg(50, vec![]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected_out"));
    }

    #[test]
    fn slippage_rounding_drift_tolerated() {
        // Single leg at 994.99 against required 995.00 — 0.005 epsilon
        // tolerates the 2-decimal-place truncation in fixture output.
        let b = bundle(vec![leg("base", "994.995")], "0.50");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(50, vec![])).is_ok());
    }

    #[test]
    fn slippage_below_rounding_epsilon_fails() {
        // 994.98 is more than 0.005 under the 995.00 floor.
        let b = bundle(vec![leg("base", "994.98")], "0.50");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(50, vec![]));
        assert!(result.is_err());
    }

    // ---- cost checks ----

    #[test]
    fn cost_equal_passes() {
        let b = bundle(vec![leg("base", "995.00")], "0.50");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(50, vec![])).is_ok());
    }

    #[test]
    fn cost_below_plan_passes() {
        let b = bundle(vec![leg("base", "995.00")], "0.30");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(50, vec![])).is_ok());
    }

    #[test]
    fn cost_exceeds_plan_fails() {
        let b = bundle(vec![leg("base", "995.00")], "1.00");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(50, vec![]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cost"));
    }

    // ---- chain allowlist ----

    #[test]
    fn empty_allowlist_allows_everything() {
        let b = bundle(vec![leg("base", "995.00")], "0.50");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(50, vec![])).is_ok());
    }

    #[test]
    fn allowed_chain_passes() {
        let b = bundle(vec![leg("base", "995.00")], "0.50");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(50, vec!["base".into()])).is_ok());
    }

    #[test]
    fn disallowed_chain_fails() {
        let b = bundle(vec![leg("ethereum", "995.00")], "0.50");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(50, vec!["base".into()]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in allowed_chains"));
    }

    #[test]
    fn multi_leg_one_disallowed_fails() {
        let l1 = leg("base", "500.00");
        let mut l2 = leg("ethereum", "500.00");
        l2.id = "leg-1".to_string();
        let b = bundle(vec![l1, l2], "0.50");
        let p = plan("1000.00", "0.50");
        let result = check(&b, &p, &cfg(50, vec!["base".into()]));
        assert!(result.is_err());
    }

    // ---- all checks passing ----

    #[test]
    fn all_checks_pass_happy_path() {
        let b = bundle(vec![leg("base", "997.50")], "0.40");
        let p = plan("1000.00", "0.50");
        assert!(check(&b, &p, &cfg(50, vec!["base".into(), "ethereum".into()])).is_ok());
    }
}
