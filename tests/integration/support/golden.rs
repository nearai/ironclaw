//! Golden-payload assertions for [`RebornIntegrationHarness`]: exact-match of
//! the FULL model-visible inference payload (system prompt, turns, tool calls/
//! results) per inference iteration, via `insta` snapshots (`cargo insta
//! review` / `INSTA_UPDATE=always` to accept drift). Complements the substring
//! checks in `assertions.rs` (`assert_system_prompt_contains` etc.) by catching
//! prompt/turn-assembly drift a substring check can't see. Payload is rendered
//! to canonical JSON (`serde_json::Value`, BTree-sorted keys) for deterministic
//! ordering across runs.
//!
//! `messages` render in FULL; `tool_surface` renders as ORDERED TOOL NAMES ONLY
//! (not full JSON schemas) — the full builtin schema (~1.2k lines) would couple
//! this golden to every unrelated tool schema edit, and schemas are already
//! pinned per-tool and via the system prompt's `surface sha256:` hash. The name
//! list still catches a tool appearing/disappearing/reordering and the
//! `.`→`__` provider-seam encoding.
//!
//! Two values are normalized (anchored on an exact literal prefix so nothing
//! else is touched), because production has no clock/date test seam (NO-WIRE
//! rule): the runtime context's model-visible wall clock (`Current date/time
//! at loop start: ...` → `<TIMESTAMP>`) and, for attachment-landing scenarios,
//! today's real UTC date embedded in the landed project path
//! (`/attachments/<date>/` → `/attachments/<DATE>/`). Everything else — tool-
//! call ids (canonicalized per-trace by `scripted_trace_llm`) and the `surface
//! sha256:` hash — stays exact.

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

/// Normalize the nondeterministic values (clock, attachment date, minted
/// result refs) — see module docs.
fn normalize_volatile(rendered: &str) -> String {
    let clock =
        regex::Regex::new(r"Current date/time at loop start: \d{4}-\d{2}-\d{2}T\d{2}:\d{2}Z")
            .expect("valid loop-start-clock regex");
    let rendered = clock.replace_all(rendered, "Current date/time at loop start: <TIMESTAMP>");
    let attachment_date = regex::Regex::new(r"/attachments/\d{4}-\d{2}-\d{2}/")
        .expect("valid attachment-landing-date regex");
    let rendered = attachment_date.replace_all(&rendered, "/attachments/<DATE>/");
    // Success observations carry the minted `result:<run>.<invocation>` ref
    // (ToolObservationDetail::ResultReference); the ids are fresh per run.
    let result_ref = regex::Regex::new(
        r"result:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
    )
    .expect("valid result-ref regex");
    result_ref
        .replace_all(&rendered, "result:<RESULT_REF>")
        .into_owned()
}

impl RebornIntegrationHarness {
    /// Assert the FULL model-visible inference payload for this thread matches
    /// the committed golden snapshot `golden_payload__{name}`. Panics (like the
    /// sibling `assert_replay_snapshot!`) on mismatch; run `cargo insta review`
    /// to inspect and accept drift.
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
