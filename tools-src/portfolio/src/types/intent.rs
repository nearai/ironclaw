use serde::{Deserialize, Serialize};

use super::position::TokenAmount;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IntentLeg {
    pub id: String,
    pub kind: String, // solver-defined: "swap", "bridge", "deposit", "withdraw", "repay", "rebalance-lp", etc.
    pub chain: String,
    /// Solver-shaped, signable as-is.
    pub near_intent_payload: serde_json::Value,
    #[serde(default)]
    pub depends_on: Option<String>,
    pub min_out: TokenAmount,
    pub quoted_by: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct BoundedChecks {
    pub min_out_per_leg: Vec<TokenAmount>,
    pub max_slippage_bps: u16,
    pub solver_quote_version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IntentBundle {
    pub id: String,
    pub legs: Vec<IntentLeg>,
    pub total_cost_usd: String,
    pub bounded_checks: BoundedChecks,
    pub expires_at: i64,
    pub signer_placeholder: String,
    pub schema_version: String,
}
