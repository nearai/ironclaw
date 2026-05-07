//! Outbound egress and projection subscription policy storage.
//!
//! This crate stores metadata-only Reborn outbound state: per-thread
//! notification policy, projection subscription cursors, and delivery attempt
//! status. It never owns transport delivery, transcript content, projection
//! payloads, prompts, tool I/O, secrets, host paths, or backend detail strings.

use std::collections::{HashMap, HashSet};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_host_api::{ThreadId, Timestamp};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum OutboundError {
    #[error("outbound state backend unavailable")]
    Backend,
    #[error("outbound state serialization failed")]
    Serialization,
    #[error("outbound state request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error("subscription cursor scope mismatch")]
    SubscriptionScopeMismatch,
    #[error("outbound delivery not found")]
    DeliveryNotFound,
}

macro_rules! bounded_ref {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_bounded_ref($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

bounded_ref!(ProjectionSubscriptionId, "projection_subscription_id");
bounded_ref!(ProjectionUpdateRef, "projection_update_ref");

fn validate_bounded_ref(kind: &'static str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{kind} must not be empty"));
    }
    if value.len() > 256 {
        return Err(format!("{kind} must be at most 256 bytes"));
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("{kind} must not contain control characters"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OutboundDeliveryId(Uuid);

impl OutboundDeliveryId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn parse(value: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(value).map(Self)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for OutboundDeliveryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for OutboundDeliveryId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundPushKind {
    FinalReply,
    Progress,
    GateRequired,
    DeliveryStatus,
}

impl OutboundPushKind {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn as_str(self) -> &'static str {
        match self {
            Self::FinalReply => "final_reply",
            Self::Progress => "progress",
            Self::GateRequired => "gate_required",
            Self::DeliveryStatus => "delivery_status",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadNotificationTarget {
    pub target: ReplyTargetBindingRef,
    pub final_replies: bool,
    pub progress: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadNotificationPolicy {
    pub scope: TurnScope,
    pub targets: Vec<ThreadNotificationTarget>,
}

impl ThreadNotificationPolicy {
    pub fn default_for_scope(scope: TurnScope) -> Self {
        Self {
            scope,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPushTargetRequest {
    pub scope: TurnScope,
    pub turn_run_id: Option<TurnRunId>,
    pub reply_target: ReplyTargetBindingRef,
    pub kind: OutboundPushKind,
    pub projection_ref: ProjectionUpdateRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPushCandidate {
    pub thread_id: ThreadId,
    pub turn_run_id: Option<TurnRunId>,
    pub target: ReplyTargetBindingRef,
    pub kind: OutboundPushKind,
    pub projection_ref: ProjectionUpdateRef,
    pub requires_reply_target_revalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPushPlan {
    pub candidates: Vec<OutboundPushCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSubscriptionRecord {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub thread_id: ThreadId,
    pub cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadSubscriptionCursorRequest {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceSubscriptionCursorRequest {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub thread_id: ThreadId,
    pub cursor: ProjectionCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundDeliveryStatus {
    Pending,
    Delivered,
    Failed,
    DeadLettered,
}

impl OutboundDeliveryStatus {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::DeadLettered => "dead_lettered",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryFailureKind {
    AuthorizationRevoked,
    TransportUnavailable,
    RateLimited,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundDeliveryAttempt {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub candidate: OutboundPushCandidate,
    pub status: OutboundDeliveryStatus,
    pub attempted_at: Timestamp,
    pub failure_kind: Option<DeliveryFailureKind>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
#[derive(Serialize)]
struct DeliveryIdentity<'a> {
    delivery_id: OutboundDeliveryId,
    scope: &'a TurnScope,
    candidate: &'a OutboundPushCandidate,
    attempted_at: &'a Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateDeliveryStatusRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub status: OutboundDeliveryStatus,
    pub updated_at: Timestamp,
    pub failure_kind: Option<DeliveryFailureKind>,
}

#[async_trait]
pub trait OutboundStateStore: Send + Sync {
    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError>;

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError>;

    async fn plan_push_targets(
        &self,
        request: OutboundPushTargetRequest,
    ) -> Result<OutboundPushPlan, OutboundError> {
        let policy = self
            .load_thread_notification_policy(request.scope.clone())
            .await?;
        plan_push_targets_from_policy(request, &policy)
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError>;

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError>;

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError>;

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError>;

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError>;

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError>;
}

fn plan_push_targets_from_policy(
    request: OutboundPushTargetRequest,
    policy: &ThreadNotificationPolicy,
) -> Result<OutboundPushPlan, OutboundError> {
    if policy.scope != request.scope {
        return Err(OutboundError::InvalidRequest {
            reason: "notification policy scope does not match request",
        });
    }

    let mut seen = HashSet::<ReplyTargetBindingRef>::new();
    let mut candidates = Vec::new();
    if request.kind == OutboundPushKind::FinalReply {
        push_candidate(
            &request,
            request.reply_target.clone(),
            &mut seen,
            &mut candidates,
        );
    }

    for target in &policy.targets {
        let allowed = match request.kind {
            OutboundPushKind::FinalReply => target.final_replies,
            OutboundPushKind::Progress
            | OutboundPushKind::GateRequired
            | OutboundPushKind::DeliveryStatus => target.progress,
        };
        if allowed {
            push_candidate(&request, target.target.clone(), &mut seen, &mut candidates);
        }
    }
    Ok(OutboundPushPlan { candidates })
}

fn push_candidate(
    request: &OutboundPushTargetRequest,
    target: ReplyTargetBindingRef,
    seen: &mut HashSet<ReplyTargetBindingRef>,
    candidates: &mut Vec<OutboundPushCandidate>,
) {
    if !seen.insert(target.clone()) {
        return;
    }
    candidates.push(OutboundPushCandidate {
        thread_id: request.scope.thread_id.clone(),
        turn_run_id: request.turn_run_id,
        target,
        kind: request.kind,
        projection_ref: request.projection_ref.clone(),
        requires_reply_target_revalidation: true,
    });
}

#[derive(Default)]
pub struct InMemoryOutboundStateStore {
    state: Mutex<InMemoryOutboundState>,
}

#[derive(Default)]
struct InMemoryOutboundState {
    policies: HashMap<ThreadScopeKey, ThreadNotificationPolicy>,
    subscriptions: HashMap<ProjectionSubscriptionId, ProjectionSubscriptionRecord>,
    deliveries: HashMap<OutboundDeliveryId, OutboundDeliveryAttempt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ThreadScopeKey {
    tenant_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    thread_id: String,
}

impl ThreadScopeKey {
    fn new(scope: &TurnScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.to_string(),
            agent_id: scope.agent_id.as_ref().map(ToString::to_string),
            project_id: scope.project_id.as_ref().map(ToString::to_string),
            thread_id: scope.thread_id.to_string(),
        }
    }
}

#[async_trait]
impl OutboundStateStore for InMemoryOutboundStateStore {
    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        validate_policy(&policy)?;
        let mut state = self.lock_state()?;
        state
            .policies
            .insert(ThreadScopeKey::new(&policy.scope), policy);
        Ok(())
    }

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        let state = self.lock_state()?;
        Ok(state
            .policies
            .get(&ThreadScopeKey::new(&scope))
            .cloned()
            .unwrap_or_else(|| ThreadNotificationPolicy::default_for_scope(scope)))
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        validate_subscription_record(&record)?;
        let mut state = self.lock_state()?;
        if let Some(existing) = state.subscriptions.get(&record.subscription_id) {
            validate_subscription_identity(existing, &record)?;
        }
        state
            .subscriptions
            .insert(record.subscription_id.clone(), record);
        Ok(())
    }

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError> {
        let state = self.lock_state()?;
        let Some(record) = state.subscriptions.get(&request.subscription_id) else {
            return Ok(None);
        };
        validate_subscription_request(record, &request)?;
        Ok(record.cursor.clone())
    }

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        let mut state = self.lock_state()?;
        let Some(record) = state.subscriptions.get_mut(&request.subscription_id) else {
            return Err(OutboundError::SubscriptionScopeMismatch);
        };
        validate_advance_request(record, &request)?;
        record.cursor = Some(request.cursor);
        Ok(())
    }

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        validate_delivery_attempt(&attempt)?;
        let mut state = self.lock_state()?;
        if let Some(existing) = state.deliveries.get(&attempt.delivery_id) {
            validate_delivery_identity(existing, &attempt)?;
            return Ok(());
        }
        state.deliveries.insert(attempt.delivery_id, attempt);
        Ok(())
    }

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        let _updated_at = request.updated_at;
        let mut state = self.lock_state()?;
        let Some(attempt) = state.deliveries.get_mut(&request.delivery_id) else {
            return Err(OutboundError::DeliveryNotFound);
        };
        if attempt.scope != request.scope {
            return Err(OutboundError::SubscriptionScopeMismatch);
        }
        attempt.status = request.status;
        attempt.failure_kind = request.failure_kind;
        Ok(())
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        let state = self.lock_state()?;
        let key = ThreadScopeKey::new(&scope);
        let mut deliveries = state
            .deliveries
            .values()
            .filter(|attempt| ThreadScopeKey::new(&attempt.scope) == key)
            .cloned()
            .collect::<Vec<_>>();
        deliveries.sort_by_key(|attempt| (attempt.attempted_at, attempt.delivery_id.to_string()));
        Ok(deliveries)
    }
}

impl InMemoryOutboundStateStore {
    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, InMemoryOutboundState>, OutboundError> {
        self.state.lock().map_err(|_| OutboundError::Backend)
    }
}

fn validate_policy(policy: &ThreadNotificationPolicy) -> Result<(), OutboundError> {
    let mut seen = HashSet::<ReplyTargetBindingRef>::new();
    for target in &policy.targets {
        if !target.final_replies && !target.progress {
            return Err(OutboundError::InvalidRequest {
                reason: "notification target must enable at least one push kind",
            });
        }
        if !seen.insert(target.target.clone()) {
            return Err(OutboundError::InvalidRequest {
                reason: "duplicate notification target",
            });
        }
    }
    Ok(())
}

fn validate_subscription_record(
    record: &ProjectionSubscriptionRecord,
) -> Result<(), OutboundError> {
    let Some(thread_id) = record.scope.read_scope.thread_id.as_ref() else {
        return Err(OutboundError::InvalidRequest {
            reason: "subscription scope must be thread-scoped",
        });
    };
    if thread_id != &record.thread_id || record.actor.user_id != record.scope.stream.user_id {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    if let Some(cursor) = record.cursor.as_ref()
        && cursor.scope != record.scope
    {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    Ok(())
}

fn validate_subscription_request(
    record: &ProjectionSubscriptionRecord,
    request: &LoadSubscriptionCursorRequest,
) -> Result<(), OutboundError> {
    if record.subscription_id != request.subscription_id
        || record.actor != request.actor
        || record.scope != request.scope
        || record.thread_id != request.thread_id
    {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    Ok(())
}
fn validate_subscription_identity(
    existing: &ProjectionSubscriptionRecord,
    incoming: &ProjectionSubscriptionRecord,
) -> Result<(), OutboundError> {
    if existing.subscription_id != incoming.subscription_id
        || existing.actor != incoming.actor
        || existing.scope != incoming.scope
        || existing.thread_id != incoming.thread_id
    {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    Ok(())
}

fn validate_advance_request(
    record: &ProjectionSubscriptionRecord,
    request: &AdvanceSubscriptionCursorRequest,
) -> Result<(), OutboundError> {
    if record.subscription_id != request.subscription_id
        || record.actor != request.actor
        || record.thread_id != request.thread_id
        || record.scope != request.cursor.scope
    {
        return Err(OutboundError::SubscriptionScopeMismatch);
    }
    Ok(())
}

fn validate_delivery_attempt(attempt: &OutboundDeliveryAttempt) -> Result<(), OutboundError> {
    if attempt.scope.thread_id != attempt.candidate.thread_id {
        return Err(OutboundError::InvalidRequest {
            reason: "delivery candidate thread does not match scope",
        });
    }
    Ok(())
}

fn validate_delivery_identity(
    existing: &OutboundDeliveryAttempt,
    incoming: &OutboundDeliveryAttempt,
) -> Result<(), OutboundError> {
    if existing.delivery_id != incoming.delivery_id
        || existing.scope != incoming.scope
        || existing.candidate != incoming.candidate
        || existing.attempted_at != incoming.attempted_at
    {
        return Err(OutboundError::Backend);
    }
    Ok(())
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn to_json<T: Serialize>(value: &T) -> Result<String, OutboundError> {
    serde_json::to_string(value).map_err(|_| OutboundError::Serialization)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, OutboundError> {
    serde_json::from_str(value).map_err(|_| OutboundError::Serialization)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn db_error(error: impl std::fmt::Display) -> OutboundError {
    let _ = error;
    OutboundError::Backend
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
const ABSENT_SCOPE_ID: &str = "";

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn scope_agent_db_value(scope: &TurnScope) -> &str {
    scope
        .agent_id
        .as_ref()
        .map(|value| value.as_str())
        .unwrap_or(ABSENT_SCOPE_ID)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn scope_project_db_value(scope: &TurnScope) -> &str {
    scope
        .project_id
        .as_ref()
        .map(|value| value.as_str())
        .unwrap_or(ABSENT_SCOPE_ID)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn projection_agent_db_value(scope: &ProjectionScope) -> &str {
    scope
        .stream
        .agent_id
        .as_ref()
        .map(|value| value.as_str())
        .unwrap_or(ABSENT_SCOPE_ID)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn subscription_identity_payload(
    record: &ProjectionSubscriptionRecord,
) -> Result<String, OutboundError> {
    #[derive(Serialize)]
    struct SubscriptionIdentity<'a> {
        subscription_id: &'a ProjectionSubscriptionId,
        actor: &'a TurnActor,
        scope: &'a ProjectionScope,
        thread_id: &'a ThreadId,
    }

    to_json(&SubscriptionIdentity {
        subscription_id: &record.subscription_id,
        actor: &record.actor,
        scope: &record.scope,
        thread_id: &record.thread_id,
    })
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn delivery_identity_payload(attempt: &OutboundDeliveryAttempt) -> Result<String, OutboundError> {
    to_json(&DeliveryIdentity {
        delivery_id: attempt.delivery_id,
        scope: &attempt.scope,
        candidate: &attempt.candidate,
        attempted_at: &attempt.attempted_at,
    })
}

#[cfg(feature = "libsql")]
const LIBSQL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_outbound_notification_policies (
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, thread_id, agent_id, project_id)
);

CREATE TABLE IF NOT EXISTS reborn_outbound_projection_subscriptions (
    subscription_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    cursor_runtime INTEGER,
    identity_payload TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_outbound_projection_subscriptions_thread
    ON reborn_outbound_projection_subscriptions(tenant_id, thread_id, user_id, agent_id);

CREATE TABLE IF NOT EXISTS reborn_outbound_delivery_attempts (
    delivery_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    attempted_at TEXT NOT NULL,
    status_updated_at TEXT,
    failure_kind TEXT,
    identity_payload TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_outbound_delivery_attempts_thread
    ON reborn_outbound_delivery_attempts(tenant_id, thread_id, agent_id, project_id, attempted_at);
"#;

#[cfg(feature = "postgres")]
const POSTGRES_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_outbound_notification_policies (
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, thread_id, agent_id, project_id)
);

CREATE TABLE IF NOT EXISTS reborn_outbound_projection_subscriptions (
    subscription_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    cursor_runtime BIGINT,
    identity_payload TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_outbound_projection_subscriptions_thread
    ON reborn_outbound_projection_subscriptions(tenant_id, thread_id, user_id, agent_id);

CREATE TABLE IF NOT EXISTS reborn_outbound_delivery_attempts (
    delivery_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    attempted_at TEXT NOT NULL,
    status_updated_at TEXT,
    failure_kind TEXT,
    identity_payload TEXT NOT NULL,
    payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reborn_outbound_delivery_attempts_thread
    ON reborn_outbound_delivery_attempts(tenant_id, thread_id, agent_id, project_id, attempted_at);
"#;

#[cfg(feature = "libsql")]
pub struct LibSqlOutboundStateStore {
    db: Arc<::libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlOutboundStateStore {
    pub fn new(db: Arc<::libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), OutboundError> {
        let conn = self.connect().await?;
        conn.execute_batch(LIBSQL_SCHEMA).await.map_err(db_error)?;
        Ok(())
    }

    async fn connect(&self) -> Result<::libsql::Connection, OutboundError> {
        let conn = self.db.connect().map_err(db_error)?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(db_error)?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl OutboundStateStore for LibSqlOutboundStateStore {
    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        validate_policy(&policy)?;
        self.run_migrations().await?;
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO reborn_outbound_notification_policies \
             (tenant_id, thread_id, agent_id, project_id, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(tenant_id, thread_id, agent_id, project_id) DO UPDATE SET \
             payload = excluded.payload",
            ::libsql::params![
                policy.scope.tenant_id.as_str(),
                policy.scope.thread_id.as_str(),
                scope_agent_db_value(&policy.scope),
                scope_project_db_value(&policy.scope),
                to_json(&policy)?,
            ],
        )
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        self.run_migrations().await?;
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT agent_id, project_id, payload FROM reborn_outbound_notification_policies \
                 WHERE tenant_id = ?1 AND thread_id = ?2 AND agent_id = ?3 AND project_id = ?4",
                ::libsql::params![
                    scope.tenant_id.as_str(),
                    scope.thread_id.as_str(),
                    scope_agent_db_value(&scope),
                    scope_project_db_value(&scope),
                ],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = rows.next().await.map_err(db_error)? else {
            return Ok(ThreadNotificationPolicy::default_for_scope(scope));
        };
        let payload: String = row.get(2).map_err(db_error)?;
        let policy = validate_policy_row(from_json::<ThreadNotificationPolicy>(&payload)?, &scope)?;
        Ok(policy)
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        validate_subscription_record(&record)?;
        self.run_migrations().await?;
        let conn = self.connect().await?;
        let identity_payload = subscription_identity_payload(&record)?;
        let affected = conn
            .execute(
                "INSERT INTO reborn_outbound_projection_subscriptions \
                 (subscription_id, tenant_id, user_id, agent_id, thread_id, cursor_runtime, identity_payload, payload) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(subscription_id) DO UPDATE SET \
                 cursor_runtime = excluded.cursor_runtime, payload = excluded.payload \
                 WHERE reborn_outbound_projection_subscriptions.identity_payload = excluded.identity_payload",
                ::libsql::params![
                    record.subscription_id.as_str(),
                    record.scope.stream.tenant_id.as_str(),
                    record.actor.user_id.as_str(),
                    projection_agent_db_value(&record.scope),
                    record.thread_id.as_str(),
                    record.cursor.as_ref().map(|cursor| cursor.runtime.as_u64() as i64),
                    identity_payload,
                    to_json(&record)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError> {
        self.run_migrations().await?;
        let Some(record) = self.load_subscription(&request.subscription_id).await? else {
            return Ok(None);
        };
        validate_subscription_request(&record, &request)?;
        Ok(record.cursor)
    }

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        self.run_migrations().await?;
        let Some(mut record) = self.load_subscription(&request.subscription_id).await? else {
            return Err(OutboundError::SubscriptionScopeMismatch);
        };
        validate_advance_request(&record, &request)?;
        record.cursor = Some(request.cursor);
        let conn = self.connect().await?;
        let identity_payload = subscription_identity_payload(&record)?;
        let affected = conn
            .execute(
                "UPDATE reborn_outbound_projection_subscriptions \
                 SET cursor_runtime = ?3, payload = ?4 WHERE subscription_id = ?1 AND identity_payload = ?2",
                ::libsql::params![
                    record.subscription_id.as_str(),
                    identity_payload,
                    record
                        .cursor
                        .as_ref()
                        .map(|cursor| cursor.runtime.as_u64() as i64),
                    to_json(&record)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        validate_delivery_attempt(&attempt)?;
        self.run_migrations().await?;
        let conn = self.connect().await?;
        let identity_payload = delivery_identity_payload(&attempt)?;
        let affected = conn
            .execute(
                "INSERT INTO reborn_outbound_delivery_attempts \
                 (delivery_id, tenant_id, thread_id, agent_id, project_id, target_ref, kind, status, attempted_at, status_updated_at, failure_kind, identity_payload, payload) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12) \
                 ON CONFLICT(delivery_id) DO UPDATE SET \
                 delivery_id = reborn_outbound_delivery_attempts.delivery_id \
                 WHERE reborn_outbound_delivery_attempts.identity_payload = excluded.identity_payload",
                ::libsql::params![
                    attempt.delivery_id.to_string(),
                    attempt.scope.tenant_id.as_str(),
                    attempt.scope.thread_id.as_str(),
                    scope_agent_db_value(&attempt.scope),
                    scope_project_db_value(&attempt.scope),
                    attempt.candidate.target.as_str(),
                    attempt.candidate.kind.as_str(),
                    attempt.status.as_str(),
                    attempt.attempted_at.to_rfc3339(),
                    attempt.failure_kind.map(failure_kind_key),
                    identity_payload,
                    to_json(&attempt)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        self.run_migrations().await?;
        let Some(mut attempt) = self.load_delivery(request.delivery_id).await? else {
            return Err(OutboundError::DeliveryNotFound);
        };
        if attempt.scope != request.scope {
            return Err(OutboundError::SubscriptionScopeMismatch);
        }
        attempt.status = request.status;
        attempt.failure_kind = request.failure_kind;
        let conn = self.connect().await?;
        let identity_payload = delivery_identity_payload(&attempt)?;
        let affected = conn
            .execute(
                "UPDATE reborn_outbound_delivery_attempts \
                 SET status = ?7, status_updated_at = ?8, failure_kind = ?9, payload = ?10 \
                 WHERE delivery_id = ?1 AND tenant_id = ?2 AND thread_id = ?3 AND agent_id = ?4 AND project_id = ?5 AND identity_payload = ?6",
                ::libsql::params![
                    request.delivery_id.to_string(),
                    request.scope.tenant_id.as_str(),
                    request.scope.thread_id.as_str(),
                    scope_agent_db_value(&request.scope),
                    scope_project_db_value(&request.scope),
                    identity_payload,
                    request.status.as_str(),
                    request.updated_at.to_rfc3339(),
                    request.failure_kind.map(failure_kind_key),
                    to_json(&attempt)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        self.run_migrations().await?;
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT payload FROM reborn_outbound_delivery_attempts \
                 WHERE tenant_id = ?1 AND thread_id = ?2 AND agent_id = ?3 AND project_id = ?4 \
                 ORDER BY attempted_at, delivery_id",
                ::libsql::params![
                    scope.tenant_id.as_str(),
                    scope.thread_id.as_str(),
                    scope_agent_db_value(&scope),
                    scope_project_db_value(&scope),
                ],
            )
            .await
            .map_err(db_error)?;
        let mut deliveries = Vec::new();
        while let Some(row) = rows.next().await.map_err(db_error)? {
            let payload: String = row.get(0).map_err(db_error)?;
            let attempt = validate_delivery_attempt_row(
                from_json::<OutboundDeliveryAttempt>(&payload)?,
                &scope,
            )?;
            deliveries.push(attempt);
        }
        Ok(deliveries)
    }
}

#[cfg(feature = "libsql")]
impl LibSqlOutboundStateStore {
    async fn load_subscription(
        &self,
        subscription_id: &ProjectionSubscriptionId,
    ) -> Result<Option<ProjectionSubscriptionRecord>, OutboundError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT tenant_id, user_id, agent_id, thread_id, cursor_runtime, identity_payload, payload \
                 FROM reborn_outbound_projection_subscriptions WHERE subscription_id = ?1",
                ::libsql::params![subscription_id.as_str()],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = rows.next().await.map_err(db_error)? else {
            return Ok(None);
        };
        let tenant_id: String = row.get(0).map_err(db_error)?;
        let user_id: String = row.get(1).map_err(db_error)?;
        let agent_id: String = row.get(2).map_err(db_error)?;
        let thread_id: String = row.get(3).map_err(db_error)?;
        let cursor_runtime: Option<i64> = row.get(4).map_err(db_error)?;
        let identity_payload: String = row.get(5).map_err(db_error)?;
        let payload: String = row.get(6).map_err(db_error)?;
        let record = validate_subscription_row(
            from_json::<ProjectionSubscriptionRecord>(&payload)?,
            subscription_id,
            SubscriptionRowColumns {
                tenant_id: &tenant_id,
                user_id: &user_id,
                agent_id: &agent_id,
                thread_id: &thread_id,
                cursor_runtime,
                identity_payload: &identity_payload,
            },
        )?;
        Ok(Some(record))
    }

    async fn load_delivery(
        &self,
        delivery_id: OutboundDeliveryId,
    ) -> Result<Option<OutboundDeliveryAttempt>, OutboundError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT tenant_id, thread_id, agent_id, project_id, target_ref, kind, status, failure_kind, identity_payload, payload \
                 FROM reborn_outbound_delivery_attempts WHERE delivery_id = ?1",
                ::libsql::params![delivery_id.to_string()],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = rows.next().await.map_err(db_error)? else {
            return Ok(None);
        };
        let tenant_id: String = row.get(0).map_err(db_error)?;
        let thread_id: String = row.get(1).map_err(db_error)?;
        let agent_id: String = row.get(2).map_err(db_error)?;
        let project_id: String = row.get(3).map_err(db_error)?;
        let target_ref: String = row.get(4).map_err(db_error)?;
        let kind: String = row.get(5).map_err(db_error)?;
        let status: String = row.get(6).map_err(db_error)?;
        let failure_kind: Option<String> = row.get(7).map_err(db_error)?;
        let identity_payload: String = row.get(8).map_err(db_error)?;
        let payload: String = row.get(9).map_err(db_error)?;
        let attempt = validate_delivery_row(
            from_json::<OutboundDeliveryAttempt>(&payload)?,
            delivery_id,
            DeliveryRowColumns {
                tenant_id: &tenant_id,
                thread_id: &thread_id,
                agent_id: &agent_id,
                project_id: &project_id,
                target_ref: &target_ref,
                kind: &kind,
                status: &status,
                failure_kind: failure_kind.as_deref(),
                identity_payload: &identity_payload,
            },
        )?;
        Ok(Some(attempt))
    }
}

#[cfg(feature = "postgres")]
pub struct PostgresOutboundStateStore {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
impl PostgresOutboundStateStore {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), OutboundError> {
        let client = self.pool.get().await.map_err(db_error)?;
        client
            .batch_execute(POSTGRES_SCHEMA)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, OutboundError> {
        self.pool.get().await.map_err(db_error)
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl OutboundStateStore for PostgresOutboundStateStore {
    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        validate_policy(&policy)?;
        self.run_migrations().await?;
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO reborn_outbound_notification_policies \
                 (tenant_id, thread_id, agent_id, project_id, payload) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT(tenant_id, thread_id, agent_id, project_id) DO UPDATE SET \
                 payload = excluded.payload",
                &[
                    &policy.scope.tenant_id.as_str(),
                    &policy.scope.thread_id.as_str(),
                    &scope_agent_db_value(&policy.scope),
                    &scope_project_db_value(&policy.scope),
                    &to_json(&policy)?,
                ],
            )
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        self.run_migrations().await?;
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT agent_id, project_id, payload FROM reborn_outbound_notification_policies \
                 WHERE tenant_id = $1 AND thread_id = $2 AND agent_id = $3 AND project_id = $4",
                &[
                    &scope.tenant_id.as_str(),
                    &scope.thread_id.as_str(),
                    &scope_agent_db_value(&scope),
                    &scope_project_db_value(&scope),
                ],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = row else {
            return Ok(ThreadNotificationPolicy::default_for_scope(scope));
        };
        let payload: String = row.get(2);
        validate_policy_row(from_json::<ThreadNotificationPolicy>(&payload)?, &scope)
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        validate_subscription_record(&record)?;
        self.run_migrations().await?;
        let client = self.client().await?;
        let cursor_runtime = record
            .cursor
            .as_ref()
            .map(|cursor| cursor.runtime.as_u64() as i64);
        let identity_payload = subscription_identity_payload(&record)?;
        let affected = client
            .execute(
                "INSERT INTO reborn_outbound_projection_subscriptions \
                 (subscription_id, tenant_id, user_id, agent_id, thread_id, cursor_runtime, identity_payload, payload) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT(subscription_id) DO UPDATE SET \
                 cursor_runtime = excluded.cursor_runtime, payload = excluded.payload \
                 WHERE reborn_outbound_projection_subscriptions.identity_payload = excluded.identity_payload",
                &[
                    &record.subscription_id.as_str(),
                    &record.scope.stream.tenant_id.as_str(),
                    &record.actor.user_id.as_str(),
                    &projection_agent_db_value(&record.scope),
                    &record.thread_id.as_str(),
                    &cursor_runtime,
                    &identity_payload,
                    &to_json(&record)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError> {
        self.run_migrations().await?;
        let Some(record) = self.load_subscription(&request.subscription_id).await? else {
            return Ok(None);
        };
        validate_subscription_request(&record, &request)?;
        Ok(record.cursor)
    }

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        self.run_migrations().await?;
        let Some(mut record) = self.load_subscription(&request.subscription_id).await? else {
            return Err(OutboundError::SubscriptionScopeMismatch);
        };
        validate_advance_request(&record, &request)?;
        record.cursor = Some(request.cursor);
        let client = self.client().await?;
        let cursor_runtime = record
            .cursor
            .as_ref()
            .map(|cursor| cursor.runtime.as_u64() as i64);
        let identity_payload = subscription_identity_payload(&record)?;
        let affected = client
            .execute(
                "UPDATE reborn_outbound_projection_subscriptions \
                 SET cursor_runtime = $3, payload = $4 WHERE subscription_id = $1 AND identity_payload = $2",
                &[
                    &record.subscription_id.as_str(),
                    &identity_payload,
                    &cursor_runtime,
                    &to_json(&record)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        validate_delivery_attempt(&attempt)?;
        self.run_migrations().await?;
        let client = self.client().await?;
        let identity_payload = delivery_identity_payload(&attempt)?;
        let affected = client
            .execute(
                "INSERT INTO reborn_outbound_delivery_attempts \
                 (delivery_id, tenant_id, thread_id, agent_id, project_id, target_ref, kind, status, attempted_at, status_updated_at, failure_kind, identity_payload, payload) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, $10, $11, $12) \
                 ON CONFLICT(delivery_id) DO UPDATE SET \
                 delivery_id = reborn_outbound_delivery_attempts.delivery_id \
                 WHERE reborn_outbound_delivery_attempts.identity_payload = excluded.identity_payload",
                &[
                    &attempt.delivery_id.to_string(),
                    &attempt.scope.tenant_id.as_str(),
                    &attempt.scope.thread_id.as_str(),
                    &scope_agent_db_value(&attempt.scope),
                    &scope_project_db_value(&attempt.scope),
                    &attempt.candidate.target.as_str(),
                    &attempt.candidate.kind.as_str(),
                    &attempt.status.as_str(),
                    &attempt.attempted_at.to_rfc3339(),
                    &attempt.failure_kind.map(failure_kind_key),
                    &identity_payload,
                    &to_json(&attempt)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        self.run_migrations().await?;
        let Some(mut attempt) = self.load_delivery(request.delivery_id).await? else {
            return Err(OutboundError::DeliveryNotFound);
        };
        if attempt.scope != request.scope {
            return Err(OutboundError::SubscriptionScopeMismatch);
        }
        attempt.status = request.status;
        attempt.failure_kind = request.failure_kind;
        let client = self.client().await?;
        let identity_payload = delivery_identity_payload(&attempt)?;
        let affected = client
            .execute(
                "UPDATE reborn_outbound_delivery_attempts \
                 SET status = $7, status_updated_at = $8, failure_kind = $9, payload = $10 \
                 WHERE delivery_id = $1 AND tenant_id = $2 AND thread_id = $3 AND agent_id = $4 AND project_id = $5 AND identity_payload = $6",
                &[
                    &request.delivery_id.to_string(),
                    &request.scope.tenant_id.as_str(),
                    &request.scope.thread_id.as_str(),
                    &scope_agent_db_value(&request.scope),
                    &scope_project_db_value(&request.scope),
                    &identity_payload,
                    &request.status.as_str(),
                    &request.updated_at.to_rfc3339(),
                    &request.failure_kind.map(failure_kind_key),
                    &to_json(&attempt)?,
                ],
            )
            .await
            .map_err(db_error)?;
        require_one_affected(affected)?;
        Ok(())
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        self.run_migrations().await?;
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT payload FROM reborn_outbound_delivery_attempts \
                 WHERE tenant_id = $1 AND thread_id = $2 AND agent_id = $3 AND project_id = $4 \
                 ORDER BY attempted_at, delivery_id",
                &[
                    &scope.tenant_id.as_str(),
                    &scope.thread_id.as_str(),
                    &scope_agent_db_value(&scope),
                    &scope_project_db_value(&scope),
                ],
            )
            .await
            .map_err(db_error)?;
        let mut deliveries = Vec::new();
        for row in rows {
            let payload: String = row.get(0);
            let attempt = validate_delivery_attempt_row(
                from_json::<OutboundDeliveryAttempt>(&payload)?,
                &scope,
            )?;
            deliveries.push(attempt);
        }
        Ok(deliveries)
    }
}

#[cfg(feature = "postgres")]
impl PostgresOutboundStateStore {
    async fn load_subscription(
        &self,
        subscription_id: &ProjectionSubscriptionId,
    ) -> Result<Option<ProjectionSubscriptionRecord>, OutboundError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT tenant_id, user_id, agent_id, thread_id, cursor_runtime, identity_payload, payload \
                 FROM reborn_outbound_projection_subscriptions WHERE subscription_id = $1",
                &[&subscription_id.as_str()],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let tenant_id: String = row.get(0);
        let user_id: String = row.get(1);
        let agent_id: String = row.get(2);
        let thread_id: String = row.get(3);
        let cursor_runtime: Option<i64> = row.get(4);
        let identity_payload: String = row.get(5);
        let payload: String = row.get(6);
        let record = validate_subscription_row(
            from_json::<ProjectionSubscriptionRecord>(&payload)?,
            subscription_id,
            SubscriptionRowColumns {
                tenant_id: &tenant_id,
                user_id: &user_id,
                agent_id: &agent_id,
                thread_id: &thread_id,
                cursor_runtime,
                identity_payload: &identity_payload,
            },
        )?;
        Ok(Some(record))
    }

    async fn load_delivery(
        &self,
        delivery_id: OutboundDeliveryId,
    ) -> Result<Option<OutboundDeliveryAttempt>, OutboundError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT tenant_id, thread_id, agent_id, project_id, target_ref, kind, status, failure_kind, identity_payload, payload \
                 FROM reborn_outbound_delivery_attempts WHERE delivery_id = $1",
                &[&delivery_id.to_string()],
            )
            .await
            .map_err(db_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let tenant_id: String = row.get(0);
        let thread_id: String = row.get(1);
        let agent_id: String = row.get(2);
        let project_id: String = row.get(3);
        let target_ref: String = row.get(4);
        let kind: String = row.get(5);
        let status: String = row.get(6);
        let failure_kind: Option<String> = row.get(7);
        let identity_payload: String = row.get(8);
        let payload: String = row.get(9);
        let attempt = validate_delivery_row(
            from_json::<OutboundDeliveryAttempt>(&payload)?,
            delivery_id,
            DeliveryRowColumns {
                tenant_id: &tenant_id,
                thread_id: &thread_id,
                agent_id: &agent_id,
                project_id: &project_id,
                target_ref: &target_ref,
                kind: &kind,
                status: &status,
                failure_kind: failure_kind.as_deref(),
                identity_payload: &identity_payload,
            },
        )?;
        Ok(Some(attempt))
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct SubscriptionRowColumns<'a> {
    tenant_id: &'a str,
    user_id: &'a str,
    agent_id: &'a str,
    thread_id: &'a str,
    cursor_runtime: Option<i64>,
    identity_payload: &'a str,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn validate_subscription_row(
    record: ProjectionSubscriptionRecord,
    subscription_id: &ProjectionSubscriptionId,
    row: SubscriptionRowColumns<'_>,
) -> Result<ProjectionSubscriptionRecord, OutboundError> {
    validate_subscription_record(&record)?;
    if record.subscription_id != *subscription_id
        || record.scope.stream.tenant_id.as_str() != row.tenant_id
        || record.actor.user_id.as_str() != row.user_id
        || projection_agent_db_value(&record.scope) != row.agent_id
        || record.thread_id.as_str() != row.thread_id
    {
        return Err(OutboundError::Backend);
    }
    let payload_cursor = record
        .cursor
        .as_ref()
        .map(|cursor| cursor.runtime.as_u64() as i64);
    if payload_cursor != row.cursor_runtime
        || subscription_identity_payload(&record)? != row.identity_payload
    {
        return Err(OutboundError::Backend);
    }
    Ok(record)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn validate_policy_row(
    policy: ThreadNotificationPolicy,
    requested_scope: &TurnScope,
) -> Result<ThreadNotificationPolicy, OutboundError> {
    validate_policy(&policy)?;
    if &policy.scope != requested_scope {
        return Err(OutboundError::Backend);
    }
    Ok(policy)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn validate_delivery_attempt_row(
    attempt: OutboundDeliveryAttempt,
    requested_scope: &TurnScope,
) -> Result<OutboundDeliveryAttempt, OutboundError> {
    validate_delivery_attempt(&attempt)?;
    if &attempt.scope != requested_scope {
        return Err(OutboundError::Backend);
    }
    Ok(attempt)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct DeliveryRowColumns<'a> {
    tenant_id: &'a str,
    thread_id: &'a str,
    agent_id: &'a str,
    project_id: &'a str,
    target_ref: &'a str,
    kind: &'a str,
    status: &'a str,
    failure_kind: Option<&'a str>,
    identity_payload: &'a str,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn validate_delivery_row(
    attempt: OutboundDeliveryAttempt,
    delivery_id: OutboundDeliveryId,
    row: DeliveryRowColumns<'_>,
) -> Result<OutboundDeliveryAttempt, OutboundError> {
    validate_delivery_attempt(&attempt)?;
    if attempt.delivery_id != delivery_id
        || attempt.scope.tenant_id.as_str() != row.tenant_id
        || attempt.scope.thread_id.as_str() != row.thread_id
        || scope_agent_db_value(&attempt.scope) != row.agent_id
        || scope_project_db_value(&attempt.scope) != row.project_id
        || attempt.candidate.target.as_str() != row.target_ref
        || attempt.candidate.kind.as_str() != row.kind
        || attempt.status.as_str() != row.status
        || attempt.failure_kind.map(failure_kind_key) != row.failure_kind
        || delivery_identity_payload(&attempt)? != row.identity_payload
    {
        return Err(OutboundError::Backend);
    }
    Ok(attempt)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn failure_kind_key(kind: DeliveryFailureKind) -> &'static str {
    match kind {
        DeliveryFailureKind::AuthorizationRevoked => "authorization_revoked",
        DeliveryFailureKind::TransportUnavailable => "transport_unavailable",
        DeliveryFailureKind::RateLimited => "rate_limited",
        DeliveryFailureKind::Rejected => "rejected",
        DeliveryFailureKind::Unknown => "unknown",
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn require_one_affected(affected: u64) -> Result<(), OutboundError> {
    if affected == 1 {
        Ok(())
    } else {
        Err(OutboundError::Backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_refs_reject_control_characters() {
        assert!(ProjectionSubscriptionId::new("sub\n1").is_err());
        assert!(ProjectionUpdateRef::new("update\0").is_err());
        assert!(serde_json::from_str::<ProjectionSubscriptionId>("\"sub\\n1\"").is_err());
        assert!(serde_json::from_str::<ProjectionUpdateRef>("\"\"").is_err());
    }
}
