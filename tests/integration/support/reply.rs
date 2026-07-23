//! `IronClawScriptedReply` — terse façade for scripting one model turn in a
//! IronClaw integration test; each reply maps 1:1 to a `TraceStep`. Raw
//! `TraceStep`/`LlmTrace`/`TraceResponse` construction is forbidden in new
//! IronClaw integration tests (design §4.2) — use this.

// dead_code: `support_unit_tests.rs` mounts `ironclaw_support` without
// consuming this module, so symbols read unused there under `-D warnings`
// (matches sibling support modules).
#![allow(dead_code)]

use crate::support::trace_llm::{TraceResponse, TraceStep, TraceToolCall};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TOOL_CALL_ID: AtomicU64 = AtomicU64::new(1);

/// One scripted model turn.
pub struct IronClawScriptedReply {
    step: TraceStep,
}

impl IronClawScriptedReply {
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

    /// Scripts one model tool-call turn (CapabilityId, e.g. `"builtin.http"`).
    /// Applies the `'.' → "__"` encoding `ProviderToolName::new` requires (it
    /// rejects dots); `model_replay.rs`'s `trace_provider_tool_name` has an
    /// identical, independent encoder for the fixture-replay seam — keep both
    /// in sync if the mapping changes. NOT collision-safe (`.` vs `__` can
    /// collide) and not length-truncated — fine for single-capability tests
    /// only. The `id` auto-fills from a process-scoped counter, canonicalized
    /// per trace by `scripted_provider::scripted_trace_llm`.
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

    /// Scripts one model turn carrying MULTIPLE tool calls (a "parallel"
    /// tool-calls turn from ONE model call). Each pair gets `tool_call`'s
    /// same encoding/id treatment. Still counts as exactly ONE script entry —
    /// the caller must follow with one more entry (the post-execution model
    /// call reacting to the tool results).
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
