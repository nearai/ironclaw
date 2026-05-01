//! `ironclaw insights` — usage analytics over the local IronClaw database.
//!
//! Reads aggregates already collected by the agent (agent_jobs, routine_runs,
//! job_actions). Operators previously tailed logs to track adoption; this
//! subcommand surfaces the same numbers without scraping.
//!
//! First-commit scope:
//!   - Time window aggregates only (`--days <N>`, default 30, capped at 90).
//!   - Token + job counts, top-N tool frequency, daily activity histogram.
//!   - Pretty table by default, structured JSON behind `--json`.
//!
//! Out of scope (follow-up PRs): per-call USD cost rollup, `--source` /
//! `--user-id` filters, average turns/session.
//!
//! Hermes parity is intentionally partial: this is the CLI surface NEAR
//! Foundation operators need today, not the full Hermes UI.

use std::path::Path;
use std::sync::Arc;

use chrono::{Duration, Utc};
use clap::Args;

use crate::db::{Database, InsightsAggregate};

/// Default window in days when `--days` is not provided.
pub const DEFAULT_INSIGHTS_DAYS: u32 = 30;
/// Hard cap on `--days`. Larger values fall back to this and emit a warning.
/// Aggregations beyond ~90 days have not been validated against real-world
/// libSQL files and can produce surprising numbers when the schema changes.
pub const MAX_INSIGHTS_DAYS: u32 = 90;
/// Number of tool rows shown in pretty output and emitted in JSON.
pub const TOP_TOOLS_LIMIT: i64 = 10;

#[derive(Args, Debug, Clone)]
pub struct InsightsArgs {
    /// Time window in days (default 30). Capped at 90; values above the cap
    /// are clamped and a warning is printed to stderr.
    #[arg(long, default_value_t = DEFAULT_INSIGHTS_DAYS)]
    pub days: u32,

    /// Emit machine-readable JSON instead of a human-readable table.
    /// The schema is additive — consumers should ignore unknown fields.
    #[arg(long)]
    pub json: bool,
}

impl Default for InsightsArgs {
    fn default() -> Self {
        Self {
            days: DEFAULT_INSIGHTS_DAYS,
            json: false,
        }
    }
}

/// Result of normalizing a user-supplied `--days` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedWindow {
    pub days: u32,
    pub clamped: bool,
}

/// Clamp `--days` into `[1, MAX_INSIGHTS_DAYS]` and report whether clamping
/// happened, so the caller can warn on stderr without re-doing the math.
pub fn resolve_window(requested: u32) -> ResolvedWindow {
    if requested == 0 {
        // Treat 0 as "default window" rather than empty; insights with a
        // zero-second window are useless and almost certainly a typo.
        return ResolvedWindow {
            days: DEFAULT_INSIGHTS_DAYS,
            clamped: true,
        };
    }
    if requested > MAX_INSIGHTS_DAYS {
        return ResolvedWindow {
            days: MAX_INSIGHTS_DAYS,
            clamped: true,
        };
    }
    ResolvedWindow {
        days: requested,
        clamped: false,
    }
}

