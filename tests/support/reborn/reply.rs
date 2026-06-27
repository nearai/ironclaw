//! `RebornScriptedReply` — the terse façade for scripting one model turn in a
//! Reborn integration test. Each reply maps 1:1 to a `TraceStep`, auto-filling
//! id/tokens/request_hint/expected_tool_results so a test body needs exactly one
//! line per model turn. Raw `TraceStep`/`LlmTrace`/`TraceResponse` construction
//! is forbidden in new Reborn integration tests (design §4.2) — use this.

use crate::support::trace_llm::{TraceResponse, TraceStep, TraceToolCall};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TOOL_CALL_ID: AtomicU64 = AtomicU64::new(1);

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

    /// Scripts one model tool-call turn. Accepts a CapabilityId (e.g. `"builtin.http"`);
    /// the name is realized in ProviderToolName format via the reversible `'.' -> "__"` mapping at this seam.
    ///
    /// The tool-call `id` is auto-filled from a process-scoped counter (`call-N`),
    /// so it is unique within a run but not stable across parallel test processes.
    /// No assertion depends on the id value.
    pub fn tool_call(capability_id: &str, arguments: serde_json::Value) -> Self {
        let name = capability_id.replace('.', "__");
        let id = format!("call-{}", NEXT_TOOL_CALL_ID.fetch_add(1, Ordering::Relaxed));
        Self {
            step: TraceStep {
                request_hint: None,
                response: TraceResponse::ToolCalls {
                    tool_calls: vec![TraceToolCall {
                        id,
                        name,
                        arguments,
                    }],
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
