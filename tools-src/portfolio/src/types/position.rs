use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Selector for which chains to scan.
///
/// Accepts either a wildcard string `"*"` or an explicit list of chain
/// IDs. Untagged on the wire so callers can pass whichever shape is
/// natural.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ChainSelector {
    /// Wildcard `"*"` — scan everything the source supports.
    Wildcard(String),
    /// Explicit list of chain IDs.
    List(Vec<String>),
}

impl ChainSelector {
    pub fn is_all(&self) -> bool {
        match self {
            ChainSelector::Wildcard(s) => s == "*",
            ChainSelector::List(_) => false,
        }
    }

    pub fn as_list(&self) -> Option<&[String]> {
        match self {
            ChainSelector::List(v) => Some(v),
            ChainSelector::Wildcard(_) => None,
        }
    }
}

impl Default for ChainSelector {
    fn default() -> Self {
        ChainSelector::Wildcard("*".to_string())
    }
}

/// Optional point-in-time scan parameter.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ScanAt {
    /// Per-chain block heights. Used by indexer backends that support
    /// historical queries by block.
    #[serde(default)]
    pub block: BTreeMap<String, u64>,
    /// Unix timestamp in seconds. Used by indexer backends that support
    /// historical queries by timestamp.
    #[serde(default)]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TokenAmount {
    pub symbol: String,
    pub address: Option<String>,
    pub chain: String,
    /// Decimal string ("1234.56").
    pub amount: String,
    /// USD value as a decimal string. May be empty if pricing
    /// is unavailable for this token.
    #[serde(default)]
    pub value_usd: String,
}

/// Reference to a protocol by stable ID.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProtocolRef {
    pub id: String,
    pub name: String,
}

/// A raw position straight from the indexer, before classification.
///
/// Backend-specific metadata is preserved in `raw_metadata` so the
/// analyzer can apply protocol-specific yield models without losing
/// information.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RawPosition {
    pub chain: String,
    pub protocol_id: String,
    pub position_type: String,
    pub address: String,
    #[serde(default)]
    pub token_balances: Vec<TokenAmount>,
    #[serde(default)]
    pub debt_balances: Vec<TokenAmount>,
    #[serde(default)]
    pub reward_balances: Vec<TokenAmount>,
    #[serde(default)]
    pub raw_metadata: serde_json::Value,
    pub block_number: u64,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthMetric {
    pub name: String,
    pub value: f64,
    pub warning: bool,
}

/// A position after classification by the analyzer stage.
///
/// Adds typed yield, risk, and liquidity fields derived from the
/// protocol registry plus the raw indexer payload.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClassifiedPosition {
    pub protocol: ProtocolRef,
    pub category: String,
    pub chain: String,
    pub address: String,
    pub principal_usd: String,
    pub debt_usd: String,
    pub net_yield_apy: f64,
    #[serde(default)]
    pub unrealized_pnl_usd: String,
    pub risk_score: u8,
    #[serde(default)]
    pub exit_cost_estimate_usd: String,
    pub withdrawal_delay_seconds: u64,
    pub liquidity_tier: String,
    #[serde(default)]
    pub health: Option<HealthMetric>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The raw position the analyzer started from. Kept so strategies
    /// can fall back to backend-specific fields if needed.
    pub raw_position: RawPosition,
}
