//! The negative half of #7276: curation is a periodic chore, not a per-turn
//! one. Below the configured interval no pass is submitted at all.
//!
//! Worth its own scenario because the failure it guards is invisible in the
//! positive one: a hook that submitted a pass after EVERY turn would still
//! rewrite the document and still pass the sibling scenario, while burning a
//! model call and a memory rewrite on every turn a user takes.
//!
//! Asserted at the model seam rather than by looking for an absent thread: the
//! pass's scope is registered with a script, so any submitted pass calls that
//! model. Zero captured requests is then direct evidence that nothing was
//! submitted — but on its own it would also be what mere latency looks like: a
//! pass already queued and not yet started reads as zero too, and would fire
//! later with nothing here to notice.
//!
//! So the scenario does not stop at the empty reading. One more turn crosses
//! the interval, and the SAME script is then watched until EXACTLY ONE pass —
//! three scripted model calls, no more — has run. That count is what makes the
//! earlier zero real: a pass left pending from the below-threshold turns would
//! run too, and two passes cannot produce one pass's worth of model calls.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_runtime::MEMORY_WRITE_CAPABILITY_ID;
use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use super::support::trace_llm::TraceLlm;

/// Three, so the first two turns are unambiguously short of it — an interval of
/// two would also be satisfied by an off-by-one that fires on `N - 1`.
const CURATION_INTERVAL: u32 = 3;

const SAVED_FACT: &str = "the user keeps their bicycle in the hallway";
const CURATED_DOCUMENT: &str = "the user stores a bicycle in the hallway";

pub async fn run() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builder()
        .with_memory_curation_interval(
            NonZeroU32::new(CURATION_INTERVAL).ok_or("the curation interval must be non-zero")?,
        )
        .builtin_tools_with_native_memory_libsql()
        .await?;

    let conversation = group
        .thread("conv-curation-below-threshold")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_WRITE_CAPABILITY_ID,
                json!({ "target": "memory", "content": SAVED_FACT, "append": true }),
            ),
            RebornScriptedReply::text("noted"),
            RebornScriptedReply::text("nothing to save this time"),
            RebornScriptedReply::text("nor this time"),
        ])
        .build()
        .await?;
    let binding = conversation.binding.clone();
    let user_id = group.canonical_actor_user();

    // Same owner-scoped thread prefix the positive scenario scripts: a pass is
    // keyed on the run that triggered it, so no test can name the thread in
    // advance. Scripting the prefix here is what turns "no pass ran" into an
    // assertion rather than an absence of evidence.
    let curation_llm = group
        .register_scope_script_prefix_for_test(
            format!(
                "memory-curation-{}-{}-",
                binding.tenant_id.as_str(),
                user_id.as_str()
            ),
            "memory-curation-below-threshold",
            [
                RebornScriptedReply::tool_call(
                    MEMORY_WRITE_CAPABILITY_ID,
                    json!({
                        "target": "memory",
                        "content": CURATED_DOCUMENT,
                        "append": false
                    }),
                ),
                RebornScriptedReply::text("Tidied one entry."),
                RebornScriptedReply::text(
                    json!({ "changed": true, "summary": "Tidied one entry." }).to_string(),
                ),
            ],
        )
        .await?;

    conversation
        .submit_turn("I keep my bicycle in the hallway, by the way.")
        .await?;
    conversation
        .submit_turn("Anything else I should know?")
        .await?;
    assert!(
        curation_llm.captured_requests().is_empty(),
        "two turns is short of an interval of {CURATION_INTERVAL} — no pass may be submitted"
    );

    // The corroborating half: one more turn crosses the interval, and the very
    // same script is now used — exactly once. Whatever made the reading above
    // empty, it was neither the machinery being slow or unwired (a pass does
    // run) nor a pass sitting queued from the earlier turns (only ONE runs).
    conversation.submit_turn("And that is all.").await?;
    wait_for_exactly_one_pass(&curation_llm).await
}

/// Number of scripted model calls ONE curation pass makes: the rewrite, an
/// ordinary work-phase candidate, and the host-owned finalizer that records the
/// structured report.
const MODEL_CALLS_PER_PASS: usize = 3;

/// Poll until one whole pass has run, then hold still and require that it was
/// the ONLY one. Polling the model rather than a run handle is deliberate: the
/// pass is submitted from a background path this test never touches, so its
/// model calls are the only synchronous evidence available to wait on.
///
/// The settle window is what turns "one pass has finished" into "exactly one
/// pass exists": a second pass — the one a below-threshold turn would have
/// wrongly queued — is already in flight by the time the first finishes, so it
/// shows up as extra calls within it.
async fn wait_for_exactly_one_pass(curation_llm: &Arc<TraceLlm>) -> HarnessResult<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while curation_llm.captured_requests().len() < MODEL_CALLS_PER_PASS {
        if std::time::Instant::now() > deadline {
            return Err(format!(
                "the {CURATION_INTERVAL}rd turn must trigger a pass, but the pass's model \
                 was called only {} of {MODEL_CALLS_PER_PASS} times",
                curation_llm.captured_requests().len()
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let captured = curation_llm.captured_requests().len();
    if captured != MODEL_CALLS_PER_PASS {
        return Err(format!(
            "exactly one curation pass may run across {CURATION_INTERVAL} turns, but the \
             pass model was called {captured} times ({MODEL_CALLS_PER_PASS} per pass) — a \
             below-threshold turn queued a pass that the earlier empty reading missed"
        )
        .into());
    }
    Ok(())
}
