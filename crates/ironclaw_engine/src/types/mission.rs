//! Missions — long-running goals that spawn threads over time.
//!
//! A mission represents an ongoing objective that periodically spawns
//! threads to make progress. Missions can run on a schedule (cron),
//! in response to events, or be triggered manually.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::project::ProjectId;
use crate::types::thread::ThreadId;

use super::{OwnerId, default_user_id};

/// Strongly-typed mission identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MissionId(pub Uuid);

impl MissionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MissionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MissionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Lifecycle status of a mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionStatus {
    /// Mission is actively spawning threads on cadence.
    Active,
    /// Mission is paused — no new threads will be spawned.
    Paused,
    /// Mission has achieved its goal.
    Completed,
    /// Mission has been abandoned or failed irrecoverably.
    Failed,
}

/// How a mission triggers new threads.
///
/// The engine defines the trigger *types*. The bridge/host implements the
/// actual trigger infrastructure (cron tickers, webhook endpoints, event
/// matchers). The engine just needs to be told "fire this mission now."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MissionCadence {
    /// Spawn on a cron schedule (e.g., "0 */6 * * *" for every 6 hours).
    Cron {
        expression: String,
        timezone: Option<String>,
    },
    /// Spawn in response to a channel message matching a regex pattern.
    /// `channel`, when set, restricts firing to messages from a specific
    /// channel name (case-insensitive).
    OnEvent {
        event_pattern: String,
        #[serde(default)]
        channel: Option<String>,
    },
    /// Spawn in response to a structured system event (from tools or external).
    /// `filters`, when non-empty, requires every key/value pair to match
    /// against the event payload's top-level fields exactly.
    OnSystemEvent {
        source: String,
        event_type: String,
        #[serde(default)]
        filters: HashMap<String, serde_json::Value>,
    },
    /// Spawn when an external webhook is received at a registered path.
    /// The bridge registers the webhook endpoint and routes payloads here.
    Webhook {
        path: String,
        secret: Option<String>,
    },
    /// Only spawn when manually triggered (via mission_fire tool or API).
    Manual,
}

/// A mission — a long-running goal that spawns threads over time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: MissionId,
    pub project_id: ProjectId,
    /// Tenant isolation: the user who owns this mission.
    #[serde(default = "default_user_id")]
    pub user_id: String,
    pub name: String,
    /// Optional human-readable description (separate from the goal statement).
    /// Routine `description` fields map here.
    #[serde(default)]
    pub description: Option<String>,
    pub goal: String,
    pub status: MissionStatus,
    pub cadence: MissionCadence,

    // ── Evolving strategy ──
    /// What the next thread should focus on (updated after each thread).
    pub current_focus: Option<String>,
    /// What approaches have been tried and what happened.
    pub approach_history: Vec<String>,

    // ── Progress tracking ──
    /// History of threads spawned by this mission.
    pub thread_history: Vec<ThreadId>,
    /// Optional criteria for declaring the mission complete.
    pub success_criteria: Option<String>,

    // ── Notification ──
    /// Channels to notify when a mission thread completes (e.g. "gateway", "repl").
    /// Empty means no proactive notification (results only in approach_history).
    #[serde(default)]
    pub notify_channels: Vec<String>,
    /// Optional per-channel user/recipient target for notifications. Maps from
    /// routine `delivery.user`. When `None`, the channel's last-seen
    /// recipient is used.
    #[serde(default)]
    pub notify_user: Option<String>,

    // ── Context preloading ──
    /// Workspace paths whose contents are loaded into the thread's meta-prompt
    /// when the mission fires (e.g. `["MEMORY.md", "context/profile.json"]`).
    /// Maps from routine `execution.context_paths`.
    #[serde(default)]
    pub context_paths: Vec<String>,

    // ── Budget / guardrails ──
    /// Maximum threads per day (0 = unlimited).
    pub max_threads_per_day: u32,
    /// Threads spawned today (reset daily by the cron ticker).
    pub threads_today: u32,
    /// Cooldown between firings, in seconds. 0 = no cooldown. Maps from
    /// routine `guardrails.cooldown_secs`.
    #[serde(default)]
    pub cooldown_secs: u64,
    /// Maximum number of mission threads that may be running concurrently
    /// (in non-terminal states). 0 = unlimited. Maps from routine
    /// `guardrails.max_concurrent`.
    #[serde(default)]
    pub max_concurrent: u32,
    /// Deduplication window for event-triggered firings, in seconds. 0 = no
    /// dedup. When set, identical event-key payloads within this window are
    /// suppressed. Maps from routine `guardrails.dedup_window`.
    #[serde(default)]
    pub dedup_window_secs: u64,
    /// Timestamp of the most recent successful fire. Used by cooldown
    /// enforcement.
    #[serde(default)]
    pub last_fire_at: Option<DateTime<Utc>>,

    // ── Trigger payload ──
    /// Payload from the most recent trigger (webhook body, event data, etc.).
    /// Injected into the thread's context so the code can access it.
    pub last_trigger_payload: Option<serde_json::Value>,

    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When the next thread should be spawned (for Cron cadence).
    pub next_fire_at: Option<DateTime<Utc>>,
}

