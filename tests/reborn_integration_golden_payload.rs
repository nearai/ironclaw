//! Reborn integration — golden inference-payload coverage.
//!
//! Exact-matches the FULL model-visible inference payload (system prompt +
//! conversation turns + tool-call/tool-result messages + ordered tool surface)
//! per inference iteration against a committed `insta` snapshot, plus the exact
//! final user-visible reply. Where `assert_system_prompt_contains` proves a
//! substring reached the model, this pins end-to-end prompt construction byte
//! for byte — catching silent drift in prompt assembly, history accumulation,
//! and tool-result feed-back. See `tests/support/reborn/golden.rs` for the
//! canonicalization + single-filter normalization rationale. Regenerate drift
//! with `cargo insta review` (or `INSTA_UPDATE=always cargo test`).
//!
//! This is the dedicated suite for watching prompt-construction drift: base
//! system-prompt assembly, context/capability surfacing (communication
//! context, capability surface), message appending across turns, and (once
//! reachable — see the "(e) Compaction" blocker note below) compaction — on a
//! deliberately small, curated set of scenarios (full payload matches are
//! expensive to review and maintain; substring checks elsewhere cover
//! everything else). Add a new scenario here only when an existing one can't
//! absorb it — see root `CLAUDE.md` Testing Discipline.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::comm_context::RecordingCommunicationContextProvider;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";
const HTTP_TOOL_URL_A: &str = "https://api.example.test/v1/items/a";
const HTTP_TOOL_URL_B: &str = "https://api.example.test/v1/items/b";
/// A vision-capable model id per `ironclaw_llm::vision_models::VISION_PATTERNS`
/// (mirrors `tests/reborn_integration_attach.rs`).
const VISION_MODEL: &str = "claude-3-5-sonnet-20241022";
const PNG_MIME: &str = "image/png";
const PNG_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2, 3, 4];

/// (a) Single-turn greeting: the one inference call's full payload + the exact
/// final reply. Pins the base system-prompt construction and text-turn shape.
#[tokio::test]
async fn golden_single_turn_greeting() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("Hello! How can I help?")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("hi there").await.expect("turn completes");
    h.assert_golden_payload("greeting");
    h.assert_reply_eq("Hello! How can I help?")
        .await
        .expect("final reply matches exactly");
}

/// (b) Tool-call turn: BOTH inference iterations exact-matched — the initial
/// call and the post-tool-result call. Pins tool-result feed-back construction
/// (the assistant `tool_calls[].id` and the following `tool` message's
/// `tool_call_id` must match).
#[tokio::test]
async fn golden_tool_call_feedback() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    h.assert_golden_payload("tool_call");
    h.assert_reply_eq("fetched")
        .await
        .expect("final reply matches exactly");
}

