//! Production-path coverage for #7276 / #7770 phase 1: after every Nth
//! completed conversation turn, the agent goes off on its own — no user
//! present, no conversation — re-reads the user's standing memory document,
//! and tidies it. Nothing is sent back to anyone; the effect is the rewritten
//! document plus a structured report.
//!
//! This drives the whole wired path rather than the policy in isolation: the
//! `after_turn` hook point fires from the real turn executor after the real
//! run reaches terminal, the registered curation hook counts the turn, and the
//! pass it submits executes on the SAME scheduler as an ordinary turn, through
//! the real capability port, writing through the real native memory provider.
//!
//! Two properties ride along because either could make this "pass" while being
//! wrong:
//!
//! - **The pass acts as the user whose turns triggered it.** Memory is
//!   per-user; a pass acting as an operator-config caller or as a different
//!   user would rewrite a different document, and the read-back below — done
//!   under that user's OWN scope — would never see the rewrite.
//! - **The pass is unbound and is not itself a curation trigger.** A
//!   background run that fired the point again would schedule its own
//!   successor forever. Pinned by the pass's model being called exactly the
//!   scripted number of times.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_runtime::{MEMORY_READ_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID};
use ironclaw_memory::content_bytes_sha256;
use serde_json::json;

use super::reborn_support::builder::RebornIntegrationHarness;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use super::support::trace_llm::TraceLlm;

/// Small on purpose: the interval is the thing under test, and two ordinary
/// turns is the cheapest run that distinguishes "every turn" from "every Nth".
const CURATION_INTERVAL: u32 = 2;

/// What two ordinary turns leave behind: the same fact in two wordings —
/// exactly the decay curation exists to fix.
const FIRST_SAVED_FACT: &str = "the user prefers their tea with no sugar";
const SECOND_SAVED_FACT: &str = "the user takes tea unsweetened";

/// What the curation pass rewrites the document to.
const CURATED_DOCUMENT: &str = "the user drinks tea unsweetened (merged from two entries)";

