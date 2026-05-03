//! IronClaw mission scaffolding for DCA schedules.
//!
//! Closes the autopilot loop: a `plan_dca_schedule` output (or its
//! natural-language compile result via `compile_intent_prompt`) goes
//! in, a YAML scaffold for an IronClaw Mission comes out. The user
//! pastes the scaffold into their project, the engine creates the
//! mission, and IronClaw fires `build_intent` once per cron tick
//! against the unsigned solver path. Signing remains a wallet action
//! outside the agent.
//!
//! The scaffold is deterministic and self-contained — no live HTTP,
//! no signing, no key access. It is *advisory*: the engine owns
//! mission lifecycle, and the agent is expected to re-run risk gates
//! at every tick before invoking `build_intent`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct DcaMissionInput {
    /// `intents-dca-schedule/1` document, passed through verbatim.
    pub schedule: Value,
    /// Project the mission should be filed under. Defaults to
    /// `intents-trading-agent` (the canonical project id for this
    /// skill).
    #[serde(default = "default_project_id")]
    pub project_id: String,
    /// Optional human-friendly name override. Defaults to a stable
    /// auto-generated string from the pair + cadence.
    #[serde(default)]
    pub name: Option<String>,
    /// Optional timezone for the cron expression. Defaults to UTC.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Optional cooldown (minutes) between mission firings.
    #[serde(default = "default_cooldown_minutes")]
    pub cooldown_minutes: u64,
}

