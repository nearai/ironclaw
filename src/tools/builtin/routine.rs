//! LLM-facing tools for managing routines.
//!
//! Seven tools let the agent manage routines conversationally:
//! - `routine_create` - Create a new routine
//! - `routine_list` - List all routines with status
//! - `routine_update` - Modify or toggle a routine
//! - `routine_delete` - Remove a routine
//! - `routine_fire` - Manually trigger a routine
//! - `routine_history` - View past runs
//! - `event_emit` - Emit a structured system event to `system_event`-triggered routines

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::agent::routine::{
    FullJobPermissionDefaultMode, FullJobPermissionMode, NotifyConfig, Routine, RoutineAction,
    RoutineGuardrails, Trigger, load_full_job_permission_settings, next_cron_fire,
    normalize_tool_names,
};
use crate::agent::routine_engine::RoutineEngine;
use crate::context::JobContext;
use crate::db::Database;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDiscoverySummary, ToolError, ToolOutput, require_str,
};

// ==================== routine_create ====================

#[derive(Debug, Clone, PartialEq, Eq)]
enum NormalizedTriggerRequest {
    Cron {
        schedule: String,
        timezone: Option<String>,
    },
    Manual,
    MessageEvent {
        pattern: String,
        channel: Option<String>,
    },
    SystemEvent {
        source: String,
        event_type: String,
        filters: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NormalizedExecutionMode {
    Lightweight,
    FullJob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestedFullJobPermissionMode {
    Explicit,
    InheritOwner,
    CopyOwner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedExecutionRequest {
    mode: NormalizedExecutionMode,
    context_paths: Vec<String>,
    use_tools: bool,
    max_tool_rounds: u32,
    tool_permissions: Vec<String>,
    permission_mode: Option<RequestedFullJobPermissionMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedDeliveryRequest {
    channel: Option<String>,
    user: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedRoutineCreateRequest {
    name: String,
    description: String,
    prompt: String,
    trigger: NormalizedTriggerRequest,
    execution: NormalizedExecutionRequest,
    delivery: NormalizedDeliveryRequest,
    cooldown_secs: u64,
}

fn routine_request_properties() -> Value {
    serde_json::json!({
        "kind": {
            "type": "string",
            "enum": ["cron", "manual", "message_event", "system_event"],
            "description": "How the routine should start."
        },
        "schedule": {
            "type": "string",
            "description": "Cron expression for request.kind='cron'. Uses 6-field cron: second minute hour day month weekday."
        },
        "timezone": {
            "type": "string",
            "description": "IANA timezone for request.kind='cron', such as 'America/New_York'."
        },
        "pattern": {
            "type": "string",
            "description": "Regex pattern for request.kind='message_event'."
        },
        "channel": {
            "type": "string",
            "description": "Optional channel filter for request.kind='message_event'."
        },
        "source": {
            "type": "string",
            "description": "Event source namespace for request.kind='system_event', such as 'github'."
        },
        "event_type": {
            "type": "string",
            "description": "Event type for request.kind='system_event', such as 'issue.opened'."
        },
        "filters": {
            "type": "object",
            "properties": {},
            "additionalProperties": {
                "type": ["string", "number", "boolean"]
            },
            "description": "Optional exact-match filters for request.kind='system_event'. Only top-level string, number, and boolean payload fields are matched."
        }
    })
}

fn execution_properties() -> Value {
    serde_json::json!({
        "mode": {
            "type": "string",
            "enum": ["lightweight", "full_job"],
            "description": "Execution mode. 'lightweight' is the default. 'full_job' runs a multi-turn autonomous job."
        },
        "context_paths": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Workspace paths to preload for lightweight routines."
        },
        "use_tools": {
            "type": "boolean",
            "description": "Only applies to lightweight mode. When true, safe non-approval tools are available."
        },
        "max_tool_rounds": {
            "type": "integer",
            "minimum": 1,
            "maximum": crate::agent::routine::MAX_TOOL_ROUNDS_LIMIT,
            "default": 3,
            "description": "Only applies when execution.mode='lightweight' and use_tools=true. Runtime-capped to prevent loops."
        },
        "tool_permissions": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Only applies when execution.mode='full_job'. These tools are pre-authorized for Always-approval checks."
        },
        "permission_mode": {
            "type": "string",
            "enum": ["inherit_owner", "explicit", "copy_owner"],
            "description": "Only applies when execution.mode='full_job'. 'inherit_owner' uses the owner defaults at run time, 'explicit' uses only tool_permissions, and 'copy_owner' snapshots the current owner allowlist into tool_permissions."
        }
    })
}

fn delivery_properties() -> Value {
    serde_json::json!({
        "channel": {
            "type": "string",
            "description": "Default channel for notifications and routine job message calls."
        },
        "user": {
            "type": "string",
            "description": "Default user or target for notifications and routine job message calls. If omitted, the owner's last-seen notification target is used."
        }
    })
}

fn advanced_properties() -> Value {
    serde_json::json!({
        "cooldown_secs": {
            "type": "integer",
            "description": "Minimum seconds between automatic fires. Manual fires still bypass cooldown."
        }
    })
}

fn manual_request_variant() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "Manual routines run only when explicitly fired.",
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["manual"],
                "description": "Manual trigger."
            }
        },
        "required": ["kind"]
    })
}

fn cron_request_variant() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "Cron routines require request.schedule and may optionally set request.timezone.",
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["cron"],
                "description": "Scheduled trigger."
            },
            "schedule": {
                "type": "string",
                "description": "Cron expression for request.kind='cron'. Uses 6-field cron: second minute hour day month weekday."
            },
            "timezone": {
                "type": "string",
                "description": "IANA timezone for request.kind='cron', such as 'America/New_York'."
            }
        },
        "required": ["kind", "schedule"]
    })
}

fn message_event_request_variant() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "Message-event routines require request.pattern and may optionally filter by request.channel.",
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["message_event"],
                "description": "Pattern-matching message trigger."
            },
            "pattern": {
                "type": "string",
                "description": "Regex pattern for request.kind='message_event'."
            },
            "channel": {
                "type": "string",
                "description": "Optional channel filter for request.kind='message_event'."
            }
        },
        "required": ["kind", "pattern"]
    })
}

fn system_event_request_variant() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "System-event routines require request.source and request.event_type. request.filters is optional.",
        "properties": {
            "kind": {
                "type": "string",
                "enum": ["system_event"],
                "description": "Structured event trigger."
            },
            "source": {
                "type": "string",
                "description": "Event source namespace for request.kind='system_event', such as 'github'."
            },
            "event_type": {
                "type": "string",
                "description": "Event type for request.kind='system_event', such as 'issue.opened'."
            },
            "filters": {
                "type": "object",
                "properties": {},
                "additionalProperties": {
                    "type": ["string", "number", "boolean"]
                },
                "description": "Optional exact-match filters for request.kind='system_event'. Only top-level string, number, and boolean payload fields are matched."
            }
        },
        "required": ["kind", "source", "event_type"]
    })
}

fn routine_request_discovery_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "Canonical trigger config. Set request.kind first, then follow the matching variant branch below.",
        "properties": routine_request_properties(),
        "required": ["kind"],
        "oneOf": [
            manual_request_variant(),
            cron_request_variant(),
            message_event_request_variant(),
            system_event_request_variant()
        ],
        "examples": [
            { "kind": "manual" },
            { "kind": "cron", "schedule": "0 0 9 * * MON-FRI", "timezone": "UTC" },
            { "kind": "message_event", "pattern": "deploy\\s+prod", "channel": "slack" },
            { "kind": "system_event", "source": "github", "event_type": "issue.opened", "filters": { "repository": "nearai/ironclaw" } }
        ]
    })
}

fn lightweight_execution_variant() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "Default lightweight execution. Applies when execution is omitted or execution.mode='lightweight'.",
        "properties": {
            "mode": {
                "type": "string",
                "enum": ["lightweight"],
                "description": "Lightweight execution mode."
            },
            "context_paths": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Workspace paths to preload for lightweight routines."
            },
            "use_tools": {
                "type": "boolean",
                "description": "When true, safe non-approval tools are available."
            },
            "max_tool_rounds": {
                "type": "integer",
                "minimum": 1,
                "maximum": crate::agent::routine::MAX_TOOL_ROUNDS_LIMIT,
                "default": 3,
                "description": "Only applies when use_tools=true. Runtime-capped to prevent loops."
            }
        }
    })
}

