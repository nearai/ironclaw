use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::error::DatabaseError;
use crate::trace_corpus_storage::{TraceAuditAction, TraceCorpusStatus};

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
