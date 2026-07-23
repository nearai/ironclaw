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

use std::sync::Arc;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_host_api::{CapabilityId, Resolution};
use ironclaw_runner::planned_driver_factory::default_planned_run_profile_resolver;
use ironclaw_turns::run_profile::{
    InMemoryLoopHostMilestoneSink, LoopCapabilityPort, LoopRequest, LoopRunContext,
    ProviderToolCall, RegisterProviderToolCallRequest, RunProfileResolutionRequest,
    RunProfileResolver,
};
use ironclaw_turns::{
    GetRunStateRequest, RunProfileRequest, TurnOriginKind, TurnStateStore, TurnStatus,
};
use serde_json::json;

/// Distinctive enough that a false-positive match against another scenario's
/// trigger name (e.g. `scenario_verbs_lifecycle`'s `"t0-triggers-once"`) is
/// not a concern.
const SELF_CREATE_ATTEMPT_TRIGGER_NAME: &str = "self-created-follow-up-should-not-exist";
const INTERACTIVE_CONTROL_TRIGGER_NAME: &str = "interactive-control-remains-scheduled";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // Control: an ordinary interactive caller can create a trigger. The same
    // record becomes the modify target below, so the triggered-origin pause
    // attempt proves both scope reachability and the origin-policy decision.
    let interactive = g
        .thread("conv-trigger-self-create-denied-interactive-control")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": INTERACTIVE_CONTROL_TRIGGER_NAME,
                    "prompt": "remain scheduled",
                    "schedule": {"kind": "once", "at": "2999-01-01T00:00:00", "timezone": "UTC"},
                }),
            ),
            RebornScriptedReply::text("created"),
        ])
        .build()
        .await?;
    interactive
        .submit_turn("create the control trigger")
        .await?;
    let control_trigger_id = interactive
        .tool_result_output("builtin.trigger_create")
        .await?["trigger"]["trigger_id"]
        .as_str()
        .ok_or("interactive trigger_create output missing trigger_id")?
        .to_string();

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

    // Capability-boundary regression: reconstruct the real loop run context
    // from the trusted-trigger run state, then call the production capability
    // port beneath the runner's model-surface deny decorator. Before the fix,
    // this path loses ScheduledTrigger lineage and both mutations succeed.
    // After the fix, the first-party trigger handler sees the typed origin and
    // rejects create and pause independently of model/provider/tool naming.
    let state = h
        .turn_state_store_for_test()
        .get_run_state(GetRunStateRequest {
            scope: submission.turn_scope.clone(),
            run_id: submission.run_id,
        })
        .await?;
    if state.product_context.as_ref().map(|ctx| ctx.origin)
        != Some(TurnOriginKind::ScheduledTrigger)
    {
        return Err("trusted-trigger run lost ScheduledTrigger product context".into());
    }
    let resolver = default_planned_run_profile_resolver()?;
    let resolved_profile = resolver
        .resolve_run_profile(
            RunProfileResolutionRequest::interactive_default().with_requested_run_profile(
                RunProfileRequest::new(state.resolved_run_profile_id.as_str())?,
            ),
        )
        .await?;
    let mut run_context = LoopRunContext::new(
        state.scope.clone(),
        state.turn_id,
        state.run_id,
        resolved_profile,
    )
    .with_accepted_message_ref(state.accepted_message_ref.clone());
    if let Some(actor) = state.actor.clone() {
        run_context = run_context.with_actor(actor);
    }
    if let Some(model_route) = state.resolved_model_route.clone() {
        run_context = run_context.with_resolved_model_route(model_route);
    }
    if let Some(product_context) = state.product_context.clone() {
        run_context = run_context.with_product_context(product_context);
    }
    let capability_harness = g
        .capability_harness()
        .ok_or("trigger group must expose its capability harness")?;
    let raw_port = capability_harness
        .create_recording_capability_port(
            &run_context,
            &Arc::new(InMemoryLoopHostMilestoneSink::default()),
            None,
        )
        .await?;
    assert_capability_denied(
        &raw_port,
        "builtin.trigger_create",
        json!({
            "name": SELF_CREATE_ATTEMPT_TRIGGER_NAME,
            "prompt": "remind me again",
            "schedule": {"kind": "once", "at": "2999-01-02T00:00:00", "timezone": "UTC"},
        }),
    )
    .await?;
    assert_capability_denied(
        &raw_port,
        "builtin.trigger_pause",
        json!({"trigger_id": control_trigger_id}),
    )
    .await?;

    // #6520 consolidated the origin policy into the host authorize step, so
    // the port-level invocation record exists even for a denial; the handler
    // must still never have executed. Absence of a recorded capability RESULT
    // is the post-consolidation observable for "denied before dispatch" (the
    // trigger_list read below additionally proves no durable side effect).
    if h.tool_result_output("builtin.trigger_create").await.is_ok() {
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

async fn assert_capability_denied(
    port: &Arc<dyn LoopCapabilityPort>,
    capability_id: &str,
    arguments: serde_json::Value,
) -> HarnessResult<()> {
    let capability_id = CapabilityId::new(capability_id)?;
    let definition = port
        .tool_definitions()?
        .into_iter()
        .find(|definition| definition.capability_id == capability_id)
        .ok_or_else(|| format!("raw capability port did not surface {capability_id}"))?;
    let candidate = port
        .register_provider_tool_call(RegisterProviderToolCallRequest::new(
            ProviderToolCall::from_parts(
                "scripted-provider",
                "scripted-model",
                Some(format!("turn-{capability_id}")),
                format!("call-{capability_id}"),
                definition.name.as_str(),
                arguments,
            )?,
        ))
        .await?;
    let resolution = port
        .invoke_capability(LoopRequest {
            activity_id: candidate.activity_id,
            surface_version: candidate.surface_version,
            capability_id: candidate.capability_id,
            input_ref: candidate.input_ref,
            approval_resume: None,
            auth_resume: None,
        })
        .await?;
    match resolution {
        // #6520: a policy denial surfaces as the terminal typed
        // `Resolution::Denied` channel (not a failed `Done` verdict); pin the
        // redacted reason kind so a different denial class cannot pass.
        Resolution::Denied(denial)
            if denial.reason_kind == Some(ironclaw_host_api::DenyReason::PolicyDenied) =>
        {
            Ok(())
        }
        other => {
            Err(format!("scheduled-trigger origin must deny {capability_id}, got {other:?}").into())
        }
    }
}
