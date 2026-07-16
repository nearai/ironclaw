use ironclaw_host_api::{
    AgentId, AuditEnvelope, MissionId, ProcessId, ProjectId, ResourceScope, TenantId, ThreadId,
    UserId,
};
use serde::{Deserialize, Serialize};

use crate::runtime_event::RuntimeEvent;

/// Monotonic replay cursor for a scoped durable log.
///
/// Cursors are not global authority. They must be validated against the
/// requesting consumer's [`EventStreamKey`] before any replay is served. A
/// cursor older than the earliest retained record yields
/// [`EventError::ReplayGap`] so transports can fetch a snapshot/rebase rather
/// than silently lose history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventCursor(u64);

impl EventCursor {
    /// The cursor that precedes every record. `read_after_cursor(.., None, ..)`
    /// is equivalent to `read_after_cursor(.., Some(EventCursor::origin()), ..)`.
    pub const fn origin() -> Self {
        Self(0)
    }

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl Default for EventCursor {
    fn default() -> Self {
        Self::origin()
    }
}

/// Stream partition key.
///
/// Reborn durable event/audit streams partition by (tenant, user, agent).
/// Cursors are monotonic within a stream and must be validated against the
/// requesting consumer's stream key. Deeper scope filtering (project,
/// mission, thread, process, invocation) is applied as a read-side filter on
/// the matching stream rather than as a separate cursor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventStreamKey {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
}

impl EventStreamKey {
    pub fn new(tenant_id: TenantId, user_id: UserId, agent_id: Option<AgentId>) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
        }
    }

    pub fn from_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
        }
    }

    pub fn matches(&self, scope: &ResourceScope) -> bool {
        self.tenant_id == scope.tenant_id
            && self.user_id == scope.user_id
            && self.agent_id == scope.agent_id
    }
}

/// Authorized read filter applied to durable replay.
///
/// `EventStreamKey` partitions cursors by `(tenant, user, agent)` per the
/// durable-log path contract. Within a single stream, multiple
/// projects/missions/threads/processes can co-exist; a project-scoped
/// consumer must still see only its own project's events. `ReadScope`
/// carries the deeper-scope dimensions and is enforced by the durable-log
/// implementation, not by the caller.
///
/// `ReadScope::any()` disables filtering and is intended for tests or
/// admin/aggregate paths that already hold authority for the whole stream.
/// Production callers must construct a tightened filter.
///
/// Filter semantics: a `Some(want)` field in the filter matches only
/// records whose corresponding scope field is `Some(want)`. A record with
/// `None` in that field does **not** match a filter that asks for
/// `Some(...)` — the filter is a tightening, never a permissive default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReadScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission_id: Option<MissionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_id: Option<ProcessId>,
}

impl ReadScope {
    /// Filter that matches every record in the stream. Use only when the
    /// caller already holds authority for the whole stream (tests,
    /// admin/aggregate paths).
    pub fn any() -> Self {
        Self::default()
    }

    pub fn set_project_id(mut self, project_id: impl Into<Option<ProjectId>>) -> Self {
        self.project_id = project_id.into();
        self
    }

    pub fn set_mission_id(mut self, mission_id: impl Into<Option<MissionId>>) -> Self {
        self.mission_id = mission_id.into();
        self
    }

    pub fn set_thread_id(mut self, thread_id: impl Into<Option<ThreadId>>) -> Self {
        self.thread_id = thread_id.into();
        self
    }

    pub fn set_process_id(mut self, process_id: impl Into<Option<ProcessId>>) -> Self {
        self.process_id = process_id.into();
        self
    }

    /// True iff every `Some` field in the filter has a matching value in the
    /// supplied [`ResourceScope`]. `process_id` is checked against the
    /// caller-supplied `process_id` because runtime events carry it on the
    /// record rather than inside the scope.
    pub fn matches_event(&self, event: &RuntimeEvent) -> bool {
        matches_optional(self.project_id.as_ref(), event.scope.project_id.as_ref())
            && matches_optional(self.mission_id.as_ref(), event.scope.mission_id.as_ref())
            && matches_optional(self.thread_id.as_ref(), event.scope.thread_id.as_ref())
            && matches_optional(self.process_id.as_ref(), event.process_id.as_ref())
    }

    /// True iff every `Some` field in the filter matches the corresponding
    /// top-level field on the audit envelope.
    pub fn matches_audit(&self, record: &AuditEnvelope) -> bool {
        matches_optional(self.project_id.as_ref(), record.project_id.as_ref())
            && matches_optional(self.mission_id.as_ref(), record.mission_id.as_ref())
            && matches_optional(self.thread_id.as_ref(), record.thread_id.as_ref())
            && matches_optional(self.process_id.as_ref(), record.process_id.as_ref())
    }
}

fn matches_optional<T: PartialEq>(want: Option<&T>, have: Option<&T>) -> bool {
    match want {
        None => true,
        Some(want) => have == Some(want),
    }
}

/// One replayed record and its durable cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLogEntry<T> {
    pub cursor: EventCursor,
    pub record: T,
}

/// Bounded replay response from a durable event/audit log.
///
/// `next_cursor` is suitable for the next `read_after_cursor` call. When
/// `entries` is empty, `next_cursor` echoes the requested cursor so the
/// consumer can resume cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventReplay<T> {
    pub entries: Vec<EventLogEntry<T>>,
    pub next_cursor: EventCursor,
}
