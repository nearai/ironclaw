use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::error::DatabaseError;
use crate::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceAuditAction, TraceCorpusStatus,
};

pub(crate) const TRACE_AUDIT_EVENT_GENESIS_HASH: &str = "sha256:genesis";

pub(crate) fn enum_to_storage<T: Serialize>(value: T) -> Result<String, DatabaseError> {
    let value = serde_json::to_value(value)
        .map_err(|e| DatabaseError::Serialization(format!("trace enum encode failed: {e}")))?;
    value.as_str().map(str::to_string).ok_or_else(|| {
        DatabaseError::Serialization("trace enum did not serialize to a string".to_string())
    })
}

pub(crate) fn enum_from_storage<T: DeserializeOwned>(
    value: &str,
    type_name: &str,
) -> Result<T, DatabaseError> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).map_err(|e| {
        DatabaseError::Serialization(format!(
            "invalid trace {type_name} storage value {value:?}: {e}"
        ))
    })
}

pub(crate) fn parse_uuid(value: &str, field: &str) -> Result<Uuid, DatabaseError> {
    value.parse::<Uuid>().map_err(|e| {
        DatabaseError::Serialization(format!("invalid trace uuid in {field}: {value:?}: {e}"))
    })
}

pub(crate) fn audit_action_for_status(status: TraceCorpusStatus) -> TraceAuditAction {
    match status {
        TraceCorpusStatus::Accepted
        | TraceCorpusStatus::Quarantined
        | TraceCorpusStatus::Rejected => TraceAuditAction::Review,
        TraceCorpusStatus::Revoked => TraceAuditAction::Revoke,
        TraceCorpusStatus::Purged => TraceAuditAction::Purge,
        TraceCorpusStatus::Expired => TraceAuditAction::Retain,
        TraceCorpusStatus::Received => TraceAuditAction::Submit,
    }
}

pub(crate) fn validate_trace_audit_append_chain(
    tenant_id: &str,
    audit_event_id: Uuid,
    latest_event_hash: Option<&str>,
    previous_event_hash: Option<&str>,
    event_hash_present: bool,
) -> Result<(), DatabaseError> {
    if !event_hash_present {
        return Ok(());
    }

    if let Some(expected_previous) = latest_event_hash {
        let provided_previous = previous_event_hash.unwrap_or(TRACE_AUDIT_EVENT_GENESIS_HASH);
        if provided_previous == expected_previous {
            return Ok(());
        }

        return Err(DatabaseError::Constraint(format!(
            "trace audit append for tenant {tenant_id} event {audit_event_id} has stale previous_event_hash: expected {expected_previous}, got {provided_previous}"
        )));
    }

    Ok(())
}

pub(crate) fn validate_tenant_scoped_trace_object_ref(
    field: &str,
    object_ref: &TenantScopedTraceObjectRef,
    tenant_id: &str,
    submission_id: Uuid,
) -> Result<(), DatabaseError> {
    if object_ref.tenant_id == tenant_id && object_ref.submission_id == submission_id {
        return Ok(());
    }

    Err(DatabaseError::Constraint(format!(
        "trace {field} object_ref_id {} does not belong to tenant {tenant_id} submission {submission_id}",
        object_ref.object_ref_id
    )))
}