pub async fn run() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builder()
        .with_memory_curation_interval(
            NonZeroU32::new(CURATION_INTERVAL).ok_or("the curation interval must be non-zero")?,
        )
        .builtin_tools_with_native_memory_libsql()
        .await?;

    let conversation = group
        .thread("conv-curation-trigger")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_WRITE_CAPABILITY_ID,
                json!({ "target": "memory", "content": FIRST_SAVED_FACT, "append": true }),
            ),
            RebornScriptedReply::text("noted"),
            RebornScriptedReply::tool_call(
                MEMORY_WRITE_CAPABILITY_ID,
                json!({ "target": "memory", "content": SECOND_SAVED_FACT, "append": true }),
            ),
            RebornScriptedReply::text("noted again"),
        ])
        .build()
        .await?;
    let binding = conversation.binding.clone();
    let user_id = group.canonical_actor_user();

    // The pass's thread id is its own idempotency key, and its distinguishing
    // part is the id of the RUN that triggered it — which is what makes each
    // interval a new pass rather than a replay of the first, and is also why
    // this test cannot name the thread in advance. It scripts the pass by the
    // owner-scoped PREFIX instead, which is the part that is knowable and is
    // itself contract: a pass belongs to the tenant/user whose turns triggered
    // it.
    let curation_thread_prefix = format!(
        "memory-curation-{}-{}-",
        binding.tenant_id.as_str(),
        user_id.as_str()
    );
    let expected_content_hash =
        content_bytes_sha256(format!("{FIRST_SAVED_FACT}\n{SECOND_SAVED_FACT}\n").as_bytes());

    // Script the pass BEFORE any turn runs: it is submitted from a background
    // path this test does not drive, so there is no later moment at which to
    // register its model. Four replies, matching the unbound-structured shape:
    // the versioned read, the rewrite, an ordinary work-phase candidate, then
    // the one host-owned finalizer call that records the validated report.
    let curation_llm = group
        .register_scope_script_prefix_for_test(
            curation_thread_prefix,
            "memory-curation-pass",
            [
                RebornScriptedReply::tool_call(
                    MEMORY_READ_CAPABILITY_ID,
                    json!({ "path": "MEMORY.md" }),
                ),
                RebornScriptedReply::tool_call(
                    MEMORY_WRITE_CAPABILITY_ID,
                    json!({
                        "target": "memory",
                        "content": CURATED_DOCUMENT,
                        "append": false,
                        "expected_content_hash": expected_content_hash
                    }),
                ),
                RebornScriptedReply::text("Merged two entries that said the same thing."),
                RebornScriptedReply::text(
                    json!({
                        "changed": true,
                        "summary": "Merged two entries that said the same thing.",
                        "entries_merged": 1
                    })
                    .to_string(),
                ),
            ],
        )
        .await?;

    // Two ordinary turns, each saving a fact. The second is the Nth, so it is
    // the one whose terminal run fires the point.
    conversation
        .submit_turn("By the way, I take my tea with no sugar.")
        .await?;
    conversation
        .submit_turn("Just so you know, I never sweeten my tea.")
        .await?;

    // The pass runs on the scheduler like any other turn, so nothing here has a
    // handle to await. Wait on the pass's own model instead: four scripted
    // replies consumed means the pass ran to its structured report.
    wait_for_pass_to_finish(&curation_llm).await?;
    let captured = curation_llm.captured_requests();
    assert_eq!(
        captured.len(),
        4,
        "the pass must read, rewrite, and finish exactly once — a background run that \
         re-triggered curation would schedule its own successor forever"
    );
    assert!(
        captured[0].iter().any(|message| message
            .content
            .contains("Never invent, infer, or extrapolate")),
        "the pass must run under the curation prompt asset, not an ordinary chat prompt"
    );
    let read_output = conversation
        .tool_result_output(MEMORY_READ_CAPABILITY_ID)
        .await?;
    assert_eq!(
        read_output["content_hash"].as_str(),
        Some(expected_content_hash.as_str()),
        "the production read result must return the exact hash supplied to the following write"
    );

    // Read the document back the way the product does — through the always-on
    // memory lane of a LATER conversation belonging to the same user. That is
    // also the scope assertion: this lane serves one user's own standing
    // document, so a pass that had acted as an operator-config caller or as a
    // different user could not have put the tidied text here.
    let reader = group
        .thread("conv-curation-reader")
        .script([RebornScriptedReply::text("answered")])
        .build()
        .await?;
    reader
        .submit_turn("Can you help me draft a short note?")
        .await?;
    reader
        .assert_model_request_contains("merged from two entries")
        .await?;
    reader
        .assert_model_request_excludes(FIRST_SAVED_FACT)
        .await?;

    run_conflict_case().await
}

const STALE_SNAPSHOT: &str = "the user prefers the original itinerary";
const CONCURRENT_DOCUMENT: &str = "the user now prefers the updated itinerary";
const STALE_CURATION: &str = "the user prefers the original itinerary (curated)";

struct CurationConflictCase {
    group: RebornIntegrationGroup,
    conversation: RebornIntegrationHarness,
    curation_llm: Arc<TraceLlm>,
    current_hash: String,
}

async fn run_conflict_case() -> HarnessResult<()> {
    let case = build_conflict_case().await?;
    case.submit_turns().await?;
    case.assert_results().await
}

