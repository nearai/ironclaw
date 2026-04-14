//! Shared types for the portfolio tool.
//!
//! Money values are serialized as strings throughout (`"1234.56"`) to
//! avoid float precision issues across the JSON boundary. Internal math
//! that needs to compare or sort uses `parse_decimal` to convert to
//! `f64`. This is sufficient for ranking and threshold checks; it is
//! not used for any settlement math.

mod intent;
mod position;
mod proposal;

pub use intent::{BoundedChecks, IntentBundle, IntentLeg};
#[allow(unused_imports)]
pub use position::HealthMetric;
pub use position::{
    ChainSelector, ClassifiedPosition, ProtocolRef, RawPosition, ScanAt, TokenAmount,
};
pub use proposal::{
    CostBreakdown, MovementLeg, MovementPlan, PositionRef, ProjectConfig, Proposal,
};

/// Parse a decimal string into f64. Falls back to 0.0 on parse failure.
/// Used for ranking/threshold math, never for settlement.
pub fn parse_decimal(s: &str) -> f64 {
    s.parse::<f64>().unwrap_or(0.0)
}

/// Format an f64 as a fixed-precision decimal string.
pub fn format_decimal(value: f64) -> String {
    format!("{value:.6}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decimal_valid() {
        assert!((parse_decimal("123.45") - 123.45).abs() < 1e-9);
    }

    #[test]
    fn parse_decimal_integer() {
        assert!((parse_decimal("100") - 100.0).abs() < 1e-9);
    }

    #[test]
    fn parse_decimal_invalid_returns_zero() {
        assert_eq!(parse_decimal("not-a-number"), 0.0);
    }

    #[test]
    fn parse_decimal_empty_returns_zero() {
        assert_eq!(parse_decimal(""), 0.0);
    }

    #[test]
    fn parse_decimal_negative() {
        assert!((parse_decimal("-50.5") - -50.5).abs() < 1e-9);
    }

    #[test]
    fn parse_decimal_scientific() {
        assert!((parse_decimal("1e3") - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn parse_decimal_very_small() {
        assert!((parse_decimal("0.000001") - 1e-6).abs() < 1e-12);
    }

    #[test]
    fn format_decimal_precision() {
        assert_eq!(format_decimal(123.456789), "123.456789");
    }

    #[test]
    fn chain_selector_wildcard_is_all() {
        assert!(ChainSelector::Wildcard("*".to_string()).is_all());
    }

    #[test]
    fn chain_selector_non_star_wildcard_not_all() {
        assert!(!ChainSelector::Wildcard("ethereum".to_string()).is_all());
    }

    #[test]
    fn chain_selector_list_not_all() {
        assert!(!ChainSelector::List(vec!["ethereum".into()]).is_all());
    }

    #[test]
    fn chain_selector_as_list_wildcard_none() {
        assert!(ChainSelector::Wildcard("*".into()).as_list().is_none());
    }

    #[test]
    fn chain_selector_as_list_returns_slice() {
        let sel = ChainSelector::List(vec!["base".into(), "ethereum".into()]);
        let list = sel.as_list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], "base");
    }

    #[test]
    fn chain_selector_default_is_wildcard_star() {
        let sel = ChainSelector::default();
        assert!(sel.is_all());
    }

    #[test]
    fn chain_selector_serde_roundtrip_wildcard() {
        let json = serde_json::to_string(&ChainSelector::Wildcard("*".into())).unwrap();
        assert_eq!(json, r#""*""#);
    }

    #[test]
    fn chain_selector_serde_roundtrip_list() {
        let json = serde_json::to_string(&ChainSelector::List(vec!["base".into()])).unwrap();
        assert_eq!(json, r#"["base"]"#);
    }
}
