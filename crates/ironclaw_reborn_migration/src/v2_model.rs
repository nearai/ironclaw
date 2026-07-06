//! Deserialization mirror of engine-v2 persisted types.
//!
//! Engine v2 was deleted from the tree (it survives only at git tag
//! `old_engine_v2`), but its state persists as JSON documents inside the v1
//! `memory_documents` table under `engine/…` / `.system/engine/…` paths. These
//! structs mirror the exact serde representation of
//! `ironclaw_engine::types::{mission, project, thread, message}` at that tag so
//! the migration can parse those blobs. They are read-only parse targets — only
//! the fields the migration consumes are declared; everything else is ignored
//! via `#[serde(default)]` tolerance, so a schema-drifted blob still parses what
//! it can rather than failing the whole run.
//!
//! Serde note: engine-v2 enums used *default* (externally-tagged, PascalCase)
//! derives, and id newtypes are transparent single-field tuple structs — these
//! mirrors reproduce that exactly.
//!
//! `dead_code` is allowed module-wide: several fields exist only to match the
//! persisted wire shape (read by serde during deserialization, then ignored by
//! the converters). Keeping them documents the on-disk contract even when a
//! given migration path does not consume every field.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

/// Path prefixes under which engine-v2 state was persisted in `memory_documents`.
/// The `.system/engine/…` form is post-#2049; the bare `engine/…` form is the
/// legacy layout. A document whose path starts with either is engine-v2 state.
pub(crate) const ENGINE_PREFIXES: [&str; 2] = ["engine/", ".system/engine/"];

/// Returns true if a `memory_documents.path` holds engine-v2 state.
pub(crate) fn is_engine_path(path: &str) -> bool {
    ENGINE_PREFIXES.iter().any(|p| path.starts_with(p))
}

/// `ironclaw_engine::types::mission::Mission` (subset).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Mission {
    pub id: Uuid,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub user_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub status: MissionStatus,
    #[serde(default)]
    pub cadence: MissionCadence,
    #[serde(default)]
    pub current_focus: Option<String>,
    #[serde(default)]
    pub approach_history: Vec<String>,
    #[serde(default)]
    pub thread_history: Vec<Uuid>,
    #[serde(default)]
    pub success_criteria: Option<String>,
    #[serde(default)]
    pub notify_channels: Vec<String>,
    #[serde(default)]
    pub next_fire_at: Option<DateTime<Utc>>,
    #[serde(default = "epoch_fallback")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
pub(crate) enum MissionStatus {
    #[default]
    Active,
    Paused,
    Completed,
    Failed,
}

/// `ironclaw_engine::types::mission::MissionCadence`. Externally tagged; unit
/// variant `Manual` is a bare string, struct variants are `{"Cron": {...}}`.
#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) enum MissionCadence {
    Cron {
        expression: String,
        #[serde(default)]
        timezone: Option<String>,
    },
    OnEvent {
        event_pattern: String,
        #[serde(default)]
        channel: Option<String>,
    },
    OnSystemEvent {
        source: String,
        event_type: String,
    },
    Webhook {
        path: String,
        #[serde(default)]
        secret: Option<String>,
    },
    #[default]
    Manual,
}

impl MissionCadence {
    /// Short tag used in loss reports for non-cron cadences.
    pub(crate) fn tag(&self) -> &'static str {
        match self {
            Self::Cron { .. } => "cron",
            Self::OnEvent { .. } => "on_event",
            Self::OnSystemEvent { .. } => "on_system_event",
            Self::Webhook { .. } => "webhook",
            Self::Manual => "manual",
        }
    }
}

/// `ironclaw_engine::types::project::Project` (subset).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Project {
    pub id: Uuid,
    #[serde(default)]
    pub user_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub goals: Vec<String>,
    #[serde(default = "epoch_fallback")]
    pub created_at: DateTime<Utc>,
}

/// `ironclaw_engine::types::thread::Thread` (subset) — a mission's execution
/// thread, whose `messages` are the user-visible transcript we migrate.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EngineThread {
    pub id: Uuid,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub state: EngineThreadState,
    #[serde(default)]
    pub messages: Vec<ThreadMessage>,
    #[serde(default = "epoch_fallback")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
pub(crate) enum EngineThreadState {
    Created,
    #[default]
    Running,
    Waiting,
    Suspended,
    Completed,
    Done,
    Failed,
}

/// `ironclaw_engine::types::message::ThreadMessage` (subset).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ThreadMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default = "epoch_fallback")]
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(crate) enum MessageRole {
    System,
    User,
    Assistant,
    ActionResult,
}

/// Epoch fallback timestamp (`1970-01-01T00:00:00Z`) for optional timestamps in
/// drifted blobs. Migration converters prefer the real persisted timestamp; this
/// only fires when a blob omits one entirely, and the epoch value makes such
/// synthesized timestamps obvious in logs/reports rather than masquerading as a
/// real "now".
fn epoch_fallback() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default()
}
