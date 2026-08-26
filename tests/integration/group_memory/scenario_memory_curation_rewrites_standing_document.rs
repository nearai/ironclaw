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

use ironclaw_host_runtime::MEMORY_WRITE_CAPABILITY_ID;
use serde_json::json;

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

    // Script the pass BEFORE any turn runs: it is submitted from a background
    // path this test does not drive, so there is no later moment at which to
    // register its model. Three replies, matching the unbound-structured
    // shape: the rewrite, an ordinary work-phase candidate, then the one
    // host-owned finalizer call that records the validated report.
    let curation_llm = group
        .register_scope_script_prefix_for_test(
            curation_thread_prefix,
            "memory-curation-pass",
            [
                RebornScriptedReply::tool_call(
                    MEMORY_WRITE_CAPABILITY_ID,
                    json!({
                        "target": "memory",
                        "content": CURATED_DOCUMENT,
                        "append": false
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
    // handle to await. Wait on the pass's own model instead: three scripted
    // replies consumed means the pass ran to its structured report.
    wait_for_pass_to_finish(&curation_llm).await?;
    let captured = curation_llm.captured_requests();
    assert_eq!(
        captured.len(),
        3,
        "the pass must run exactly once — a background run that re-triggered curation \
         would schedule its own successor forever"
    );
    assert!(
        captured[0].iter().any(|message| message
            .content
            .contains("Never invent, infer, or extrapolate")),
        "the pass must run under the curation prompt asset, not an ordinary chat prompt"
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

    Ok(())
}

/// Poll until the pass has consumed its whole script. Polling the model rather
/// than a run handle is deliberate: the pass is submitted from a background
/// path this test never touches, so its model calls are the only synchronous
/// evidence available to wait on.
async fn wait_for_pass_to_finish(curation_llm: &Arc<TraceLlm>) -> HarnessResult<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while curation_llm.captured_requests().len() < 3 {
        if std::time::Instant::now() > deadline {
            return Err(format!(
                "the curation pass never finished; it made {} of 3 scripted model calls",
                curation_llm.captured_requests().len()
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
