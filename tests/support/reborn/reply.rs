//! `RebornScriptedReply` — the terse façade for scripting one model turn in a
//! Reborn integration test. Each reply maps 1:1 to a `TraceStep`, auto-filling
//! id/tokens/request_hint/expected_tool_results so a test body needs exactly one
//! line per model turn. Raw `TraceStep`/`LlmTrace`/`TraceResponse` construction
//! is forbidden in new Reborn integration tests (design §4.2) — use this.

use crate::support::trace_llm::{TraceResponse, TraceStep};

/// One scripted model turn.
pub struct RebornScriptedReply {
    step: TraceStep,
}

impl RebornScriptedReply {
    /// A plain assistant text reply.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            step: TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: content.into(),
                    input_tokens: 0,
                    output_tokens: 0,
                },
                expected_tool_results: Vec::new(),
            },
        }
    }

    /// Consume into the underlying replay step (crate-internal seam used by
    /// `scripted_provider::scripted_trace_llm`).
    pub(crate) fn into_step(self) -> TraceStep {
        self.step
    }
}
