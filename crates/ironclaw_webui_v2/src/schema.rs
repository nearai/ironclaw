//! Browser-visible WebChat v2 timeline/event schema.
//!
//! This module is the route-owned rendering contract between the WebUI SSE
//! transport and the browser. It deliberately does not expose adapter routing
//! metadata such as installation ids, reply binding refs, external conversation
//! refs, or delivery attempt ids.

use ironclaw_product_workflow::{
    AuthPromptView, FinalReplyView, GatePromptView, ProductOutboundEnvelope,
    ProductOutboundPayload, ProductProjectionState, ProgressKind, ProgressUpdateView,
    ProjectionCursor, RebornCancelRunResponse, RebornGetRunStateResponse, RebornSubmitTurnResponse,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebChatV2EventFrame {
    pub cursor: ProjectionCursor,
    #[serde(flatten)]
    pub event: WebChatV2Event,
}

impl WebChatV2EventFrame {
    pub fn from_outbound(envelope: ProductOutboundEnvelope) -> Self {
        Self::from(envelope)
    }

    pub fn cursor(&self) -> &ProjectionCursor {
        &self.cursor
    }

    pub fn event_name(&self) -> &'static str {
        self.event.event_name()
    }
}

impl From<ProductOutboundEnvelope> for WebChatV2EventFrame {
    fn from(envelope: ProductOutboundEnvelope) -> Self {
        let ProductOutboundEnvelope {
            projection_cursor,
            payload,
            ..
        } = envelope;
        Self {
            cursor: projection_cursor,
            event: WebChatV2Event::from(payload),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebChatV2Event {
    Accepted {
        ack: RebornSubmitTurnResponse,
    },
    Running {
        progress: ProgressUpdateView,
    },
    CapabilityProgress {
        progress: ProgressUpdateView,
    },
    Gate {
        prompt: GatePromptView,
    },
    AuthRequired {
        prompt: AuthPromptView,
    },
    FinalReply {
        reply: FinalReplyView,
    },
    Cancelled {
        response: RebornCancelRunResponse,
    },
    Failed {
        run_state: RebornGetRunStateResponse,
    },
    ProjectionSnapshot {
        state: ProductProjectionState,
    },
    ProjectionUpdate {
        state: ProductProjectionState,
    },
    KeepAlive,
}

impl WebChatV2Event {
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Accepted { .. } => "accepted",
            Self::Running { .. } => "running",
            Self::CapabilityProgress { .. } => "capability_progress",
            Self::Gate { .. } => "gate",
            Self::AuthRequired { .. } => "auth_required",
            Self::FinalReply { .. } => "final_reply",
            Self::Cancelled { .. } => "cancelled",
            Self::Failed { .. } => "failed",
            Self::ProjectionSnapshot { .. } => "projection_snapshot",
            Self::ProjectionUpdate { .. } => "projection_update",
            Self::KeepAlive => "keep_alive",
        }
    }
}

impl From<ProductOutboundPayload> for WebChatV2Event {
    fn from(value: ProductOutboundPayload) -> Self {
        match value {
            ProductOutboundPayload::FinalReply(reply) => Self::FinalReply { reply },
            ProductOutboundPayload::Progress(progress)
                if progress.kind == ProgressKind::ToolRunning =>
            {
                Self::CapabilityProgress { progress }
            }
            ProductOutboundPayload::Progress(progress) => Self::Running { progress },
            ProductOutboundPayload::GatePrompt(prompt) => Self::Gate { prompt },
            ProductOutboundPayload::AuthPrompt(prompt) => Self::AuthRequired { prompt },
            ProductOutboundPayload::ProjectionSnapshot { state } => {
                Self::ProjectionSnapshot { state }
            }
            ProductOutboundPayload::ProjectionUpdate { state } => Self::ProjectionUpdate { state },
        }
    }
}
