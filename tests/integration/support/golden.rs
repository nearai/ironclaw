//! Golden-payload assertions for [`RebornIntegrationHarness`] — exact-match of
//! the FULL model-visible inference payload (system prompt, conversation turns,
//! and tool-call/tool-result messages) per inference iteration, plus a compact
//! ordered tool surface and the exact final user-visible reply.
//!
//! ## Why exact-match, and what is normalized
//!
//! The `assert_system_prompt_contains` / `assert_model_request_contains` family
//! (assertions.rs) proves a *substring* reached the model. This module proves the
//! WHOLE payload — every byte the model saw — is constructed exactly as pinned,
//! catching silent drift in prompt assembly, turn/history accumulation, and
//! tool-result feed-back that a substring check cannot see.
//!
//! The captured payload is rendered to canonical JSON (via `serde_json::Value`,
//! whose object keys are BTree-sorted, so key order is deterministic across
//! runs and builds) and snapshotted with `insta` — the repo's established
//! snapshot tool (`assert_replay_snapshot!` in `replay_outcome.rs`). Review /
//! regenerate drift with `cargo insta review` (or `INSTA_UPDATE=always`); on a
//! mismatch insta prints the full expected-vs-actual diff.
//!
//! ### Messages exact, tool surface compact — deliberately
//!
//! Per call we render `messages` in FULL (the crux: system prompt, every turn,
//! and the assistant-tool-call/tool-result pair) but the tool surface only as
//! the ORDERED LIST OF PROVIDER-SEAM TOOL NAMES (`tool_surface`), not the full
//! JSON parameter schemas. Reasons:
//!   - The named target of this coverage is prompt/turn construction (system
//!     prompt + conversation turns + tool results) — the messages.
//!   - The full 14-tool builtin schema is ~1.2k lines and would couple this
//!     golden to every unrelated edit of any builtin tool's help text/schema.
//!   - The surface's *content* is already pinned inside the exact-matched system
//!     prompt: the `surface sha256:…` line is a content hash over the capability
//!     surface, and the capability id/name/description list is rendered inline.
//!     Per-tool parameter schemas are pinned by each tool's own tests.
//!
//! The name list still catches a tool appearing / disappearing / reordering and
//! the `.`→`__` provider-seam encoding (`builtin.http` → `builtin__http`).
//!
//! ### Normalization set
//!
//! Two values in the payload are genuinely nondeterministic; both are
//! anchored on an exact literal prefix so nothing else is touched:
//!
//!   - The runtime context's model-visible wall clock, rendered as
//!     `Current date/time at loop start: <RFC3339-minute>Z`. Production has no
//!     clock seam the harness substitutes (per the NO-WIRE rule we do not add
//!     one), so the real minute leaks in — rewritten to `<TIMESTAMP>`.
//!   - An image-attachment scenario's landed project path, which embeds
//!     today's real UTC date (`chrono::Utc::now()` at
//!     `crates/ironclaw_reborn_composition/src/attachment_landing.rs`, no test
//!     seam either): `.../attachments/<YYYY-MM-DD>/...` — rewritten to
//!     `.../attachments/<DATE>/...`. Only scenarios that land an attachment
//!     (`RebornIntegrationGroup::attachment_tools()`) ever contain this
//!     substring; every other golden test is a no-op match.
//!
//! Everything else stays EXACT on purpose:
//!   - Tool-call ids (`call-1`, `call-2`, …) come from `RebornScriptedReply`'s
//!     `NEXT_TOOL_CALL_ID` — a counter shared by every test in this ONE
//!     compiled binary (`reply.rs` is per-binary, not per-test), so its raw
//!     value depends on which sibling golden test's `tool_call`/`tool_calls`
//!     happened to run concurrently first (`cargo test` runs a binary's tests
//!     on a thread pool by default) — not reproducible across runs once more
//!     than one golden scenario scripts a tool call. `scripted_trace_llm`
//!     canonicalizes the scripted trace to stable per-trace `call-1`,
//!     `call-2`, … ids before the model sees it, so the golden pins the actual
//!     materialized ids without depending on the specific, racy raw counter
//!     value.
//!   - The `surface sha256:…` line is a content hash of the capability surface:
//!     deterministic given the surface, and a surface change SHOULD ripple into
//!     the golden (that is the point).

#![allow(dead_code)]

use ironclaw_llm::{ChatMessage, ToolDefinition};

