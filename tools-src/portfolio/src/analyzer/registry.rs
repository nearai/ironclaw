//! Loads embedded protocol JSON files into `ProtocolEntry` instances.
//!
//! Adding a new protocol is two lines: drop a JSON file under
//! `protocols/` and add it to `PROTOCOL_SOURCES` below. The
//! hand-rolled list (vs `include_dir!`) is intentional — it makes
//! the embedded protocol set diff-visible in code review.

use serde::Deserialize;

use super::ProtocolEntry;

const PROTOCOL_SOURCES: &[(&str, &str)] = &[
    ("test-lending", include_str!("../../protocols/test-lending.json")),
    ("aave-v3", include_str!("../../protocols/aave-v3.json")),
    ("compound-v3", include_str!("../../protocols/compound-v3.json")),
    ("uniswap-v3", include_str!("../../protocols/uniswap-v3.json")),
    ("lido", include_str!("../../protocols/lido.json")),
    ("morpho-blue", include_str!("../../protocols/morpho-blue.json")),
];

#[derive(Debug, Deserialize)]
struct ProtocolFile {
    id: String,
    name: String,
    category: String,
    #[serde(default)]
    match_protocol_ids: Vec<String>,
    #[serde(default)]
    base_risk_score: u8,
    #[serde(default = "default_liquidity_tier")]
    liquidity_tier: String,
    #[serde(default)]
    withdrawal_delay_seconds: u64,
    #[serde(default = "default_yield_field")]
    yield_field: String,
}

fn default_liquidity_tier() -> String {
    "instant".to_string()
}

fn default_yield_field() -> String {
    "supply_apy".to_string()
}

pub(crate) fn load() -> Result<Vec<ProtocolEntry>, String> {
    let mut entries = Vec::with_capacity(PROTOCOL_SOURCES.len());
    for (label, src) in PROTOCOL_SOURCES {
        let file: ProtocolFile = serde_json::from_str(src)
            .map_err(|e| format!("Embedded protocol JSON '{label}' is invalid: {e}"))?;
        entries.push(ProtocolEntry {
            id: file.id,
            name: file.name,
            category: file.category,
            match_protocol_ids: file.match_protocol_ids,
            base_risk_score: file.base_risk_score,
            liquidity_tier: file.liquidity_tier,
            withdrawal_delay_seconds: file.withdrawal_delay_seconds,
            yield_field: file.yield_field,
        });
    }
    Ok(entries)
}
