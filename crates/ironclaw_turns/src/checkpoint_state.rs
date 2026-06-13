use std::{collections::HashMap, fmt, sync::Mutex};

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    CheckpointSchemaId, LoopCheckpointKind, LoopCheckpointStateRef, LoopGateRef, RunProfileVersion,
    TurnCheckpointId, TurnError, TurnId, TurnRunId, TurnScope, TurnTimestamp,
};

pub const MAX_CHECKPOINT_STATE_PAYLOAD_BYTES: usize = 64 * 1024;

/// Internal loop checkpoint payload bytes.
///
/// This value is intentionally not serializable. It is host-owned resume state,
/// not public turn status, event, milestone, or transcript content.
#[derive(Clone, PartialEq, Eq)]
pub struct RedactedCheckpointPayload {
    bytes: Vec<u8>,
}

impl RedactedCheckpointPayload {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, String> {
        let bytes = bytes.into();
        validate_checkpoint_payload_len(bytes.len())?;
        Ok(Self { bytes })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl fmt::Debug for RedactedCheckpointPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedactedCheckpointPayload")
            .field("len", &self.bytes.len())
            .field("payload", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointStateRecord {
    pub state_ref: LoopCheckpointStateRef,
    pub scope: TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub schema_id: CheckpointSchemaId,
    pub schema_version: RunProfileVersion,
    pub kind: LoopCheckpointKind,
    pub payload: RedactedCheckpointPayload,
    pub created_at: TurnTimestamp,
}

#[derive(Clone, Copy)]
pub struct CheckpointStateMatchMetadata<'a> {
    pub state_ref: &'a LoopCheckpointStateRef,
    pub scope: &'a TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub schema_id: &'a CheckpointSchemaId,
    pub schema_version: RunProfileVersion,
    pub kind: LoopCheckpointKind,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PutCheckpointStateRequest {
    pub scope: TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub schema_id: CheckpointSchemaId,
    pub schema_version: RunProfileVersion,
    pub kind: LoopCheckpointKind,
    payload: Vec<u8>,
}

impl PutCheckpointStateRequest {
    pub fn new(
        scope: TurnScope,
        turn_id: TurnId,
        run_id: TurnRunId,
        schema_id: CheckpointSchemaId,
        schema_version: RunProfileVersion,
        kind: LoopCheckpointKind,
        payload: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            scope,
            turn_id,
            run_id,
            schema_id,
            schema_version,
            kind,
            payload: payload.into(),
        }
    }

    pub fn payload_len(&self) -> usize {
        self.payload.len()
    }

    pub fn payload_bytes(&self) -> &[u8] {
        &self.payload
    }

    pub fn into_payload_bytes(self) -> Vec<u8> {
        self.payload
    }
}

impl fmt::Debug for PutCheckpointStateRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PutCheckpointStateRequest")
            .field("scope", &self.scope)
            .field("turn_id", &self.turn_id)
            .field("run_id", &self.run_id)
            .field("schema_id", &self.schema_id)
            .field("schema_version", &self.schema_version)
            .field("kind", &self.kind)
            .field("payload_len", &self.payload.len())
            .field("payload", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetCheckpointStateRequest {
    pub scope: TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub state_ref: LoopCheckpointStateRef,
    pub schema_id: CheckpointSchemaId,
    pub schema_version: RunProfileVersion,
    pub kind: LoopCheckpointKind,
}

#[async_trait]
pub trait CheckpointStateStore: Send + Sync {
    async fn put_checkpoint_state(
        &self,
        request: PutCheckpointStateRequest,
    ) -> Result<CheckpointStateRecord, TurnError>;

    async fn get_checkpoint_state(
        &self,
        request: GetCheckpointStateRequest,
    ) -> Result<Option<CheckpointStateRecord>, TurnError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopCheckpointRecord {
    pub checkpoint_id: TurnCheckpointId,
    pub scope: TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub state_ref: LoopCheckpointStateRef,
    pub schema_id: CheckpointSchemaId,
    pub schema_version: RunProfileVersion,
    pub kind: LoopCheckpointKind,
    /// Gate that triggered this checkpoint. `None` for checkpoint kinds other
    /// than `BeforeBlock` and for legacy records persisted before this field
    /// was added.
    #[serde(default)]
    pub gate_ref: Option<LoopGateRef>,
    pub created_at: TurnTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutLoopCheckpointRequest {
    pub scope: TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub state_ref: LoopCheckpointStateRef,
    pub schema_id: CheckpointSchemaId,
    pub schema_version: RunProfileVersion,
    pub kind: LoopCheckpointKind,
    /// Gate identity for `BeforeBlock` checkpoints; `None` for other kinds.
    pub gate_ref: Option<LoopGateRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetLoopCheckpointRequest {
    pub scope: TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub checkpoint_id: TurnCheckpointId,
}

#[async_trait]
pub trait LoopCheckpointStore: Send + Sync {
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError>;

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError>;
}

#[derive(Default)]
pub struct InMemoryCheckpointStateStore {
    records: Mutex<HashMap<LoopCheckpointStateRef, CheckpointStateRecord>>,
}

#[derive(Default)]
pub struct InMemoryLoopCheckpointStore {
    records: Mutex<HashMap<TurnCheckpointId, LoopCheckpointRecord>>,
}

#[async_trait]
impl CheckpointStateStore for InMemoryCheckpointStateStore {
    async fn put_checkpoint_state(
        &self,
        request: PutCheckpointStateRequest,
    ) -> Result<CheckpointStateRecord, TurnError> {
        validate_checkpoint_payload_len(request.payload.len())
            .map_err(|reason| TurnError::InvalidRequest { reason })?;
        let state_ref = new_checkpoint_state_ref()?;
        let payload = RedactedCheckpointPayload::new(request.payload)
            .map_err(|reason| TurnError::InvalidRequest { reason })?;
        let record = CheckpointStateRecord {
            state_ref: state_ref.clone(),
            scope: request.scope,
            turn_id: request.turn_id,
            run_id: request.run_id,
            schema_id: request.schema_id,
            schema_version: request.schema_version,
            kind: request.kind,
            payload,
            created_at: Utc::now(),
        };

        let mut records = self.records.lock().map_err(|_| TurnError::Unavailable {
            reason: "checkpoint state store lock poisoned".to_string(),
        })?;
        records.insert(state_ref, record.clone());
        Ok(record)
    }

    async fn get_checkpoint_state(
        &self,
        request: GetCheckpointStateRequest,
    ) -> Result<Option<CheckpointStateRecord>, TurnError> {
        let records = self.records.lock().map_err(|_| TurnError::Unavailable {
            reason: "checkpoint state store lock poisoned".to_string(),
        })?;
        let Some(record) = records.get(&request.state_ref) else {
            return Ok(None);
        };
        if checkpoint_state_record_matches_request(record, &request) {
            Ok(Some(record.clone()))
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl LoopCheckpointStore for InMemoryLoopCheckpointStore {
    async fn put_loop_checkpoint(
        &self,
        request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        let checkpoint_id = TurnCheckpointId::new();
        let record = LoopCheckpointRecord {
            checkpoint_id,
            scope: request.scope,
            turn_id: request.turn_id,
            run_id: request.run_id,
            state_ref: request.state_ref,
            schema_id: request.schema_id,
            schema_version: request.schema_version,
            kind: request.kind,
            gate_ref: request.gate_ref,
            created_at: Utc::now(),
        };
        let mut records = self.records.lock().map_err(|_| TurnError::Unavailable {
            reason: "loop checkpoint store lock poisoned".to_string(),
        })?;
        records.insert(checkpoint_id, record.clone());
        Ok(record)
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        let records = self.records.lock().map_err(|_| TurnError::Unavailable {
            reason: "loop checkpoint store lock poisoned".to_string(),
        })?;
        let Some(record) = records.get(&request.checkpoint_id) else {
            return Ok(None);
        };
        if loop_checkpoint_record_matches_request(record, &request) {
            Ok(Some(record.clone()))
        } else {
            Ok(None)
        }
    }
}

pub fn checkpoint_state_record_matches_request(
    record: &CheckpointStateRecord,
    request: &GetCheckpointStateRequest,
) -> bool {
    checkpoint_state_metadata_matches_request(
        CheckpointStateMatchMetadata {
            state_ref: &record.state_ref,
            scope: &record.scope,
            turn_id: record.turn_id,
            run_id: record.run_id,
            schema_id: &record.schema_id,
            schema_version: record.schema_version,
            kind: record.kind,
        },
        request,
    )
}

pub fn checkpoint_state_metadata_matches_request(
    metadata: CheckpointStateMatchMetadata<'_>,
    request: &GetCheckpointStateRequest,
) -> bool {
    metadata.state_ref == &request.state_ref
        && metadata.scope == &request.scope
        && metadata.turn_id == request.turn_id
        && metadata.run_id == request.run_id
        && metadata.schema_id == &request.schema_id
        && metadata.schema_version == request.schema_version
        && metadata.kind == request.kind
}

fn loop_checkpoint_record_matches_request(
    record: &LoopCheckpointRecord,
    request: &GetLoopCheckpointRequest,
) -> bool {
    record.scope == request.scope
        && record.turn_id == request.turn_id
        && record.run_id == request.run_id
        && record.checkpoint_id == request.checkpoint_id
}

pub fn new_checkpoint_state_ref() -> Result<LoopCheckpointStateRef, TurnError> {
    LoopCheckpointStateRef::new(format!("checkpoint:{}", Uuid::new_v4())).map_err(|reason| {
        TurnError::Unavailable {
            reason: format!("generated checkpoint state ref was invalid: {reason}"),
        }
    })
}

fn validate_checkpoint_payload_len(len: usize) -> Result<(), String> {
    if len > MAX_CHECKPOINT_STATE_PAYLOAD_BYTES {
        return Err(format!(
            "checkpoint payload must be at most {MAX_CHECKPOINT_STATE_PAYLOAD_BYTES} bytes"
        ));
    }
    Ok(())
}