impl Mission {
    pub fn new(
        project_id: ProjectId,
        user_id: impl Into<String>,
        name: impl Into<String>,
        goal: impl Into<String>,
        cadence: MissionCadence,
    ) -> Self {
        let now = Utc::now();

        // Event-triggered cadences (OnEvent / OnSystemEvent / Webhook) are
        // *reactive*: a single noisy channel can fire them on every
        // matching message. Cron / Manual cadences are *proactive* and
        // self-paced. Set tighter defaults for the reactive variants so a
        // mission created without explicit guardrails cannot accidentally
        // flood the LLM if its pattern is too loose. The routine alias
        // path overrides these via post-create update when the LLM
        // supplies explicit guardrails / advanced settings.
        let is_reactive = matches!(
            cadence,
            MissionCadence::OnEvent { .. }
                | MissionCadence::OnSystemEvent { .. }
                | MissionCadence::Webhook { .. }
        );
        let (default_max_threads_per_day, default_cooldown_secs, default_max_concurrent) =
            if is_reactive {
                // 5-minute cooldown + 24/day cap + single-instance — same
                // floor v1 routine_create used for event-driven routines.
                (24, 300, 1)
            } else {
                // Existing defaults for cron/manual missions; no cooldown,
                // no concurrency cap, generous daily budget.
                (10, 0, 0)
            };

        Self {
            id: MissionId::new(),
            project_id,
            user_id: user_id.into(),
            name: name.into(),
            description: None,
            goal: goal.into(),
            status: MissionStatus::Active,
            cadence,
            current_focus: None,
            approach_history: Vec::new(),
            thread_history: Vec::new(),
            success_criteria: None,
            notify_channels: Vec::new(),
            notify_user: None,
            context_paths: Vec::new(),
            max_threads_per_day: default_max_threads_per_day,
            threads_today: 0,
            cooldown_secs: default_cooldown_secs,
            max_concurrent: default_max_concurrent,
            dedup_window_secs: 0,
            last_fire_at: None,
            last_trigger_payload: None,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            created_at: now,
            updated_at: now,
            next_fire_at: None,
        }
    }

    pub fn with_success_criteria(mut self, criteria: impl Into<String>) -> Self {
        self.success_criteria = Some(criteria.into());
        self
    }

    pub fn owner_id(&self) -> OwnerId<'_> {
        OwnerId::from_user_id(&self.user_id)
    }

    pub fn is_owned_by(&self, user_id: &str) -> bool {
        self.owner_id().matches_user(user_id)
    }

    /// Record that a thread was spawned for this mission.
    pub fn record_thread(&mut self, thread_id: ThreadId) {
        self.thread_history.push(thread_id);
        self.updated_at = Utc::now();
    }

    /// Whether the mission is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            MissionStatus::Completed | MissionStatus::Failed
        )
    }
}
