use ironclaw_events::{EventError, EventReplay, EventStreamKey, ReadScope, RuntimeEvent};
use ironclaw_host_api::{AgentId, AuditEnvelope};
use serde::{Serialize, de::DeserializeOwned};

use crate::durable_error;

#[derive(Debug, Clone)]
pub(super) struct SqlRecordMetadata {
    pub record_id: String,
    pub record_kind: String,
    pub project_id: Option<String>,
    pub mission_id: Option<String>,
    pub thread_id: Option<String>,
    pub process_id: Option<String>,
    pub occurred_at: String,
    pub record_json: serde_json::Value,
}

pub(super) fn runtime_metadata(event: &RuntimeEvent) -> Result<SqlRecordMetadata, EventError> {
    Ok(SqlRecordMetadata {
        record_id: event.event_id.as_uuid().to_string(),
        record_kind: serde_tag(&event.kind)?,
        project_id: event
            .scope
            .project_id
            .as_ref()
            .map(|id| id.as_str().to_string()),
        mission_id: event
            .scope
            .mission_id
            .as_ref()
            .map(|id| id.as_str().to_string()),
        thread_id: event
            .scope
            .thread_id
            .as_ref()
            .map(|id| id.as_str().to_string()),
        process_id: event.process_id.as_ref().map(|id| id.as_uuid().to_string()),
        occurred_at: event.timestamp.to_rfc3339(),
        record_json: serde_json::to_value(event).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?,
    })
}

pub(super) fn audit_metadata(record: &AuditEnvelope) -> Result<SqlRecordMetadata, EventError> {
    Ok(SqlRecordMetadata {
        record_id: record.event_id.as_uuid().to_string(),
        record_kind: serde_tag(&record.stage)?,
        project_id: record.project_id.as_ref().map(|id| id.as_str().to_string()),
        mission_id: record.mission_id.as_ref().map(|id| id.as_str().to_string()),
        thread_id: record.thread_id.as_ref().map(|id| id.as_str().to_string()),
        process_id: record
            .process_id
            .as_ref()
            .map(|id| id.as_uuid().to_string()),
        occurred_at: record.timestamp.to_rfc3339(),
        record_json: serde_json::to_value(record).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?,
    })
}

pub(super) fn agent_db_key(agent_id: Option<&AgentId>) -> &str {
    agent_id.map(AgentId::as_str).unwrap_or("")
}

pub(super) fn decode_record<T>(value: serde_json::Value) -> Result<T, EventError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value).map_err(|error| EventError::Serialize {
        reason: error.to_string(),
    })
}

pub(super) fn validate_replay_request(
    next_cursor: u64,
    earliest_retained: u64,
    after: ironclaw_events::EventCursor,
    limit: usize,
) -> Result<(), EventError> {
    if limit == 0 {
        return Err(EventError::InvalidReplayRequest {
            reason: "limit must be greater than zero".to_string(),
        });
    }
    if after.as_u64() > next_cursor {
        return Err(EventError::ReplayGap {
            requested: after,
            earliest: ironclaw_events::EventCursor::new(next_cursor),
        });
    }
    if earliest_retained > 0 && after.as_u64() < earliest_retained.saturating_sub(1) {
        return Err(EventError::ReplayGap {
            requested: after,
            earliest: ironclaw_events::EventCursor::new(earliest_retained),
        });
    }
    Ok(())
}

pub(super) fn empty_or_foreign_stream<T>(
    after: ironclaw_events::EventCursor,
    limit: usize,
) -> Result<EventReplay<T>, EventError> {
    if limit == 0 {
        return Err(EventError::InvalidReplayRequest {
            reason: "limit must be greater than zero".to_string(),
        });
    }
    if after.as_u64() > 0 {
        return Err(EventError::ReplayGap {
            requested: after,
            earliest: ironclaw_events::EventCursor::origin(),
        });
    }
    Ok(EventReplay {
        entries: Vec::new(),
        next_cursor: after,
    })
}

pub(super) fn stream_from_runtime(event: &RuntimeEvent) -> EventStreamKey {
    EventStreamKey::from_scope(&event.scope)
}

pub(super) fn stream_from_audit(record: &AuditEnvelope) -> EventStreamKey {
    EventStreamKey::new(
        record.tenant_id.clone(),
        record.user_id.clone(),
        record.agent_id.clone(),
    )
}

pub(super) fn filter_runtime(filter: &ReadScope, event: &RuntimeEvent) -> bool {
    filter.matches_event(event)
}

pub(super) fn filter_audit(filter: &ReadScope, record: &AuditEnvelope) -> bool {
    filter.matches_audit(record)
}

fn serde_tag<T>(value: &T) -> Result<String, EventError>
where
    T: Serialize,
{
    match serde_json::to_value(value).map_err(|error| EventError::Serialize {
        reason: error.to_string(),
    })? {
        serde_json::Value::String(tag) => Ok(tag),
        _ => Err(durable_error(
            "event store record kind must serialize to a string",
        )),
    }
}
