//! Loop input stream DTOs (`LoopInput*`) and the [`LoopInputPort`] used to poll
//! and acknowledge queued run inputs.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use ironclaw_host_api::turn::{LoopGateRef, LoopMessageRef, TurnRunId};

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
    UserMessage {
        message_ref: LoopMessageRef,
    },
    FollowUp {
        message_ref: LoopMessageRef,
    },
    Steering {
        message_ref: LoopMessageRef,
    },
    /// A background subagent child settled; its framed result is already a
    /// durable row on this thread. Refs only — never child content (D4).
    SubagentSettled {
        child_run_id: TurnRunId,
        message_ref: LoopMessageRef,
    },
    Interrupt {
        kind: LoopInterruptKind,
    },
    Cancel {
        reason_kind: LoopCancelReasonKind,
    },
    GateResolved {
        gate_ref: LoopGateRef,
    },
    CapabilitySurfaceChanged {
        version: CapabilitySurfaceVersion,
    },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subagent_settled_round_trips_snake_case() {
        let input = LoopInput::SubagentSettled {
            child_run_id: TurnRunId::new(),
            message_ref: LoopMessageRef::new("msg:child-result-1").expect("valid message ref"),
        };
        let value = serde_json::to_value(&input).expect("serializes");
        assert!(
            value.get("subagent_settled").is_some(),
            "expected snake_case tag, got {value}"
        );
        assert_eq!(
            serde_json::from_value::<LoopInput>(value).expect("deserializes"),
            input
        );
    }

    /// Historical wire forms must keep deserializing — this enum is persisted
    /// verbatim inside the durable run-queue document
    /// (`durable_input_queue.rs:109`), where a parse failure corrupts the
    /// WHOLE queue, not one entry.
    #[test]
    fn historical_loop_input_forms_still_deserialize() {
        for raw in [
            r#"{"user_message":{"message_ref":"msg:a"}}"#,
            r#"{"follow_up":{"message_ref":"msg:b"}}"#,
            r#"{"steering":{"message_ref":"msg:c"}}"#,
            r#"{"interrupt":{"kind":"user_interrupt"}}"#,
            r#"{"cancel":{"reason_kind":"user_requested"}}"#,
            r#"{"gate_resolved":{"gate_ref":"gate:d"}}"#,
            r#"{"capability_surface_changed":{"version":"v1"}}"#,
        ] {
            serde_json::from_str::<LoopInput>(raw)
                .unwrap_or_else(|error| panic!("{raw} must still parse: {error}"));
        }
    }
}
