use std::collections::BTreeSet;
use std::time::Duration;

use ironclaw_engine::{ActionDef, ActionDiscoverySummary, ActionInventory};

use crate::tools::require_str;
use crate::tools::{ToolError, ToolOutput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionInfoDetail {
    Names,
    Summary,
    Schema,
}

impl ActionInfoDetail {
    fn parse(params: &serde_json::Value) -> Result<Self, ToolError> {
        if params
            .get("include_schema")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            return Ok(Self::Schema);
        }

        match params.get("detail").and_then(|value| value.as_str()) {
            None | Some("names") => Ok(Self::Names),
            Some("summary") => Ok(Self::Summary),
            Some("schema") => Ok(Self::Schema),
            Some(other) => Err(ToolError::InvalidParameters(format!(
                "invalid detail '{other}' (expected 'names', 'summary', or 'schema')"
            ))),
        }
    }
}

pub(crate) struct ActionDiscovery;

impl ActionDiscovery {
    pub(crate) fn tool_info(
        params: &serde_json::Value,
        inventory: &ActionInventory,
    ) -> Result<Option<ToolOutput>, ToolError> {
        let name = require_str(params, "name")?;
        let detail = ActionInfoDetail::parse(params)?;
        let action = Self::resolve(&inventory.inline, name)
            .or_else(|| Self::resolve(&inventory.discoverable, name));
        let Some(action) = action else {
            return Ok(None);
        };

        Ok(Some(Self::tool_output(action, detail)?))
    }

    pub(crate) fn tool_info_from_actions(
        params: &serde_json::Value,
        actions: &[ActionDef],
    ) -> Result<Option<ToolOutput>, ToolError> {
        let name = require_str(params, "name")?;
        let detail = ActionInfoDetail::parse(params)?;
        let Some(action) = Self::resolve(actions, name) else {
            return Ok(None);
        };

        Ok(Some(Self::tool_output(action, detail)?))
    }

    fn tool_output(action: &ActionDef, detail: ActionInfoDetail) -> Result<ToolOutput, ToolError> {
        let schema = action.discovery_schema();
        let mut info = serde_json::json!({
            "name": action.discovery_name(),
            "description": action.description.as_str(),
            "parameters": schema_param_names(schema),
        });

        match detail {
            ActionInfoDetail::Names => {}
            ActionInfoDetail::Summary => {
                let summary = action
                    .discovery_summary()
                    .cloned()
                    .unwrap_or_else(|| fallback_summary(schema));
                info["summary"] = serde_json::to_value(summary).map_err(|error| {
                    ToolError::ExecutionFailed(format!(
                        "failed to serialize action discovery summary: {error}"
                    ))
                })?;
            }
            ActionInfoDetail::Schema => {
                info["schema"] = schema.clone();
            }
        }

        Ok(ToolOutput::success(info, Duration::from_millis(1)))
    }

    pub(crate) fn resolve<'a>(actions: &'a [ActionDef], name: &str) -> Option<&'a ActionDef> {
        actions.iter().find(|action| action.matches_name(name))
    }
}

fn schema_param_names(schema: &serde_json::Value) -> Vec<String> {
    let mut names = BTreeSet::new();

    if let Some(props) = schema.get("properties").and_then(|value| value.as_object()) {
        names.extend(props.keys().cloned());
    }

    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(variants) = schema.get(key).and_then(|value| value.as_array()) {
            for variant in variants {
                if let Some(props) = variant
                    .get("properties")
                    .and_then(|value| value.as_object())
                {
                    names.extend(props.keys().cloned());
                }
            }
        }
    }

    names.into_iter().collect()
}

