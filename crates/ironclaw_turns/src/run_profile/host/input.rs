//! Loop input stream DTOs (`LoopInput*`) and the [`LoopInputPort`] used to poll
//! and acknowledge queued run inputs.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{LoopGateRef, LoopMessageRef};

use super::context::LoopInputCursor;
use super::error::AgentLoopHostError;
use super::refs::{CapabilitySurfaceVersion, LoopInputAckToken};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInputBatch {
    pub inputs: Vec<LoopInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_acks: Vec<LoopInputAck>,
    pub next_cursor: LoopInputCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInputAck {
    pub cursor: LoopInputCursor,
    pub token: LoopInputAckToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopInput {
    UserMessage { message_ref: LoopMessageRef },
    FollowUp { message_ref: LoopMessageRef },
    Steering { message_ref: LoopMessageRef },
    Interrupt { kind: LoopInterruptKind },
    Cancel { reason_kind: LoopCancelReasonKind },
    GateResolved { gate_ref: LoopGateRef },
    CapabilitySurfaceChanged { version: CapabilitySurfaceVersion },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopInterruptKind {
    UserInterrupt,
    HostShutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopCancelReasonKind {
    UserRequested,
    Superseded,
    Policy,
}

#[async_trait]
pub trait LoopInputPort: Send + Sync {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError>;

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError>;
}
