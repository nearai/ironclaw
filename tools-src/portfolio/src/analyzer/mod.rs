//! Analyzer stage — classify raw positions against the embedded
//! protocol registry.
//!
//! Each protocol JSON declares: a `position_detector` (matches a
//! `RawPosition.protocol_id` against a stable id), a `yield_model`
//! (how to compute net APY from the raw payload), a `risk` block
//! (base score, audits, TVL floor), and a `liquidity_tier`.
//!
//! M1 ships only one synthetic protocol (`test-lending`) so the
//! whole pipeline can be exercised end-to-end without depending on
//! a real chain. M2 expands to Aave/Compound/Uniswap/Lido/Morpho.

use crate::types::{ClassifiedPosition, ProtocolRef, RawPosition};

mod registry;

pub fn classify(raw: Vec<RawPosition>) -> Result<Vec<ClassifiedPosition>, String> {
    static REGISTRY: std::sync::OnceLock<Result<Vec<ProtocolEntry>, String>> =
        std::sync::OnceLock::new();
    let registry = REGISTRY
        .get_or_init(registry::load)
        .as_ref()
        .map_err(|e| e.clone())?;

    let mut out = Vec::with_capacity(raw.len());
    for position in raw {
        let entry = registry.iter().find(|p| p.detector_matches(&position));
        let Some(entry) = entry else {
            // Unknown protocol — skip rather than crash. Plan §5
            // hostile/malicious-protocol scenario depends on this.
            continue;
        };
        out.push(entry.classify(position)?);
    }
    Ok(out)
}

/// Internal classifier built from a JSON protocol entry.
pub(crate) struct ProtocolEntry {
    pub id: String,
    pub name: String,
    pub category: String,
    /// Stable id strings this protocol's positions can be matched on.
    pub match_protocol_ids: Vec<String>,
    pub base_risk_score: u8,
    pub liquidity_tier: String,
    pub withdrawal_delay_seconds: u64,
    /// Field path inside `raw_metadata` for net APY (very simple model
    /// for M1: a single field. M2 protocol entries can extend this).
    pub yield_field: String,
}

impl ProtocolEntry {
    fn detector_matches(&self, position: &RawPosition) -> bool {
        self.match_protocol_ids
            .iter()
            .any(|id| id == &position.protocol_id)
    }

    fn classify(&self, position: RawPosition) -> Result<ClassifiedPosition, String> {
        let principal_usd = sum_value_usd(&position.token_balances);
        let debt_usd = sum_value_usd(&position.debt_balances);

        let net_yield_apy = position
            .raw_metadata
            .get(&self.yield_field)
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        // Stablecoin idle category if a single stablecoin balance and
        // category is lending. Cheap heuristic for M1.
        let is_stablecoin = position
            .token_balances
            .iter()
            .all(|b| matches!(b.symbol.as_str(), "USDC" | "USDT" | "DAI" | "USDe"));
        let category = if self.category == "lending" && is_stablecoin {
            "stablecoin-idle".to_string()
        } else {
            self.category.clone()
        };

        let health = extract_health(&position.raw_metadata);

        Ok(ClassifiedPosition {
            protocol: ProtocolRef {
                id: self.id.clone(),
                name: self.name.clone(),
            },
            category,
            chain: position.chain.clone(),
            address: position.address.clone(),
            principal_usd: format!("{principal_usd:.2}"),
            debt_usd: format!("{debt_usd:.2}"),
            net_yield_apy,
            unrealized_pnl_usd: "0.00".to_string(),
            risk_score: self.base_risk_score,
            exit_cost_estimate_usd: "0.00".to_string(),
            withdrawal_delay_seconds: self.withdrawal_delay_seconds,
            liquidity_tier: self.liquidity_tier.clone(),
            health,
            tags: Vec::new(),
            raw_position: position,
        })
    }
}

/// Extract a health metric from raw metadata. Returns `None` when
/// no known field is present. The `warning` flag is left `false`
/// here — strategies apply their own threshold.
fn extract_health(metadata: &serde_json::Value) -> Option<crate::types::HealthMetric> {
    use crate::types::HealthMetric;
    for (name, field) in [
        ("health_factor", "health_factor"),
        ("borrow_collateralization", "borrow_collateralization"),
        ("ltv", "ltv"),
    ] {
        if let Some(v) = metadata.get(field).and_then(|v| v.as_f64()) {
            return Some(HealthMetric {
                name: name.to_string(),
                value: v,
                warning: false,
            });
        }
    }
    None
}