fn fallback_summary(schema: &serde_json::Value) -> ActionDiscoverySummary {
    ActionDiscoverySummary {
        always_required: schema
            .get("required")
            .and_then(|value| value.as_array())
            .map(|required| {
                required
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        ..ActionDiscoverySummary::default()
    }
}

#[cfg(test)]
mod tests {
    use super::ActionDiscovery;
    use ironclaw_engine::{
        ActionDef, ActionDiscoveryMetadata, ActionDiscoverySummary, ActionInventory,
        ModelToolSurface,
    };

    fn action(name: &str) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: format!("Action {name}"),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"}
                },
                "required": ["id"]
            }),
            effects: vec![],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }
    }

    #[test]
    fn resolves_underscored_and_hyphenated_names() {
        let actions = vec![action("mission_create")];
        assert!(ActionDiscovery::resolve(&actions, "mission_create").is_some());
        assert!(ActionDiscovery::resolve(&actions, "mission-create").is_some());
    }

    #[test]
    fn resolve_does_not_match_unrelated_action_before_alias_target() {
        let actions = vec![action("echo"), action("mission_create")];
        let resolved = ActionDiscovery::resolve(&actions, "mission-create")
            .expect("hyphenated alias should resolve");
        assert_eq!(resolved.name, "mission_create");
    }

    #[test]
    fn tool_info_uses_curated_summary_when_present() {
        let mut action = action("mission_create");
        action.discovery = Some(ActionDiscoveryMetadata {
            name: "mission_create".to_string(),
            summary: Some(ActionDiscoverySummary {
                always_required: vec!["name".to_string(), "goal".to_string()],
                conditional_requirements: vec!["Use cadence for scheduled runs".to_string()],
                notes: vec![],
                examples: vec![],
            }),
            schema_override: None,
        });

        let output = ActionDiscovery::tool_info(
            &serde_json::json!({"name": "mission_create", "detail": "summary"}),
            &ActionInventory {
                inline: vec![action],
                discoverable: Vec::new(),
            },
        )
        .expect("tool_info should succeed")
        .expect("action should resolve");

        assert_eq!(
            output.result["summary"]["always_required"],
            serde_json::json!(["name", "goal"])
        );
    }

    #[test]
    fn tool_info_falls_back_to_required_fields_for_summary() {
        let output = ActionDiscovery::tool_info(
            &serde_json::json!({"name": "mission_create", "detail": "summary"}),
            &ActionInventory {
                inline: vec![action("mission_create")],
                discoverable: Vec::new(),
            },
        )
        .expect("tool_info should succeed")
        .expect("action should resolve");

        assert_eq!(
            output.result["summary"]["always_required"],
            serde_json::json!(["id"])
        );
    }

    #[test]
    fn tool_info_from_actions_reads_borrowed_snapshot() {
        let output = ActionDiscovery::tool_info_from_actions(
            &serde_json::json!({"name": "mission_create", "detail": "summary"}),
            &[action("mission_create")],
        )
        .expect("tool_info should succeed")
        .expect("action should resolve");

        assert_eq!(output.result["name"], serde_json::json!("mission_create"));
    }

    #[test]
    fn tool_info_falls_back_to_discoverable_actions() {
        let output = ActionDiscovery::tool_info(
            &serde_json::json!({"name": "gmail_send", "detail": "summary"}),
            &ActionInventory {
                inline: Vec::new(),
                discoverable: vec![action("gmail_send")],
            },
        )
        .expect("tool_info should succeed")
        .expect("discoverable action should resolve");

        assert_eq!(output.result["name"], serde_json::json!("gmail_send"));
    }

    #[test]
    fn tool_info_prefers_inline_action_over_discoverable_duplicate() {
        let mut inline = action("gmail_send");
        inline.description = "Inline action".to_string();
        let mut discoverable = action("gmail_send");
        discoverable.description = "Discoverable action".to_string();

        let output = ActionDiscovery::tool_info(
            &serde_json::json!({"name": "gmail_send"}),
            &ActionInventory {
                inline: vec![inline],
                discoverable: vec![discoverable],
            },
        )
        .expect("tool_info should succeed")
        .expect("inline action should resolve");

        assert_eq!(
            output.result["description"],
            serde_json::json!("Inline action")
        );
    }
}