fn full_job_execution_variant() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "Full-job execution. Uses owner-scoped permission defaults plus tool_permissions and ignores lightweight-only fields such as use_tools, max_tool_rounds, and context_paths.",
        "properties": {
            "mode": {
                "type": "string",
                "enum": ["full_job"],
                "description": "Full-job execution mode."
            },
            "tool_permissions": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Tools pre-authorized for Always-approval checks."
            },
            "permission_mode": {
                "type": "string",
                "enum": ["inherit_owner", "explicit", "copy_owner"],
                "description": "When omitted, new routines use the owner default. 'copy_owner' snapshots the current owner allowlist into this routine."
            }
        },
        "required": ["mode"]
    })
}

fn execution_discovery_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "description": "Optional execution settings. Omit this block for the default lightweight mode.",
        "properties": execution_properties(),
        "oneOf": [
            lightweight_execution_variant(),
            full_job_execution_variant()
        ],
        "examples": [
            { "mode": "lightweight", "use_tools": true, "max_tool_rounds": 3 },
            { "mode": "full_job", "permission_mode": "inherit_owner", "tool_permissions": ["message", "http"] }
        ]
    })
}

fn routine_create_examples() -> Vec<Value> {
    vec![
        serde_json::json!({
            "name": "manual-check",
            "prompt": "Inspect the repo for issues.",
            "request": { "kind": "manual" }
        }),
        serde_json::json!({
            "name": "weekday-digest",
            "prompt": "Prepare the morning digest.",
            "request": {
                "kind": "cron",
                "schedule": "0 0 9 * * MON-FRI",
                "timezone": "UTC"
            },
            "delivery": {
                "channel": "telegram",
                "user": "ops-team"
            }
        }),
        serde_json::json!({
            "name": "deploy-watch",
            "prompt": "Look for deploy requests.",
            "request": {
                "kind": "message_event",
                "pattern": "deploy\\s+prod",
                "channel": "slack"
            },
            "execution": {
                "mode": "lightweight",
                "use_tools": true,
                "max_tool_rounds": 5
            }
        }),
        serde_json::json!({
            "name": "issue-watch",
            "prompt": "Summarize new GitHub issues.",
            "request": {
                "kind": "system_event",
                "source": "github",
                "event_type": "issue.opened",
                "filters": { "repository": "nearai/ironclaw" }
            },
            "execution": {
                "mode": "full_job",
                "permission_mode": "inherit_owner",
                "tool_permissions": ["message"]
            }
        }),
    ]
}

fn routine_create_tool_summary() -> ToolDiscoverySummary {
    ToolDiscoverySummary {
        always_required: vec!["name".into(), "prompt".into(), "request.kind".into()],
        conditional_requirements: vec![
            "request.kind='cron' requires request.schedule.".into(),
            "request.kind='message_event' requires request.pattern.".into(),
            "request.kind='system_event' requires request.source and request.event_type.".into(),
            "execution.mode='full_job' uses permission_mode and tool_permissions, and ignores use_tools, max_tool_rounds, and context_paths.".into(),
        ],
        notes: vec![
            "Omitting execution defaults to lightweight mode.".into(),
            "Omitting delivery.user falls back to the owner's last-seen notification target.".into(),
            "advanced.cooldown_secs defaults to 300.".into(),
            "Legacy flat aliases are still accepted for compatibility, but grouped fields are preferred.".into(),
        ],
        examples: routine_create_examples(),
    }
}

fn routine_create_schema(include_compatibility_aliases: bool) -> Value {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Unique name for the routine (e.g. 'daily-pr-review')."
            },
            "prompt": {
                "type": "string",
                "description": "Instructions for what the routine should do when it fires."
            },
            "description": {
                "type": "string",
                "description": "Optional human-readable summary of what the routine does."
            },
            "request": if include_compatibility_aliases {
                routine_request_discovery_schema()
            } else {
                serde_json::json!({
                    "type": "object",
                    "description": "Canonical trigger config. Set request.kind first, then only fill fields that match that kind.",
                    "properties": routine_request_properties(),
                    "required": ["kind"]
                })
            },
            "execution": if include_compatibility_aliases {
                execution_discovery_schema()
            } else {
                serde_json::json!({
                    "type": "object",
                    "description": "Optional execution settings. Omit for the default lightweight mode.",
                    "properties": execution_properties()
                })
            },
            "delivery": {
                "type": "object",
                "description": "Optional delivery defaults for notifications and message tool calls inside routine jobs.",
                "properties": delivery_properties()
            },
            "advanced": {
                "type": "object",
                "description": "Optional advanced knobs. Most routines can omit this block.",
                "properties": advanced_properties()
            }
        },
        "required": ["name", "prompt"]
    });

    if include_compatibility_aliases {
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            properties.insert(
                "trigger_type".to_string(),
                serde_json::json!({
                    "type": "string",
                    "enum": ["cron", "event", "system_event", "manual"],
                    "description": "Compatibility alias for request.kind. Prefer request.kind."
                }),
            );
            properties.insert(
                "schedule".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Compatibility alias for request.schedule. Prefer request.schedule."
                }),
            );
            properties.insert(
                "timezone".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Compatibility alias for request.timezone. Prefer request.timezone."
                }),
            );
            properties.insert(
                "event_pattern".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Compatibility alias for request.pattern when request.kind='message_event'."
                }),
            );
            properties.insert(
                "event_channel".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Compatibility alias for request.channel when request.kind='message_event'."
                }),
            );
            properties.insert(
                "event_source".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Compatibility alias for request.source when request.kind='system_event'."
                }),
            );
            properties.insert(
                "event_type".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Compatibility alias for request.event_type when request.kind='system_event'."
                }),
            );
            properties.insert(
                "event_filters".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": {
                        "type": ["string", "number", "boolean"]
                    },
                    "description": "Compatibility alias for request.filters when request.kind='system_event'."
                }),
            );
            properties.insert(
                "action_type".to_string(),
                serde_json::json!({
                    "type": "string",
                    "enum": ["lightweight", "full_job"],
                    "description": "Compatibility alias for execution.mode."
                }),
            );
            properties.insert(
                "context_paths".to_string(),
                serde_json::json!({
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Compatibility alias for execution.context_paths."
                }),
            );
            properties.insert(
                "use_tools".to_string(),
                serde_json::json!({
                    "type": "boolean",
                    "description": "Compatibility alias for execution.use_tools."
                }),
            );
            properties.insert(
                "max_tool_rounds".to_string(),
                serde_json::json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": crate::agent::routine::MAX_TOOL_ROUNDS_LIMIT,
                    "default": 3,
                    "description": "Compatibility alias for execution.max_tool_rounds."
                }),
            );
            properties.insert(
                "tool_permissions".to_string(),
                serde_json::json!({
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Compatibility alias for execution.tool_permissions."
                }),
            );
            properties.insert(
                "permission_mode".to_string(),
                serde_json::json!({
                    "type": "string",
                    "enum": ["inherit_owner", "explicit", "copy_owner"],
                    "description": "Compatibility alias for execution.permission_mode."
                }),
            );
            properties.insert(
                "notify_channel".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Compatibility alias for delivery.channel."
                }),
            );
            properties.insert(
                "notify_user".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Compatibility alias for delivery.user."
                }),
            );
            properties.insert(
                "cooldown_secs".to_string(),
                serde_json::json!({
                    "type": "integer",
                    "description": "Compatibility alias for advanced.cooldown_secs."
                }),
            );
        }
        if let Some(schema_obj) = schema.as_object_mut() {
            schema_obj.insert(
                "anyOf".to_string(),
                serde_json::json!([
                    { "required": ["request"] },
                    { "required": ["trigger_type"] }
                ]),
            );
            schema_obj.insert(
                "examples".to_string(),
                Value::Array(routine_create_examples()),
            );
        }
    } else if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.push(Value::String("request".to_string()));
    }

    schema
}

pub(crate) fn routine_create_parameters_schema() -> Value {
    routine_create_schema(false)
}

fn routine_create_discovery_schema() -> Value {
    static CACHE: OnceLock<Value> = OnceLock::new();
    CACHE.get_or_init(|| routine_create_schema(true)).clone()
}

