//! IronClaw integration — golden inference-payload coverage.
//!
//! Exact-matches the FULL model-visible inference payload (system prompt +
//! turns + tool-call/tool-result messages + ordered tool surface) per
//! inference iteration against a committed `insta` snapshot, plus the exact
//! final reply — catching silent drift in prompt assembly, history
//! accumulation, and tool-result feed-back that a substring check can't see.
//! See `tests/integration/support/golden.rs` for canonicalization rationale.
//! Regenerate with `cargo insta review` (or `INSTA_UPDATE=always cargo test`).
//!
//! Deliberately small, curated scenario set (full payload matches are
//! expensive to review); add a scenario only when an existing one can't
//! absorb it — see root `CLAUDE.md` Testing Discipline.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::builder::IronClawIntegrationHarness;
use ironclaw_support::comm_context::RecordingCommunicationContextProvider;
use ironclaw_support::group::IronClawIntegrationGroup;
use ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";
const HTTP_TOOL_URL_A: &str = "https://api.example.test/v1/items/a";
const HTTP_TOOL_URL_B: &str = "https://api.example.test/v1/items/b";
/// A vision-capable model id per `ironclaw_llm::vision_models::VISION_PATTERNS`
/// (mirrors `tests/ironclaw_integration_attach.rs`).
const VISION_MODEL: &str = "claude-3-5-sonnet-20241022";
const PNG_MIME: &str = "image/png";
const PNG_BYTES: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2, 3, 4];

/// (a) Single-turn greeting: the one inference call's full payload + the exact
/// final reply. Pins the base system-prompt construction and text-turn shape.
#[tokio::test]
async fn golden_single_turn_greeting() {
    let h = IronClawIntegrationHarness::test_default()
        .script([IronClawScriptedReply::text("Hello! How can I help?")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("hi there").await.expect("turn completes");
    h.assert_golden_payload("greeting");
    h.assert_reply_eq("Hello! How can I help?")
        .await
        .expect("final reply matches exactly");
}

/// (b) Tool-call turn: both inference iterations exact-matched (initial call
/// + post-tool-result call), pinning that `tool_calls[].id` matches the
/// following `tool` message's `tool_call_id`.
#[tokio::test]
async fn golden_tool_call_feedback() {
    let h = IronClawIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            IronClawScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            IronClawScriptedReply::text("fetched"),
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
    let h = IronClawIntegrationHarness::test_default()
        .script([
            IronClawScriptedReply::text("First reply"),
            IronClawScriptedReply::text("Second reply"),
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

/// (d) Context surfacing: a wired `CommunicationContextProvider` plus the real
/// builtin capability surface, on one plain-text turn. Pins byte-for-byte how
/// the two sections render together in the system prompt — ordering/duplication
/// a substring check like `assert_model_request_contains` cannot see.
#[tokio::test]
async fn golden_context_surfacing() {
    let provider = RecordingCommunicationContextProvider::with_target_and_channel(
        "reborn-golden-target",
        "slack",
        "reborn-golden-channel",
    );
    let h = IronClawIntegrationHarness::test_default()
        .with_communication_context_provider(provider)
        .with_builtin_http_tools()
        .script([IronClawScriptedReply::text("context noted")])
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

/// (f) Parallel tool_calls: one assistant response carrying TWO `tool_calls[]`
/// entries, both exact-matched. Distinct from (b): pins that multiple
/// tool_calls in one message each get a distinct id, and each following `tool`
/// message's `tool_call_id` lines up with the right one, in order.
#[tokio::test]
async fn golden_parallel_tool_calls() {
    let h = IronClawIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            IronClawScriptedReply::tool_calls([
                ("builtin.http", json!({"url": HTTP_TOOL_URL_A})),
                ("builtin.http", json!({"url": HTTP_TOOL_URL_B})),
            ]),
            IronClawScriptedReply::text("fetched both"),
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
/// `submit_inbound_with_attachments` entry point, routed through a
/// vision-pattern model id. Pins byte-for-byte how `ContentPart::ImageUrl`
/// renders alongside the text part — complements
/// `tests/ironclaw_integration_attach.rs`'s substring check by catching drift in
/// part ordering/shape a substring check can't see.
#[tokio::test]
async fn golden_image_attachment_turn() {
    let group = IronClawIntegrationGroup::attachment_tools()
        .await
        .expect("attachment-tools group builds");
    let h = group
        .thread("golden-image-attach")
        .with_model_override(VISION_MODEL)
        .script([IronClawScriptedReply::text("I see a diagram")])
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

/// (h) Gated turn (approve arm): a real `BlockedApproval` gate raised by a
/// scripted `builtin.write_file` call, approved, and resumed. Snapshots both
/// captured inference calls around the gate, pinning that a resume doesn't
/// silently drop, duplicate, or reorder history. Distinct from (b): this one
/// actually parks on `TurnStatus::BlockedApproval` between the two calls.
#[tokio::test]
async fn golden_gated_turn_approve() {
    let group = IronClawIntegrationGroup::live_approvals()
        .await
        .expect("live-approvals group builds");
    let h = group
        .thread("golden-gated-approve")
        .script([
            IronClawScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/golden.txt", "content": "golden write"}),
            ),
            IronClawScriptedReply::text("file written"),
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

// (e) Compaction: NOT implemented — blocked. The byte-cap-overflow force-
// compaction path (`ByteCapStrategy` / `PostCapabilityStage`) sets
// `state.compaction_state.force_compact_on_next_iteration`, but IronClaw's
// `DefaultPlanner` wires `PromptCompactionStep` to
// `ActiveTaskPreservingCompactionStrategy::should_compact`
// (crates/ironclaw_agent_loop/src/strategies/active_task_compaction.rs:41-61),
// which never reads that flag — it only compacts on a genuine ~8,000+ token
// accumulated tail. The force path is a dead letter under IronClaw's installed
// strategy; exercising it here needs a production fix (out of scope) or a
// fragile multi-thousand-token scripted transcript that breaks golden-snapshot
// reviewability.
