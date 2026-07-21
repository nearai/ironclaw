//! Frozen mirror of v1 row types.
//!
//! `ironclaw_legacy` (`src/`) retires under Tier B (see
//! `docs/plans/2026-07-02-reborn-internal-module-refactor.md` §8). These
//! structs reproduce the exact v1 row shapes this crate's converters consume
//! (`src/agent/routine.rs`, `src/history/mod.rs`, `src/workspace/document.rs`,
//! `src/db/mod.rs`) so migration keeps working once the legacy crate is gone —
//! the same freeze-and-port pattern `v2_model.rs` already applies to the
//! deleted engine-v2 types. Read-only: nothing here writes v1 state.
#![allow(dead_code)]

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::error::LegacyError;

// ── routines (src/agent/routine.rs) ─────────────────────────────────────────

/// Mirror of `ironclaw::agent::routine::Routine`.
#[derive(Debug, Clone)]
pub(crate) struct Routine {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) user_id: String,
    pub(crate) enabled: bool,
    pub(crate) trigger: Trigger,
    pub(crate) action: RoutineAction,
    pub(crate) guardrails: RoutineGuardrails,
    pub(crate) notify: NotifyConfig,
    pub(crate) last_run_at: Option<DateTime<Utc>>,
    pub(crate) next_fire_at: Option<DateTime<Utc>>,
    pub(crate) run_count: u64,
    pub(crate) consecutive_failures: u32,
    pub(crate) state: serde_json::Value,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

/// Mirror of `ironclaw::agent::routine::Trigger`.
#[derive(Debug, Clone)]
pub(crate) enum Trigger {
    Cron {
        schedule: String,
        timezone: Option<String>,
    },
    Event {
        channel: Option<String>,
        pattern: String,
    },
    SystemEvent {
        source: String,
        event_type: String,
        filters: HashMap<String, String>,
    },
    Webhook {
        path: Option<String>,
        secret: Option<String>,
    },
    Manual,
}

impl Trigger {
    /// Mirror of `Trigger::from_db` — parses the `trigger_type`/`trigger_config`
    /// columns. A stored timezone that no longer parses as a valid IANA name is
    /// dropped to `None` (same tolerance as the original), not an error.
    pub(crate) fn from_db(
        trigger_type: &str,
        config: serde_json::Value,
    ) -> Result<Self, LegacyError> {
        match trigger_type {
            "cron" => {
                let schedule = config
                    .get("schedule")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| LegacyError::Decode {
                        what: "cron trigger".into(),
                        field: "schedule".into(),
                    })?
                    .to_string();
                let timezone = config
                    .get("timezone")
                    .and_then(|v| v.as_str())
                    .and_then(|tz| tz.parse::<chrono_tz::Tz>().ok().map(|_| tz.to_string()));
                Ok(Trigger::Cron { schedule, timezone })
            }
            "event" => {
                let pattern = config
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| LegacyError::Decode {
                        what: "event trigger".into(),
                        field: "pattern".into(),
                    })?
                    .to_string();
                let channel = config
                    .get("channel")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                Ok(Trigger::Event { channel, pattern })
            }
            "system_event" => {
                let source = config
                    .get("source")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| LegacyError::Decode {
                        what: "system_event trigger".into(),
                        field: "source".into(),
                    })?
                    .to_string();
                let event_type = config
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| LegacyError::Decode {
                        what: "system_event trigger".into(),
                        field: "event_type".into(),
                    })?
                    .to_string();
                let filters = config
                    .get("filters")
                    .and_then(|v| v.as_object())
                    .map(|m| {
                        m.iter()
                            .filter_map(|(k, v)| {
                                json_value_as_filter_string(v).map(|s| (k.clone(), s))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(Trigger::SystemEvent {
                    source,
                    event_type,
                    filters,
                })
            }
            "webhook" => {
                let path = config
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let secret = config
                    .get("secret")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                Ok(Trigger::Webhook { path, secret })
            }
            "manual" => Ok(Trigger::Manual),
            other => Err(LegacyError::Decode {
                what: "routine trigger".into(),
                field: format!("unknown trigger_type '{other}'"),
            }),
        }
    }
}

fn json_value_as_filter_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Mirror of `ironclaw::agent::routine::RoutineAction`.
#[derive(Debug, Clone)]
pub(crate) enum RoutineAction {
    Lightweight {
        prompt: String,
        context_paths: Vec<String>,
        max_tokens: u32,
        use_tools: bool,
        max_tool_rounds: u32,
    },
    FullJob {
        title: String,
        description: String,
        max_iterations: u32,
    },
}

const MAX_TOOL_ROUNDS_LIMIT: u32 = 20;