pub(crate) fn routine_update_parameters_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Name of the routine to update"
            },
            "enabled": {
                "type": "boolean",
                "description": "Enable or disable the routine"
            },
            "prompt": {
                "type": "string",
                "description": "New prompt/instructions"
            },
            "schedule": {
                "type": "string",
                "description": "New cron schedule (for cron triggers)"
            },
            "timezone": {
                "type": "string",
                "description": "IANA timezone for cron schedule (e.g. 'America/New_York'). Only valid for cron triggers."
            },
            "description": {
                "type": "string",
                "description": "New description"
            },
            "tool_permissions": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Updated Always-approval tool allowlist for full_job routines only."
            },
            "permission_mode": {
                "type": "string",
                "enum": ["inherit_owner", "explicit", "copy_owner"],
                "description": "Updated permission mode for full_job routines only. 'copy_owner' snapshots the current owner allowlist into the routine and persists as explicit."
            }
        },
        "required": ["name"]
    })
}

fn nested_object<'a>(params: &'a Value, field: &str) -> Option<&'a Map<String, Value>> {
    params.get(field).and_then(Value::as_object)
}

fn string_field(params: &Value, group: &str, field: &str, aliases: &[&str]) -> Option<String> {
    nested_object(params, group)
        .and_then(|obj| obj.get(field))
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| {
            aliases
                .iter()
                .find_map(|alias| params.get(*alias).and_then(Value::as_str).map(String::from))
        })
}

fn bool_field(params: &Value, group: &str, field: &str, aliases: &[&str]) -> Option<bool> {
    nested_object(params, group)
        .and_then(|obj| obj.get(field))
        .and_then(Value::as_bool)
        .or_else(|| {
            aliases
                .iter()
                .find_map(|alias| params.get(*alias).and_then(Value::as_bool))
        })
}

fn u64_field(params: &Value, group: &str, field: &str, aliases: &[&str]) -> Option<u64> {
    nested_object(params, group)
        .and_then(|obj| obj.get(field))
        .and_then(Value::as_u64)
        .or_else(|| {
            aliases
                .iter()
                .find_map(|alias| params.get(*alias).and_then(Value::as_u64))
        })
}

fn string_array_field(params: &Value, group: &str, field: &str, aliases: &[&str]) -> Vec<String> {
    normalize_tool_names(
        nested_object(params, group)
            .and_then(|obj| obj.get(field))
            .and_then(Value::as_array)
            .or_else(|| {
                aliases
                    .iter()
                    .find_map(|alias| params.get(*alias).and_then(Value::as_array))
            })
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(String::from)),
    )
}

fn optional_string_array_field(
    params: &Value,
    group: &str,
    field: &str,
    aliases: &[&str],
) -> Option<Vec<String>> {
    nested_object(params, group)
        .and_then(|obj| obj.get(field))
        .and_then(Value::as_array)
        .or_else(|| {
            aliases
                .iter()
                .find_map(|alias| params.get(*alias).and_then(Value::as_array))
        })
        .map(|arr| {
            normalize_tool_names(
                arr.iter()
                    .filter_map(|value| value.as_str().map(String::from)),
            )
        })
}

fn object_field(
    params: &Value,
    group: &str,
    field: &str,
    aliases: &[&str],
) -> Option<Map<String, Value>> {
    nested_object(params, group)
        .and_then(|obj| obj.get(field))
        .and_then(Value::as_object)
        .cloned()
        .or_else(|| {
            aliases
                .iter()
                .find_map(|alias| params.get(*alias).and_then(Value::as_object).cloned())
        })
}

fn validate_timezone_param(timezone: Option<String>) -> Result<Option<String>, ToolError> {
    timezone
        .map(|tz| {
            crate::timezone::parse_timezone(&tz)
                .map(|_| tz.clone())
                .ok_or_else(|| {
                    ToolError::InvalidParameters(format!("invalid IANA timezone: '{tz}'"))
                })
        })
        .transpose()
}

fn parse_system_event_filters(
    filters: Option<Map<String, Value>>,
) -> Result<HashMap<String, String>, ToolError> {
    let Some(obj) = filters else {
        return Ok(HashMap::new());
    };

    let mut parsed = HashMap::with_capacity(obj.len());
    for (key, value) in obj {
        let rendered = crate::agent::routine::json_value_as_filter_string(&value).ok_or_else(|| {
            ToolError::InvalidParameters(format!(
                "system_event filters only support string, number, and boolean values (invalid '{key}')"
            ))
        })?;
        parsed.insert(key, rendered);
    }

    Ok(parsed)
}

fn parse_routine_trigger(params: &Value) -> Result<NormalizedTriggerRequest, ToolError> {
    let kind = string_field(params, "request", "kind", &["trigger_type"])
        .map(|value| match value.as_str() {
            "event" => "message_event".to_string(),
            other => other.to_string(),
        })
        .ok_or_else(|| {
            ToolError::InvalidParameters(
                "routine_create requires request.kind (canonical) or trigger_type (legacy)"
                    .to_string(),
            )
        })?;

    match kind.as_str() {
        "cron" => {
            let schedule =
                string_field(params, "request", "schedule", &["schedule"]).ok_or_else(|| {
                    ToolError::InvalidParameters("cron request requires 'schedule'".to_string())
                })?;
            let timezone = validate_timezone_param(string_field(
                params,
                "request",
                "timezone",
                &["timezone"],
            ))?;
            next_cron_fire(&schedule, timezone.as_deref())
                .map_err(|e| ToolError::InvalidParameters(format!("invalid cron schedule: {e}")))?;
            Ok(NormalizedTriggerRequest::Cron { schedule, timezone })
        }
        "manual" => Ok(NormalizedTriggerRequest::Manual),
        "message_event" => {
            let pattern = string_field(params, "request", "pattern", &["event_pattern"])
                .ok_or_else(|| {
                    ToolError::InvalidParameters(
                        "message_event request requires 'pattern'".to_string(),
                    )
                })?;
            regex::RegexBuilder::new(&pattern)
                .size_limit(64 * 1024)
                .build()
                .map_err(|e| {
                    ToolError::InvalidParameters(format!("invalid or too complex regex: {e}"))
                })?;
            let channel = string_field(params, "request", "channel", &["event_channel"]);
            Ok(NormalizedTriggerRequest::MessageEvent { pattern, channel })
        }
        "system_event" => {
            let source =
                string_field(params, "request", "source", &["event_source"]).ok_or_else(|| {
                    ToolError::InvalidParameters(
                        "system_event request requires 'source'".to_string(),
                    )
                })?;
            let event_type = string_field(params, "request", "event_type", &["event_type"])
                .ok_or_else(|| {
                    ToolError::InvalidParameters(
                        "system_event request requires 'event_type'".to_string(),
                    )
                })?;
            let filters = parse_system_event_filters(object_field(
                params,
                "request",
                "filters",
                &["event_filters"],
            ))?;
            Ok(NormalizedTriggerRequest::SystemEvent {
                source,
                event_type,
                filters,
            })
        }
        other => Err(ToolError::InvalidParameters(format!(
            "unknown request.kind: {other}"
        ))),
    }
}

fn parse_execution_mode(value: Option<String>) -> Result<NormalizedExecutionMode, ToolError> {
    match value.as_deref().unwrap_or("lightweight") {
        "lightweight" => Ok(NormalizedExecutionMode::Lightweight),
        "full_job" => Ok(NormalizedExecutionMode::FullJob),
        other => Err(ToolError::InvalidParameters(format!(
            "unknown execution mode: {other}"
        ))),
    }
}

fn parse_requested_full_job_permission_mode(
    value: Option<String>,
) -> Result<Option<RequestedFullJobPermissionMode>, ToolError> {
    match value.as_deref() {
        None => Ok(None),
        Some("explicit") => Ok(Some(RequestedFullJobPermissionMode::Explicit)),
        Some("inherit_owner") => Ok(Some(RequestedFullJobPermissionMode::InheritOwner)),
        Some("copy_owner") => Ok(Some(RequestedFullJobPermissionMode::CopyOwner)),
        Some(other) => Err(ToolError::InvalidParameters(format!(
            "unknown full_job permission_mode: {other}"
        ))),
    }
}

