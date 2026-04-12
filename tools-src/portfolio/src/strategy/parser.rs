//! Strategy doc parser.
//!
//! Strategy docs are Markdown files with a YAML frontmatter block:
//!
//! ```text
//! ---
//! id: stablecoin-yield-floor
//! version: 1
//! applies_to:
//!   category: stablecoin-idle
//!   min_principal_usd: 100
//! constraints:
//!   min_projected_delta_apy_bps: 50
//!   max_risk_score: 3
//!   gas_payback_days: 30
//! inputs:
//!   floor_apy: 0.04
//! ---
//!
//! # Stablecoin Yield Floor
//!
//! Markdown body the LLM reads for nuance during ranking.
//! ```
//!
//! The body is preserved (the skill playbook hands it back to the
//! LLM at ranking time) but is not interpreted by the deterministic
//! filter.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StrategyAppliesTo {
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub min_principal_usd: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StrategyConstraints {
    #[serde(default)]
    pub min_projected_delta_apy_bps: Option<i32>,
    #[serde(default)]
    pub max_risk_score: Option<u8>,
    #[serde(default)]
    pub max_bridge_legs: Option<u8>,
    #[serde(default)]
    pub gas_payback_days: Option<f32>,
    #[serde(default)]
    pub prefer_same_chain: bool,
    #[serde(default)]
    pub prefer_near_intents: bool,
}

/// Which concrete candidate-enumeration routine a strategy doc drives.
///
/// The routine is selected by the optional `kind` field in the YAML
/// frontmatter, or inferred from the doc's `id` prefix when omitted.
/// This keeps strategy docs backward-compatible (M1/M2 docs didn't
/// carry a `kind` and defaulted to `YieldFloor`) while letting newer
/// docs be explicit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrategyKind {
    #[default]
    YieldFloor,
    HealthGuard,
    LpImpermanentLossWatch,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StrategyFrontmatter {
    pub id: String,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub kind: StrategyKind,
    #[serde(default)]
    pub applies_to: StrategyAppliesTo,
    #[serde(default)]
    pub constraints: StrategyConstraints,
    #[serde(default)]
    pub inputs: BTreeMap<String, serde_json::Value>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone)]
pub struct StrategyDoc {
    pub frontmatter: StrategyFrontmatter,
    /// The Markdown body after the second `---`. Preserved for the LLM.
    #[allow(dead_code)]
    pub body: String,
}

pub fn parse(source: &str) -> Result<StrategyDoc, String> {
    let trimmed = source.trim_start();
    let after_open = trimmed
        .strip_prefix("---")
        .ok_or_else(|| "Strategy doc missing opening '---' frontmatter delimiter".to_string())?;

    // Find the closing '---' on its own line.
    let close_idx = find_frontmatter_close(after_open).ok_or_else(|| {
        "Strategy doc missing closing '---' frontmatter delimiter".to_string()
    })?;
    let frontmatter_yaml = &after_open[..close_idx];
    let body = after_open[close_idx..]
        .trim_start_matches("---")
        .trim_start_matches('\n')
        .to_string();

    let frontmatter: StrategyFrontmatter = serde_yaml::from_str(frontmatter_yaml)
        .map_err(|e| format!("Strategy frontmatter parse error: {e}"))?;

    Ok(StrategyDoc { frontmatter, body })
}

