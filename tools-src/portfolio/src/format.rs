//! Deterministic output formatters.
//!
//! These are pure functions that the skill playbook calls to produce
//! the files it writes to the project workspace:
//!
//! - `format_suggestion_md` → `projects/<id>/suggestions/<date>.md`
//! - `format_progress` → the progress metric line the mission writes
//!   to its memory docs.
//!
//! Widgets get their own formatter in `widget.rs` (M5).
//!
//! All output is deterministic so snapshot tests are meaningful.

use serde::{Deserialize, Serialize};

use crate::types::{parse_decimal, ClassifiedPosition, ProjectConfig, Proposal};

#[derive(Debug, Clone, Deserialize)]
pub struct FormatSuggestionInput {
    pub positions: Vec<ClassifiedPosition>,
    pub proposals: Vec<Proposal>,
    pub config: ProjectConfig,
    #[serde(default)]
    pub generated_at: Option<String>,
    #[serde(default)]
    pub previous_total_value_usd: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FormatSuggestionOutput {
    pub markdown: String,
    pub totals: Totals,
}

#[derive(Debug, Serialize)]
pub struct Totals {
    pub net_value_usd: String,
    pub weighted_net_apy: f64,
    pub delta_vs_previous_usd: Option<String>,
}

pub fn format_suggestion_md(input: FormatSuggestionInput) -> FormatSuggestionOutput {
    let totals = compute_totals(&input);

    let mut md = String::new();
    md.push_str("# Portfolio update\n\n");
    if let Some(ts) = &input.generated_at {
        md.push_str(&format!("_Generated: {ts}_\n\n"));
    }
    md.push_str(&format!(
        "**Net value**: ${} · **Weighted net APY**: {:.2}%",
        totals.net_value_usd,
        totals.weighted_net_apy * 100.0
    ));
    if let Some(delta) = &totals.delta_vs_previous_usd {
        md.push_str(&format!(" · **Δ vs last run**: ${delta}"));
    }
    md.push_str("\n\n");

    md.push_str("## Positions\n\n");
    if input.positions.is_empty() {
        md.push_str("_No positions._\n\n");
    } else {
        md.push_str("| Protocol | Chain | Category | Principal (USD) | Net APY | Risk |\n");
        md.push_str("|---|---|---|---|---|---|\n");
        for p in &input.positions {
            md.push_str(&format!(
                "| {} | {} | {} | ${} | {:.2}% | {} |\n",
                p.protocol.name,
                p.chain,
                p.category,
                p.principal_usd,
                p.net_yield_apy * 100.0,
                p.risk_score
            ));
        }
        md.push('\n');
    }

    let ready: Vec<&Proposal> = input
        .proposals
        .iter()
        .filter(|p| p.status == "ready")
        .collect();
    let blocked: Vec<&Proposal> = input
        .proposals
        .iter()
        .filter(|p| p.status != "ready")
        .collect();

    md.push_str("## Suggestions\n\n");
    if ready.is_empty() {
        md.push_str("_No actionable proposals this run._\n\n");
    } else {
        md.push_str(
            "| # | Strategy | Source → Target | Δ APY (bps) | Annual gain (USD) | Payback | Status |\n",
        );
        md.push_str("|---|---|---|---|---|---|---|\n");
        for (idx, p) in ready.iter().enumerate() {
            let src = p
                .from_positions
                .first()
                .map(|r| r.protocol_id.as_str())
                .unwrap_or("?");
            md.push_str(&format!(
                "| {} | {} | {} → {} | {} | ${} | {:.0}d | {} |\n",
                idx + 1,
                p.strategy_id,
                src,
                p.to_protocol.name,
                p.projected_delta_apy_bps,
                p.projected_annual_gain_usd,
                p.gas_payback_days,
                p.status
            ));
        }
        md.push('\n');
        md.push_str("### Rationale\n\n");
        for (idx, p) in ready.iter().enumerate() {
            md.push_str(&format!("{}. {}\n", idx + 1, p.rationale));
        }
        md.push('\n');
    }

    if !blocked.is_empty() {
        md.push_str("## Non-actionable\n\n");
        for p in &blocked {
            md.push_str(&format!("- **{}** ({}): {}\n", p.strategy_id, p.status, p.rationale));
        }
        md.push('\n');
    }

    md.push_str("## Config\n\n");
    md.push_str(&format!(
        "- floor_apy: {:.2}%\n- max_risk_score: {}\n- notify_threshold_usd: ${:.0}\n- auto_intent_ceiling_usd: ${:.0}\n- max_slippage_bps: {}\n",
        input.config.floor_apy * 100.0,
        input.config.max_risk_score,
        input.config.notify_threshold_usd,
        input.config.auto_intent_ceiling_usd,
        input.config.max_slippage_bps,
    ));

    FormatSuggestionOutput { markdown: md, totals }
}

fn compute_totals(input: &FormatSuggestionInput) -> Totals {
    let mut net_value = 0.0f64;
    let mut weighted_apy_numerator = 0.0f64;
    for p in &input.positions {
        let principal = parse_decimal(&p.principal_usd);
        net_value += principal;
        weighted_apy_numerator += principal * p.net_yield_apy;
    }
    let weighted_apy = if net_value > 0.0 {
        weighted_apy_numerator / net_value
    } else {
        0.0
    };

    let delta_vs_previous_usd = input
        .previous_total_value_usd
        .as_ref()
        .map(|prev_s| {
            let prev = parse_decimal(prev_s);
            let delta = net_value - prev;
            format!("{delta:+.2}")
        });

    Totals {
        net_value_usd: format!("{net_value:.2}"),
        weighted_net_apy: weighted_apy,
        delta_vs_previous_usd,
    }
}

// -------------------- progress metric --------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ProgressInput {
    /// Dated historical state snapshots. Each snapshot is a prior
    /// run's `state/history/<date>.json` content. Order doesn't
    /// matter — the metric computes weighted APY across them.
    pub history: Vec<ProgressSnapshot>,
    pub config: ProjectConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProgressSnapshot {
    pub date: String,
    pub positions: Vec<ClassifiedPosition>,
}

#[derive(Debug, Serialize)]
pub struct ProgressOutput {
    pub samples: usize,
    pub realized_net_apy_7d: f64,
    pub delta_vs_floor: f64,
    pub floor_apy: f64,
    pub average_total_value_usd: String,
    /// Negative when the portfolio is below floor — the keeper
    /// mission's goal says the metric should be >= 0.
    pub progress_score: f64,
}

/// Weighted-average realized APY across the supplied history,
/// compared against the config's floor_apy.
pub fn format_progress(input: ProgressInput) -> ProgressOutput {
    // Take the most recent 7 snapshots (M3: we don't care about
    // strict date parsing; we sort by date string which is ISO-ish
    // so it sorts chronologically).
    let mut sorted = input.history.clone();
    sorted.sort_by(|a, b| a.date.cmp(&b.date));
    let window: Vec<&ProgressSnapshot> = sorted.iter().rev().take(7).collect();

    let mut total_value_sum = 0.0f64;
    let mut apy_numerator = 0.0f64;
    let mut apy_denominator = 0.0f64;
    for snap in &window {
        for p in &snap.positions {
            let principal = parse_decimal(&p.principal_usd);
            total_value_sum += principal;
            apy_numerator += principal * p.net_yield_apy;
            apy_denominator += principal;
        }
    }
    let samples = window.len();
    let realized = if apy_denominator > 0.0 {
        apy_numerator / apy_denominator
    } else {
        0.0
    };
    let average_total = if samples > 0 {
        total_value_sum / samples as f64
    } else {
        0.0
    };
    let floor = input.config.floor_apy;
    let delta = realized - floor;
    let score = if floor > 0.0 { delta / floor } else { delta };

    ProgressOutput {
        samples,
        realized_net_apy_7d: realized,
        delta_vs_floor: delta,
        floor_apy: floor,
        average_total_value_usd: format!("{average_total:.2}"),
        progress_score: score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ProtocolRef, RawPosition};

    fn pos(protocol_id: &str, principal: &str, apy: f64) -> ClassifiedPosition {
        ClassifiedPosition {
            protocol: ProtocolRef {
                id: protocol_id.to_string(),
                name: protocol_id.to_string(),
            },
            category: "lending".to_string(),
            chain: "base".to_string(),
            address: "0x0".to_string(),
            principal_usd: principal.to_string(),
            debt_usd: "0.00".to_string(),
            net_yield_apy: apy,
            unrealized_pnl_usd: "0.00".to_string(),
            risk_score: 2,
            exit_cost_estimate_usd: "0.00".to_string(),
            withdrawal_delay_seconds: 0,
            liquidity_tier: "instant".to_string(),
            health: None,
            tags: vec![],
            raw_position: RawPosition {
                chain: "base".to_string(),
                protocol_id: protocol_id.to_string(),
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
    fn suggestion_md_has_stable_structure() {
        let input = FormatSuggestionInput {
            positions: vec![pos("aave-v3", "1000.00", 0.03), pos("morpho-blue", "500.00", 0.055)],
            proposals: vec![],
            config: ProjectConfig::default(),
            generated_at: Some("2026-04-11T12:00:00Z".to_string()),
            previous_total_value_usd: Some("1400.00".to_string()),
        };
        let out = format_suggestion_md(input);
        assert!(out.markdown.contains("# Portfolio update"));
        assert!(out.markdown.contains("## Positions"));
        assert!(out.markdown.contains("## Suggestions"));
        assert!(out.markdown.contains("## Config"));
        assert_eq!(out.totals.net_value_usd, "1500.00");
        assert_eq!(out.totals.delta_vs_previous_usd.as_deref(), Some("+100.00"));
    }

    #[test]
    fn progress_computes_weighted_apy() {
        let config = ProjectConfig {
            floor_apy: 0.04,
            ..ProjectConfig::default()
        };
        let input = ProgressInput {
            history: vec![
                ProgressSnapshot {
                    date: "2026-04-01".to_string(),
                    positions: vec![pos("aave-v3", "1000.00", 0.03)],
                },
                ProgressSnapshot {
                    date: "2026-04-02".to_string(),
                    positions: vec![pos("morpho-blue", "1000.00", 0.05)],
                },
            ],
            config,
        };
        let out = format_progress(input);
        assert_eq!(out.samples, 2);
        // (1000*0.03 + 1000*0.05) / 2000 = 0.04
        assert!((out.realized_net_apy_7d - 0.04).abs() < 1e-9);
        assert!((out.delta_vs_floor).abs() < 1e-9);
        assert!((out.progress_score).abs() < 1e-9);
    }

    #[test]
    fn progress_handles_empty_history() {
        let input = ProgressInput {
            history: vec![],
            config: ProjectConfig::default(),
        };
        let out = format_progress(input);
        assert_eq!(out.samples, 0);
        assert_eq!(out.realized_net_apy_7d, 0.0);
    }

    // ---- compute_totals edge cases ----

    #[test]
    fn totals_single_position() {
        let input = FormatSuggestionInput {
            positions: vec![pos("aave", "2500.50", 0.05)],
            proposals: vec![],
            config: ProjectConfig::default(),
            generated_at: None,
            previous_total_value_usd: None,
        };
        let out = format_suggestion_md(input);
        assert_eq!(out.totals.net_value_usd, "2500.50");
        assert!((out.totals.weighted_net_apy - 0.05).abs() < 1e-9);
        assert!(out.totals.delta_vs_previous_usd.is_none());
    }

    #[test]
    fn totals_zero_positions() {
        let input = FormatSuggestionInput {
            positions: vec![],
            proposals: vec![],
            config: ProjectConfig::default(),
            generated_at: None,
            previous_total_value_usd: None,
        };
        let out = format_suggestion_md(input);
        assert_eq!(out.totals.net_value_usd, "0.00");
        assert_eq!(out.totals.weighted_net_apy, 0.0);
    }

    #[test]
    fn totals_weighted_apy_multiple() {
        // 1000 @ 3% + 3000 @ 5% = (30 + 150) / 4000 = 0.045
        let input = FormatSuggestionInput {
            positions: vec![pos("aave", "1000.00", 0.03), pos("morpho", "3000.00", 0.05)],
            proposals: vec![],
            config: ProjectConfig::default(),
            generated_at: None,
            previous_total_value_usd: None,
        };
        let out = format_suggestion_md(input);
        assert!((out.totals.weighted_net_apy - 0.045).abs() < 1e-9);
    }

    #[test]
    fn totals_negative_delta() {
        let input = FormatSuggestionInput {
            positions: vec![pos("aave", "900.00", 0.03)],
            proposals: vec![],
            config: ProjectConfig::default(),
            generated_at: None,
            previous_total_value_usd: Some("1000.00".to_string()),
        };
        let out = format_suggestion_md(input);
        assert_eq!(out.totals.delta_vs_previous_usd.as_deref(), Some("-100.00"));
    }

    // ---- markdown structure ----

    #[test]
    fn no_positions_shows_no_positions_msg() {
        let input = FormatSuggestionInput {
            positions: vec![],
            proposals: vec![],
            config: ProjectConfig::default(),
            generated_at: None,
            previous_total_value_usd: None,
        };
        let out = format_suggestion_md(input);
        assert!(out.markdown.contains("_No positions._"));
    }

    #[test]
    fn no_ready_proposals_shows_no_actionable_msg() {
        let input = FormatSuggestionInput {
            positions: vec![pos("aave", "1000", 0.03)],
            proposals: vec![],
            config: ProjectConfig::default(),
            generated_at: None,
            previous_total_value_usd: None,
        };
        let out = format_suggestion_md(input);
        assert!(out.markdown.contains("_No actionable proposals this run._"));
    }

    #[test]
    fn generated_at_rendered_when_present() {
        let input = FormatSuggestionInput {
            positions: vec![],
            proposals: vec![],
            config: ProjectConfig::default(),
            generated_at: Some("2026-04-12T10:00:00Z".to_string()),
            previous_total_value_usd: None,
        };
        let out = format_suggestion_md(input);
        assert!(out.markdown.contains("2026-04-12T10:00:00Z"));
    }

    #[test]
    fn generated_at_omitted_when_absent() {
        let input = FormatSuggestionInput {
            positions: vec![],
            proposals: vec![],
            config: ProjectConfig::default(),
            generated_at: None,
            previous_total_value_usd: None,
        };
        let out = format_suggestion_md(input);
        assert!(!out.markdown.contains("_Generated:"));
    }

    // ---- progress edge cases ----

    #[test]
    fn progress_uses_last_7_snapshots() {
        let snapshots: Vec<ProgressSnapshot> = (0..10)
            .map(|i| ProgressSnapshot {
                date: format!("2026-04-{:02}", i + 1),
                positions: vec![pos("aave", "1000.00", 0.03 + i as f64 * 0.001)],
            })
            .collect();
        let input = ProgressInput {
            history: snapshots,
            config: ProjectConfig { floor_apy: 0.04, ..ProjectConfig::default() },
        };
        let out = format_progress(input);
        assert_eq!(out.samples, 7);
    }

    #[test]
    fn progress_single_snapshot() {
        let input = ProgressInput {
            history: vec![ProgressSnapshot {
                date: "2026-04-01".to_string(),
                positions: vec![pos("aave", "1000.00", 0.05)],
            }],
            config: ProjectConfig { floor_apy: 0.04, ..ProjectConfig::default() },
        };
        let out = format_progress(input);
        assert_eq!(out.samples, 1);
        assert!((out.realized_net_apy_7d - 0.05).abs() < 1e-9);
        assert!((out.delta_vs_floor - 0.01).abs() < 1e-9);
        assert!(out.progress_score > 0.0);
    }

    #[test]
    fn progress_below_floor_negative_score() {
        let input = ProgressInput {
            history: vec![ProgressSnapshot {
                date: "2026-04-01".to_string(),
                positions: vec![pos("aave", "1000.00", 0.02)],
            }],
            config: ProjectConfig { floor_apy: 0.04, ..ProjectConfig::default() },
        };
        let out = format_progress(input);
        assert!(out.progress_score < 0.0);
        assert!(out.delta_vs_floor < 0.0);
    }

    #[test]
    fn progress_zero_floor_score_equals_delta() {
        let input = ProgressInput {
            history: vec![ProgressSnapshot {
                date: "2026-04-01".to_string(),
                positions: vec![pos("aave", "1000.00", 0.05)],
            }],
            config: ProjectConfig { floor_apy: 0.0, ..ProjectConfig::default() },
        };
        let out = format_progress(input);
        assert!((out.progress_score - out.delta_vs_floor).abs() < 1e-9);
    }
}