fn parse_routine_execution(params: &Value) -> Result<NormalizedExecutionRequest, ToolError> {
    let mode = parse_execution_mode(string_field(params, "execution", "mode", &["action_type"]))?;
    let context_paths =
        string_array_field(params, "execution", "context_paths", &["context_paths"]);
    let use_tools = bool_field(params, "execution", "use_tools", &["use_tools"]).unwrap_or(false);
    let max_tool_rounds = u64_field(params, "execution", "max_tool_rounds", &["max_tool_rounds"])
        .unwrap_or(3)
        .clamp(1, crate::agent::routine::MAX_TOOL_ROUNDS_LIMIT as u64)
        as u32;
    let tool_permissions = string_array_field(
        params,
        "execution",
        "tool_permissions",
        &["tool_permissions"],
    );
    let permission_mode = parse_requested_full_job_permission_mode(string_field(
        params,
        "execution",
        "permission_mode",
        &["permission_mode"],
    ))?;

    Ok(NormalizedExecutionRequest {
        mode,
        context_paths,
        use_tools,
        max_tool_rounds,
        tool_permissions,
        permission_mode,
    })
}

fn parse_routine_delivery(params: &Value) -> NormalizedDeliveryRequest {
    NormalizedDeliveryRequest {
        channel: string_field(params, "delivery", "channel", &["notify_channel"]),
        user: string_field(params, "delivery", "user", &["notify_user"]),
    }
}

fn parse_routine_create_request(
    params: &Value,
) -> Result<NormalizedRoutineCreateRequest, ToolError> {
    let name = require_str(params, "name")?.to_string();
    let prompt = require_str(params, "prompt")?.to_string();
    let description = params
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let trigger = parse_routine_trigger(params)?;
    let execution = parse_routine_execution(params)?;
    let delivery = parse_routine_delivery(params);
    let cooldown_secs =
        u64_field(params, "advanced", "cooldown_secs", &["cooldown_secs"]).unwrap_or(300);

    Ok(NormalizedRoutineCreateRequest {
        name,
        description,
        prompt,
        trigger,
        execution,
        delivery,
        cooldown_secs,
    })
}

fn build_routine_trigger(trigger: &NormalizedTriggerRequest) -> Trigger {
    match trigger {
        NormalizedTriggerRequest::Cron { schedule, timezone } => Trigger::Cron {
            schedule: schedule.clone(),
            timezone: timezone.clone(),
        },
        NormalizedTriggerRequest::Manual => Trigger::Manual,
        NormalizedTriggerRequest::MessageEvent { pattern, channel } => Trigger::Event {
            channel: channel.clone(),
            pattern: pattern.clone(),
        },
        NormalizedTriggerRequest::SystemEvent {
            source,
            event_type,
            filters,
        } => Trigger::SystemEvent {
            source: source.clone(),
            event_type: event_type.clone(),
            filters: filters.clone(),
        },
    }
}

async fn build_routine_action(
    store: &dyn Database,
    user_id: &str,
    name: &str,
    prompt: &str,
    execution: &NormalizedExecutionRequest,
) -> Result<RoutineAction, ToolError> {
    match execution.mode {
        NormalizedExecutionMode::Lightweight => Ok(RoutineAction::Lightweight {
            prompt: prompt.to_string(),
            context_paths: execution.context_paths.clone(),
            max_tokens: 4096,
            use_tools: execution.use_tools,
            max_tool_rounds: execution.max_tool_rounds,
        }),
        NormalizedExecutionMode::FullJob => {
            let mut owner_settings = None;
            let requested_mode = match execution.permission_mode {
                Some(mode) => mode,
                None => {
                    let settings = load_full_job_permission_settings(store, user_id)
                        .await
                        .map_err(|e| {
                            ToolError::ExecutionFailed(format!(
                                "failed to load routine permission settings: {e}"
                            ))
                        })?;
                    let mode = match settings.default_mode {
                        FullJobPermissionDefaultMode::Explicit => {
                            RequestedFullJobPermissionMode::Explicit
                        }
                        FullJobPermissionDefaultMode::InheritOwner => {
                            RequestedFullJobPermissionMode::InheritOwner
                        }
                        FullJobPermissionDefaultMode::CopyOwner => {
                            RequestedFullJobPermissionMode::CopyOwner
                        }
                    };
                    owner_settings = Some(settings);
                    mode
                }
            };
            let (permission_mode, tool_permissions) = match requested_mode {
                RequestedFullJobPermissionMode::Explicit => (
                    FullJobPermissionMode::Explicit,
                    execution.tool_permissions.clone(),
                ),
                RequestedFullJobPermissionMode::InheritOwner => (
                    FullJobPermissionMode::InheritOwner,
                    execution.tool_permissions.clone(),
                ),
                RequestedFullJobPermissionMode::CopyOwner => {
                    let owner_allowed_tools = match owner_settings {
                        Some(settings) => settings.owner_allowed_tools,
                        None => {
                            load_full_job_permission_settings(store, user_id)
                                .await
                                .map_err(|e| {
                                    ToolError::ExecutionFailed(format!(
                                        "failed to load routine permission settings: {e}"
                                    ))
                                })?
                                .owner_allowed_tools
                        }
                    };
                    (
                        FullJobPermissionMode::Explicit,
                        normalize_tool_names(
                            owner_allowed_tools
                                .into_iter()
                                .chain(execution.tool_permissions.iter().cloned()),
                        ),
                    )
                }
            };
            Ok(RoutineAction::FullJob {
                title: name.to_string(),
                description: prompt.to_string(),
                max_iterations: 10,
                tool_permissions,
                permission_mode,
            })
        }
    }
}

fn routine_requests_full_job(params: &Value) -> bool {
    matches!(
        string_field(params, "execution", "mode", &["action_type"]).as_deref(),
        Some("full_job")
    )
}

fn routine_permission_fields_present(params: &Value) -> bool {
    nested_object(params, "execution").is_some_and(|execution| {
        execution.contains_key("tool_permissions") || execution.contains_key("permission_mode")
    }) || params.get("tool_permissions").is_some()
        || params.get("permission_mode").is_some()
}

fn event_emit_schema(include_source_alias: bool) -> Value {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "event_source": {
                "type": "string",
                "description": "Canonical event source, such as 'github'."
            },
            "event_type": {
                "type": "string",
                "description": "Event type, such as 'issue.opened'."
            },
            "payload": {
                "properties": {},
                "type": "object",
                "description": "Structured event payload."
            }
        },
        "required": ["event_type"]
    });

    if include_source_alias {
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            properties.insert(
                "source".to_string(),
                serde_json::json!({
                    "type": "string",
                    "description": "Compatibility alias for event_source."
                }),
            );
        }
        if let Some(schema_obj) = schema.as_object_mut() {
            schema_obj.insert(
                "anyOf".to_string(),
                serde_json::json!([
                    { "required": ["event_source"] },
                    { "required": ["source"] }
                ]),
            );
        }
    } else if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.push(Value::String("event_source".to_string()));
    }

    schema
}

pub(crate) fn event_emit_parameters_schema() -> Value {
    event_emit_schema(false)
}

fn event_emit_discovery_schema() -> Value {
    static CACHE: OnceLock<Value> = OnceLock::new();
    CACHE.get_or_init(|| event_emit_schema(true)).clone()
}

fn parse_event_emit_args(params: &Value) -> Result<(String, String, Value), ToolError> {
    let source = params
        .get("event_source")
        .and_then(Value::as_str)
        .or_else(|| params.get("source").and_then(Value::as_str))
        .ok_or_else(|| {
            ToolError::InvalidParameters(
                "event_emit requires 'event_source' (canonical) or 'source' (alias)".to_string(),
            )
        })?
        .to_string();
    let event_type = require_str(params, "event_type")?.to_string();
    let payload = params
        .get("payload")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    Ok((source, event_type, payload))
}

pub struct RoutineCreateTool {
    store: Arc<dyn Database>,
    engine: Arc<RoutineEngine>,
}

impl RoutineCreateTool {
    pub fn new(store: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Self {
        Self { store, engine }
    }
}

#[async_trait]
impl Tool for RoutineCreateTool {
    fn name(&self) -> &str {
        "routine_create"
    }

    fn description(&self) -> &str {
        "Create a new routine (scheduled or event-driven task). \
         Supports cron schedules, event pattern matching, system events, and manual triggers. \
         Use this when the user wants something to happen periodically or reactively."
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        if routine_requests_full_job(params) {
            ApprovalRequirement::UnlessAutoApproved
        } else {
            ApprovalRequirement::Never
        }
    }

    fn parameters_schema(&self) -> serde_json::Value {
        routine_create_parameters_schema()
    }

    fn discovery_schema(&self) -> serde_json::Value {
        routine_create_discovery_schema()
    }