/// (c) Multi-turn (two user turns): the second turn's inference call carries the
/// accumulated history (turn-1 user + assistant reply + turn-2 user). Golden
/// pins history/turns accumulation across turns.
#[tokio::test]
async fn golden_multi_turn_history() {
    let h = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::text("First reply"),
            RebornScriptedReply::text("Second reply"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("first question")
        .await
        .expect("turn 1 completes");
    h.submit_turn("second question")
        .await
        .expect("turn 2 completes");
    // Structural pin alongside the golden snapshot (see assert messages below
    // for the exact shape).
    let requests = h.scripted_llm.captured_requests();
    assert_eq!(requests.len(), 2, "one inference call per turn");
    assert_eq!(
        requests[1].len(),
        4,
        "second call carries system prompt + turn-1 user/assistant + turn-2 user"
    );
    h.assert_golden_payload("multi_turn");
    h.assert_reply_eq("Second reply")
        .await
        .expect("final reply matches exactly");
}

/// (d) Context surfacing: a wired `CommunicationContextProvider` PLUS the real
/// builtin capability surface, on one plain-text turn. Pins byte-for-byte how
/// the communication/context section and the capability surface render into
/// the system prompt alongside each other (a single-value substring check,
/// e.g. `assert_model_request_contains`, cannot see whether the two sections
/// interleave, reorder, or duplicate content).
#[tokio::test]
async fn golden_context_surfacing() {
    let provider = RecordingCommunicationContextProvider::with_target_and_channel(
        "reborn-golden-target",
        "slack",
        "reborn-golden-channel",
    );
    let h = RebornIntegrationHarness::test_default()
        .with_communication_context_provider(provider)
        .with_builtin_http_tools()
        .script([RebornScriptedReply::text("context noted")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("what's my delivery target?")
        .await
        .expect("turn completes");
    h.assert_golden_payload("context_surfacing");
    h.assert_reply_eq("context noted")
        .await
        .expect("final reply matches exactly");
}

/// (f) Parallel tool_calls: ONE assistant response carrying TWO `tool_calls[]`
/// entries (`RebornScriptedReply::tool_calls`), both exact-matched — the
/// initial multi-call request and the post-both-results call. Distinct from
/// (b) `golden_tool_call_feedback`: that pins a SINGLE tool_call's id/result
/// feedback; this pins that MULTIPLE tool_calls in one assistant message each
/// get a distinct id and each following `tool` role message's `tool_call_id`
/// lines up with the right one, in order — a shape (b) cannot exercise with
/// only one call.
#[tokio::test]
async fn golden_parallel_tool_calls() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_calls([
                ("builtin.http", json!({"url": HTTP_TOOL_URL_A})),
                ("builtin.http", json!({"url": HTTP_TOOL_URL_B})),
            ]),
            RebornScriptedReply::text("fetched both"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch both items")
        .await
        .expect("turn completes");
    h.assert_golden_payload("parallel_tool_calls");
    h.assert_reply_eq("fetched both")
        .await
        .expect("final reply matches exactly");
}

/// (g) Image/user-parts: an inline image attachment landed through the real
/// `submit_inbound_with_attachments` entry point (`RebornIntegrationGroup::attachment_tools()`)
/// and routed through a vision-pattern model id. Pins byte-for-byte how the
/// user turn's multimodal content parts (`ContentPart::ImageUrl` `data:` URL
/// alongside the text part) render into the request — a shape none of the
/// plain-text scenarios above can exercise. Complements
/// `tests/reborn_integration_attach.rs`'s substring assertion
/// (`assert_model_saw_image_attachment`) with the full-payload byte-for-byte
/// pin, catching drift in part ORDERING/shape a substring check cannot see.
#[tokio::test]
async fn golden_image_attachment_turn() {
    let group = RebornIntegrationGroup::attachment_tools()
        .await
        .expect("attachment-tools group builds");
    let h = group
        .thread("golden-image-attach")
        .with_model_override(VISION_MODEL)
        .script([RebornScriptedReply::text("I see a diagram")])
        .build()
        .await
        .expect("thread builds");
    h.submit_turn_with_image_attachment(
        "what's in this image?",
        "diagram.png",
        PNG_MIME,
        PNG_BYTES.to_vec(),
    )
    .await
    .expect("turn completes");
    h.assert_golden_payload("image_attachment");
    h.assert_reply_eq("I see a diagram")
        .await
        .expect("final reply matches exactly");
}

/// (h) Gated turn (approve arm): a real `BlockedApproval` gate
/// (`RebornIntegrationGroup::live_approvals()`) raised by a scripted
/// `builtin.write_file` call, approved, and resumed. Snapshots BOTH captured
/// inference calls around the gate in one golden — the pre-gate tool_call
/// request AND the post-resume request that reacts to the granted write's
/// result — pinning that a gate resume does not silently drop, duplicate, or
/// reorder the accumulated turn history the way a mid-run resume easily
/// could. Distinct from (b): that tool call dispatches immediately
/// (auto-approved, `test_default()`'s Echo backend has no gate); this one
/// actually parks on `TurnStatus::BlockedApproval` between the two calls.
#[tokio::test]
async fn golden_gated_turn_approve() {
    let group = RebornIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h = group
        .thread("golden-gated-approve")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/golden.txt", "content": "golden write"}),
            ),
            RebornScriptedReply::text("file written"),
        ])
        .build()
        .await
        .expect("thread builds");
    let (run_id, gate_ref) = h
        .submit_turn_until_blocked("write the golden file")
        .await
        .expect("turn blocks on the approval gate");
    h.approve_gate(run_id, &gate_ref)
        .await
        .expect("gate approves");
    h.wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await
        .expect("run completes after resume");
    h.assert_golden_payload("gated_turn_approve");
    h.assert_reply_eq("file written")
        .await
        .expect("final reply matches exactly");
}

// (e) Compaction: NOT implemented — blocked. Investigated the byte-cap-overflow
// force-compaction path (`ByteCapStrategy` / `PostCapabilityStage`,
// crates/ironclaw_agent_loop/src/executor/post_capability.rs:73-91) end-to-end
// through this harness with a scripted `builtin.http` result exceeding the
// 32,000-byte cap (crates/ironclaw_agent_loop/src/strategies/compaction.rs:190-213,
// `ByteCapStrategy::with_defaults`). Confirmed via instrumented local runs (not
// committed) that `pending_capability_bytes` correctly accumulates past the cap
// and `state.compaction_state.force_compact_on_next_iteration` /
// `skip_model_this_iteration` DO get set, and that `PromptStage`
// (crates/ironclaw_agent_loop/src/executor/prompt.rs:219-241) correctly detects
// the flag on the very next iteration and enters `PromptCompactionStep`.
//
// The wall: `PromptCompactionStep::run` decides via
// `self.ctx.planner.compaction().should_compact(..)`
// (crates/ironclaw_agent_loop/src/executor/prompt.rs:442-446), and the Reborn
// `DefaultPlanner` wires that seam to `ActiveTaskPreservingCompactionStrategy`,
// not the bare `DefaultCompactionStrategy`
// (crates/ironclaw_agent_loop/src/default_planner.rs:339).
// `ActiveTaskPreservingCompactionStrategy::should_compact`
// (crates/ironclaw_agent_loop/src/strategies/active_task_compaction.rs:41-61)
// never reads `state.compaction_state.force_compact_on_next_iteration` at all —
// it always falls through to `active_task_preserving_user_boundary`, which
// requires a genuine accumulated tail of at least `preserve_tail_tokens`
// (8,000 tokens, `DefaultCompactionStrategy::DEFAULT_PRESERVE_TAIL_TOKENS`) of
// real message content plus `minimum_tail_messages`/`minimum_compacted_messages`
// (3 each) before it will trigger at all. The byte-cap-overflow force path
// (`CompactionInitiator::CapabilityResultOverflow`) is consequently a dead
// letter under the strategy Reborn's `default_planner.rs` actually installs —
// it sets state flags this strategy's `should_compact` never consults.
//
// Exercising compaction here would need either a production fix (out of scope
// per this suite's test-only mandate) or inflating the scripted transcript to a
// genuine ~8,000+ token natural tail, which no longer tests the reported
// byte-cap mechanism, would make the golden snapshot unreviewable, and is
// fragile against token-estimation drift. Reporting the blocker instead of
// faking the scenario.
