//! Deterministic constraint filter — turn classified positions +
//! a strategy doc into a candidate list of `Proposal`s.
//!
//! Two candidate sources, in order:
//!
//! 1. **Observed alternatives** — other `ClassifiedPosition`s in the
//!    same category with higher `net_yield_apy` and risk within the
//!    strategy's budget. This is the "real" candidate enumeration —
//!    if the user holds both an under-yielding Aave USDC position and
//!    an over-yielding Morpho USDC position, the filter proposes
//!    moving the Aave → Morpho.
//!
//! 2. **Synthetic fallback** — if no observed alternative exists, a
//!    synthetic target at `floor_apy + 1%` so smoke tests still see a
//!    ready proposal with a single-position input. M4+ replaces this
//!    with a registry-level "best known yield per category" once live
//!    APY data is available.
//!
//! The filter emits deterministic `Proposal`s (hash-stable IDs). The
//! LLM does the final ranking; this stage only filters and shapes.

use crate::types::{
    parse_decimal, ClassifiedPosition, CostBreakdown, MovementLeg, MovementPlan, PositionRef,
    ProjectConfig, Proposal, ProtocolRef, TokenAmount,
};

use super::parser::{StrategyDoc, StrategyKind};

pub fn candidates(
    doc: &StrategyDoc,
    positions: &[ClassifiedPosition],
    config: &ProjectConfig,
) -> Vec<Proposal> {
    match doc.frontmatter.kind {
        StrategyKind::YieldFloor => yield_floor_candidates(doc, positions, config),
        StrategyKind::HealthGuard => health_guard_candidates(doc, positions, config),
        StrategyKind::LpImpermanentLossWatch => lp_watch_candidates(doc, positions),
    }
}

// -------------------- yield floor --------------------

fn yield_floor_candidates(
    doc: &StrategyDoc,
    positions: &[ClassifiedPosition],
    config: &ProjectConfig,
) -> Vec<Proposal> {
    let fm = &doc.frontmatter;
    let target_category = fm.applies_to.category.clone();
    let min_principal = fm.applies_to.min_principal_usd.unwrap_or(0.0);
    let floor_apy = fm
        .inputs
        .get("floor_apy")
        .and_then(|v| v.as_f64())
        .unwrap_or(config.floor_apy);
    let min_delta_bps = fm.constraints.min_projected_delta_apy_bps.unwrap_or(0);
    let max_risk = fm
        .constraints
        .max_risk_score
        .unwrap_or(config.max_risk_score);
    let max_payback_days = fm.constraints.gas_payback_days.unwrap_or(f32::INFINITY);

    let target_chains = &fm.applies_to.chains;
    let target_tokens = &fm.applies_to.tokens;

    let mut out = Vec::new();
    for (idx, source) in positions.iter().enumerate() {
        if !applies_category(&target_category, &source.category) {
            continue;
        }
        if !target_chains.is_empty() && !target_chains.iter().any(|c| c == &source.chain) {
            continue;
        }
        if !target_tokens.is_empty() {
            let has_matching_token = source
                .raw_position
                .token_balances
                .iter()
                .any(|t| target_tokens.iter().any(|tt| tt == &t.symbol));
            if !has_matching_token {
                continue;
            }
        }
        let principal = parse_decimal(&source.principal_usd);
        if principal < min_principal {
            continue;
        }
        if source.net_yield_apy >= floor_apy {
            continue;
        }
        if source.risk_score > max_risk {
            continue;
        }

        let (target_protocol, target_apy, source_tag) =
            find_best_alternative(source, positions, max_risk).unwrap_or_else(|| {
                (
                    ProtocolRef {
                        id: "synthetic-better-venue".to_string(),
                        name: "Synthetic Better Venue".to_string(),
                    },
                    floor_apy + 0.01,
                    "synthetic",
                )
            });

        let delta_apy = target_apy - source.net_yield_apy;
        let delta_bps = (delta_apy * 10_000.0).round() as i32;
        if delta_bps < min_delta_bps {
            out.push(below_threshold_proposal(doc, source, delta_bps));
            continue;
        }

        let projected_annual_gain = principal * delta_apy;
        let cost_usd = estimate_cost_usd(source, &target_protocol);
        let payback_days = gas_payback_days(cost_usd, projected_annual_gain);
        if payback_days > max_payback_days {
            out.push(blocked_proposal(
                doc,
                source,
                &target_protocol,
                delta_bps,
                format!("gas payback {payback_days:.0} days exceeds max {max_payback_days:.0}"),
            ));
            continue;
        }

        let proposal_id = format!("{}-{}-{idx}", fm.id, source_tag);
        out.push(Proposal {
            id: proposal_id.clone(),
            strategy_id: fm.id.clone(),
            from_positions: vec![PositionRef {
                chain: source.chain.clone(),
                protocol_id: source.protocol.id.clone(),
                address: source.address.clone(),
            }],
            to_protocol: target_protocol.clone(),
            movement_plan: MovementPlan {
                legs: build_same_chain_legs(source, &target_protocol),
                expected_out: TokenAmount {
                    symbol: "USDC".to_string(),
                    address: None,
                    chain: source.chain.clone(),
                    amount: source.principal_usd.clone(),
                    value_usd: source.principal_usd.clone(),
                },
                expected_cost_usd: format!("{cost_usd:.2}"),
                proposal_id: proposal_id.clone(),
            },
            projected_delta_apy_bps: delta_bps,
            projected_annual_gain_usd: format!("{projected_annual_gain:.2}"),
            confidence: if source_tag == "observed" { 0.8 } else { 0.6 },
            risk_delta: 0,
            cost_breakdown: CostBreakdown {
                gas_estimate_usd: format!("{cost_usd:.2}"),
                bridge_fee_estimate_usd: "0.00".to_string(),
                solver_fee_estimate_usd: "0.00".to_string(),
                slippage_budget_usd: "0.00".to_string(),
            },
            gas_payback_days: payback_days,
            rationale: format!(
                "{}: {} @ {:.2}% < floor {:.2}%; move to {} @ {:.2}% (Δ {} bps, payback {:.0}d, source: {})",
                fm.id,
                source.protocol.name,
                source.net_yield_apy * 100.0,
                floor_apy * 100.0,
                target_protocol.name,
                target_apy * 100.0,
                delta_bps,
                payback_days,
                source_tag,
            ),
            status: "ready".to_string(),
        });
    }
    out
}

