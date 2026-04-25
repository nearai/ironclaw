use ironclaw_engine::{ActionDef, ActionDiscoveryMetadata, ActionDiscoverySummary};

use crate::bridge::action_projector::default_model_tool_surface;

fn action_discovery_summary(
    always_required: &[&str],
    conditional_requirements: &[&str],
    notes: &[&str],
) -> ActionDiscoverySummary {
    ActionDiscoverySummary {
        always_required: always_required
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        conditional_requirements: conditional_requirements
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        notes: notes.iter().map(|value| (*value).to_string()).collect(),
        examples: Vec::new(),
    }
}

fn mission_action(
    name: &str,
    description: &str,
    parameters_schema: serde_json::Value,
    summary: Option<ActionDiscoverySummary>,
) -> ActionDef {
    let discovery = summary.map(|summary| ActionDiscoveryMetadata {
        name: name.to_string(),
        summary: Some(summary),
        schema_override: None,
    });
    ActionDef {
        name: name.to_string(),
        description: description.to_string(),
        parameters_schema,
        effects: vec![],
        requires_approval: false,
        model_tool_surface: default_model_tool_surface(name),
        discovery,
    }
}

pub(crate) fn mission_capability_actions() -> Vec<ActionDef> {
    vec![
        mission_action(
            "mission_create",
            "Create a new mission (routine). Use only when the user explicitly wants to set up a recurring task, scheduled check, automation, monitor, or persistent manual mission. Do not use for immediate one-shot requests like 'do it now', 'right now', or 'immediately'; complete those in the current thread. Results are delivered to the current channel by default.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Short name for the mission/routine"},
                    "goal": {"type": "string", "description": "What this mission should accomplish each run"},
                    "cadence": {"type": "string", "description": "Required. How to trigger: 'manual', a cron expression (e.g. '0 9 * * *'), 'event:<channel>:<regex_pattern>' (e.g. 'event:telegram:.*', use 'event:*:<pattern>' for any channel), or 'webhook:<path>'"},
                    "timezone": {"type": "string", "description": "IANA timezone for cron scheduling (e.g. 'America/New_York'). Defaults to the user's channel timezone."},
                    "notify_channels": {"type": "array", "items": {"type": "string"}, "description": "Channels to deliver results to (e.g. ['gateway', 'repl']). Defaults to current channel."},
                    "project_id": {"type": "string", "description": "Project ID to scope this mission to. If omitted, uses the current thread's project."},
                    "cooldown_secs": {"type": "integer", "minimum": 0, "description": "Minimum seconds between triggers (default: 300 for event/webhook, 0 for cron/manual)"},
                    "max_concurrent": {"type": "integer", "minimum": 0, "description": "Max simultaneous running threads (default: 1 for event/webhook, unlimited for cron/manual)"},
                    "dedup_window_secs": {"type": "integer", "minimum": 0, "description": "Suppress duplicate event triggers within this window in seconds (default: 0)"},
                    "max_threads_per_day": {"type": "integer", "minimum": 0, "description": "Daily thread budget (default: 24 for event/webhook, 10 for cron/manual)"},
                    "success_criteria": {"type": "string", "description": "Criteria for declaring mission complete"}
                },
                "required": ["name", "goal", "cadence"]
            }),
            Some(action_discovery_summary(
                &["name", "goal", "cadence"],
                &[
                    "Use mission_create only when the user explicitly wants a recurring, scheduled, event-driven, webhook, or manual reusable mission.",
                    "For immediate one-shot work, complete the task in the current thread instead of creating a mission.",
                    "Use notify_channels to override where results are delivered; otherwise the current channel is used by default.",
                ],
                &[
                    "cadence accepts manual, cron, event:<channel>:<pattern>, or webhook:<path>",
                    "timezone only matters for cron-based schedules",
                ],
            )),
        ),
        mission_action(
            "mission_list",
            "List all missions and routines in the current project.",
            serde_json::json!({"type": "object"}),
            None,
        ),
        mission_action(
            "mission_get",
            "Get detailed status and results of a specific mission or routine. Returns the mission state, approach history, and recent thread outputs. Use when the user asks about mission results, outcome, or progress.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Mission/routine ID to retrieve"}
                },
                "required": ["id"]
            }),
            None,
        ),
        mission_action(
            "mission_fire",
            "Manually trigger a mission or routine to run immediately.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Mission/routine ID to trigger"}
                },
                "required": ["id"]
            }),
            None,
        ),
        mission_action(
            "mission_pause",
            "Pause a running mission or routine.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Mission/routine ID to pause"}
                },
                "required": ["id"]
            }),
            None,
        ),
        mission_action(
            "mission_resume",
            "Resume a paused mission or routine.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Mission/routine ID to resume"}
                },
                "required": ["id"]
            }),
            None,
        ),
        mission_action(
            "mission_update",
            "Update an existing mission or routine. Only include fields you want to change; omitted fields remain unchanged.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Mission/routine ID to update"},
                    "name": {"type": "string", "description": "New display name"},
                    "goal": {"type": "string", "description": "New mission goal"},
                    "cadence": {"type": "string", "description": "New cadence: manual, cron, event:<channel>:<pattern>, or webhook:<path>"},
                    "timezone": {"type": "string", "description": "IANA timezone for cron scheduling"},
                    "notify_channels": {"type": "array", "items": {"type": "string"}, "description": "Channels to notify with results"},
                    "cooldown_secs": {"type": "integer", "minimum": 0, "description": "Minimum seconds between triggers"},
                    "max_concurrent": {"type": "integer", "minimum": 0, "description": "Maximum simultaneous runs"},
                    "dedup_window_secs": {"type": "integer", "minimum": 0, "description": "Duplicate event suppression window"},
                    "max_threads_per_day": {"type": "integer", "minimum": 0, "description": "Daily thread budget"},
                    "success_criteria": {"type": "string", "description": "Completion criteria"}
                },
                "required": ["id"]
            }),
            Some(action_discovery_summary(
                &["id"],
                &[
                    "Only include the fields you want to change; omitted fields keep their existing values.",
                    "When updating cadence, keep timezone aligned with cron-based schedules.",
                ],
                &[
                    "Use mission_update for edits to an existing mission; use mission_create only for a brand new mission.",
                ],
            )),
        ),
        mission_action(
            "mission_complete",
            "Mark a mission or routine complete.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Mission/routine ID to complete"}
                },
                "required": ["id"]
            }),
            None,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::mission_capability_actions;

    fn action(name: &str) -> ironclaw_engine::ActionDef {
        mission_capability_actions()
            .into_iter()
            .find(|action| action.name == name)
            .unwrap_or_else(|| panic!("missing mission action {name}"))
    }

    #[test]
    fn mission_create_exposes_full_schema_and_curated_summary() {
        let action = action("mission_create");
        assert_eq!(action.discovery_name(), "mission_create");
        assert_eq!(
            action
                .parameters_schema
                .get("required")
                .expect("required fields"),
            &serde_json::json!(["name", "goal", "cadence"])
        );
        let summary = action.discovery_summary().expect("curated summary");
        assert_eq!(summary.always_required, vec!["name", "goal", "cadence"]);
        assert!(!summary.conditional_requirements.is_empty());
    }

    #[test]
    fn mission_update_has_curated_summary_but_minimal_actions_do_not() {
        let update = action("mission_update");
        assert!(update.discovery_summary().is_some());
        let props = update
            .parameters_schema
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("mission_update properties");
        assert!(!props.contains_key("project_id"));
        assert!(!props.contains_key("paused"));
        assert!(!props.contains_key("config"));

        let list = action("mission_list");
        assert!(list.discovery.is_none());
        assert!(list.discovery_summary().is_none());
        assert_eq!(list.discovery_schema(), &list.parameters_schema);
    }
}