    fn discovery_summary(&self) -> Option<ToolDiscoverySummary> {
        Some(routine_create_tool_summary())
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let normalized = parse_routine_create_request(&params)?;
        let trigger = build_routine_trigger(&normalized.trigger);
        let action = build_routine_action(
            self.store.as_ref(),
            &ctx.user_id,
            &normalized.name,
            &normalized.prompt,
            &normalized.execution,
        )
        .await?;

        // Compute next fire time for cron
        let next_fire = if let Trigger::Cron {
            ref schedule,
            ref timezone,
        } = trigger
        {
            next_cron_fire(schedule, timezone.as_deref()).unwrap_or(None)
        } else {
            None
        };

        let routine = Routine {
            id: Uuid::new_v4(),
            name: normalized.name.clone(),
            description: normalized.description.clone(),
            user_id: ctx.user_id.clone(),
            enabled: true,
            trigger,
            action,
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs(normalized.cooldown_secs),
                max_concurrent: 1,
                dedup_window: None,
            },
            notify: NotifyConfig {
                channel: normalized.delivery.channel.clone(),
                user: normalized.delivery.user.clone(),
                ..NotifyConfig::default()
            },
            last_run_at: None,
            next_fire_at: next_fire,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.store
            .create_routine(&routine)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to create routine: {e}")))?;

        // Refresh event cache if this is an event trigger
        if matches!(
            routine.trigger,
            Trigger::Event { .. } | Trigger::SystemEvent { .. }
        ) {
            self.engine.refresh_event_cache().await;
        }

        let result = serde_json::json!({
            "id": routine.id.to_string(),
            "name": routine.name,
            "trigger_type": routine.trigger.type_tag(),
            "next_fire_at": routine.next_fire_at.map(|t| t.to_rfc3339()),
            "status": "created",
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_list ====================

pub struct RoutineListTool {
    store: Arc<dyn Database>,
}

impl RoutineListTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RoutineListTool {
    fn name(&self) -> &str {
        "routine_list"
    }

    fn description(&self) -> &str {
        "List all routines with their status, trigger info, and next fire time."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let routines = self
            .store
            .list_routines(&ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to list routines: {e}")))?;

        let list: Vec<serde_json::Value> = routines
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id.to_string(),
                    "name": r.name,
                    "description": r.description,
                    "enabled": r.enabled,
                    "trigger_type": r.trigger.type_tag(),
                    "action_type": r.action.type_tag(),
                    "last_run_at": r.last_run_at.map(|t| t.to_rfc3339()),
                    "next_fire_at": r.next_fire_at.map(|t| t.to_rfc3339()),
                    "run_count": r.run_count,
                    "consecutive_failures": r.consecutive_failures,
                })
            })
            .collect();

        let result = serde_json::json!({
            "count": list.len(),
            "routines": list,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_update ====================

pub struct RoutineUpdateTool {
    store: Arc<dyn Database>,
    engine: Arc<RoutineEngine>,
}

impl RoutineUpdateTool {
    pub fn new(store: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Self {
        Self { store, engine }
    }
}

#[async_trait]
impl Tool for RoutineUpdateTool {
    fn name(&self) -> &str {
        "routine_update"
    }

    fn description(&self) -> &str {
        "Update an existing routine. Can change prompt, description, enabled state, cron schedule/timezone, \
         or full_job permission settings. Pass the routine name and only the fields you want to change. \
         This does not convert trigger types."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        routine_update_parameters_schema()
    }

    fn requires_approval(&self, params: &serde_json::Value) -> ApprovalRequirement {
        if routine_permission_fields_present(params) {
            ApprovalRequirement::UnlessAutoApproved
        } else {
            ApprovalRequirement::Never
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let mut routine = self
            .store
            .get_routine_by_name(&ctx.user_id, name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DB error: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("routine '{}' not found", name)))?;

        // Apply updates
        if let Some(enabled) = params.get("enabled").and_then(|v| v.as_bool()) {
            routine.enabled = enabled;
        }

        if let Some(desc) = params.get("description").and_then(|v| v.as_str()) {
            routine.description = desc.to_string();
        }

        if let Some(prompt) = params.get("prompt").and_then(|v| v.as_str()) {
            match &mut routine.action {
                RoutineAction::Lightweight { prompt: p, .. } => *p = prompt.to_string(),
                RoutineAction::FullJob { description: d, .. } => *d = prompt.to_string(),
            }
        }

        let requested_permission_mode = parse_requested_full_job_permission_mode(string_field(
            &params,
            "execution",
            "permission_mode",
            &["permission_mode"],
        ))?;
        let requested_tool_permissions = optional_string_array_field(
            &params,
            "execution",
            "tool_permissions",
            &["tool_permissions"],
        );
        let updates_permissions =
            requested_permission_mode.is_some() || requested_tool_permissions.is_some();

        if updates_permissions {
            match &mut routine.action {
                RoutineAction::FullJob {
                    tool_permissions,
                    permission_mode,
                    ..
                } => {
                    let next_tool_permissions =
                        requested_tool_permissions.unwrap_or_else(|| tool_permissions.clone());
                    match requested_permission_mode {
                        Some(RequestedFullJobPermissionMode::Explicit) => {
                            *permission_mode = FullJobPermissionMode::Explicit;
                            *tool_permissions = next_tool_permissions;
                        }
                        Some(RequestedFullJobPermissionMode::InheritOwner) => {
                            *permission_mode = FullJobPermissionMode::InheritOwner;
                            *tool_permissions = next_tool_permissions;
                        }
                        Some(RequestedFullJobPermissionMode::CopyOwner) => {
                            let owner_settings = load_full_job_permission_settings(
                                self.store.as_ref(),
                                &ctx.user_id,
                            )
                            .await
                            .map_err(|e| {
                                ToolError::ExecutionFailed(format!(
                                    "failed to load routine permission settings: {e}"
                                ))
                            })?;
                            *permission_mode = FullJobPermissionMode::Explicit;
                            *tool_permissions = normalize_tool_names(
                                owner_settings
                                    .owner_allowed_tools
                                    .into_iter()
                                    .chain(next_tool_permissions),
                            );
                        }
                        None => {
                            *tool_permissions = next_tool_permissions;
                        }
                    }
                }
                RoutineAction::Lightweight { .. } => {
                    return Err(ToolError::InvalidParameters(
                        "permission_mode and tool_permissions can only be updated for full_job routines"
                            .to_string(),
                    ));
                }
            }
        }

        // Validate timezone param if provided
        let new_timezone = params
            .get("timezone")
            .and_then(|v| v.as_str())
            .map(|tz| {
                crate::timezone::parse_timezone(tz)
                    .map(|_| tz.to_string())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(format!("invalid IANA timezone: '{tz}'"))
                    })
            })
            .transpose()?;

        let new_schedule = params.get("schedule").and_then(|v| v.as_str());

        if new_schedule.is_some() || new_timezone.is_some() {
            // Extract existing cron fields (cloned to avoid borrow conflict)
            let existing_cron = match &routine.trigger {
                Trigger::Cron { schedule, timezone } => Some((schedule.clone(), timezone.clone())),
                _ => None,
            };

            if let Some((old_schedule, old_tz)) = existing_cron {
                let effective_schedule = new_schedule.unwrap_or(&old_schedule);
                let effective_tz = new_timezone.or(old_tz);
                // Validate
                next_cron_fire(effective_schedule, effective_tz.as_deref()).map_err(|e| {
                    ToolError::InvalidParameters(format!("invalid cron schedule: {e}"))
                })?;

                routine.trigger = Trigger::Cron {
                    schedule: effective_schedule.to_string(),
                    timezone: effective_tz.clone(),
                };
                routine.next_fire_at =
                    next_cron_fire(effective_schedule, effective_tz.as_deref()).unwrap_or(None);
            } else {
                return Err(ToolError::InvalidParameters(
                    "Cannot update schedule or timezone on a non-cron routine.".to_string(),
                ));
            }
        }

        self.store
            .update_routine(&routine)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to update: {e}")))?;

        // Refresh event cache in case trigger changed
        self.engine.refresh_event_cache().await;

        let result = serde_json::json!({
            "name": routine.name,
            "enabled": routine.enabled,
            "trigger_type": routine.trigger.type_tag(),
            "next_fire_at": routine.next_fire_at.map(|t| t.to_rfc3339()),
            "status": "updated",
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_delete ====================

pub struct RoutineDeleteTool {
    store: Arc<dyn Database>,
    engine: Arc<RoutineEngine>,
}

impl RoutineDeleteTool {
    pub fn new(store: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Self {
        Self { store, engine }
    }
}

#[async_trait]
impl Tool for RoutineDeleteTool {
    fn name(&self) -> &str {
        "routine_delete"
    }

    fn description(&self) -> &str {
        "Delete a routine permanently. This also removes all run history."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the routine to delete"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let routine = self
            .store
            .get_routine_by_name(&ctx.user_id, name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DB error: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("routine '{}' not found", name)))?;

        let deleted = self
            .store
            .delete_routine(routine.id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to delete: {e}")))?;

        // Refresh event cache
        self.engine.refresh_event_cache().await;

        let result = serde_json::json!({
            "name": name,
            "deleted": deleted,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_fire ====================

pub struct RoutineFireTool {
    store: Arc<dyn Database>,
    engine: Arc<RoutineEngine>,
}

impl RoutineFireTool {
    pub fn new(store: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Self {
        Self { store, engine }
    }
}

#[async_trait]
impl Tool for RoutineFireTool {
    fn name(&self) -> &str {
        "routine_fire"
    }

    fn description(&self) -> &str {
        "Manually trigger a routine to run immediately, bypassing schedule, trigger type, and cooldown."
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Firing a routine can dispatch a full_job with pre-authorized Always-gated tools,
        // so this is a meaningful escalation that warrants auto-approval gating.
        ApprovalRequirement::UnlessAutoApproved
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the routine to fire"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let routine = self
            .store
            .get_routine_by_name(&ctx.user_id, name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DB error: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("routine '{}' not found", name)))?;

        let run_id = self
            .engine
            .fire_manual(routine.id, None)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("failed to fire routine '{}': {e}", name))
            })?;

        let result = serde_json::json!({
            "name": name,
            "run_id": run_id.to_string(),
            "status": "fired",
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_history ====================

pub struct RoutineHistoryTool {
    store: Arc<dyn Database>,
}

impl RoutineHistoryTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RoutineHistoryTool {
    fn name(&self) -> &str {
        "routine_history"
    }

    fn description(&self) -> &str {
        "View the execution history of a routine. Shows recent runs with status, duration, and results."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the routine"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max runs to return (default: 10)",
                    "default": 10
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(10)
            .min(50);

        let routine = self
            .store
            .get_routine_by_name(&ctx.user_id, name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DB error: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("routine '{}' not found", name)))?;

        let runs = self
            .store
            .list_routine_runs(routine.id, limit)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to list runs: {e}")))?;

        let run_list: Vec<serde_json::Value> = runs
            .iter()
            .map(|r| {
                let duration_secs = r
                    .completed_at
                    .map(|c| c.signed_duration_since(r.started_at).num_seconds());
                serde_json::json!({
                    "id": r.id.to_string(),
                    "trigger_type": r.trigger_type,
                    "trigger_detail": r.trigger_detail,
                    "started_at": r.started_at.to_rfc3339(),
                    "completed_at": r.completed_at.map(|t| t.to_rfc3339()),
                    "duration_secs": duration_secs,
                    "status": r.status.to_string(),
                    "result_summary": r.result_summary,
                    "tokens_used": r.tokens_used,
                })
            })
            .collect();

        let result = serde_json::json!({
            "routine": name,
            "total_runs": routine.run_count,
            "runs": run_list,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== event_emit ====================

pub struct EventEmitTool {
    engine: Arc<RoutineEngine>,
}

impl EventEmitTool {
    pub fn new(engine: Arc<RoutineEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for EventEmitTool {
    fn name(&self) -> &str {
        "event_emit"
    }

    fn description(&self) -> &str {
        "Emit a structured system event to routines with a system_event trigger. \
         Use this to trigger routines from tool workflows without waiting for cron."
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Emitting an event can fire system_event routines that dispatch full_jobs
        // with pre-authorized Always-gated tools — same escalation risk as routine_fire.
        ApprovalRequirement::UnlessAutoApproved
    }

    fn parameters_schema(&self) -> serde_json::Value {
        event_emit_parameters_schema()
    }

    fn discovery_schema(&self) -> serde_json::Value {
        event_emit_discovery_schema()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let (source, event_type, payload) = parse_event_emit_args(&params)?;

        let fired = self
            .engine
            .emit_system_event(&source, &event_type, &payload, Some(&ctx.user_id))
            .await;

        let result = serde_json::json!({
            "event_source": &source,
            "event_type": &event_type,
            "user_id": &ctx.user_id,
            "fired_routines": fired,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::validate_tool_schema;

    // These tests intentionally use direct assertion macros.
    const ROUTINE_CREATE_LEGACY_ALIASES: &[&str] = &[
        "trigger_type",
        "schedule",
        "timezone",
        "event_pattern",
        "event_channel",
        "event_source",
        "event_type",
        "event_filters",
        "action_type",
        "context_paths",
        "use_tools",
        "max_tool_rounds",
        "tool_permissions",
        "permission_mode",
        "notify_channel",
        "notify_user",
        "cooldown_secs",
    ];

    fn schema_property<'a>(schema: &'a Value, name: &str) -> &'a Value {
        schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(name))
            .unwrap_or_else(|| panic!("missing schema property {name}"))
    }

    fn maybe_schema_property<'a>(schema: &'a Value, name: &str) -> Option<&'a Value> {
        schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(name))
    }

    fn nested_schema_property<'a>(schema: &'a Value, object_name: &str, name: &str) -> &'a Value {
        schema_property(schema, object_name)
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(name))
            .unwrap_or_else(|| panic!("missing nested schema property {object_name}.{name}"))
    }

    fn variant_with_kind<'a>(variants: &'a [Value], kind: &str) -> &'a Value {
        variants
            .iter()
            .find(|variant| {
                variant
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|properties| properties.get("kind"))
                    .and_then(|kind_schema| kind_schema.get("enum"))
                    .and_then(Value::as_array)
                    .is_some_and(|enums| enums.contains(&Value::String(kind.to_string())))
            })
            .unwrap_or_else(|| panic!("missing variant for kind={kind}"))
    }

    fn variant_with_mode<'a>(variants: &'a [Value], mode: &str) -> &'a Value {
        variants
            .iter()
            .find(|variant| {
                variant
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|properties| properties.get("mode"))
                    .and_then(|mode_schema| mode_schema.get("enum"))
                    .and_then(Value::as_array)
                    .is_some_and(|enums| enums.contains(&Value::String(mode.to_string())))
            })
            .unwrap_or_else(|| panic!("missing variant for mode={mode}"))
    }

    #[test]
    fn parses_grouped_manual_lightweight_request() {
        let params = serde_json::json!({
            "name": "manual-check",
            "prompt": "Inspect the repo for issues.",
            "request": {
                "kind": "manual"
            }
        });

        let parsed = parse_routine_create_request(&params).expect("parse grouped manual request");

        assert_eq!(parsed.name.as_str(), "manual-check");
        assert_eq!(parsed.prompt.as_str(), "Inspect the repo for issues.");
        assert!(
            matches!(parsed.trigger, NormalizedTriggerRequest::Manual),
            "expected manual trigger",
        );
        assert!(
            matches!(parsed.execution.mode, NormalizedExecutionMode::Lightweight),
            "expected lightweight execution mode",
        );
        assert_eq!(parsed.cooldown_secs, 300);
        assert!(
            parsed.delivery.user.is_none(),
            "expected omitted delivery.user to remain unspecified",
        );
    }

    #[test]
    fn parses_grouped_cron_full_job_request() {
        let params = serde_json::json!({
            "name": "weekday-digest",
            "prompt": "Prepare the morning digest.",
            "request": {
                "kind": "cron",
                "schedule": "0 0 9 * * MON-FRI",
                "timezone": "UTC"
            },
            "execution": {
                "mode": "full_job",
                "tool_permissions": ["message", "http"]
            },
            "delivery": {
                "channel": "telegram",
                "user": "ops-team"
            },
            "advanced": {
                "cooldown_secs": 30
            }
        });

        let parsed = parse_routine_create_request(&params).expect("parse grouped cron request");

        assert!(
            matches!(
                parsed.trigger,
                NormalizedTriggerRequest::Cron { ref schedule, ref timezone }
                if schedule == "0 0 9 * * MON-FRI" && timezone.as_deref() == Some("UTC")
            ),
            "expected grouped cron trigger",
        );
        assert!(
            matches!(parsed.execution.mode, NormalizedExecutionMode::FullJob),
            "expected full_job execution mode",
        );
        assert_eq!(
            parsed.execution.tool_permissions,
            vec!["message".to_string(), "http".to_string()],
        );
        assert_eq!(parsed.execution.permission_mode, None);
        assert_eq!(parsed.delivery.channel.as_deref(), Some("telegram"));
        assert_eq!(parsed.delivery.user.as_deref(), Some("ops-team"));
        assert_eq!(parsed.cooldown_secs, 30);
    }

    #[test]
    fn parses_grouped_message_event_with_tools() {
        let params = serde_json::json!({
            "name": "deploy-watch",
            "prompt": "Look for deploy requests.",
            "request": {
                "kind": "message_event",
                "pattern": "deploy\\s+prod",
                "channel": "slack"
            },
            "execution": {
                "use_tools": true,
                "max_tool_rounds": 5,
                "context_paths": ["context/deploy.md"]
            }
        });

        let parsed =
            parse_routine_create_request(&params).expect("parse grouped message event request");

        assert!(
            matches!(
                parsed.trigger,
                NormalizedTriggerRequest::MessageEvent { ref pattern, ref channel }
                if pattern == "deploy\\s+prod" && channel.as_deref() == Some("slack")
            ),
            "expected grouped message_event trigger",
        );
        assert!(parsed.execution.use_tools, "expected use_tools=true");
        assert_eq!(parsed.execution.max_tool_rounds, 5);
        assert_eq!(
            parsed.execution.context_paths,
            vec!["context/deploy.md".to_string()],
        );
    }

    #[test]
    fn parses_grouped_system_event_request() {
        let params = serde_json::json!({
            "name": "issue-watch",
            "prompt": "Summarize new GitHub issues.",
            "request": {
                "kind": "system_event",
                "source": "github",
                "event_type": "issue.opened",
                "filters": {
                    "repository": "nearai/ironclaw",
                    "public": true,
                    "issue_number": 42
                }
            },
            "execution": {
                "mode": "full_job"
            }
        });

        let parsed =
            parse_routine_create_request(&params).expect("parse grouped system event request");

        assert!(
            matches!(
                parsed.trigger,
                NormalizedTriggerRequest::SystemEvent { ref source, ref event_type, ref filters }
                if source == "github"
                    && event_type == "issue.opened"
                    && filters.get("repository") == Some(&"nearai/ironclaw".to_string())
                    && filters.get("public") == Some(&"true".to_string())
                    && filters.get("issue_number") == Some(&"42".to_string())
            ),
            "expected grouped system_event trigger",
        );
    }

    #[test]
    fn rejects_system_event_filters_with_nested_values() {
        let params = serde_json::json!({
            "name": "issue-watch",
            "prompt": "Summarize new GitHub issues.",
            "request": {
                "kind": "system_event",
                "source": "github",
                "event_type": "issue.opened",
                "filters": {
                    "repository": {
                        "owner": "nearai",
                        "name": "ironclaw"
                    }
                }
            }
        });

        let err = parse_routine_create_request(&params)
            .expect_err("reject nested system event filter values");
        match err {
            ToolError::InvalidParameters(message) => {
                assert!(
                    message.contains(
                        "system_event filters only support string, number, and boolean values",
                    ),
                    "unexpected invalid filter error: {message}",
                )
            }
            other => panic!("expected InvalidParameters, got {other:?}"),
        }
    }

    #[test]
    fn parses_legacy_flat_shape() {
        let params = serde_json::json!({
            "name": "legacy-routine",
            "prompt": "Legacy create path.",
            "trigger_type": "event",
            "event_pattern": "hello",
            "event_channel": "telegram",
            "action_type": "full_job",
            "tool_permissions": ["message"],
            "notify_channel": "telegram",
            "notify_user": "123"
        });

        let parsed = parse_routine_create_request(&params).expect("parse legacy flat request");

        assert!(
            matches!(
                parsed.trigger,
                NormalizedTriggerRequest::MessageEvent { ref pattern, ref channel }
                if pattern == "hello" && channel.as_deref() == Some("telegram")
            ),
            "expected legacy message_event trigger",
        );
        assert!(
            matches!(parsed.execution.mode, NormalizedExecutionMode::FullJob),
            "expected full_job execution mode",
        );
        assert_eq!(
            parsed.execution.tool_permissions,
            vec!["message".to_string()],
        );
        assert_eq!(parsed.delivery.channel.as_deref(), Some("telegram"));
        assert_eq!(parsed.delivery.user.as_deref(), Some("123"));
    }

    #[test]
    fn parses_mixed_grouped_and_legacy_aliases() {
        let params = serde_json::json!({
            "name": "mixed-routine",
            "prompt": "Mixed payload.",
            "request": {
                "kind": "cron"
            },
            "schedule": "0 0 8 * * *",
            "timezone": "UTC",
            "execution": {
                "mode": "lightweight"
            },
            "notify_user": "fallback-user",
            "advanced": {
                "cooldown_secs": 45
            }
        });

        let parsed = parse_routine_create_request(&params).expect("parse mixed request");

        assert!(
            matches!(
                parsed.trigger,
                NormalizedTriggerRequest::Cron { ref schedule, ref timezone }
                if schedule == "0 0 8 * * *" && timezone.as_deref() == Some("UTC")
            ),
            "expected mixed cron trigger",
        );
        assert_eq!(parsed.delivery.user.as_deref(), Some("fallback-user"));
        assert_eq!(parsed.cooldown_secs, 45);
    }

    #[test]
    fn parses_event_emit_with_source_alias() {
        let params = serde_json::json!({
            "source": "github",
            "event_type": "issue.opened",
            "payload": { "issue_number": 7 }
        });

        let (source, event_type, payload) =
            parse_event_emit_args(&params).expect("parse event_emit source alias");

        assert_eq!(source, "github".to_string());
        assert_eq!(event_type, "issue.opened".to_string());
        assert_eq!(payload["issue_number"].clone(), serde_json::json!(7));
    }

    #[test]
    fn parses_event_emit_with_event_source() {
        let params = serde_json::json!({
            "event_source": "github",
            "event_type": "issue.opened"
        });

        let (source, event_type, payload) =
            parse_event_emit_args(&params).expect("parse canonical event_emit args");

        assert_eq!(source, "github".to_string());
        assert_eq!(event_type, "issue.opened".to_string());
        assert_eq!(payload, serde_json::json!({}));
    }

    #[test]
    fn routine_create_parameters_schema_prefers_grouped_request_shape() {
        let schema = routine_create_parameters_schema();
        let errors = validate_tool_schema(&schema, "routine_create");
        assert!(
            errors.is_empty(),
            "routine_create schema should validate cleanly: {errors:?}",
        );

        let request = schema_property(&schema, "request");
        assert!(
            request.is_object(),
            "request should be present in compact schema",
        );
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .expect("routine_create required list");
        assert!(
            required.contains(&Value::String("request".to_string())),
            "compact parameters schema should require request",
        );

        for legacy_alias in ROUTINE_CREATE_LEGACY_ALIASES {
            assert!(
                maybe_schema_property(&schema, legacy_alias).is_none(),
                "compact parameters schema should hide legacy alias",
            );
        }
    }

    #[test]
    fn routine_create_discovery_schema_keeps_legacy_aliases() {
        let schema = routine_create_discovery_schema();
        let any_of = schema
            .get("anyOf")
            .and_then(Value::as_array)
            .expect("routine_create discovery anyOf");
        assert_eq!(any_of.len(), 2usize);

        for legacy_alias in ROUTINE_CREATE_LEGACY_ALIASES {
            assert!(
                schema_property(&schema, legacy_alias).is_object(),
                "discovery schema should retain legacy alias",
            );
        }
    }

    #[test]
    fn routine_create_discovery_schema_splits_request_variants() {
        let schema = routine_create_discovery_schema();
        let request = schema_property(&schema, "request");
        let variants = request
            .get("oneOf")
            .and_then(Value::as_array)
            .expect("request.oneOf variants");
        assert_eq!(variants.len(), 4usize);

        let cron = variant_with_kind(variants, "cron");
        let cron_required = cron
            .get("required")
            .and_then(Value::as_array)
            .expect("cron required list");
        assert!(
            cron_required.contains(&Value::String("schedule".to_string())),
            "cron variant should require schedule",
        );

        let message_event = variant_with_kind(variants, "message_event");
        let message_required = message_event
            .get("required")
            .and_then(Value::as_array)
            .expect("message_event required list");
        assert!(
            message_required.contains(&Value::String("pattern".to_string())),
            "message_event variant should require pattern",
        );

        let system_event = variant_with_kind(variants, "system_event");
        let system_required = system_event
            .get("required")
            .and_then(Value::as_array)
            .expect("system_event required list");
        assert!(
            system_required.contains(&Value::String("source".to_string()))
                && system_required.contains(&Value::String("event_type".to_string())),
            "system_event variant should require source and event_type",
        );
    }

    #[test]
    fn routine_create_discovery_schema_splits_execution_variants() {
        let schema = routine_create_discovery_schema();
        let execution = schema_property(&schema, "execution");
        let variants = execution
            .get("oneOf")
            .and_then(Value::as_array)
            .expect("execution.oneOf variants");
        assert_eq!(variants.len(), 2usize);

        let lightweight = variant_with_mode(variants, "lightweight");
        let lightweight_props = lightweight
            .get("properties")
            .and_then(Value::as_object)
            .expect("lightweight properties");
        assert!(
            lightweight_props.contains_key("use_tools")
                && lightweight_props.contains_key("context_paths")
                && lightweight_props.contains_key("max_tool_rounds"),
            "lightweight variant should expose lightweight-only fields",
        );

        let full_job = variant_with_mode(variants, "full_job");
        let full_job_props = full_job
            .get("properties")
            .and_then(Value::as_object)
            .expect("full_job properties");
        assert!(
            full_job_props.contains_key("tool_permissions")
                && full_job_props.contains_key("permission_mode"),
            "full_job variant should expose permission fields",
        );
    }

    #[test]
    fn routine_create_discovery_summary_explains_rules_and_examples() {
        let summary = routine_create_tool_summary();

        assert_eq!(
            summary.always_required,
            vec![
                "name".to_string(),
                "prompt".to_string(),
                "request.kind".to_string()
            ],
        );
        assert!(
            summary
                .conditional_requirements
                .iter()
                .any(|rule| rule.contains("request.kind='cron'")),
            "summary should explain cron requirement",
        );
        assert!(
            summary
                .notes
                .iter()
                .any(|note| note.contains("Legacy flat aliases")),
            "summary should mention legacy aliases",
        );
        assert_eq!(summary.examples.len(), 4usize);
    }

    #[test]
    fn routine_create_parameters_schema_describes_grouped_trigger_fields() {
        let schema = routine_create_parameters_schema();

        let request_description = schema_property(&schema, "request")
            .get("description")
            .and_then(Value::as_str)
            .expect("request description");
        assert!(
            request_description.contains("Set request.kind first"),
            "request description should mention kind-first guidance",
        );

        let pattern_description = nested_schema_property(&schema, "request", "pattern")
            .get("description")
            .and_then(Value::as_str)
            .expect("request.pattern description");
        assert!(
            pattern_description.contains("message_event"),
            "pattern description should mention message_event",
        );

        let source_description = nested_schema_property(&schema, "request", "source")
            .get("description")
            .and_then(Value::as_str)
            .expect("request.source description");
        assert!(
            source_description.contains("system_event"),
            "source description should mention system_event",
        );

        let filters_description = nested_schema_property(&schema, "request", "filters")
            .get("description")
            .and_then(Value::as_str)
            .expect("request.filters description");
        assert!(
            filters_description.contains("top-level string, number, and boolean"),
            "filters description should mention supported scalar payload types",
        );

        let filters_schema = nested_schema_property(&schema, "request", "filters");
        let additional_properties = filters_schema
            .get("additionalProperties")
            .expect("request.filters additionalProperties");
        let allowed_types = additional_properties
            .get("type")
            .and_then(Value::as_array)
            .expect("request.filters additionalProperties.type");
        assert!(
            allowed_types.contains(&Value::String("string".to_string()))
                && allowed_types.contains(&Value::String("number".to_string()))
                && allowed_types.contains(&Value::String("boolean".to_string())),
            "filters schema should constrain additionalProperties to scalar values",
        );
    }

    #[test]
    fn routine_update_schema_exposes_supported_fields_and_limits() {
        let schema = routine_update_parameters_schema();
        let errors = validate_tool_schema(&schema, "routine_update");
        assert!(
            errors.is_empty(),
            "routine_update schema should validate cleanly: {errors:?}",
        );

        for field in [
            "name",
            "enabled",
            "prompt",
            "schedule",
            "timezone",
            "description",
            "tool_permissions",
            "permission_mode",
        ] {
            let _ = schema_property(&schema, field);
        }

        let schedule_description = schema_property(&schema, "schedule")
            .get("description")
            .and_then(Value::as_str)
            .expect("schedule description");
        assert!(
            schedule_description.contains("cron triggers"),
            "schedule description should mention cron triggers",
        );

        let timezone_description = schema_property(&schema, "timezone")
            .get("description")
            .and_then(Value::as_str)
            .expect("timezone description");
        assert!(
            timezone_description.contains("cron triggers"),
            "timezone description should mention cron triggers",
        );
    }

    #[test]
    fn routine_create_detects_full_job_requests_for_approval() {
        let full_job = serde_json::json!({
            "name": "approve-me",
            "prompt": "Run autonomously",
            "request": { "kind": "manual" },
            "execution": { "mode": "full_job" }
        });
        let lightweight = serde_json::json!({
            "name": "safe",
            "prompt": "Stay lightweight",
            "request": { "kind": "manual" }
        });

        assert!(routine_requests_full_job(&full_job));
        assert!(!routine_requests_full_job(&lightweight));
    }

    #[test]
    fn event_emit_parameters_schema_prefers_canonical_event_source() {
        let schema = event_emit_parameters_schema();
        let errors = validate_tool_schema(&schema, "event_emit");
        assert!(
            errors.is_empty(),
            "event_emit schema should validate cleanly: {errors:?}",
        );

        assert!(
            schema_property(&schema, "event_source").is_object(),
            "event_emit parameters schema should expose event_source",
        );
        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .expect("event_emit required list");
        assert!(
            required.contains(&Value::String("event_source".to_string())),
            "event_emit parameters schema should require event_source",
        );
        assert!(
            maybe_schema_property(&schema, "source").is_none(),
            "event_emit parameters schema should hide source alias",
        );
    }

    #[test]
    fn event_emit_discovery_schema_keeps_source_alias() {
        let schema = event_emit_discovery_schema();
        let any_of = schema
            .get("anyOf")
            .and_then(Value::as_array)
            .expect("event_emit discovery anyOf");
        assert_eq!(any_of.len(), 2usize);
        assert!(
            schema_property(&schema, "source").is_object(),
            "event_emit discovery schema should keep source alias",
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn build_full_job_action_defaults_to_inherit_owner_for_new_routines() {
        let (db, _tmp) = crate::testing::test_db().await;
        let execution = NormalizedExecutionRequest {
            mode: NormalizedExecutionMode::FullJob,
            context_paths: Vec::new(),
            use_tools: false,
            max_tool_rounds: 3,
            tool_permissions: vec!["shell".to_string()],
            permission_mode: None,
        };

        let action =
            build_routine_action(db.as_ref(), "default", "issue-1316", "Run it", &execution)
                .await
                .expect("build action");

        assert!(matches!(
            action,
            RoutineAction::FullJob {
                permission_mode: FullJobPermissionMode::InheritOwner,
                tool_permissions,
                ..
            } if tool_permissions == vec!["shell".to_string()]
        ));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn build_full_job_action_copy_owner_snapshots_allowlist() {
        let (db, _tmp) = crate::testing::test_db().await;
        db.set_setting(
            "default",
            crate::agent::routine::FULL_JOB_OWNER_ALLOWED_TOOLS_SETTING_KEY,
            &serde_json::json!(["http", "shell"]),
        )
        .await
        .expect("set owner allowlist");
        let execution = NormalizedExecutionRequest {
            mode: NormalizedExecutionMode::FullJob,
            context_paths: Vec::new(),
            use_tools: false,
            max_tool_rounds: 3,
            tool_permissions: vec!["message".to_string(), "shell".to_string()],
            permission_mode: Some(RequestedFullJobPermissionMode::CopyOwner),
        };

        let action =
            build_routine_action(db.as_ref(), "default", "issue-1316", "Run it", &execution)
                .await
                .expect("build action");

        assert!(matches!(
            action,
            RoutineAction::FullJob {
                permission_mode: FullJobPermissionMode::Explicit,
                tool_permissions,
                ..
            } if tool_permissions
                == vec![
                    "http".to_string(),
                    "shell".to_string(),
                    "message".to_string(),
                ]
        ));
    }
}