/// Find the highest-yielding observed position in the same category
/// whose risk score is within the strategy's budget. Returns
/// `(target_protocol, target_apy, "observed")` or `None`.
fn find_best_alternative(
    source: &ClassifiedPosition,
    positions: &[ClassifiedPosition],
    max_risk: u8,
) -> Option<(ProtocolRef, f64, &'static str)> {
    positions
        .iter()
        .filter(|p| p.protocol.id != source.protocol.id)
        .filter(|p| p.category == source.category)
        .filter(|p| p.risk_score <= max_risk)
        .filter(|p| p.net_yield_apy > source.net_yield_apy)
        .max_by(|a, b| {
            a.net_yield_apy
                .partial_cmp(&b.net_yield_apy)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|best| (best.protocol.clone(), best.net_yield_apy, "observed"))
}

fn build_same_chain_legs(source: &ClassifiedPosition, target: &ProtocolRef) -> Vec<MovementLeg> {
    vec![
        MovementLeg {
            kind: "withdraw".to_string(),
            chain: source.chain.clone(),
            from_token: None,
            to_token: None,
            description: format!(
                "Withdraw {} from {}",
                source.principal_usd, source.protocol.name
            ),
        },
        MovementLeg {
            kind: "deposit".to_string(),
            chain: source.chain.clone(),
            from_token: None,
            to_token: None,
            description: format!("Deposit into {}", target.name),
        },
    ]
}

fn estimate_cost_usd(source: &ClassifiedPosition, _target: &ProtocolRef) -> f64 {
    // M3 uses a flat $0.50 for same-chain moves; M4 plugs in solver
    // quotes. Cross-chain moves get their cost from the solver at
    // build_intent time, not here.
    match source.chain.as_str() {
        "ethereum" => 3.00,
        "base" | "arbitrum" | "optimism" => 0.50,
        "polygon" | "avalanche" => 0.20,
        _ => 1.00,
    }
}

fn gas_payback_days(cost_usd: f64, annual_gain_usd: f64) -> f32 {
    if annual_gain_usd <= 0.0 {
        return f32::INFINITY;
    }
    let daily_gain = annual_gain_usd / 365.0;
    (cost_usd / daily_gain) as f32
}

fn applies_category(target: &Option<String>, actual: &str) -> bool {
    match target {
        Some(cat) => cat == actual,
        None => true,
    }
}

// -------------------- health guard --------------------

fn health_guard_candidates(
    doc: &StrategyDoc,
    positions: &[ClassifiedPosition],
    _config: &ProjectConfig,
) -> Vec<Proposal> {
    let fm = &doc.frontmatter;
    let danger = fm
        .inputs
        .get("danger_threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.2);
    let critical = fm
        .inputs
        .get("critical_threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.05);

    let mut out = Vec::new();
    for (idx, position) in positions.iter().enumerate() {
        let Some(health) = &position.health else {
            continue;
        };
        if health.value >= danger {
            continue;
        }

        let severity = if health.value <= critical {
            "critical"
        } else {
            "warning"
        };

        let proposal_id = format!("{}-{severity}-{idx}", fm.id);
        out.push(Proposal {
            id: proposal_id.clone(),
            strategy_id: fm.id.clone(),
            from_positions: vec![PositionRef {
                chain: position.chain.clone(),
                protocol_id: position.protocol.id.clone(),
                address: position.address.clone(),
            }],
            to_protocol: ProtocolRef {
                id: "self".to_string(),
                name: format!("{} (partial deleverage)", position.protocol.name),
            },
            movement_plan: MovementPlan {
                legs: vec![MovementLeg {
                    kind: "repay".to_string(),
                    chain: position.chain.clone(),
                    from_token: None,
                    to_token: None,
                    description: format!(
                        "Repay ~25% of debt on {} to restore health factor",
                        position.protocol.name
                    ),
                }],
                expected_out: TokenAmount {
                    symbol: "USDC".to_string(),
                    address: None,
                    chain: position.chain.clone(),
                    amount: "0".to_string(),
                    value_usd: "0".to_string(),
                },
                expected_cost_usd: "1.00".to_string(),
                proposal_id: proposal_id.clone(),
            },
            projected_delta_apy_bps: 0,
            projected_annual_gain_usd: "0.00".to_string(),
            confidence: if severity == "critical" { 0.95 } else { 0.7 },
            risk_delta: -1,
            cost_breakdown: CostBreakdown {
                gas_estimate_usd: "1.00".to_string(),
                ..CostBreakdown::default()
            },
            gas_payback_days: 0.0,
            rationale: format!(
                "{}: {} health factor {:.2} < {:.2} ({severity}). Suggest repaying or adding collateral.",
                fm.id,
                position.protocol.name,
                health.value,
                danger
            ),
            status: "ready".to_string(),
        });
    }
    out
}

// -------------------- LP impermanent-loss watch --------------------

fn lp_watch_candidates(doc: &StrategyDoc, positions: &[ClassifiedPosition]) -> Vec<Proposal> {
    let fm = &doc.frontmatter;
    let mut out = Vec::new();
    for (idx, position) in positions.iter().enumerate() {
        if position.category != "dex-lp" {
            continue;
        }
        let in_range = position
            .raw_position
            .raw_metadata
            .get("in_range")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if in_range {
            continue;
        }

        let proposal_id = format!("{}-oor-{idx}", fm.id);
        out.push(Proposal {
            id: proposal_id.clone(),
            strategy_id: fm.id.clone(),
            from_positions: vec![PositionRef {
                chain: position.chain.clone(),
                protocol_id: position.protocol.id.clone(),
                address: position.address.clone(),
            }],
            to_protocol: ProtocolRef {
                id: "self".to_string(),
                name: format!("{} (rebalance range)", position.protocol.name),
            },
            movement_plan: MovementPlan {
                legs: vec![MovementLeg {
                    kind: "rebalance-lp".to_string(),
                    chain: position.chain.clone(),
                    from_token: None,
                    to_token: None,
                    description: "Close and reopen LP around current price".to_string(),
                }],
                expected_out: TokenAmount {
                    symbol: "LP".to_string(),
                    address: None,
                    chain: position.chain.clone(),
                    amount: "1".to_string(),
                    value_usd: position.principal_usd.clone(),
                },
                expected_cost_usd: "5.00".to_string(),
                proposal_id: proposal_id.clone(),
            },
            projected_delta_apy_bps: 0,
            projected_annual_gain_usd: "0.00".to_string(),
            confidence: 0.5,
            risk_delta: 0,
            cost_breakdown: CostBreakdown {
                gas_estimate_usd: "5.00".to_string(),
                ..CostBreakdown::default()
            },
            gas_payback_days: 0.0,
            rationale: format!(
                "{}: {} LP is out of range (impermanent loss accumulating). Consider rebalancing the tick range.",
                fm.id, position.protocol.name
            ),
            status: "ready".to_string(),
        });
    }
    out
}

// -------------------- shared helpers --------------------

fn below_threshold_proposal(
    doc: &StrategyDoc,
    position: &ClassifiedPosition,
    delta_bps: i32,
) -> Proposal {
    Proposal {
        id: format!("{}-below-{}", doc.frontmatter.id, position.protocol.id),
        strategy_id: doc.frontmatter.id.clone(),
        from_positions: vec![PositionRef {
            chain: position.chain.clone(),
            protocol_id: position.protocol.id.clone(),
            address: position.address.clone(),
        }],
        to_protocol: ProtocolRef {
            id: "n/a".to_string(),
            name: "n/a".to_string(),
        },
        movement_plan: MovementPlan {
            legs: vec![],
            expected_out: TokenAmount {
                symbol: "USDC".to_string(),
                address: None,
                chain: position.chain.clone(),
                amount: "0".to_string(),
                value_usd: "0".to_string(),
            },
            expected_cost_usd: "0.00".to_string(),
            proposal_id: format!("{}-below-{}", doc.frontmatter.id, position.protocol.id),
        },
        projected_delta_apy_bps: delta_bps,
        projected_annual_gain_usd: "0.00".to_string(),
        confidence: 0.0,
        risk_delta: 0,
        cost_breakdown: CostBreakdown::default(),
        gas_payback_days: 0.0,
        rationale: format!(
            "Δ {} bps below the strategy's min_projected_delta_apy_bps threshold.",
            delta_bps
        ),
        status: "below-threshold".to_string(),
    }
}

fn blocked_proposal(
    doc: &StrategyDoc,
    position: &ClassifiedPosition,
    target: &ProtocolRef,
    delta_bps: i32,
    reason: String,
) -> Proposal {
    Proposal {
        id: format!("{}-blocked-{}", doc.frontmatter.id, position.protocol.id),
        strategy_id: doc.frontmatter.id.clone(),
        from_positions: vec![PositionRef {
            chain: position.chain.clone(),
            protocol_id: position.protocol.id.clone(),
            address: position.address.clone(),
        }],
        to_protocol: target.clone(),
        movement_plan: MovementPlan {
            legs: vec![],
            expected_out: TokenAmount {
                symbol: "USDC".to_string(),
                address: None,
                chain: position.chain.clone(),
                amount: "0".to_string(),
                value_usd: "0".to_string(),
            },
            expected_cost_usd: "0.00".to_string(),
            proposal_id: format!("{}-blocked-{}", doc.frontmatter.id, position.protocol.id),
        },
        projected_delta_apy_bps: delta_bps,
        projected_annual_gain_usd: "0.00".to_string(),
        confidence: 0.0,
        risk_delta: 0,
        cost_breakdown: CostBreakdown::default(),
        gas_payback_days: 0.0,
        rationale: format!("blocked by constraint: {reason}"),
        status: "blocked-by-constraint".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::parser;
    use crate::types::{HealthMetric, RawPosition};

    fn make_position(
        protocol_id: &str,
        category: &str,
        chain: &str,
        principal: &str,
        apy: f64,
        risk: u8,
    ) -> ClassifiedPosition {
        ClassifiedPosition {
            protocol: ProtocolRef {
                id: protocol_id.to_string(),
                name: protocol_id.to_string(),
            },
            category: category.to_string(),
            chain: chain.to_string(),
            address: "0xtest".to_string(),
            principal_usd: principal.to_string(),
            debt_usd: "0.00".to_string(),
            net_yield_apy: apy,
            unrealized_pnl_usd: "0.00".to_string(),
            risk_score: risk,
            exit_cost_estimate_usd: "0.00".to_string(),
            withdrawal_delay_seconds: 0,
            liquidity_tier: "instant".to_string(),
            health: None,
            tags: vec![],
            raw_position: RawPosition {
                chain: chain.to_string(),
                protocol_id: protocol_id.to_string(),
                position_type: "supply".to_string(),
                address: "0xtest".to_string(),
                token_balances: vec![],
                debt_balances: vec![],
                reward_balances: vec![],
                raw_metadata: serde_json::Value::Null,
                block_number: 0,
                fetched_at: 0,
            },
        }
    }

    fn make_position_with_health(
        protocol_id: &str,
        chain: &str,
        health_value: f64,
    ) -> ClassifiedPosition {
        let mut p = make_position(protocol_id, "lending", chain, "5000.00", 0.04, 2);
        p.health = Some(HealthMetric {
            name: "health_factor".to_string(),
            value: health_value,
            warning: false,
        });
        p
    }

    fn make_lp_position(protocol_id: &str, in_range: bool) -> ClassifiedPosition {
        let mut p = make_position(protocol_id, "dex-lp", "ethereum", "5000.00", 0.09, 3);
        p.raw_position.raw_metadata = serde_json::json!({"in_range": in_range});
        p
    }

    fn yield_floor_doc(floor: f64) -> String {
        format!(
            "---\nid: test-yield-floor\nkind: yield-floor\napplies_to:\n  category: stablecoin-idle\nconstraints:\n  min_projected_delta_apy_bps: 50\n  max_risk_score: 5\n  gas_payback_days: 90\ninputs:\n  floor_apy: {floor}\n---\nBody\n"
        )
    }

    fn health_guard_doc(danger: f64, critical: f64) -> String {
        format!(
            "---\nid: test-health-guard\nkind: health-guard\ninputs:\n  danger_threshold: {danger}\n  critical_threshold: {critical}\n---\nBody\n"
        )
    }

    fn lp_watch_doc() -> String {
        "---\nid: test-lp-watch\nkind: lp-impermanent-loss-watch\n---\nBody\n".to_string()
    }

    fn config() -> ProjectConfig {
        ProjectConfig::default()
    }

    // ---- applies_category ----

    #[test]
    fn applies_category_none_matches_everything() {
        assert!(applies_category(&None, "stablecoin-idle"));
        assert!(applies_category(&None, "lending"));
        assert!(applies_category(&None, "dex-lp"));
    }

    #[test]
    fn applies_category_match() {
        assert!(applies_category(
            &Some("stablecoin-idle".to_string()),
            "stablecoin-idle"
        ));
    }

    #[test]
    fn applies_category_mismatch() {
        assert!(!applies_category(
            &Some("stablecoin-idle".to_string()),
            "lending"
        ));
    }

    // ---- gas_payback_days ----

    #[test]
    fn payback_positive_gain() {
        let days = gas_payback_days(3.0, 365.0);
        assert!((days - 3.0).abs() < 0.01);
    }

    #[test]
    fn payback_zero_gain_is_infinity() {
        assert!(gas_payback_days(3.0, 0.0).is_infinite());
    }

    #[test]
    fn payback_negative_gain_is_infinity() {
        assert!(gas_payback_days(3.0, -10.0).is_infinite());
    }

    #[test]
    fn payback_zero_cost_is_zero() {
        let days = gas_payback_days(0.0, 100.0);
        assert!(days.abs() < 0.001);
    }

    // ---- estimate_cost_usd ----

    #[test]
    fn cost_ethereum_is_3() {
        let p = make_position("x", "stablecoin-idle", "ethereum", "1000", 0.02, 2);
        let target = ProtocolRef {
            id: "y".into(),
            name: "Y".into(),
        };
        assert!((estimate_cost_usd(&p, &target) - 3.0).abs() < 0.01);
    }

    #[test]
    fn cost_base_is_half() {
        let p = make_position("x", "stablecoin-idle", "base", "1000", 0.02, 2);
        let target = ProtocolRef {
            id: "y".into(),
            name: "Y".into(),
        };
        assert!((estimate_cost_usd(&p, &target) - 0.50).abs() < 0.01);
    }

    #[test]
    fn cost_polygon_is_twenty_cents() {
        let p = make_position("x", "stablecoin-idle", "polygon", "1000", 0.02, 2);
        let target = ProtocolRef {
            id: "y".into(),
            name: "Y".into(),
        };
        assert!((estimate_cost_usd(&p, &target) - 0.20).abs() < 0.01);
    }

    #[test]
    fn cost_unknown_chain_default() {
        let p = make_position("x", "stablecoin-idle", "solana", "1000", 0.02, 2);
        let target = ProtocolRef {
            id: "y".into(),
            name: "Y".into(),
        };
        assert!((estimate_cost_usd(&p, &target) - 1.0).abs() < 0.01);
    }

    // ---- find_best_alternative ----

    #[test]
    fn best_alternative_picks_highest_apy() {
        let source = make_position("aave", "stablecoin-idle", "base", "1000", 0.02, 2);
        let better = make_position("morpho", "stablecoin-idle", "base", "500", 0.06, 2);
        let good = make_position("compound", "stablecoin-idle", "base", "500", 0.05, 2);
        let result = find_best_alternative(&source, &[source.clone(), better, good], 5);
        assert!(result.is_some());
        let (proto, apy, tag) = result.unwrap();
        assert_eq!(proto.id, "morpho");
        assert!((apy - 0.06).abs() < 1e-9);
        assert_eq!(tag, "observed");
    }

    #[test]
    fn best_alternative_excludes_same_protocol() {
        let source = make_position("aave", "stablecoin-idle", "base", "1000", 0.02, 2);
        let same = make_position("aave", "stablecoin-idle", "base", "500", 0.06, 2);
        assert!(find_best_alternative(&source, &[source.clone(), same], 5).is_none());
    }

    #[test]
    fn best_alternative_excludes_different_category() {
        let source = make_position("aave", "stablecoin-idle", "base", "1000", 0.02, 2);
        let other_cat = make_position("morpho", "lending", "base", "500", 0.08, 2);
        assert!(find_best_alternative(&source, &[source.clone(), other_cat], 5).is_none());
    }

    #[test]
    fn best_alternative_excludes_high_risk() {
        let source = make_position("aave", "stablecoin-idle", "base", "1000", 0.02, 2);
        let risky = make_position("morpho", "stablecoin-idle", "base", "500", 0.08, 6);
        assert!(find_best_alternative(&source, &[source.clone(), risky], 5).is_none());
    }

    #[test]
    fn best_alternative_excludes_lower_apy() {
        let source = make_position("aave", "stablecoin-idle", "base", "1000", 0.05, 2);
        let worse = make_position("morpho", "stablecoin-idle", "base", "500", 0.03, 2);
        assert!(find_best_alternative(&source, &[source.clone(), worse], 5).is_none());
    }

    #[test]
    fn best_alternative_returns_none_for_single_position() {
        let source = make_position("aave", "stablecoin-idle", "base", "1000", 0.02, 2);
        assert!(find_best_alternative(&source, std::slice::from_ref(&source), 5).is_none());
    }

    // ---- yield floor candidates ----

    #[test]
    fn yield_floor_below_floor_gets_proposed() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        let pos = make_position("aave", "stablecoin-idle", "base", "5000.00", 0.02, 2);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].status, "ready");
        assert!(proposals[0].confidence > 0.5);
    }

    #[test]
    fn yield_floor_above_floor_skipped() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        let pos = make_position("aave", "stablecoin-idle", "base", "5000.00", 0.05, 2);
        let proposals = candidates(&doc, &[pos], &config());
        assert!(proposals.is_empty());
    }

    #[test]
    fn yield_floor_exactly_at_floor_skipped() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        let pos = make_position("aave", "stablecoin-idle", "base", "5000.00", 0.04, 2);
        let proposals = candidates(&doc, &[pos], &config());
        assert!(proposals.is_empty());
    }

    #[test]
    fn yield_floor_wrong_category_skipped() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        let pos = make_position("aave", "lending", "base", "5000.00", 0.02, 2);
        let proposals = candidates(&doc, &[pos], &config());
        assert!(proposals.is_empty());
    }

    #[test]
    fn yield_floor_risk_exceeds_max_skipped() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        let pos = make_position("aave", "stablecoin-idle", "base", "5000.00", 0.02, 6);
        let proposals = candidates(&doc, &[pos], &config());
        assert!(proposals.is_empty());
    }

    #[test]
    fn yield_floor_risk_at_max_included() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        let pos = make_position("aave", "stablecoin-idle", "base", "5000.00", 0.02, 5);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals.len(), 1);
    }

    #[test]
    fn yield_floor_observed_vs_synthetic_confidence() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        let low = make_position("aave", "stablecoin-idle", "base", "5000.00", 0.02, 2);
        let high = make_position("morpho", "stablecoin-idle", "base", "1000.00", 0.06, 2);
        let proposals = candidates(&doc, &[low.clone(), high], &config());
        let observed = proposals
            .iter()
            .find(|p| p.from_positions[0].protocol_id == "aave");
        assert!(observed.is_some());
        assert!(
            (observed.unwrap().confidence - 0.8).abs() < 0.01,
            "observed should be 0.8"
        );

        let proposals_synthetic = candidates(&doc, &[low], &config());
        assert!(
            (proposals_synthetic[0].confidence - 0.6).abs() < 0.01,
            "synthetic should be 0.6"
        );
    }

    #[test]
    fn yield_floor_multiple_positions_each_proposed() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        let p1 = make_position("aave", "stablecoin-idle", "base", "3000.00", 0.02, 2);
        let p2 = make_position("compound", "stablecoin-idle", "base", "2000.00", 0.01, 2);
        let proposals = candidates(&doc, &[p1, p2], &config());
        assert_eq!(proposals.len(), 2);
    }

    #[test]
    fn yield_floor_below_delta_threshold_creates_below_threshold() {
        let src = "---\nid: test-yield-floor\nkind: yield-floor\napplies_to:\n  category: stablecoin-idle\nconstraints:\n  min_projected_delta_apy_bps: 500\n  max_risk_score: 5\n---\nBody\n";
        let doc = parser::parse(src).unwrap();
        let pos = make_position("aave", "stablecoin-idle", "base", "5000.00", 0.038, 2);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].status, "below-threshold");
    }

    #[test]
    fn yield_floor_gas_payback_exceeded_creates_blocked() {
        let src = "---\nid: test-yield-floor\nkind: yield-floor\napplies_to:\n  category: stablecoin-idle\nconstraints:\n  max_risk_score: 5\n  gas_payback_days: 0.001\n---\nBody\n";
        let doc = parser::parse(src).unwrap();
        let pos = make_position("aave", "stablecoin-idle", "ethereum", "100.00", 0.02, 2);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].status, "blocked-by-constraint");
        assert!(proposals[0].rationale.contains("gas payback"));
    }

    #[test]
    fn yield_floor_principal_below_min_skipped() {
        let src = "---\nid: test-yield-floor\nkind: yield-floor\napplies_to:\n  category: stablecoin-idle\n  min_principal_usd: 1000\nconstraints:\n  max_risk_score: 5\n---\nBody\n";
        let doc = parser::parse(src).unwrap();
        let pos = make_position("aave", "stablecoin-idle", "base", "500.00", 0.02, 2);
        assert!(candidates(&doc, &[pos], &config()).is_empty());
    }

    #[test]
    fn yield_floor_movement_plan_has_two_legs() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        let pos = make_position("aave", "stablecoin-idle", "base", "5000.00", 0.02, 2);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals[0].movement_plan.legs.len(), 2);
        assert_eq!(proposals[0].movement_plan.legs[0].kind, "withdraw");
        assert_eq!(proposals[0].movement_plan.legs[1].kind, "deposit");
    }

    #[test]
    fn yield_floor_empty_positions() {
        let doc = parser::parse(&yield_floor_doc(0.04)).unwrap();
        assert!(candidates(&doc, &[], &config()).is_empty());
    }

    // ---- health guard candidates ----

    #[test]
    fn health_guard_below_danger_proposed() {
        let doc = parser::parse(&health_guard_doc(1.2, 1.05)).unwrap();
        let pos = make_position_with_health("aave", "ethereum", 1.1);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].status, "ready");
        assert!(proposals[0].rationale.contains("warning"));
    }

    #[test]
    fn health_guard_at_danger_skipped() {
        let doc = parser::parse(&health_guard_doc(1.2, 1.05)).unwrap();
        let pos = make_position_with_health("aave", "ethereum", 1.2);
        assert!(candidates(&doc, &[pos], &config()).is_empty());
    }

    #[test]
    fn health_guard_above_danger_skipped() {
        let doc = parser::parse(&health_guard_doc(1.2, 1.05)).unwrap();
        let pos = make_position_with_health("aave", "ethereum", 1.5);
        assert!(candidates(&doc, &[pos], &config()).is_empty());
    }

    #[test]
    fn health_guard_critical_severity() {
        let doc = parser::parse(&health_guard_doc(1.2, 1.05)).unwrap();
        let pos = make_position_with_health("aave", "ethereum", 1.02);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals.len(), 1);
        assert!(proposals[0].rationale.contains("critical"));
        assert!((proposals[0].confidence - 0.95).abs() < 0.01);
    }

    #[test]
    fn health_guard_warning_severity() {
        let doc = parser::parse(&health_guard_doc(1.2, 1.05)).unwrap();
        let pos = make_position_with_health("aave", "ethereum", 1.1);
        let proposals = candidates(&doc, &[pos], &config());
        assert!((proposals[0].confidence - 0.7).abs() < 0.01);
    }

    #[test]
    fn health_guard_no_health_field_skipped() {
        let doc = parser::parse(&health_guard_doc(1.2, 1.05)).unwrap();
        let pos = make_position("aave", "lending", "ethereum", "5000.00", 0.04, 2);
        assert!(candidates(&doc, &[pos], &config()).is_empty());
    }

    #[test]
    fn health_guard_multiple_positions() {
        let doc = parser::parse(&health_guard_doc(1.2, 1.05)).unwrap();
        let p1 = make_position_with_health("aave", "ethereum", 1.0);
        let p2 = make_position_with_health("compound", "ethereum", 1.1);
        let p3 = make_position_with_health("lido", "ethereum", 1.5);
        let proposals = candidates(&doc, &[p1, p2, p3], &config());
        assert_eq!(proposals.len(), 2);
    }

    #[test]
    fn health_guard_proposal_has_repay_leg() {
        let doc = parser::parse(&health_guard_doc(1.2, 1.05)).unwrap();
        let pos = make_position_with_health("aave", "ethereum", 1.1);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals[0].movement_plan.legs[0].kind, "repay");
    }

    // ---- LP watch candidates ----

    #[test]
    fn lp_watch_out_of_range_proposed() {
        let doc = parser::parse(&lp_watch_doc()).unwrap();
        let pos = make_lp_position("uniswap", false);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].status, "ready");
        assert!(proposals[0].rationale.contains("out of range"));
    }

    #[test]
    fn lp_watch_in_range_skipped() {
        let doc = parser::parse(&lp_watch_doc()).unwrap();
        let pos = make_lp_position("uniswap", true);
        assert!(candidates(&doc, &[pos], &config()).is_empty());
    }

    #[test]
    fn lp_watch_non_dex_lp_skipped() {
        let doc = parser::parse(&lp_watch_doc()).unwrap();
        let mut pos = make_lp_position("uniswap", false);
        pos.category = "lending".to_string();
        assert!(candidates(&doc, &[pos], &config()).is_empty());
    }

    #[test]
    fn lp_watch_missing_in_range_defaults_true_and_skips() {
        let doc = parser::parse(&lp_watch_doc()).unwrap();
        let mut pos = make_position("uniswap", "dex-lp", "ethereum", "5000", 0.09, 3);
        pos.raw_position.raw_metadata = serde_json::json!({});
        assert!(candidates(&doc, &[pos], &config()).is_empty());
    }

    #[test]
    fn lp_watch_rebalance_leg_kind() {
        let doc = parser::parse(&lp_watch_doc()).unwrap();
        let pos = make_lp_position("uniswap", false);
        let proposals = candidates(&doc, &[pos], &config());
        assert_eq!(proposals[0].movement_plan.legs[0].kind, "rebalance-lp");
    }
}