/// Top-level entry point used by `main.rs`. Loads config and connects to the
/// configured database (libSQL or Postgres), then delegates to
/// [`run_insights_with_db`] for testability.
pub async fn run_insights_command(
    args: InsightsArgs,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let config = crate::config::Config::from_env_with_toml(config_path)
        .await
        .map_err(|e| anyhow::anyhow!("{e:#}"))?;
    let db: Arc<dyn Database> = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{e:#}"))?;
    run_insights_with_db(args, db).await
}

/// Run insights with an injected database. Used by tests to avoid touching
/// `Config::from_env`.
pub async fn run_insights_with_db(
    args: InsightsArgs,
    db: Arc<dyn Database>,
) -> anyhow::Result<()> {
    let resolved = resolve_window(args.days);
    if resolved.clamped {
        eprintln!(
            "warning: requested window {} day(s) clamped to {} (max). \
             Wider windows are not supported in this release.",
            args.days, resolved.days
        );
    }

    let since = Utc::now() - Duration::days(resolved.days as i64);
    let aggregate = db
        .aggregate_insights(since, TOP_TOOLS_LIMIT)
        .await
        .map_err(|e| anyhow::anyhow!("aggregate_insights failed: {e}"))?;

    if args.json {
        emit_json(&aggregate, resolved.days)?;
    } else {
        emit_table(&aggregate, resolved.days);
    }
    Ok(())
}

fn emit_json(agg: &InsightsAggregate, window_days: u32) -> anyhow::Result<()> {
    // `version` lets dashboards detect breaking schema changes without sniffing
    // optional fields. Bump it on rename/removal, never on additions.
    let payload = serde_json::json!({
        "version": 1,
        "window_days": window_days,
        "total_jobs": agg.total_jobs,
        "total_routine_runs": agg.total_routine_runs,
        "total_tokens_used": agg.total_tokens_used,
        "top_tools": agg.top_tools,
        "daily_activity": agg.daily_activity,
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

fn emit_table(agg: &InsightsAggregate, window_days: u32) {
    if agg.total_jobs == 0 && agg.total_routine_runs == 0 {
        println!(
            "No agent activity in the last {} day(s). Run `ironclaw run` to start the agent.",
            window_days
        );
        return;
    }

    println!("IronClaw insights — last {} day(s)\n", window_days);

    println!(
        "  jobs:           {}\n  routine runs:   {}\n  tokens used:    {}",
        agg.total_jobs, agg.total_routine_runs, agg.total_tokens_used,
    );

    if !agg.top_tools.is_empty() {
        println!("\n  Top tools by invocation:");
        let max_tool_len = agg
            .top_tools
            .iter()
            .map(|t| t.tool_name.chars().count())
            .max()
            .unwrap_or(8)
            .max(8);
        for tool in &agg.top_tools {
            println!(
                "    {:<width$}  {:>6}",
                tool.tool_name,
                tool.invocations,
                width = max_tool_len,
            );
        }
    }

    if !agg.daily_activity.is_empty() {
        println!("\n  Daily activity:");
        for day in &agg.daily_activity {
            println!("    {}  {:>6}", day.date, day.jobs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_window_default_clamps_zero() {
        let r = resolve_window(0);
        assert_eq!(r.days, DEFAULT_INSIGHTS_DAYS);
        assert!(r.clamped);
    }

    #[test]
    fn resolve_window_caps_at_max() {
        let r = resolve_window(365);
        assert_eq!(r.days, MAX_INSIGHTS_DAYS);
        assert!(r.clamped);
    }

    #[test]
    fn resolve_window_passes_through_in_range() {
        let r = resolve_window(7);
        assert_eq!(r.days, 7);
        assert!(!r.clamped);
    }

    #[test]
    fn resolve_window_passes_through_at_boundary() {
        let r = resolve_window(MAX_INSIGHTS_DAYS);
        assert_eq!(r.days, MAX_INSIGHTS_DAYS);
        assert!(!r.clamped);
    }

    #[test]
    fn json_is_stable_for_empty_aggregate() {
        let agg = InsightsAggregate::default();
        let payload = serde_json::json!({
            "version": 1,
            "window_days": 30,
            "total_jobs": agg.total_jobs,
            "total_routine_runs": agg.total_routine_runs,
            "total_tokens_used": agg.total_tokens_used,
            "top_tools": agg.top_tools,
            "daily_activity": agg.daily_activity,
        });
        // Snapshot fixture: this is the canonical empty-data JSON shape.
        // If this test fails, dashboards consuming the JSON will break too.
        let expected = serde_json::json!({
            "version": 1,
            "window_days": 30,
            "total_jobs": 0,
            "total_routine_runs": 0,
            "total_tokens_used": 0,
            "top_tools": [],
            "daily_activity": [],
        });
        assert_eq!(payload, expected);
    }
}
