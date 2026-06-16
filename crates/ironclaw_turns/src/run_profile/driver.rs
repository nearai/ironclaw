use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{LoopExit, RunProfileVersion, TurnCheckpointId, TurnId, TurnRunId};

use super::{
    host::AgentLoopDriverHost,
    refs::{CheckpointSchemaId, LoopDriverId},
    snapshot::ResolvedRunProfile,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLoopDriverDescriptor {
    pub id: LoopDriverId,
    pub version: RunProfileVersion,
    pub checkpoint_schema_id: Option<CheckpointSchemaId>,
    pub checkpoint_schema_version: Option<RunProfileVersion>,
}

impl AgentLoopDriverDescriptor {
    pub fn new(id: impl Into<String>, version: RunProfileVersion) -> Result<Self, String> {
        Ok(Self {
            id: LoopDriverId::new(id)?,
            version,
            checkpoint_schema_id: None,
            checkpoint_schema_version: None,
        })
    }

    pub fn from_trusted_static(
        id: &'static str,
        version: RunProfileVersion,
    ) -> Result<Self, String> {
        Ok(Self {
            id: LoopDriverId::new(id)?,
            version,
            checkpoint_schema_id: None,
            checkpoint_schema_version: None,
        })
    }

    pub fn with_checkpoint_schema(
        mut self,
        checkpoint_schema_id: impl Into<String>,
        checkpoint_schema_version: RunProfileVersion,
    ) -> Result<Self, String> {
        self.checkpoint_schema_id = Some(CheckpointSchemaId::new(checkpoint_schema_id)?);
        self.checkpoint_schema_version = Some(checkpoint_schema_version);
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLoopDriverRunRequest {
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub resolved_run_profile: ResolvedRunProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLoopDriverResumeRequest {
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub checkpoint_id: TurnCheckpointId,
    pub resolved_run_profile: ResolvedRunProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_resume_disposition: Option<crate::AuthResumeDisposition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentLoopDriverError {
    #[error("agent loop driver rejected request: {reason}")]
    InvalidRequest { reason: String },
    #[error("agent loop driver is unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("agent loop driver failed: {reason_kind}")]
    Failed { reason_kind: String },
}

/// Userland loop implementation contract.
///
/// Implementations own loop mechanics and return a [`LoopExit`] handshake to the
/// trusted runner. They do not mutate turn state directly and do not receive raw
/// authority handles.
#[async_trait]
pub trait AgentLoopDriver: Send + Sync {
    fn descriptor(&self) -> AgentLoopDriverDescriptor;

    async fn run(
        &self,
        request: AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError>;

    async fn resume(
        &self,
        request: AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError>;
}
