//! `format_widget` operation — build the `projects/<id>/widgets/state.json`
//! view model the web widget consumes.
//!
//! This is deliberately a render-ready flat shape, NOT a copy of
//! `state/latest.json`. The widget is a thin view layer; every value
//! it displays lives here already, pre-formatted as strings where the
//! widget would otherwise have to do arithmetic.
//!
//! Shape locked at `portfolio-widget/1`. A breaking change bumps to
//! `portfolio-widget/2`; additive fields don't bump.

use serde::{Deserialize, Serialize};

use crate::format::{format_suggestion_md, FormatSuggestionInput};
use crate::types::{parse_decimal, ClassifiedPosition, IntentBundle, ProjectConfig, Proposal};

#[derive(Debug, Clone, Deserialize)]
pub struct FormatWidgetInput {
    pub positions: Vec<ClassifiedPosition>,
    pub proposals: Vec<Proposal>,
    pub config: ProjectConfig,
    #[serde(default)]
    pub pending_intents: Vec<PendingIntentInput>,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub next_mission_run: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub previous_total_value_usd: Option<String>,
    #[serde(default)]
    pub progress: Option<ProgressSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PendingIntentInput {
    pub bundle: IntentBundle,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProgressSummary {
    pub name: String,
    pub value: f64,
}

#[derive(Debug, Serialize)]
pub struct WidgetState {
    pub schema_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub totals: WidgetTotals,
    pub positions: Vec<WidgetPosition>,
    pub top_suggestions: Vec<WidgetSuggestion>,
    pub pending_intents: Vec<WidgetPendingIntent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_mission_run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_metric: Option<ProgressSummary>,
}

#[derive(Debug, Serialize)]
pub struct WidgetTotals {
    pub net_value_usd: String,
    pub realized_net_apy_7d: f64,
    pub floor_apy: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_vs_last_run_usd: Option<String>,
    pub risk_score_weighted: f64,
}

#[derive(Debug, Serialize)]
pub struct WidgetPosition {
    pub protocol: String,
    pub chain: String,
    pub category: String,
    pub principal_usd: String,
    pub net_apy: f64,
    pub risk_score: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<WidgetHealth>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct WidgetHealth {
    pub name: String,
    pub value: f64,
    pub warning: bool,
}

#[derive(Debug, Serialize)]
pub struct WidgetSuggestion {
    pub id: String,
    pub strategy: String,
    pub rationale: String,
    pub projected_delta_apy_bps: i32,
    pub projected_annual_gain_usd: String,
    pub gas_payback_days: f32,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct WidgetPendingIntent {
    pub id: String,
    pub status: String,
    pub legs: usize,
    pub total_cost_usd: String,
    pub expires_at: i64,
}

pub fn format_widget(input: FormatWidgetInput) -> WidgetState {
    let totals = compute_totals(&input);

    let positions = input
        .positions
        .iter()
        .map(|p| WidgetPosition {
            protocol: p.protocol.name.clone(),
            chain: p.chain.clone(),
            category: p.category.clone(),
            principal_usd: p.principal_usd.clone(),
            net_apy: p.net_yield_apy,
            risk_score: p.risk_score,
            health: p.health.as_ref().map(|h| WidgetHealth {
                name: h.name.clone(),
                value: h.value,
                warning: h.warning,
            }),
            tags: p.tags.clone(),
        })
        .collect();

    // Top 3 ready proposals, stable order (the filter already emits
    // them deterministically).
    let top_suggestions = input
        .proposals
        .iter()
        .filter(|p| p.status == "ready")
        .take(3)
        .map(|p| WidgetSuggestion {
            id: p.id.clone(),
            strategy: p.strategy_id.clone(),
            rationale: p.rationale.clone(),
            projected_delta_apy_bps: p.projected_delta_apy_bps,
            projected_annual_gain_usd: p.projected_annual_gain_usd.clone(),
            gas_payback_days: p.gas_payback_days,
            status: p.status.clone(),
        })
        .collect();

    let pending_intents = input
        .pending_intents
        .iter()
        .map(|pi| WidgetPendingIntent {
            id: pi.bundle.id.clone(),
            status: pi.status.clone(),
            legs: pi.bundle.legs.len(),
            total_cost_usd: pi.bundle.total_cost_usd.clone(),
            expires_at: pi.bundle.expires_at,
        })
        .collect();

    WidgetState {
        schema_version: "portfolio-widget/1",
        generated_at: input.generated_at.clone(),
        project_id: input.project_id.clone(),
        totals,
        positions,
        top_suggestions,
        pending_intents,
        next_mission_run: input.next_mission_run.clone(),
        progress_metric: input.progress.clone(),
    }
}

fn compute_totals(input: &FormatWidgetInput) -> WidgetTotals {
    // Reuse the suggestion formatter's math for net value + weighted
    // APY so the markdown and the widget never disagree about
    // totals.
    let suggestion_input = FormatSuggestionInput {
        positions: input.positions.clone(),
        proposals: vec![],
        config: input.config.clone(),
        generated_at: None,
        previous_total_value_usd: input.previous_total_value_usd.clone(),
    };
    let out = format_suggestion_md(suggestion_input);

    // Weighted risk score across the portfolio.
    let mut total_principal = 0.0f64;
    let mut risk_numerator = 0.0f64;
    for p in &input.positions {
        let principal = parse_decimal(&p.principal_usd);
        total_principal += principal;
        risk_numerator += principal * p.risk_score as f64;
    }
    let risk_weighted = if total_principal > 0.0 {
        risk_numerator / total_principal
    } else {
        0.0
    };

    WidgetTotals {
        net_value_usd: out.totals.net_value_usd,
        realized_net_apy_7d: out.totals.weighted_net_apy,
        floor_apy: input.config.floor_apy,
        delta_vs_last_run_usd: out.totals.delta_vs_previous_usd,
        risk_score_weighted: risk_weighted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ProtocolRef, RawPosition};

    fn pos(protocol: &str, principal: &str, apy: f64, risk: u8) -> ClassifiedPosition {
        ClassifiedPosition {
            protocol: ProtocolRef {
                id: protocol.to_string(),
                name: protocol.to_string(),
            },
            category: "lending".to_string(),
            chain: "base".to_string(),
            address: "0x0".to_string(),
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
                chain: "base".to_string(),
                protocol_id: protocol.to_string(),
                position_type: "supply".to_string(),
                address: "0x0".to_string(),
                token_balances: vec![],
                debt_balances: vec![],
                reward_balances: vec![],
                raw_metadata: serde_json::Value::Null,
                block_number: 0,
                fetched_at: 0,
            },
        }
    }

    #[test]
    fn widget_state_has_locked_schema_version() {
        let w = format_widget(FormatWidgetInput {
            positions: vec![pos("aave-v3", "1000.00", 0.03, 2)],
            proposals: vec![],
            config: ProjectConfig::default(),
            pending_intents: vec![],
            generated_at: Some("2026-04-11T12:00:00Z".to_string()),
            next_mission_run: None,
            project_id: Some("portfolio".to_string()),
            previous_total_value_usd: None,
            progress: None,
        });
        assert_eq!(w.schema_version, "portfolio-widget/1");
        assert_eq!(w.totals.net_value_usd, "1000.00");
        assert_eq!(w.positions.len(), 1);
    }

    #[test]
    fn widget_weights_risk_score_by_principal() {
        let w = format_widget(FormatWidgetInput {
            positions: vec![
                pos("aave-v3", "3000.00", 0.03, 2),
                pos("risky", "1000.00", 0.10, 4),
            ],
            proposals: vec![],
            config: ProjectConfig::default(),
            pending_intents: vec![],
            generated_at: None,
            next_mission_run: None,
            project_id: None,
            previous_total_value_usd: None,
            progress: None,
        });
        // (3000*2 + 1000*4) / 4000 = 2.5
        assert!((w.totals.risk_score_weighted - 2.5).abs() < 1e-9);
    }

    #[test]
    fn widget_caps_top_suggestions_at_three() {
        use crate::types::{CostBreakdown, MovementPlan, TokenAmount};
        let mk = |i: usize| Proposal {
            id: format!("p-{i}"),
            strategy_id: "stablecoin-yield-floor".to_string(),
            from_positions: vec![],
            to_protocol: ProtocolRef {
                id: "x".to_string(),
                name: "X".to_string(),
            },
            movement_plan: MovementPlan {
                legs: vec![],
                expected_out: TokenAmount {
                    symbol: "USDC".into(),
                    address: None,
                    chain: "base".into(),
                    amount: "0".into(),
                    value_usd: "0".into(),
                },
                expected_cost_usd: "0".into(),
                proposal_id: format!("p-{i}"),
            },
            projected_delta_apy_bps: 100 + i as i32,
            projected_annual_gain_usd: "10".into(),
            confidence: 0.8,
            risk_delta: 0,
            cost_breakdown: CostBreakdown::default(),
            gas_payback_days: 5.0,
            rationale: format!("proposal {i}"),
            status: "ready".to_string(),
        };
        let w = format_widget(FormatWidgetInput {
            positions: vec![],
            proposals: vec![mk(1), mk(2), mk(3), mk(4), mk(5)],
            config: ProjectConfig::default(),
            pending_intents: vec![],
            generated_at: None,
            next_mission_run: None,
            project_id: None,
            previous_total_value_usd: None,
            progress: None,
        });
        assert_eq!(w.top_suggestions.len(), 3);
        assert_eq!(w.top_suggestions[0].id, "p-1");
    }

    // ---- totals edge cases ----

    #[test]
    fn widget_empty_positions_totals() {
        let w = format_widget(FormatWidgetInput {
            positions: vec![],
            proposals: vec![],
            config: ProjectConfig::default(),
            pending_intents: vec![],
            generated_at: None,
            next_mission_run: None,
            project_id: None,
            previous_total_value_usd: None,
            progress: None,
        });
        assert_eq!(w.totals.net_value_usd, "0.00");
        assert_eq!(w.totals.risk_score_weighted, 0.0);
        assert!(w.totals.delta_vs_last_run_usd.is_none());
    }

    #[test]
    fn widget_delta_vs_last_run_present() {
        let w = format_widget(FormatWidgetInput {
            positions: vec![pos("aave-v3", "1200.00", 0.03, 2)],
            proposals: vec![],
            config: ProjectConfig::default(),
            pending_intents: vec![],
            generated_at: None,
            next_mission_run: None,
            project_id: None,
            previous_total_value_usd: Some("1000.00".to_string()),
            progress: None,
        });
        assert_eq!(w.totals.delta_vs_last_run_usd.as_deref(), Some("+200.00"));
    }

    // ---- positions rendering ----

    #[test]
    fn widget_position_with_health() {
        let mut p = pos("aave-v3", "5000.00", 0.03, 2);
        p.health = Some(crate::types::HealthMetric {
            name: "health_factor".to_string(),
            value: 1.15,
            warning: true,
        });
        let w = format_widget(FormatWidgetInput {
            positions: vec![p],
            proposals: vec![],
            config: ProjectConfig::default(),
            pending_intents: vec![],
            generated_at: None,
            next_mission_run: None,
            project_id: None,
            previous_total_value_usd: None,
            progress: None,
        });
        let wp = &w.positions[0];
        assert!(wp.health.is_some());
        let h = wp.health.as_ref().unwrap();
        assert_eq!(h.name, "health_factor");
        assert!((h.value - 1.15).abs() < 1e-9);
        assert!(h.warning);
    }

    #[test]
    fn widget_position_with_tags() {
        let mut p = pos("aave-v3", "5000.00", 0.03, 2);
        p.tags = vec!["high-yield".to_string(), "stable".to_string()];
        let w = format_widget(FormatWidgetInput {
            positions: vec![p],
            proposals: vec![],
            config: ProjectConfig::default(),
            pending_intents: vec![],
            generated_at: None,
            next_mission_run: None,
            project_id: None,
            previous_total_value_usd: None,
            progress: None,
        });
        assert_eq!(w.positions[0].tags, vec!["high-yield", "stable"]);
    }

    // ---- pending intents ----

    #[test]
    fn widget_pending_intents_rendered() {
        use crate::types::{BoundedChecks, IntentBundle, IntentLeg, TokenAmount as TA};
        let bundle = IntentBundle {
            id: "bundle-1".to_string(),
            legs: vec![
                IntentLeg {
                    id: "leg-0".into(), kind: "deposit".into(), chain: "base".into(),
                    near_intent_payload: serde_json::Value::Null, depends_on: None,
                    min_out: TA { symbol: "USDC".into(), address: None, chain: "base".into(), amount: "995".into(), value_usd: "995".into() },
                    quoted_by: "fixture".into(),
                },
                IntentLeg {
                    id: "leg-1".into(), kind: "swap".into(), chain: "base".into(),
                    near_intent_payload: serde_json::Value::Null, depends_on: Some("leg-0".into()),
                    min_out: TA { symbol: "USDC".into(), address: None, chain: "base".into(), amount: "990".into(), value_usd: "990".into() },
                    quoted_by: "fixture".into(),
                },
            ],
            total_cost_usd: "0.50".to_string(),
            bounded_checks: BoundedChecks::default(),
            expires_at: 1712345678,
            signer_placeholder: "<signed>".into(),
            schema_version: "portfolio-intent/1".into(),
        };
        let w = format_widget(FormatWidgetInput {
            positions: vec![],
            proposals: vec![],
            config: ProjectConfig::default(),
            pending_intents: vec![PendingIntentInput { bundle, status: "awaiting-signature".to_string() }],
            generated_at: None,
            next_mission_run: None,
            project_id: None,
            previous_total_value_usd: None,
            progress: None,
        });
        assert_eq!(w.pending_intents.len(), 1);
        assert_eq!(w.pending_intents[0].id, "bundle-1");
        assert_eq!(w.pending_intents[0].status, "awaiting-signature");
        assert_eq!(w.pending_intents[0].legs, 2);
        assert_eq!(w.pending_intents[0].total_cost_usd, "0.50");
        assert_eq!(w.pending_intents[0].expires_at, 1712345678);
    }

    // ---- only ready proposals in suggestions ----

    #[test]
    fn widget_excludes_non_ready_proposals() {
        use crate::types::{CostBreakdown, MovementPlan, TokenAmount as TA};
        let mk = |status: &str, i: usize| Proposal {
            id: format!("p-{i}"),
            strategy_id: "test".to_string(),
            from_positions: vec![],
            to_protocol: ProtocolRef { id: "x".into(), name: "X".into() },
            movement_plan: MovementPlan {
                legs: vec![], expected_out: TA { symbol: "USDC".into(), address: None, chain: "base".into(), amount: "0".into(), value_usd: "0".into() },
                expected_cost_usd: "0".into(), proposal_id: format!("p-{i}"),
            },
            projected_delta_apy_bps: 100,
            projected_annual_gain_usd: "10".into(),
            confidence: 0.8, risk_delta: 0,
            cost_breakdown: CostBreakdown::default(),
            gas_payback_days: 5.0,
            rationale: "test".into(),
            status: status.to_string(),
        };
        let w = format_widget(FormatWidgetInput {
            positions: vec![],
            proposals: vec![mk("ready", 1), mk("blocked-by-constraint", 2), mk("below-threshold", 3), mk("ready", 4)],
            config: ProjectConfig::default(),
            pending_intents: vec![],
            generated_at: None,
            next_mission_run: None,
            project_id: None,
            previous_total_value_usd: None,
            progress: None,
        });
        assert_eq!(w.top_suggestions.len(), 2);
        assert!(w.top_suggestions.iter().all(|s| s.status == "ready"));
    }

    // ---- progress metric passthrough ----

    #[test]
    fn widget_progress_metric_passthrough() {
        let w = format_widget(FormatWidgetInput {
            positions: vec![],
            proposals: vec![],
            config: ProjectConfig::default(),
            pending_intents: vec![],
            generated_at: None,
            next_mission_run: Some("2026-04-12T18:00:00Z".to_string()),
            project_id: Some("my-portfolio".to_string()),
            previous_total_value_usd: None,
            progress: Some(ProgressSummary { name: "realized_apy_vs_floor".into(), value: 0.25 }),
        });
        assert_eq!(w.next_mission_run.as_deref(), Some("2026-04-12T18:00:00Z"));
        assert_eq!(w.project_id.as_deref(), Some("my-portfolio"));
        let pm = w.progress_metric.unwrap();
        assert_eq!(pm.name, "realized_apy_vs_floor");
        assert!((pm.value - 0.25).abs() < 1e-9);
    }
}
