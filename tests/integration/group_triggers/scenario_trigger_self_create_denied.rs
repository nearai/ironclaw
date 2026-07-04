//! C-DENYEDGE (row 4): a scheduled-trigger fire must not be able to create
//! (or remove/pause/resume) triggers of its own — the int-tier twin of the
//! `ironclaw_reborn::runtime` unit coverage for issue #5505
//! (`SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS`, PR #5515).
//!
//! Drives a REAL triggered-origin run (`submit_triggered_turn_scripted`,
//! `TurnOriginKind::ScheduledTrigger`) that scripts a `builtin.trigger_create`
//! call. The trusted-trigger submit path (`ironclaw_conversations::inbound`)
//! sets `requested_run_profile: RunProfileId::scheduled_trigger()` on the
//! `SubmitTurnRequest`, so the run resolves under the dedicated
//! `scheduled_trigger` run profile
//! (`crate::planned_driver_factory::scheduled_trigger_planned_profile_definition`),
//! whose `capability_surface_profile_id` is
//! `SCHEDULED_TRIGGER_CAPABILITY_SURFACE_PROFILE_ID`. The host's
//! `PerSurfaceCapabilityDenyDecorator` — wired unconditionally in
//! `ironclaw_reborn::runtime::build_default_planned_runtime`, keyed on that
//! profile id, resolving to a `CapabilitySurfaceDenyFilter` — strips
//! `builtin.trigger_create` (and remove/pause/resume; `trigger_list` stays
//! visible) from the fire's model-visible capability surface.
//!
//! ## Where the denial actually happens (traced, not assumed)
//!
//! A model tool call targeting a denied capability is rejected earlier than
//! `CapabilityStage`'s per-call `denied_calls` bucket
//! (`crates/ironclaw_agent_loop/src/executor/capabilities.rs`, the
//! `"capability is not visible in the filtered surface"` literal — that arm
//! is for a capability that WAS registered as a `CapabilityCallCandidate`
//! but fell outside the surface between advertisement and dispatch, e.g. a
//! stale-surface race). For a capability denied from the start (ours),
//! `ironclaw_reborn::model_gateway`'s response classification calls
//! `capabilities.validate_provider_tool_call(provider_call)` on every raw
//! provider tool call BEFORE registering it
//! (`crates/ironclaw_reborn/src/model_gateway.rs` ~line 1226); the deny
//! filter's `CapabilitySurfaceDenyFilter::validate_provider_tool_call`
//! (`crates/ironclaw_loop_support/src/capability_surface_filter.rs`) returns
//! `AgentLoopHostErrorKind::InvalidInvocation` with the fixed message
//! `"provider tool call targets a disabled capability"`, and
//! `map_provider_tool_output_error` maps that to
//! `HostManagedModelError::safe(HostManagedModelErrorKind::InvalidOutput, ..)`.
//! The whole model response is rejected at the GATEWAY seam — no
//! `CapabilityCallCandidate` is ever constructed, so `CapabilityStage` never
//! runs for this call, and nothing is appended via
//! `append_capability_result_ref`/`append_tool_result_reference` (confirmed
//! empirically: the triggered thread's persisted history is exactly
//! `[User, Assistant]` — no `ToolResultReference` message at all). The
//! executor transparently re-issues the model call (consuming the second
//! scripted `text(..)` reply) rather than gating or failing the run.
//!
//! Because nothing is persisted to thread history or the in-process capability
//! recorder for this seam, `assert_tool_error`/`assert_tool_error_summary_contains`
//! (which both read persisted `ToolResultReference` envelopes) cannot observe
//! it — there is no host-authored summary string to pin here, unlike the
//! gate-declined/filtered-surface-race families those assertions were built
//! for. This scenario instead asserts the actual security property directly:
//! (1) `builtin.trigger_create` was never dispatched (`assert_tool_invoked`
//! returns `Err`), (2) the run completes cleanly (no hang, no terminal
//! failure), and (3) no trigger with the attempted name exists in the shared
//! repository afterward, verified by dispatching a real (non-triggered)
//! `builtin.trigger_list` call, whose surface is unaffected by the
//! scheduled-trigger deny map.
//!
//! Distinct from `scenario_verbs_lifecycle`'s `trigger_create` coverage in
//! this same binary: that scenario submits through the plain (non-triggered)
//! `submit_turn` wire, which resolves the default/interactive run profile —
//! the trigger-mutator surface is fully visible there, so it never exercises
//! this deny map at all.

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

    // The capability must never have reached dispatch at all — a
    // discriminating negative check against the real in-process invocation
    // recorder (not a bare `.is_err()` on something unrelated: this reads
    // the SAME recorder `assert_tool_invoked` uses for the positive case
    // in `scenario_verbs_lifecycle`).
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

    // Strongest proof of the security property: no trigger with the
    // attempted name actually exists in the shared repository. Verified
    // through a genuine INTERACTIVE (non-triggered) `trigger_list` call on a
    // fresh thread in the SAME group — `trigger_list` is read-only and stays
    // visible on every profile, so this reads the real, current repository
    // state rather than anything scoped to the denied run.
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
