use std::fmt;

use crate::{
    CheckpointSchemaId, LoopCheckpointKind, LoopCheckpointStateRef, LoopGateRef, RunProfileVersion,
    TurnCheckpointId, TurnError, TurnId, TurnRunId, TurnScope, TurnTimestamp,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub const MAX_CHECKPOINT_STATE_PAYLOAD_BYTES: usize =
    ironclaw_processes::MAX_PROCESS_CHECKPOINT_PAYLOAD_BYTES;

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

    pub fn into_payload_bytes(self) -> Vec<u8> {
        self.bytes
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopCheckpointRecord {
    pub checkpoint_id: TurnCheckpointId,
    pub scope: TurnScope,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub state_ref: LoopCheckpointStateRef,
    /// Host-private payload projected from the process journal. It is never
    /// serialized into turn records, events, or transport DTOs.
    #[serde(skip)]
    pub payload: Option<RedactedCheckpointPayload>,
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
    pub payload: RedactedCheckpointPayload,
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

fn validate_checkpoint_payload_len(len: usize) -> Result<(), String> {
    if len > MAX_CHECKPOINT_STATE_PAYLOAD_BYTES {
        return Err(format!(
            "checkpoint payload must be at most {MAX_CHECKPOINT_STATE_PAYLOAD_BYTES} bytes"
        ));
    }
    Ok(())
}