fn default_project_id() -> String {
    "intents-trading-agent".to_string()
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_cooldown_minutes() -> u64 {
    15
}

#[derive(Debug, Serialize)]
pub struct DcaMissionOutput {
    pub schema_version: &'static str,
    pub mission_name: String,
    pub project_id: String,
    pub goal: String,
    pub cadence_cron: String,
    pub cadence_timezone: String,
    pub cooldown_minutes: u64,
    pub safe_to_quote: bool,
    pub yaml: String,
    pub per_tick_bridge_calls: Vec<BridgeCall>,
    pub guardrails: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BridgeCall {
    pub step: u32,
    pub tool: String,
    pub action: String,
    pub purpose: String,
    pub example_params: Value,
}

pub fn format_dca_mission(input: DcaMissionInput) -> Result<DcaMissionOutput, String> {
    let schema = input
        .schedule
        .get("schema_version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "schedule.schema_version is required".to_string())?;
    if schema != "intents-dca-schedule/1" {
        return Err(format!(
            "schedule.schema_version must be 'intents-dca-schedule/1' (got '{schema}')"
        ));
    }
    let pair = field_str(&input.schedule, "pair")?;
    let mode = field_str(&input.schedule, "mode")?;
    let cron = field_str(&input.schedule, "cron")?;
    let cadence = field_str(&input.schedule, "cadence")?;
    let total_periods = input
        .schedule
        .get("total_periods")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "schedule.total_periods missing".to_string())?;
    let source_asset = field_str(&input.schedule, "source_asset")?;
    let destination_asset = field_str(&input.schedule, "destination_asset")?;
    let source_chain = field_str(&input.schedule, "source_chain")?;
    let destination_chain = field_str(&input.schedule, "destination_chain")?;
    let notional_per_period_usd = field_str(&input.schedule, "notional_per_period_usd")?;
    let max_slippage_bps = input
        .schedule
        .get("max_slippage_bps")
        .and_then(|v| v.as_f64())
        .unwrap_or(50.0);
    let safe_to_quote = input
        .schedule
        .get("safe_to_quote")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let solver = field_str(&input.schedule, "solver").unwrap_or_else(|_| "fixture".to_string());
    let template = input
        .schedule
        .get("build_intent_request_template")
        .cloned()
        .unwrap_or(Value::Null);

    let mission_name = input.name.clone().unwrap_or_else(|| {
        format!(
            "DCA {} {} → {} ({})",
            slug(&notional_per_period_usd),
            source_asset,
            destination_asset,
            cadence
        )
    });

    let goal = format!(
        "Maintain a {cadence} dollar-cost-average buy schedule of ${notional_per_period_usd} \
        from {source_asset} on {source_chain} into {destination_asset} on {destination_chain}. \
        On every cron tick: (1) re-run risk gates against current portfolio config, \
        (2) call `portfolio.build_intent` with the embedded template (solver={solver}) \
        to produce an unsigned intent, (3) journal the unsigned bundle and update the \
        Projects widget. Stop after {total_periods} ticks. Never sign. \
        Pause if max_slippage_bps={max_slippage_bps:.0} is breached or solver returns empty quote."
    );

    let warnings: Vec<String> = if !safe_to_quote {
        vec!["schedule.safe_to_quote=false: do not enable mission until gates clear".to_string()]
    } else {
        vec![]
    };

    let guardrails = vec![
        "Mission cadence is advisory — the engine schedules each tick separately.".to_string(),
        "Per-tick build_intent must be re-quoted before user signs; agent never signs.".to_string(),
        "Skip the tick when max_slippage_bps is breached; journal the miss.".to_string(),
        format!(
            "Honor cooldown: at least {} minutes must elapse between mission firings.",
            input.cooldown_minutes
        ),
        "Failed quotes do not advance the period counter; only successful unsigned bundles do."
            .to_string(),
    ];

    let per_tick = vec![
        BridgeCall {
            step: 1,
            tool: "portfolio".to_string(),
            action: "build_intent".to_string(),
            purpose: "Produce one unsigned intent bundle for this period.".to_string(),
            example_params: template.clone(),
        },
        BridgeCall {
            step: 2,
            tool: "portfolio".to_string(),
            action: "format_intents_widget".to_string(),
            purpose:
                "Refresh the Projects widget state with the latest paper PnL and pending intent."
                    .to_string(),
            example_params: serde_json::json!({
                "action": "format_intents_widget",
                "pair": pair,
                "mode": mode,
                "stance": "paper-intent",
                "risk_gates": [{
                    "name": "dca-mission-active",
                    "status": "pass",
                    "detail": "Cron-driven autopilot — per-tick build_intent only."
                }]
            }),
        },
    ];

    let mut yaml = String::new();
    yaml.push_str("# IronClaw mission scaffold for a DCA schedule.\n");
    yaml.push_str("# Generated by `portfolio.format_dca_mission`. Paste under your\n");
    yaml.push_str("# project's missions/ directory or pass through the engine's\n");
    yaml.push_str("# `mission_create` bridge tool. Adjust thresholds before activation.\n");
    yaml.push_str("---\n");
    yaml.push_str("kind: mission\n");
    yaml.push_str(&format!("name: \"{}\"\n", yaml_escape(&mission_name)));
    yaml.push_str(&format!(
        "project_id: \"{}\"\n",
        yaml_escape(&input.project_id)
    ));
    yaml.push_str(&format!(
        "description: \"DCA {} from {} on {} into {} on {} ({} cadence, {} periods total)\"\n",
        notional_per_period_usd,
        source_asset,
        source_chain,
        destination_asset,
        destination_chain,
        cadence,
        total_periods
    ));
    yaml.push_str("goal: |\n");
    for line in goal.lines() {
        yaml.push_str("  ");
        yaml.push_str(line);
        yaml.push('\n');
    }
    yaml.push_str("status: paused\n");
    yaml.push_str("cadence:\n");
    yaml.push_str("  kind: cron\n");
    yaml.push_str(&format!("  expression: \"{cron}\"\n"));
    yaml.push_str(&format!(
        "  timezone: \"{}\"\n",
        yaml_escape(&input.timezone)
    ));
    yaml.push_str(&format!("cooldown_minutes: {}\n", input.cooldown_minutes));
    yaml.push_str(&format!("max_threads_today: {total_periods}\n"));
    yaml.push_str("safety:\n");
    yaml.push_str("  unsigned_only: true\n");
    yaml.push_str("  require_user_signature_outside_agent: true\n");
    yaml.push_str(&format!("  safe_to_quote: {safe_to_quote}\n"));
    yaml.push_str("per_tick_bridge_calls:\n");
    for call in &per_tick {
        yaml.push_str(&format!(
            "  - step: {}\n    tool: \"{}\"\n    action: \"{}\"\n    purpose: \"{}\"\n",
            call.step,
            yaml_escape(&call.tool),
            yaml_escape(&call.action),
            yaml_escape(&call.purpose)
        ));
    }
    yaml.push_str("source_schedule:\n");
    yaml.push_str(&format!("  pair: \"{}\"\n", yaml_escape(&pair)));
    yaml.push_str(&format!("  cadence: \"{}\"\n", yaml_escape(&cadence)));
    yaml.push_str(&format!("  total_periods: {}\n", total_periods));
    yaml.push_str(&format!(
        "  notional_per_period_usd: {}\n",
        notional_per_period_usd
    ));
    yaml.push_str(&format!("  max_slippage_bps: {max_slippage_bps:.0}\n"));
    yaml.push_str(&format!("  solver: \"{}\"\n", yaml_escape(&solver)));

    Ok(DcaMissionOutput {
        schema_version: "intents-dca-mission-scaffold/1",
        mission_name,
        project_id: input.project_id,
        goal,
        cadence_cron: cron,
        cadence_timezone: input.timezone,
        cooldown_minutes: input.cooldown_minutes,
        safe_to_quote,
        yaml,
        per_tick_bridge_calls: per_tick,
        guardrails,
        warnings,
    })
}

fn field_str(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("schedule.{key} missing or not a string"))
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn yaml_escape(s: &str) -> String {
    s.replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule_fixture() -> Value {
        serde_json::json!({
            "schema_version": "intents-dca-schedule/1",
            "pair": "NEAR/USDC",
            "mode": "paper",
            "cadence": "weekly",
            "cron": "0 12 * * 1",
            "total_periods": 26,
            "source_asset": "USDC",
            "destination_asset": "NEAR",
            "source_chain": "near",
            "destination_chain": "near",
            "notional_per_period_usd": "100.00",
            "total_notional_usd": "2600.00",
            "assumed_price_usd": 3.0,
            "max_slippage_bps": 50.0,
            "solver": "fixture",
            "safe_to_quote": true,
            "build_intent_request_template": {
                "action": "build_intent",
                "solver": "fixture",
                "plan": {"proposal_id": "dca-near-usdc-weekly", "legs": []}
            }
        })
    }

    #[test]
    fn scaffold_emits_yaml_with_cron_and_goal() {
        let out = format_dca_mission(DcaMissionInput {
            schedule: schedule_fixture(),
            project_id: "intents-trading-agent".to_string(),
            name: None,
            timezone: "UTC".to_string(),
            cooldown_minutes: 30,
        })
        .unwrap();
        assert_eq!(out.schema_version, "intents-dca-mission-scaffold/1");
        assert_eq!(out.cadence_cron, "0 12 * * 1");
        assert!(out.yaml.contains("kind: mission"));
        assert!(out.yaml.contains("0 12 * * 1"));
        assert!(out.yaml.contains("status: paused"));
        assert!(out.yaml.contains("unsigned_only: true"));
        assert_eq!(out.per_tick_bridge_calls.len(), 2);
    }

    #[test]
    fn scaffold_warns_when_not_safe_to_quote() {
        let mut sched = schedule_fixture();
        sched["safe_to_quote"] = serde_json::json!(false);
        let out = format_dca_mission(DcaMissionInput {
            schedule: sched,
            project_id: "intents-trading-agent".to_string(),
            name: None,
            timezone: "UTC".to_string(),
            cooldown_minutes: 30,
        })
        .unwrap();
        assert!(!out.safe_to_quote);
        assert!(!out.warnings.is_empty());
        assert!(out.yaml.contains("safe_to_quote: false"));
    }

    #[test]
    fn scaffold_rejects_wrong_schema() {
        let err = format_dca_mission(DcaMissionInput {
            schedule: serde_json::json!({"schema_version": "intents-backtest/1"}),
            project_id: "intents-trading-agent".to_string(),
            name: None,
            timezone: "UTC".to_string(),
            cooldown_minutes: 30,
        })
        .unwrap_err();
        assert!(err.contains("intents-dca-schedule/1"));
    }
}
