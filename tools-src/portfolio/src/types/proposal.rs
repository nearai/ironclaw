use serde::{Deserialize, Serialize};

use super::position::{ProtocolRef, TokenAmount};

/// Configuration for a portfolio project. Lives in workspace at
/// `projects/<id>/config.json` and is passed in by the skill playbook.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProjectConfig {
    #[serde(default = "default_floor_apy")]
    pub floor_apy: f64,
    #[serde(default = "default_max_risk_score")]
    pub max_risk_score: u8,
    #[serde(default = "default_notify_threshold_usd")]
    pub notify_threshold_usd: f64,
    #[serde(default = "default_auto_intent_ceiling_usd")]
    pub auto_intent_ceiling_usd: f64,
    #[serde(default = "default_max_slippage_bps")]
    pub max_slippage_bps: u16,
    #[serde(default)]
    pub allowed_chains: Vec<String>,
}

fn default_floor_apy() -> f64 {
    0.04
}

fn default_max_risk_score() -> u8 {
    3
}

fn default_notify_threshold_usd() -> f64 {
    100.0
}

fn default_auto_intent_ceiling_usd() -> f64 {
    1000.0
}

fn default_max_slippage_bps() -> u16 {
    50
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            floor_apy: default_floor_apy(),
            max_risk_score: default_max_risk_score(),
            notify_threshold_usd: default_notify_threshold_usd(),
            auto_intent_ceiling_usd: default_auto_intent_ceiling_usd(),
            max_slippage_bps: default_max_slippage_bps(),
            allowed_chains: vec![],
        }
    }
}

/// A reference to a position by its (chain, protocol, address) tuple.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PositionRef {
    pub chain: String,
    pub protocol_id: String,
    pub address: String,
}

/// One leg of a movement plan: withdraw, bridge, swap, or deposit.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MovementLeg {
    pub kind: String, // "withdraw" | "bridge" | "swap" | "deposit"
    pub chain: String,
    #[serde(default)]
    pub from_token: Option<TokenAmount>,
    #[serde(default)]
    pub to_token: Option<TokenAmount>,
    /// Free-form description for the rationale.
    pub description: String,
}

/// The full set of legs to execute a proposal.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MovementPlan {
    pub legs: Vec<MovementLeg>,
    /// Expected output token+amount after all legs.
    pub expected_out: TokenAmount,
    /// Total cost in USD as a decimal string.
    pub expected_cost_usd: String,
    /// Reference back to the originating proposal.
    pub proposal_id: String,
}

/// Cost breakdown for a proposal.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CostBreakdown {
    pub gas_estimate_usd: String,
    pub bridge_fee_estimate_usd: String,
    pub solver_fee_estimate_usd: String,
    pub slippage_budget_usd: String,
}

/// A ranked rebalancing proposal produced by the strategy stage.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Proposal {
    pub id: String,
    pub strategy_id: String,
    pub from_positions: Vec<PositionRef>,
    pub to_protocol: ProtocolRef,
    pub movement_plan: MovementPlan,
    pub projected_delta_apy_bps: i32,
    pub projected_annual_gain_usd: String,
    pub confidence: f32,
    pub risk_delta: i8,
    pub cost_breakdown: CostBreakdown,
    pub gas_payback_days: f32,
    pub rationale: String,
    pub status: String, // "ready" | "unmet-route" | "below-threshold" | "blocked-by-constraint"
}
