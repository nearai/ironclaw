//! C-DENYEDGE (row 4): a scheduled-trigger fire must not be able to create (or
//! remove/pause/resume) triggers of its own — int-tier twin of the
//! `ironclaw_runner::runtime` unit coverage for issue #5505
//! (`SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS`, PR #5515).
//!
//! Drives a triggered-origin run (`submit_triggered_turn_scripted`) that scripts
//! `builtin.trigger_create`. The host's `PerSurfaceCapabilityDenyDecorator`,
//! keyed on the `scheduled_trigger` run profile's capability-surface id, strips
//! trigger_create/remove/pause/resume from the model-visible surface
//! (`trigger_list` stays visible).
//!
//! Traced, not assumed: denial happens at the model-gateway seam
//! (`ironclaw_runner::model_gateway`'s `validate_provider_tool_call`, via
//! `CapabilitySurfaceDenyFilter`), BEFORE a `CapabilityCallCandidate` is ever
//! constructed — so `CapabilityStage` never runs and nothing is appended via
//! `append_tool_result_reference` (confirmed empirically: persisted history is
//! exactly `[User, Assistant]`, no `ToolResultReference`). The executor
//! transparently re-issues the model call rather than gating or failing.
//!
//! Because nothing is persisted for this seam, `assert_tool_error`/
//! `assert_tool_error_summary_contains` (which read persisted envelopes) can't
//! observe it. This scenario instead asserts the security property directly:
//! (1) `builtin.trigger_create` was never dispatched, (2) the run completes
//! cleanly, (3) no trigger with the attempted name exists afterward (verified
//! via a real, non-triggered `builtin.trigger_list` call).
//!
//! Distinct from `scenario_verbs_lifecycle`'s `trigger_create` coverage: that
//! submits through the plain `submit_turn` wire (interactive run profile),
//! where the trigger-mutator surface is fully visible and never exercises
//! this deny map.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

/// Distinctive enough that a false-positive match against another scenario's
/// trigger name (e.g. `scenario_verbs_lifecycle`'s `"t0-triggers-once"`) is
/// not a concern.
const SELF_CREATE_ATTEMPT_TRIGGER_NAME: &str = "self-created-follow-up-should-not-exist";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g.thread("conv-trigger-self-create-denied").build().await?;

    let submission = h
        .submit_triggered_turn_scripted(
            "create a follow-up reminder",
            [
                RebornScriptedReply::tool_call(
                    "builtin.trigger_create",
                    json!({
                        "name": SELF_CREATE_ATTEMPT_TRIGGER_NAME,
                        "prompt": "remind me again",
                        "schedule": {"kind": "once", "at": "2999-01-01T00:00:00", "timezone": "UTC"},
                    }),
                ),
                RebornScriptedReply::text("understood, I can't schedule that myself"),
            ],
        )
        .await?;

    // Must NOT hang or fail: the denial is a model-recoverable outcome, not
    // a gate or a terminal failure.
    h.wait_for_status_in_scope(
        &submission.turn_scope,
        submission.run_id,
        TurnStatus::Completed,
    )
    .await?;

    // The capability must never have reached dispatch — reads the SAME
    // invocation recorder `assert_tool_invoked` uses for the positive case in
    // `scenario_verbs_lifecycle`, not a bare `.is_err()` on something unrelated.
    if h.assert_tool_invoked("builtin.trigger_create")
        .await
        .is_ok()
    {
        return Err(
            "expected builtin.trigger_create to be denied for a scheduled-trigger fire, \
             but the capability recorder shows it was invoked"
                .into(),
        );
    }

    // Strongest proof: no trigger with the attempted name exists, verified via
    // a genuine INTERACTIVE `trigger_list` call (read-only, visible on every
    // profile) rather than anything scoped to the denied run.
    let verifier = g
        .thread("conv-trigger-self-create-denied-verify")
        .script([
            RebornScriptedReply::tool_call("builtin.trigger_list", json!({})),
            RebornScriptedReply::text("listed"),
        ])
        .build()
        .await?;
    verifier.submit_turn("list my triggers").await?;
    let listed = verifier.tool_result_output("builtin.trigger_list").await?;
    let triggers = listed["triggers"]
        .as_array()
        .ok_or("trigger_list output missing triggers array")?;
    if triggers
        .iter()
        .any(|t| t["name"] == json!(SELF_CREATE_ATTEMPT_TRIGGER_NAME))
    {
        return Err(format!(
            "expected no trigger named {SELF_CREATE_ATTEMPT_TRIGGER_NAME:?} to exist, \
             but trigger_list returned {listed}"
        )
        .into());
    }
    Ok(())
}
