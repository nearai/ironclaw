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
pub struct FormatIntentsTradingWidgetInput {
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default = "default_mode")]
    pub mode: String,
    pub pair: String,
    #[serde(default = "default_stance")]
    pub stance: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub backtest_suite: Option<serde_json::Value>,
    #[serde(default)]
    pub risk_gates: Vec<IntentsRiskGateInput>,
    #[serde(default)]
    pub intent: Option<IntentsIntentInput>,
    #[serde(default)]
    pub research_sources: Vec<String>,
    #[serde(default)]
    pub paid_research_plan: Option<serde_json::Value>,
    #[serde(default)]
    pub next_action: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PendingIntentInput {
    pub bundle: IntentBundle,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntentsRiskGateInput {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntentsIntentInput {
    pub bundle: IntentBundle,
    #[serde(default = "default_intent_status")]
    pub status: String,
    #[serde(default)]
    pub route_label: Option<String>,
    #[serde(default)]
    pub quote_source: Option<String>,
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

#[derive(Debug, Serialize)]
pub struct IntentsTradingWidgetState {
    pub schema_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub mode: String,
    pub pair: String,
    pub stance: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub top_candidates: Vec<IntentsStrategyCandidate>,
    pub risk_gates: Vec<IntentsRiskGate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<IntentsIntentPreview>,
    pub source_count: usize,
    pub research_sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paid_research: Option<IntentsPaidResearchSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IntentsStrategyCandidate {
    pub rank: usize,
    pub id: String,
    pub strategy_kind: String,
    pub selection_score: f64,
    pub passes_basic_gate: bool,
    pub total_return_pct: f64,
    pub alpha_vs_buy_hold_pct: f64,
    pub max_drawdown_pct: f64,
    pub trades: usize,
}

#[derive(Debug, Serialize)]
pub struct IntentsRiskGate {
    pub name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IntentsIntentPreview {
    pub id: String,
    pub status: String,
    pub route_label: String,
    pub quote_source: String,
    pub schema_version: String,
    pub legs: usize,
    pub total_cost_usd: String,
    pub expires_at: i64,
    pub signer_placeholder: String,
    pub min_out: Vec<IntentMinOutPreview>,
}

#[derive(Debug, Serialize)]
pub struct IntentMinOutPreview {
    pub symbol: String,
    pub chain: String,
    pub amount: String,
    pub value_usd: String,
}

#[derive(Debug, Serialize)]
pub struct IntentsPaidResearchSummary {
    pub schema_version: String,
    pub query: String,
    pub budget_usd: f64,
    pub allocated_usd: f64,
    pub unspent_usd: f64,
    pub selected_count: usize,
    pub ready_for_paid_fetch: bool,
    pub ready_for_trade_research: bool,
    pub payment_rails: Vec<IntentsPaidResearchRail>,
    pub payable_sources: Vec<IntentsPayableResearchSource>,
    pub near_funding_routes: Vec<IntentsPaidResearchFundingRoute>,
    pub policy_gates: Vec<IntentsRiskGate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallet_policy: Option<IntentsPaidResearchWalletPolicy>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IntentsPaidResearchRail {
    pub protocol: String,
    pub count: usize,
    pub allocated_usd: f64,
}

#[derive(Debug, Serialize)]
pub struct IntentsPayableResearchSource {
    pub id: String,
    pub title: String,
    pub author: String,
    pub protocol: String,
    pub network: String,
    pub amount_usd: f64,
    pub evidence_weight: f64,
    pub receipt_required: bool,
}

#[derive(Debug, Serialize)]
pub struct IntentsPaidResearchFundingRoute {
    pub target_protocol: String,
    pub target_network: String,
    pub amount_usd: f64,
    pub via: String,
}

#[derive(Debug, Serialize)]
pub struct IntentsPaidResearchWalletPolicy {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    pub network: String,
    pub balance_usd: f64,
    pub max_articles_at_default_price: usize,
    pub per_article_cap_usd: f64,
    pub daily_cap_usd: f64,
    pub safe_to_autopay: bool,
    pub audit_urls: Vec<String>,
}

fn default_mode() -> String {
    "paper".to_string()
}

fn default_stance() -> String {
    "watch".to_string()
}

fn default_intent_status() -> String {
    "none".to_string()
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

pub fn format_intents_trading_widget(
    input: FormatIntentsTradingWidgetInput,
) -> IntentsTradingWidgetState {
    let top_candidates = input
        .backtest_suite
        .as_ref()
        .and_then(|suite| suite.get("ranked"))
        .and_then(|ranked| ranked.as_array())
        .into_iter()
        .flatten()
        .take(5)
        .map(strategy_candidate_from_value)
        .collect();

    let risk_gates = input
        .risk_gates
        .into_iter()
        .map(|gate| IntentsRiskGate {
            name: gate.name,
            status: gate.status,
            detail: gate.detail,
        })
        .collect();

    let intent = input.intent.map(|intent| {
        let min_out = intent
            .bundle
            .bounded_checks
            .min_out_per_leg
            .iter()
            .map(|token| IntentMinOutPreview {
                symbol: token.symbol.clone(),
                chain: token.chain.clone(),
                amount: token.amount.clone(),
                value_usd: token.value_usd.clone(),
            })
            .collect();
        IntentsIntentPreview {
            id: intent.bundle.id,
            status: intent.status,
            route_label: intent
                .route_label
                .unwrap_or_else(|| "NEAR Intents route".to_string()),
            quote_source: intent.quote_source.unwrap_or_else(|| "fixture".to_string()),
            schema_version: intent.bundle.schema_version,
            legs: intent.bundle.legs.len(),
            total_cost_usd: intent.bundle.total_cost_usd,
            expires_at: intent.bundle.expires_at,
            signer_placeholder: intent.bundle.signer_placeholder,
            min_out,
        }
    });
    let paid_research = input
        .paid_research_plan
        .as_ref()
        .map(paid_research_summary_from_value);

    IntentsTradingWidgetState {
        schema_version: "intents-trading-widget/1",
        generated_at: input.generated_at,
        project_id: input.project_id,
        mode: input.mode,
        pair: input.pair,
        stance: input.stance,
        confidence: input.confidence,
        top_candidates,
        risk_gates,
        intent,
        source_count: input.research_sources.len(),
        research_sources: input.research_sources,
        paid_research,
        next_action: input.next_action,
    }
}

fn strategy_candidate_from_value(value: &serde_json::Value) -> IntentsStrategyCandidate {
    let default_metrics = serde_json::Value::Null;
    let metrics = value.get("metrics").unwrap_or(&default_metrics);
    IntentsStrategyCandidate {
        rank: value.get("rank").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
        id: value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("candidate")
            .to_string(),
        strategy_kind: value
            .pointer("/strategy/kind")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string(),
        selection_score: value
            .get("selection_score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        passes_basic_gate: value
            .get("passes_basic_gate")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        total_return_pct: metrics
            .get("total_return_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        alpha_vs_buy_hold_pct: metrics
            .get("alpha_vs_buy_hold_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        max_drawdown_pct: metrics
            .get("max_drawdown_pct")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        trades: metrics.get("trades").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
    }
}

fn paid_research_summary_from_value(value: &serde_json::Value) -> IntentsPaidResearchSummary {
    let selected_sources = value
        .get("selected_sources")
        .and_then(|sources| sources.as_array())
        .cloned()
        .unwrap_or_default();
    let payable_sources = selected_sources
        .iter()
        .take(6)
        .map(|source| {
            let payment = source.get("payment").unwrap_or(&serde_json::Value::Null);
            let attribution = source
                .get("attribution")
                .unwrap_or(&serde_json::Value::Null);
            IntentsPayableResearchSource {
                id: string_field(source, "id", "source"),
                title: string_field(source, "title", "Untitled source"),
                author: string_field(source, "author", "Unknown author"),
                protocol: string_field(payment, "protocol", "manual"),
                network: string_field(payment, "network", "manual"),
                amount_usd: number_field(payment, "amount_usd"),
                evidence_weight: number_field(source, "evidence_weight"),
                receipt_required: attribution
                    .get("receipt_required")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }
        })
        .collect();

    let payment_rails = value
        .get("payment_rails")
        .and_then(|rails| rails.as_array())
        .into_iter()
        .flatten()
        .map(|rail| IntentsPaidResearchRail {
            protocol: string_field(rail, "protocol", "manual"),
            count: rail.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            allocated_usd: number_field(rail, "allocated_usd"),
        })
        .collect();

    let near_funding_routes = value
        .get("near_funding_routes")
        .and_then(|routes| routes.as_array())
        .into_iter()
        .flatten()
        .map(|route| IntentsPaidResearchFundingRoute {
            target_protocol: string_field(route, "target_protocol", "manual"),
            target_network: string_field(route, "target_network", "manual"),
            amount_usd: number_field(route, "amount_usd"),
            via: string_field(route, "via", "near-intents"),
        })
        .collect();

    let policy_gates = value
        .get("policy_gates")
        .and_then(|gates| gates.as_array())
        .into_iter()
        .flatten()
        .map(|gate| IntentsRiskGate {
            name: string_field(gate, "name", "paid-research-gate"),
            status: string_field(gate, "status", "unknown"),
            detail: gate
                .get("detail")
                .and_then(|detail| detail.as_str())
                .map(|detail| detail.to_string()),
        })
        .collect();
    let wallet_policy = value
        .get("wallet_policy")
        .map(|wallet| IntentsPaidResearchWalletPolicy {
            provider: string_field(wallet, "provider", "AgentCash"),
            address: wallet
                .get("address")
                .and_then(|address| address.as_str())
                .map(|address| address.to_string()),
            network: string_field(wallet, "network", "base"),
            balance_usd: number_field(wallet, "balance_usd"),
            max_articles_at_default_price: wallet
                .get("max_articles_at_default_price")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            per_article_cap_usd: number_field(wallet, "per_article_cap_usd"),
            daily_cap_usd: number_field(wallet, "daily_cap_usd"),
            safe_to_autopay: wallet
                .get("safe_to_autopay")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            audit_urls: wallet
                .get("audit_urls")
                .and_then(|urls| urls.as_array())
                .into_iter()
                .flatten()
                .filter_map(|url| url.as_str().map(|url| url.to_string()))
                .collect(),
        });

    let warnings = value
        .get("warnings")
        .and_then(|warnings| warnings.as_array())
        .into_iter()
        .flatten()
        .filter_map(|warning| warning.as_str().map(|warning| warning.to_string()))
        .collect();

    IntentsPaidResearchSummary {
        schema_version: string_field(value, "schema_version", "paid-research-plan/1"),
        query: string_field(value, "query", ""),
        budget_usd: number_field(value, "budget_usd"),
        allocated_usd: number_field(value, "allocated_usd"),
        unspent_usd: number_field(value, "unspent_usd"),
        selected_count: selected_sources.len(),
        ready_for_paid_fetch: value
            .get("ready_for_paid_fetch")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        ready_for_trade_research: value
            .get("ready_for_trade_research")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        payment_rails,
        payable_sources,
        near_funding_routes,
        policy_gates,
        wallet_policy,
        warnings,
    }
}

fn string_field(value: &serde_json::Value, field: &str, default: &str) -> String {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}

fn number_field(value: &serde_json::Value, field: &str) -> f64 {
    value.get(field).and_then(|v| v.as_f64()).unwrap_or(0.0)
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
                    id: "leg-0".into(),
                    kind: "deposit".into(),
                    chain: "base".into(),
                    near_intent_payload: serde_json::Value::Null,
                    depends_on: None,
                    min_out: TA {
                        symbol: "USDC".into(),
                        address: None,
                        chain: "base".into(),
                        amount: "995".into(),
                        value_usd: "995".into(),
                    },
                    quoted_by: "fixture".into(),
                },
                IntentLeg {
                    id: "leg-1".into(),
                    kind: "swap".into(),
                    chain: "base".into(),
                    near_intent_payload: serde_json::Value::Null,
                    depends_on: Some("leg-0".into()),
                    min_out: TA {
                        symbol: "USDC".into(),
                        address: None,
                        chain: "base".into(),
                        amount: "990".into(),
                        value_usd: "990".into(),
                    },
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
            pending_intents: vec![PendingIntentInput {
                bundle,
                status: "awaiting-signature".to_string(),
            }],
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

    #[test]
    fn intents_trading_widget_summarizes_suite_and_intent() {
        use crate::types::{BoundedChecks, IntentBundle, IntentLeg, TokenAmount};

        let token = TokenAmount {
            symbol: "BTC".into(),
            address: None,
            chain: "near".into(),
            amount: "0.001".into(),
            value_usd: "70.00".into(),
        };

        let w = format_intents_trading_widget(FormatIntentsTradingWidgetInput {
            generated_at: Some("2026-05-02T16:00:00Z".to_string()),
            project_id: Some("intents-trading-agent".to_string()),
            mode: "paper".to_string(),
            pair: "NEAR/BTC".to_string(),
            stance: "paper-intent".to_string(),
            confidence: Some(0.74),
            backtest_suite: Some(serde_json::json!({
                "schema_version": "intents-backtest-suite/1",
                "ranked": [{
                    "rank": 1,
                    "id": "sma_cross_fast",
                    "strategy": { "kind": "sma-cross" },
                    "selection_score": 18.5,
                    "passes_basic_gate": true,
                    "metrics": {
                        "total_return_pct": 12.0,
                        "alpha_vs_buy_hold_pct": 3.0,
                        "max_drawdown_pct": 8.0,
                        "trades": 6
                    }
                }]
            })),
            risk_gates: vec![IntentsRiskGateInput {
                name: "unsigned-only".to_string(),
                status: "pass".to_string(),
                detail: Some("Wallet signature required outside the agent.".to_string()),
            }],
            intent: Some(IntentsIntentInput {
                bundle: IntentBundle {
                    id: "intent-1".to_string(),
                    legs: vec![IntentLeg {
                        id: "leg-1".to_string(),
                        kind: "swap".to_string(),
                        chain: "near".to_string(),
                        near_intent_payload: serde_json::json!({ "intent": "token_diff" }),
                        depends_on: None,
                        min_out: token.clone(),
                        quoted_by: "fixture".to_string(),
                    }],
                    total_cost_usd: "0.42".to_string(),
                    bounded_checks: BoundedChecks {
                        min_out_per_leg: vec![token],
                        max_slippage_bps: 50,
                        solver_quote_version: "fixture".to_string(),
                    },
                    expires_at: 1_775_000_000,
                    signer_placeholder: "<signed-by-user>".to_string(),
                    schema_version: "portfolio-intent/1".to_string(),
                },
                status: "paper-built".to_string(),
                route_label: Some("NEAR -> BTC via NEAR Intents".to_string()),
                quote_source: Some("fixture".to_string()),
            }),
            research_sources: vec!["https://docs.near-intents.org/".to_string()],
            paid_research_plan: Some(serde_json::json!({
                "schema_version": "paid-research-plan/1",
                "query": "NEAR/BTC route risk",
                "budget_usd": 0.05,
                "allocated_usd": 0.02,
                "unspent_usd": 0.03,
                "ready_for_paid_fetch": true,
                "ready_for_trade_research": true,
                "payment_rails": [{
                    "protocol": "x402",
                    "count": 1,
                    "allocated_usd": 0.02
                }],
                "selected_sources": [{
                    "id": "paid-alpha-1",
                    "title": "NEAR/BTC route risk",
                    "author": "Researcher",
                    "evidence_weight": 1.0,
                    "payment": {
                        "protocol": "x402",
                        "network": "base",
                        "amount_usd": 0.02
                    },
                    "attribution": {
                        "receipt_required": true
                    }
                }],
                "near_funding_routes": [{
                    "target_protocol": "x402",
                    "target_network": "base",
                    "amount_usd": 0.02,
                    "via": "near-intents"
                }],
                "policy_gates": [{
                    "name": "receipt-before-use",
                    "status": "pass",
                    "detail": "Receipt required."
                }],
                "warnings": []
            })),
            next_action: Some("Request live quote only after user approval.".to_string()),
        });

        assert_eq!(w.schema_version, "intents-trading-widget/1");
        assert_eq!(w.top_candidates.len(), 1);
        assert!(w.top_candidates[0].passes_basic_gate);
        assert_eq!(w.intent.unwrap().status, "paper-built");
        assert_eq!(w.source_count, 1);
        let paid_research = w.paid_research.unwrap();
        assert_eq!(paid_research.selected_count, 1);
        assert_eq!(paid_research.payable_sources[0].protocol, "x402");
        assert_eq!(paid_research.near_funding_routes[0].via, "near-intents");
    }

    // ---- only ready proposals in suggestions ----

    #[test]
    fn widget_excludes_non_ready_proposals() {
        use crate::types::{CostBreakdown, MovementPlan, TokenAmount as TA};
        let mk = |status: &str, i: usize| Proposal {
            id: format!("p-{i}"),
            strategy_id: "test".to_string(),
            from_positions: vec![],
            to_protocol: ProtocolRef {
                id: "x".into(),
                name: "X".into(),
            },
            movement_plan: MovementPlan {
                legs: vec![],
                expected_out: TA {
                    symbol: "USDC".into(),
                    address: None,
                    chain: "base".into(),
                    amount: "0".into(),
                    value_usd: "0".into(),
                },
                expected_cost_usd: "0".into(),
                proposal_id: format!("p-{i}"),
            },
            projected_delta_apy_bps: 100,
            projected_annual_gain_usd: "10".into(),
            confidence: 0.8,
            risk_delta: 0,
            cost_breakdown: CostBreakdown::default(),
            gas_payback_days: 5.0,
            rationale: "test".into(),
            status: status.to_string(),
        };
        let w = format_widget(FormatWidgetInput {
            positions: vec![],
            proposals: vec![
                mk("ready", 1),
                mk("blocked-by-constraint", 2),
                mk("below-threshold", 3),
                mk("ready", 4),
            ],
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
            progress: Some(ProgressSummary {
                name: "realized_apy_vs_floor".into(),
                value: 0.25,
            }),
        });
        assert_eq!(w.next_mission_run.as_deref(), Some("2026-04-12T18:00:00Z"));
        assert_eq!(w.project_id.as_deref(), Some("my-portfolio"));
        let pm = w.progress_metric.unwrap();
        assert_eq!(pm.name, "realized_apy_vs_floor");
        assert!((pm.value - 0.25).abs() < 1e-9);
    }
}
