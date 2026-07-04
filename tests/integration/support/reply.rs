//! `RebornScriptedReply` — the terse façade for scripting one model turn in a
//! Reborn integration test. Each reply maps 1:1 to a `TraceStep`, auto-filling
//! id/tokens/request_hint/expected_tool_results so a test body needs exactly one
//! line per model turn. Raw `TraceStep`/`LlmTrace`/`TraceResponse` construction
//! is forbidden in new Reborn integration tests (design §4.2) — use this.

// Shared integration-test support: `support_unit_tests.rs` mounts the
// `reborn_support` tree without consuming this module, so its symbols read as
// dead there under `-D warnings`. Module-level allow matches the sibling
// support modules (`assertions.rs`, `test_channel.rs`).
#![allow(dead_code)]

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

    /// Scripts one model tool-call turn. Accepts a CapabilityId (e.g. `"builtin.http"`).
    ///
    /// **Why the encoding lives here:** `TraceToolCall.name` flows through `TraceLlm` into
    /// `LlmProviderModelGateway::provider_tool_call_from_llm`, which calls
    /// `ProviderToolName::new(tool_call.name)` with no intermediate conversion.
    /// `ProviderToolName` rejects dots, so the `'.' → "__"` encoding must be applied before
    /// storing into `TraceToolCall`. This is distinct from the `RebornTraceReplayModelGateway`
    /// JSON-fixture-replay path, which has its own identical encoding in `trace_provider_tool_name`
    /// (`model_replay.rs`) at that seam. The two encoders serve different paths and are not
    /// redundant; if the mapping ever needs to change (e.g. collision-safety or truncation),
    /// update both sites together.
    ///
    /// **Collision caveat:** this mapping is NOT collision-safe — two distinct capability IDs
    /// that differ only by `.` vs `__` would produce the same `ProviderToolName`, and long
    /// names are not truncated to `ProviderToolName::MAX_BYTES`. It is valid for the
    /// single-capability tests in the current slice. Any future slice that scripts colliding
    /// or long capability IDs must instead resolve the name against the advertised
    /// `ProviderToolName` from the tool list rather than applying this heuristic mapping.
    ///
    /// The provisional tool-call `id` is auto-filled from a process-scoped
    /// counter. `scripted_provider::scripted_trace_llm` canonicalizes those
    /// ids per trace before the model sees them, so assertions can rely on the
    /// materialized `call-1`, `call-2`, … order.
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

    /// Scripts one model turn carrying MULTIPLE tool calls in a single
    /// assistant response (a "parallel" tool-calls turn — multiple
    /// `tool_calls[]` entries from ONE model call, as opposed to separate
    /// sequential turns). Each `(capability_id, arguments)` pair gets its own
    /// `'.' → "__"` provider-seam encoding and its own provisional id (same
    /// counter `tool_call` uses, canonicalized per trace by
    /// `scripted_trace_llm`). Still counts as exactly ONE script
    /// entry per the harness's "one entry per model call" discipline — the
    /// caller must follow it with exactly one more entry (the post-execution
    /// model call reacting to however many tool results come back).
    pub fn tool_calls<'a>(calls: impl IntoIterator<Item = (&'a str, serde_json::Value)>) -> Self {
        let tool_calls = calls
            .into_iter()
            .map(|(capability_id, arguments)| TraceToolCall {
                id: format!("call-{}", NEXT_TOOL_CALL_ID.fetch_add(1, Ordering::Relaxed)),
                name: capability_id.replace('.', "__"),
                arguments,
            })
            .collect();
        Self {
            step: TraceStep {
                request_hint: None,
                response: TraceResponse::ToolCalls {
                    tool_calls,
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