use super::builder::RebornIntegrationHarness;

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Render every captured inference request as one canonical, human-readable
/// block per call: `===== inference call {i} =====` followed by the pretty
/// JSON of `{ "messages": [...], "tool_surface": ["name", ...] }`. `messages`
/// is rendered in full; `tool_surface` is the ordered provider-seam tool names
/// only (see the module docs for why schemas are excluded). Key order is
/// deterministic (BTree-sorted through `serde_json::Value`); no volatile
/// normalization happens here — that is the caller's `insta` filter.
fn render_inference_payloads(
    requests: &[Vec<ChatMessage>],
    tool_definitions: &[Vec<ToolDefinition>],
) -> String {
    let empty = Vec::new();
    let mut out = String::new();
    for (index, messages) in requests.iter().enumerate() {
        let tool_surface: Vec<&str> = tool_definitions
            .get(index)
            .unwrap_or(&empty)
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        let payload = serde_json::json!({ "messages": messages, "tool_surface": tool_surface });
        let pretty = serde_json::to_string_pretty(&payload)
            .expect("captured inference payload serializes to JSON");
        out.push_str(&format!("===== inference call {index} =====\n{pretty}\n"));
    }
    out
}

/// Replace the two nondeterministic values in the payload — the runtime
/// context's model-visible wall clock (`Current date/time at loop start:
/// <RFC3339-minute>Z`) and, for attachment-landing scenarios, today's real
/// UTC date embedded in the landed project path (`.../attachments/<date>/...`)
/// — with `<TIMESTAMP>`/`<DATE>` respectively. Each is anchored on an exact
/// literal prefix so no other field is touched; tool-call ids and the
/// `surface sha256:` hash stay exact after scripted-provider materialization
/// (see module docs).
fn normalize_volatile(rendered: &str) -> String {
    let clock =
        regex::Regex::new(r"Current date/time at loop start: \d{4}-\d{2}-\d{2}T\d{2}:\d{2}Z")
            .expect("valid loop-start-clock regex");
    let rendered = clock.replace_all(rendered, "Current date/time at loop start: <TIMESTAMP>");
    let attachment_date = regex::Regex::new(r"/attachments/\d{4}-\d{2}-\d{2}/")
        .expect("valid attachment-landing-date regex");
    attachment_date
        .replace_all(&rendered, "/attachments/<DATE>/")
        .into_owned()
}

impl RebornIntegrationHarness {
    /// Assert the FULL model-visible inference payload for this thread (every
    /// captured inference call: system prompt + turns + tool messages + tool
    /// surface) matches the committed golden snapshot `golden_payload__{name}`.
    ///
    /// Reads the retained scripted `TraceLlm`'s `captured_requests()` /
    /// `captured_tool_definitions()` (the same capture source
    /// `assert_system_prompt_contains` reads), renders them canonically, and
    /// snapshots via `insta` with loop-start-clock/date normalization applied.
    /// Panics (like the sibling `assert_replay_snapshot!`) on mismatch; run
    /// `cargo insta review` to inspect and accept drift.
    pub fn assert_golden_payload(&self, name: &str) {
        let rendered = normalize_volatile(&render_inference_payloads(
            &self.scripted_llm.captured_requests(),
            &self.scripted_llm.captured_tool_definitions(),
        ));
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
        settings.set_prepend_module_to_snapshot(false);
        settings.set_omit_expression(true);
        settings.bind(|| {
            insta::assert_snapshot!(format!("golden_payload__{name}"), rendered);
        });
    }

    /// Assert the finalized assistant reply on this thread is EXACTLY `expected`
    /// (not a substring — the output-seam counterpart to `assert_golden_payload`,
    /// pinning that the model's final text reaches the user verbatim).
    pub async fn assert_reply_eq(&self, expected: &str) -> HarnessResult<()> {
        let actual = self.final_reply_text().await?;
        if actual == expected {
            return Ok(());
        }
        Err(format!("finalized reply {actual:?} does not exactly equal {expected:?}").into())
    }

    /// The exact finalized assistant reply text on this thread (last finalized
    /// `Assistant` message). Errors if none is present.
    async fn final_reply_text(&self) -> HarnessResult<String> {
        let history = self
            .thread_harness
            .history(self.binding.thread_id.clone())
            .await?;
        history
            .iter()
            .rev()
            .find(|message| {
                message.kind == ironclaw_threads::MessageKind::Assistant
                    && message.status == ironclaw_threads::MessageStatus::Finalized
            })
            .and_then(|message| message.content.clone())
            .ok_or_else(|| "no finalized assistant reply on thread".into())
    }
}