impl RoutineAction {
    /// Mirror of `RoutineAction::from_db` — parses `action_type`/`action_config`.
    pub(crate) fn from_db(
        action_type: &str,
        config: serde_json::Value,
    ) -> Result<Self, LegacyError> {
        match action_type {
            "lightweight" => {
                let prompt = config
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| LegacyError::Decode {
                        what: "lightweight action".into(),
                        field: "prompt".into(),
                    })?
                    .to_string();
                let context_paths = config
                    .get("context_paths")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let max_tokens = config
                    .get("max_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(4096) as u32;
                let use_tools = config
                    .get("use_tools")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let max_tool_rounds = config
                    .get("max_tool_rounds")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3)
                    .clamp(1, MAX_TOOL_ROUNDS_LIMIT as u64)
                    as u32;
                Ok(RoutineAction::Lightweight {
                    prompt,
                    context_paths,
                    max_tokens,
                    use_tools,
                    max_tool_rounds,
                })
            }
            "full_job" => {
                let title = config
                    .get("title")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| LegacyError::Decode {
                        what: "full_job action".into(),
                        field: "title".into(),
                    })?
                    .to_string();
                let description = config
                    .get("description")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| LegacyError::Decode {
                        what: "full_job action".into(),
                        field: "description".into(),
                    })?
                    .to_string();
                let max_iterations = config
                    .get("max_iterations")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(25) as u32;
                Ok(RoutineAction::FullJob {
                    title,
                    description,
                    max_iterations,
                })
            }
            other => Err(LegacyError::Decode {
                what: "routine action".into(),
                field: format!("unknown action_type '{other}'"),
            }),
        }
    }
}

/// Mirror of `ironclaw::agent::routine::RoutineGuardrails`.
#[derive(Debug, Clone)]
pub(crate) struct RoutineGuardrails {
    pub(crate) cooldown: std::time::Duration,
    pub(crate) max_concurrent: u32,
    pub(crate) dedup_window: Option<std::time::Duration>,
}

/// Mirror of `ironclaw::agent::routine::NotifyConfig`.
#[derive(Debug, Clone)]
pub(crate) struct NotifyConfig {
    pub(crate) channel: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) on_attention: bool,
    pub(crate) on_failure: bool,
    pub(crate) on_success: bool,
}

/// `NotifyConfig.user` column normalizer (`src/db/libsql/mod.rs::normalize_notify_user`):
/// empty/whitespace-only/the literal `"default"` all mean "no explicit notify user".
pub(crate) fn normalize_notify_user(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed == "default" {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

// ── conversations (src/history/mod.rs) ──────────────────────────────────────

/// Mirror of `ironclaw::history::ConversationSummary` (subset returned by
/// `list_conversations_all_channels`).
#[derive(Debug, Clone)]
pub(crate) struct Conversation {
    pub(crate) id: Uuid,
    pub(crate) title: Option<String>,
    pub(crate) channel: String,
    pub(crate) thread_type: Option<String>,
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) last_activity: DateTime<Utc>,
}

/// Mirror of `ironclaw::history::ConversationMessage`.
#[derive(Debug, Clone)]
pub(crate) struct ConversationMessage {
    pub(crate) id: Uuid,
    pub(crate) role: String,
    pub(crate) content: String,
    pub(crate) created_at: DateTime<Utc>,
}

// ── memory documents (src/workspace/document.rs) ────────────────────────────

/// Mirror of `ironclaw::workspace::MemoryDocument`.
#[derive(Debug, Clone)]
pub(crate) struct MemoryDocument {
    pub(crate) id: Uuid,
    pub(crate) user_id: String,
    pub(crate) agent_id: Option<Uuid>,
    pub(crate) path: String,
    pub(crate) content: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) metadata: serde_json::Value,
}

// ── agent jobs (src/history/mod.rs) ─────────────────────────────────────────

/// Mirror of `ironclaw::history::AgentJobRecord` (subset returned by
/// `list_agent_jobs`).
#[derive(Debug, Clone)]
pub(crate) struct AgentJobRecord {
    pub(crate) id: Uuid,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) user_id: String,
    pub(crate) failure_reason: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) started_at: Option<DateTime<Utc>>,
    pub(crate) completed_at: Option<DateTime<Utc>>,
}

// ── identities (src/db/mod.rs) ───────────────────────────────────────────────

/// Mirror of `ironclaw::db::UserIdentityRecord`.
#[derive(Debug, Clone)]
pub(crate) struct UserIdentityRecord {
    pub(crate) id: Uuid,
    pub(crate) user_id: String,
    pub(crate) provider: String,
    pub(crate) provider_user_id: String,
    pub(crate) email: Option<String>,
    pub(crate) email_verified: bool,
    pub(crate) display_name: Option<String>,
    pub(crate) avatar_url: Option<String>,
    pub(crate) raw_profile: serde_json::Value,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}
