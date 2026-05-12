use ironclaw_event_projections::ProjectionScope;
use ironclaw_host_api::ThreadId;
use ironclaw_turns::{TurnActor, TurnScope};
use serde::Serialize;

use crate::validation::{validate_delivery_attempt, validate_policy, validate_subscription_record};
use crate::{
    DeliveryFailureKind, OutboundDeliveryAttempt, OutboundDeliveryId, OutboundError,
    OutboundPushCandidate, ProjectionSubscriptionId, ProjectionSubscriptionRecord,
    ThreadNotificationPolicy,
};

#[derive(Serialize)]
struct DeliveryIdentity<'a> {
    delivery_id: OutboundDeliveryId,
    scope: &'a TurnScope,
    candidate: &'a OutboundPushCandidate,
    attempted_at: &'a ironclaw_host_api::Timestamp,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn to_json<T: Serialize>(value: &T) -> Result<String, OutboundError> {
    ironclaw_storage::encode_json(value).map_err(|_| OutboundError::Serialization)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, OutboundError> {
    ironclaw_storage::decode_json(value).map_err(|_| OutboundError::Serialization)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn db_error(error: impl std::fmt::Display) -> OutboundError {
    tracing::debug!(error = %&error, "outbound storage backend error");
    let redacted = ironclaw_storage::redacted_backend_error(error);
    debug_assert_eq!(redacted, ironclaw_storage::StorageError::Backend);
    OutboundError::Backend
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
const ABSENT_SCOPE_ID: &str = ironclaw_storage::ABSENT_SCOPE_COMPONENT;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn scope_agent_db_value(scope: &TurnScope) -> &str {
    scope
        .agent_id
        .as_ref()
        .map(|value| value.as_str())
        .unwrap_or(ABSENT_SCOPE_ID)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn scope_project_db_value(scope: &TurnScope) -> &str {
    scope
        .project_id
        .as_ref()
        .map(|value| value.as_str())
        .unwrap_or(ABSENT_SCOPE_ID)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn projection_agent_db_value(scope: &ProjectionScope) -> &str {
    scope
        .stream
        .agent_id
        .as_ref()
        .map(|value| value.as_str())
        .unwrap_or(ABSENT_SCOPE_ID)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn subscription_identity_payload(
    record: &ProjectionSubscriptionRecord,
) -> Result<String, OutboundError> {
    subscription_identity_payload_from_parts(
        &record.subscription_id,
        &record.actor,
        &record.scope,
        &record.thread_id,
    )
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn subscription_identity_payload_from_parts(
    subscription_id: &ProjectionSubscriptionId,
    actor: &TurnActor,
    scope: &ProjectionScope,
    thread_id: &ThreadId,
) -> Result<String, OutboundError> {
    #[derive(Serialize)]
    struct SubscriptionIdentity<'a> {
        subscription_id: &'a ProjectionSubscriptionId,
        actor: &'a TurnActor,
        scope: &'a ProjectionScope,
        thread_id: &'a ThreadId,
    }

    to_json(&SubscriptionIdentity {
        subscription_id,
        actor,
        scope,
        thread_id,
    })
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn delivery_identity_payload(
    attempt: &OutboundDeliveryAttempt,
) -> Result<String, OutboundError> {
    to_json(&DeliveryIdentity {
        delivery_id: attempt.delivery_id,
        scope: &attempt.scope,
        candidate: &attempt.candidate,
        attempted_at: &attempt.attempted_at,
    })
}

#[cfg(feature = "libsql")]
pub(crate) const LIBSQL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_outbound_notification_policies (
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, thread_id, agent_id, project_id)
);

CREATE TABLE IF NOT EXISTS reborn_outbound_projection_subscriptions (
    subscription_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    cursor_runtime INTEGER,
    identity_payload TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (subscription_id, identity_payload)
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
pub(crate) const POSTGRES_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_outbound_notification_policies (
    tenant_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (tenant_id, thread_id, agent_id, project_id)
);

CREATE TABLE IF NOT EXISTS reborn_outbound_projection_subscriptions (
    subscription_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    cursor_runtime BIGINT,
    identity_payload TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (subscription_id, identity_payload)
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) struct SubscriptionRowColumns<'a> {
    pub(crate) tenant_id: &'a str,
    pub(crate) user_id: &'a str,
    pub(crate) agent_id: &'a str,
    pub(crate) thread_id: &'a str,
    pub(crate) cursor_runtime: Option<i64>,
    pub(crate) identity_payload: &'a str,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn validate_subscription_row(
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
pub(crate) fn validate_policy_row(
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
pub(crate) fn validate_delivery_attempt_row(
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
pub(crate) struct DeliveryRowColumns<'a> {
    pub(crate) tenant_id: &'a str,
    pub(crate) thread_id: &'a str,
    pub(crate) agent_id: &'a str,
    pub(crate) project_id: &'a str,
    pub(crate) target_ref: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) status: &'a str,
    pub(crate) failure_kind: Option<&'a str>,
    pub(crate) identity_payload: &'a str,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn validate_delivery_row(
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
pub(crate) fn failure_kind_key(kind: DeliveryFailureKind) -> &'static str {
    match kind {
        DeliveryFailureKind::AuthorizationRevoked => "authorization_revoked",
        DeliveryFailureKind::TransportUnavailable => "transport_unavailable",
        DeliveryFailureKind::RateLimited => "rate_limited",
        DeliveryFailureKind::Rejected => "rejected",
        DeliveryFailureKind::Unknown => "unknown",
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn require_one_affected(affected: u64) -> Result<(), OutboundError> {
    if affected == 1 {
        Ok(())
    } else {
        Err(OutboundError::Backend)
    }
}