fn find_frontmatter_close(after_open: &str) -> Option<usize> {
    after_open.find("\n---").map(|rel| rel + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_frontmatter() {
        let src = "---\nid: test\napplies_to:\n  category: stablecoin-idle\n---\nbody here\n";
        let doc = parse(src).expect("parse");
        assert_eq!(doc.frontmatter.id, "test");
        assert_eq!(doc.frontmatter.version, 1);
        assert!(doc.body.contains("body here"));
    }

    #[test]
    fn parses_all_fields() {
        let src = "---\nid: stablecoin-yield-floor\nversion: 2\nkind: yield-floor\napplies_to:\n  category: stablecoin-idle\n  min_principal_usd: 100\nconstraints:\n  min_projected_delta_apy_bps: 50\n  max_risk_score: 3\n  gas_payback_days: 30\ninputs:\n  floor_apy: 0.04\n---\n# Title\n\nBody\n";
        let doc = parse(src).unwrap();
        assert_eq!(doc.frontmatter.id, "stablecoin-yield-floor");
        assert_eq!(doc.frontmatter.version, 2);
        assert_eq!(doc.frontmatter.kind, StrategyKind::YieldFloor);
        assert_eq!(doc.frontmatter.applies_to.category, Some("stablecoin-idle".to_string()));
        assert_eq!(doc.frontmatter.applies_to.min_principal_usd, Some(100.0));
        assert_eq!(doc.frontmatter.constraints.min_projected_delta_apy_bps, Some(50));
        assert_eq!(doc.frontmatter.constraints.max_risk_score, Some(3));
        assert!((doc.frontmatter.constraints.gas_payback_days.unwrap() - 30.0).abs() < 0.1);
        assert_eq!(doc.frontmatter.inputs.get("floor_apy").and_then(|v| v.as_f64()), Some(0.04));
    }

    #[test]
    fn missing_opening_delimiter_error() {
        let result = parse("id: test\n---\nbody\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("opening"));
    }

    #[test]
    fn missing_closing_delimiter_error() {
        let result = parse("---\nid: test\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("closing"));
    }

    #[test]
    fn invalid_yaml_error() {
        let result = parse("---\n{{{{not: yaml: at: all\n---\nbody\n");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("parse error"));
    }

    #[test]
    fn whitespace_before_opening_delimiter() {
        let src = "  \n  ---\nid: test\n---\nbody\n";
        let doc = parse(src).unwrap();
        assert_eq!(doc.frontmatter.id, "test");
    }

    #[test]
    fn body_with_frontmatter_like_text() {
        let src = "---\nid: test\n---\nHere is some text.\n\n---\n\nMore text after a horizontal rule.\n";
        let doc = parse(src).unwrap();
        assert!(doc.body.contains("More text"));
    }

    #[test]
    fn body_preserves_markdown() {
        let src = "---\nid: test\n---\n# Header\n\n```python\nprint('hi')\n```\n";
        let doc = parse(src).unwrap();
        assert!(doc.body.contains("# Header"));
        assert!(doc.body.contains("```python"));
    }

    #[test]
    fn empty_body_valid() {
        let src = "---\nid: test\n---\n";
        let doc = parse(src).unwrap();
        assert!(doc.body.is_empty() || doc.body.trim().is_empty());
    }

    #[test]
    fn kind_health_guard() {
        let src = "---\nid: test\nkind: health-guard\n---\nbody\n";
        let doc = parse(src).unwrap();
        assert_eq!(doc.frontmatter.kind, StrategyKind::HealthGuard);
    }

    #[test]
    fn kind_lp_watch() {
        let src = "---\nid: test\nkind: lp-impermanent-loss-watch\n---\nbody\n";
        let doc = parse(src).unwrap();
        assert_eq!(doc.frontmatter.kind, StrategyKind::LpImpermanentLossWatch);
    }

    #[test]
    fn kind_defaults_to_yield_floor() {
        let src = "---\nid: test\n---\nbody\n";
        let doc = parse(src).unwrap();
        assert_eq!(doc.frontmatter.kind, StrategyKind::YieldFloor);
    }

    #[test]
    fn missing_applies_to_defaults_empty() {
        let src = "---\nid: test\n---\nbody\n";
        let doc = parse(src).unwrap();
        assert!(doc.frontmatter.applies_to.category.is_none());
        assert!(doc.frontmatter.applies_to.min_principal_usd.is_none());
    }

    #[test]
    fn missing_constraints_defaults_empty() {
        let src = "---\nid: test\n---\nbody\n";
        let doc = parse(src).unwrap();
        assert!(doc.frontmatter.constraints.min_projected_delta_apy_bps.is_none());
        assert!(doc.frontmatter.constraints.max_risk_score.is_none());
        assert!(doc.frontmatter.constraints.gas_payback_days.is_none());
        assert!(!doc.frontmatter.constraints.prefer_same_chain);
    }

    #[test]
    fn find_frontmatter_close_normal() {
        assert_eq!(find_frontmatter_close("\nid: test\n---\nbody"), Some(10));
    }

    #[test]
    fn find_frontmatter_close_not_found() {
        assert!(find_frontmatter_close("\nid: test\n").is_none());
    }

    #[test]
    fn parses_real_stablecoin_strategy() {
        let src = include_str!("../../strategies/stablecoin-yield-floor.md");
        let doc = parse(src).expect("parse stablecoin strategy");
        assert_eq!(doc.frontmatter.id, "stablecoin-yield-floor");
    }

    #[test]
    fn parses_real_health_guard_strategy() {
        let src = include_str!("../../strategies/lending-health-guard.md");
        let doc = parse(src).expect("parse health guard strategy");
        assert_eq!(doc.frontmatter.kind, StrategyKind::HealthGuard);
    }

    #[test]
    fn parses_real_lp_watch_strategy() {
        let src = include_str!("../../strategies/lp-impermanent-loss-watch.md");
        let doc = parse(src).expect("parse lp watch strategy");
        assert_eq!(doc.frontmatter.kind, StrategyKind::LpImpermanentLossWatch);
    }
}
