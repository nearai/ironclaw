use async_trait::async_trait;
use ironclaw_loop_contracts::{
    CheckpointSchemaId, LoopCheckpointKind, LoopCheckpointStateRef, RedactedCheckpointPayload,
};
use serde::{Deserialize, Serialize};

use crate::{
    LoopGateRef, RunProfileVersion, TurnCheckpointId, TurnError, TurnId, TurnRunId, TurnScope,
    TurnTimestamp,
};

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

#[cfg(test)]
mod tests {
    use ironclaw_loop_contracts::MAX_CHECKPOINT_STATE_PAYLOAD_BYTES;

    /// The contracts crate cannot depend on the process kernel, so it states the
    /// checkpoint payload ceiling as a literal. This crate depends on both and
    /// pins them equal: a loop checkpoint is journalled as a process checkpoint,
    /// so a divergence would let a loop stage a payload the journal rejects.
    #[test]
    fn checkpoint_payload_ceiling_matches_process_journal() {
        assert_eq!(
            MAX_CHECKPOINT_STATE_PAYLOAD_BYTES,
            ironclaw_processes::MAX_PROCESS_CHECKPOINT_PAYLOAD_BYTES,
            "loop checkpoint payload ceiling must equal the process journal ceiling"
        );
    }
}
