//! Checkpoint load/stage DTOs, checkpoint kinds, and the [`LoopCheckpointPort`]
//! host boundary.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::run_profile::refs::CheckpointSchemaId;
use crate::{RedactedCheckpointPayload, RunProfileVersion, TurnCheckpointId};

use super::error::{AgentLoopHostError, AgentLoopHostErrorKind, unsupported_host_method};
use super::refs::LoopCheckpointStateRef;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopCheckpointRequest {
    pub kind: LoopCheckpointKind,
    pub state_ref: LoopCheckpointStateRef,
    /// Gate identity for `BeforeBlock` checkpoints; `None` for other kinds.
    /// Defaults to `None` for backward-compatible deserialization of older
    /// records that predate this field.
    #[serde(default)]
    pub gate_ref: Option<crate::ids::LoopGateRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadCheckpointPayloadRequest {
    pub checkpoint_id: TurnCheckpointId,
    pub expected_schema_id: CheckpointSchemaId,
    pub expected_schema_version: RunProfileVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedCheckpointPayload {
    pub kind: LoopCheckpointKind,
    pub schema_id: CheckpointSchemaId,
    pub schema_version: RunProfileVersion,
    pub payload: RedactedCheckpointPayload,
}

/// Request to stage a checkpoint payload's raw bytes before calling
/// [`LoopCheckpointPort::checkpoint`] with the resulting state ref.
///
/// The two-step write keeps byte-storage and metadata-write responsibilities
/// cleanly split.
///
/// `kind` is required so adapters that bridge to
/// `CheckpointStateStorePort::put_checkpoint_state` can persist the correct kind
/// without having to guess. The subsequent `checkpoint(kind, state_ref)` call
/// must use the same `kind`; the read-side `get_checkpoint_state` validates
/// the staged kind against the metadata write's kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageCheckpointPayloadRequest {
    /// Checkpoint boundary the staged payload belongs to. Must match the
    /// `kind` passed to the subsequent `LoopCheckpointPort::checkpoint(...)`
    /// call.
    pub kind: LoopCheckpointKind,
    /// Schema id of the payload — usually the framework's
    /// `CHECKPOINT_SCHEMA_ID` constant. Stored alongside the bytes so the
    /// read-side can authenticate the boundary on resume.
    pub schema_id: CheckpointSchemaId,
    /// Canonical payload bytes (e.g. `serde_json::to_vec(&state)`). The
    /// implementation does not parse the bytes; it persists them and returns
    /// an opaque ref.
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCheckpointKind {
    BeforeModel,
    BeforeSideEffect,
    BeforeBlock,
    Final,
}

impl LoopCheckpointKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BeforeModel => "before_model",
            Self::BeforeSideEffect => "before_side_effect",
            Self::BeforeBlock => "before_block",
            Self::Final => "final",
        }
    }
}

#[async_trait]
pub trait LoopCheckpointPort: Send + Sync {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError>;

    /// Stage a checkpoint payload's raw bytes and return an opaque
    /// [`LoopCheckpointStateRef`] that subsequent `checkpoint(...)` calls
    /// can reference. The default impl fails closed; concrete impls live in
    /// `ironclaw_loop_host` and wrap the host's `CheckpointStateStorePort`.
    ///
    /// The executor's checkpoint helper calls this method before invoking
    /// `LoopCheckpointPort::checkpoint(...)` so the metadata write references
    /// a payload that's already durably stored.
    async fn stage_checkpoint_payload(
        &self,
        _request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "stage_checkpoint_payload not implemented",
        ))
    }

    /// Load the redacted state payload behind a previously-written
    /// checkpoint. Resume callers go through this host port so metadata
    /// validation stays with the backend that owns checkpoint storage.
    async fn load_checkpoint_payload(
        &self,
        _request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        Err(unsupported_host_method("load_checkpoint_payload"))
    }
}