fn sum_value_usd(balances: &[crate::types::TokenAmount]) -> f64 {
    balances
        .iter()
        .map(|b| crate::types::parse_decimal(&b.value_usd))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RawPosition, TokenAmount};
    use serde_json::json;

    fn make_raw(
        protocol_id: &str,
        chain: &str,
        symbol: &str,
        amount: &str,
        value_usd: &str,
        metadata: serde_json::Value,
    ) -> RawPosition {
        RawPosition {
            chain: chain.to_string(),
            protocol_id: protocol_id.to_string(),
            position_type: "supply".to_string(),
            address: "0xtest".to_string(),
            token_balances: vec![TokenAmount {
                symbol: symbol.to_string(),
                address: None,
                chain: chain.to_string(),
                amount: amount.to_string(),
                value_usd: value_usd.to_string(),
            }],
            debt_balances: Vec::new(),
            reward_balances: Vec::new(),
            raw_metadata: metadata,
            block_number: 1,
            fetched_at: 0,
        }
    }

    fn classify_one(raw: RawPosition) -> ClassifiedPosition {
        let out = classify(vec![raw]).expect("classify");
        assert_eq!(out.len(), 1, "expected exactly one classified position");
        out.into_iter().next().unwrap()
    }

    #[test]
    fn classifies_aave_v3_usdc_as_stablecoin_idle() {
        let raw = make_raw(
            "aave_v3",
            "base",
            "USDC",
            "5000",
            "5000.00",
            json!({"supply_apy": 0.038}),
        );
        let p = classify_one(raw);
        assert_eq!(p.protocol.id, "aave-v3");
        assert_eq!(p.category, "stablecoin-idle");
        assert_eq!(p.risk_score, 2);
        assert!((p.net_yield_apy - 0.038).abs() < 1e-9);
        assert_eq!(p.liquidity_tier, "instant");
    }

    #[test]
    fn classifies_compound_v3_dai() {
        let raw = make_raw(
            "compound_v3",
            "ethereum",
            "DAI",
            "1000",
            "1000.00",
            json!({"supply_apy": 0.041}),
        );
        let p = classify_one(raw);
        assert_eq!(p.protocol.id, "compound-v3");
        assert_eq!(p.category, "stablecoin-idle");
        assert_eq!(p.risk_score, 2);
    }

    #[test]
    fn classifies_uniswap_v3_lp_as_dex_lp_not_stablecoin_idle() {
        let raw = make_raw(
            "uniswap_v3",
            "ethereum",
            "UNI-V3-LP",
            "1",
            "5400.00",
            json!({"fee_apy": 0.092}),
        );
        let p = classify_one(raw);
        assert_eq!(p.protocol.id, "uniswap-v3");
        assert_eq!(p.category, "dex-lp");
        assert_eq!(p.risk_score, 3);
        assert!((p.net_yield_apy - 0.092).abs() < 1e-9);
    }

    #[test]
    fn classifies_lido_steth_as_staking_with_delay() {
        let raw = make_raw(
            "lido",
            "ethereum",
            "stETH",
            "3.5",
            "12250.00",
            json!({"staking_apy": 0.034}),
        );
        let p = classify_one(raw);
        assert_eq!(p.protocol.id, "lido");
        assert_eq!(p.category, "staking");
        assert_eq!(p.liquidity_tier, "minutes");
        assert_eq!(p.withdrawal_delay_seconds, 86400);
        assert!((p.net_yield_apy - 0.034).abs() < 1e-9);
    }

    #[test]
    fn classifies_morpho_blue_usdc_as_stablecoin_idle() {
        let raw = make_raw(
            "morpho_blue",
            "base",
            "USDC",
            "1500",
            "1500.00",
            json!({"supply_apy": 0.054}),
        );
        let p = classify_one(raw);
        assert_eq!(p.protocol.id, "morpho-blue");
        assert_eq!(p.category, "stablecoin-idle");
        assert_eq!(p.risk_score, 2);
    }

    #[test]
    fn classifier_skips_unknown_protocols_without_crashing() {
        let raw = make_raw(
            "totally-unheard-of",
            "ethereum",
            "GOLD",
            "1",
            "0",
            json!({}),
        );
        let out = classify(vec![raw]).expect("classify");
        assert!(
            out.is_empty(),
            "unknown protocols should be silently dropped"
        );
    }

    #[test]
    fn classifier_handles_empty_input() {
        let out = classify(vec![]).expect("classify");
        assert!(out.is_empty());
    }

    // ---- stablecoin detection ----

    #[test]
    fn usdt_in_lending_becomes_stablecoin_idle() {
        let raw = make_raw(
            "aave_v3",
            "base",
            "USDT",
            "1000",
            "1000.00",
            json!({"supply_apy": 0.03}),
        );
        let p = classify_one(raw);
        assert_eq!(p.category, "stablecoin-idle");
    }

    #[test]
    fn usde_in_lending_becomes_stablecoin_idle() {
        let raw = make_raw(
            "morpho_blue",
            "base",
            "USDe",
            "1000",
            "1000.00",
            json!({"supply_apy": 0.05}),
        );
        let p = classify_one(raw);
        assert_eq!(p.category, "stablecoin-idle");
    }

    #[test]
    fn non_stablecoin_in_lending_stays_lending() {
        let raw = make_raw(
            "aave_v3",
            "ethereum",
            "WETH",
            "1",
            "3500.00",
            json!({"supply_apy": 0.01}),
        );
        let p = classify_one(raw);
        assert_eq!(p.category, "lending");
    }

    // ---- health extraction ----

    #[test]
    fn extracts_health_factor() {
        let raw = make_raw(
            "aave_v3",
            "base",
            "USDC",
            "1000",
            "1000.00",
            json!({"supply_apy": 0.03, "health_factor": 1.15}),
        );
        let p = classify_one(raw);
        let health = p.health.unwrap();
        assert_eq!(health.name, "health_factor");
        assert!((health.value - 1.15).abs() < 1e-9);
        assert!(!health.warning);
    }

    #[test]
    fn extracts_ltv_as_health_when_no_health_factor() {
        let raw = make_raw(
            "compound_v3",
            "ethereum",
            "DAI",
            "1000",
            "1000.00",
            json!({"supply_apy": 0.04, "ltv": 0.75}),
        );
        let p = classify_one(raw);
        let health = p.health.unwrap();
        assert_eq!(health.name, "ltv");
    }

    #[test]
    fn no_health_fields_returns_none() {
        let raw = make_raw(
            "aave_v3",
            "base",
            "USDC",
            "1000",
            "1000.00",
            json!({"supply_apy": 0.03}),
        );
        let p = classify_one(raw);
        assert!(p.health.is_none());
    }

    #[test]
    fn health_factor_non_numeric_returns_none() {
        let raw = make_raw(
            "aave_v3",
            "base",
            "USDC",
            "1000",
            "1000.00",
            json!({"supply_apy": 0.03, "health_factor": "not-a-number"}),
        );
        let p = classify_one(raw);
        assert!(p.health.is_none());
    }

    // ---- debt and principal ----

    #[test]
    fn debt_balances_summed() {
        let mut raw = make_raw(
            "compound_v3",
            "ethereum",
            "USDC",
            "5000",
            "5000.00",
            json!({"supply_apy": 0.04}),
        );
        raw.debt_balances = vec![
            TokenAmount {
                symbol: "ETH".into(),
                address: None,
                chain: "ethereum".into(),
                amount: "1".into(),
                value_usd: "3500.00".into(),
            },
            TokenAmount {
                symbol: "WBTC".into(),
                address: None,
                chain: "ethereum".into(),
                amount: "0.1".into(),
                value_usd: "6000.00".into(),
            },
        ];
        let p = classify_one(raw);
        assert_eq!(p.debt_usd, "9500.00");
    }

    #[test]
    fn multiple_token_balances_summed() {
        let mut raw = make_raw(
            "aave_v3",
            "base",
            "USDC",
            "1000",
            "1000.00",
            json!({"supply_apy": 0.03}),
        );
        raw.token_balances.push(TokenAmount {
            symbol: "USDC".into(),
            address: None,
            chain: "base".into(),
            amount: "500".into(),
            value_usd: "500.00".into(),
        });
        let p = classify_one(raw);
        assert_eq!(p.principal_usd, "1500.00");
    }

    // ---- yield extraction ----

    #[test]
    fn missing_yield_field_defaults_to_zero() {
        let raw = make_raw("aave_v3", "base", "USDC", "1000", "1000.00", json!({}));
        let p = classify_one(raw);
        assert!((p.net_yield_apy - 0.0).abs() < 1e-9);
    }

    #[test]
    fn null_metadata_handled() {
        let raw = make_raw(
            "aave_v3",
            "base",
            "USDC",
            "1000",
            "1000.00",
            serde_json::Value::Null,
        );
        let p = classify_one(raw);
        assert!((p.net_yield_apy - 0.0).abs() < 1e-9);
    }

    // ---- registry completeness ----

    #[test]
    fn registry_loads_all_protocols() {
        let reg = registry::load().unwrap();
        let ids: Vec<&str> = reg.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"test-lending"));
        assert!(ids.contains(&"aave-v3"));
        assert!(ids.contains(&"compound-v3"));
        assert!(ids.contains(&"uniswap-v3"));
        assert!(ids.contains(&"lido"));
        assert!(ids.contains(&"morpho-blue"));
    }
}