async fn build_conflict_case() -> HarnessResult<CurationConflictCase> {
    let group = RebornIntegrationGroup::builder()
        .with_memory_curation_interval(
            NonZeroU32::new(CURATION_INTERVAL).ok_or("the curation interval must be non-zero")?,
        )
        .builtin_tools_with_native_memory_libsql()
        .await?;
    let conversation = group
        .thread("conv-curation-conflict-trigger")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_WRITE_CAPABILITY_ID,
                json!({
                    "target": "memory",
                    "content": STALE_SNAPSHOT,
                    "append": false
                }),
            ),
            RebornScriptedReply::text("recorded snapshot"),
            RebornScriptedReply::tool_call(
                MEMORY_WRITE_CAPABILITY_ID,
                json!({
                    "target": "memory",
                    "content": CONCURRENT_DOCUMENT,
                    "append": false
                }),
            ),
            RebornScriptedReply::text("recorded newer fact"),
        ])
        .build()
        .await?;
    let curation_thread_prefix = format!(
        "memory-curation-{}-{}-",
        conversation.binding.tenant_id.as_str(),
        group.canonical_actor_user().as_str()
    );
    let stale_hash = content_bytes_sha256(STALE_SNAPSHOT.as_bytes());
    let curation_llm = group
        .register_scope_script_prefix_for_test(
            curation_thread_prefix,
            "memory-curation-conflict-pass",
            [
                RebornScriptedReply::tool_call(
                    MEMORY_READ_CAPABILITY_ID,
                    json!({ "path": "MEMORY.md" }),
                ),
                RebornScriptedReply::tool_call(
                    MEMORY_WRITE_CAPABILITY_ID,
                    json!({
                        "target": "memory",
                        "content": STALE_CURATION,
                        "append": false,
                        "expected_content_hash": stale_hash
                    }),
                ),
                RebornScriptedReply::text("The document changed; no edit was made."),
                RebornScriptedReply::text(
                    json!({
                        "changed": false,
                        "summary": "No edit: the document changed during curation.",
                        "entries_merged": 0
                    })
                    .to_string(),
                ),
            ],
        )
        .await?;

    Ok(CurationConflictCase {
        group,
        conversation,
        curation_llm,
        current_hash: content_bytes_sha256(CONCURRENT_DOCUMENT.as_bytes()),
    })
}

impl CurationConflictCase {
    async fn submit_turns(&self) -> HarnessResult<()> {
        self.conversation
            .submit_turn("Remember my original itinerary.")
            .await?;
        self.conversation
            .submit_turn("I changed my itinerary preference.")
            .await?;
        wait_for_pass_to_finish(&self.curation_llm).await
    }

    async fn assert_results(&self) -> HarnessResult<()> {
        assert_eq!(
            self.curation_llm.captured_requests().len(),
            4,
            "a conflict must complete the pass without a second write attempt"
        );
        let read_output = self
            .conversation
            .tool_result_output(MEMORY_READ_CAPABILITY_ID)
            .await?;
        assert_eq!(
            read_output["content_hash"].as_str(),
            Some(self.current_hash.as_str()),
            "the curation read must observe the newer concurrent document"
        );
        let write_output = self
            .conversation
            .tool_result_output(MEMORY_WRITE_CAPABILITY_ID)
            .await?;
        assert_eq!(
            write_output["status"], "conflict",
            "the stale conditional write must return a model-visible conflict"
        );
        self.conversation
            .assert_capability_result_count(MEMORY_WRITE_CAPABILITY_ID, 3)
            .await?;

        let reader = self
            .group
            .thread("conv-curation-conflict-reader")
            .script([RebornScriptedReply::text("answered")])
            .build()
            .await?;
        reader.submit_turn("Help with an unrelated note.").await?;
        reader
            .assert_model_request_contains(CONCURRENT_DOCUMENT)
            .await?;
        reader.assert_model_request_excludes(STALE_CURATION).await
    }
}

/// Poll until the pass has consumed its whole script. Polling the model rather
/// than a run handle is deliberate: the pass is submitted from a background
/// path this test never touches, so its model calls are the only synchronous
/// evidence available to wait on.
async fn wait_for_pass_to_finish(curation_llm: &Arc<TraceLlm>) -> HarnessResult<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while curation_llm.captured_requests().len() < 4 {
        if std::time::Instant::now() > deadline {
            return Err(format!(
                "the curation pass never finished; it made {} of 4 scripted model calls",
                curation_llm.captured_requests().len()
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
