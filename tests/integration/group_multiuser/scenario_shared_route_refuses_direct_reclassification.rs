//! Scenario: a channel conversation established through the Shared route (a
//! bot mention) refuses a later Direct-route probe of the same conversation,
//! and its run acts as the pinger.
//!
//! Ephemeral-per-ping redesign (replacing the retired shared-canonical-thread
//! scenario this file used to hold): there is no shared "canonical thread"
//! and no cross-user transcript continuity — each ping mints a fresh
//! pinger-owned thread, pinned at the conversations tier (`shared_event_thread`)
//! and the full-path channel e2e
//! (`slack_in_thread_mentions_each_run_in_their_own_thread_replying_in_the_vendor_thread`).
//! What remains uniquely pinned HERE, at the integration tier, are two live
//! guards the redesign keeps: (1) a run acts as the actor who pinged —
//! DISCRIMINATINGLY, by pinging the same conversation as a SECOND, distinct
//! actor and proving that run acts as the second actor, never the first binder
//! (owner == actor, so no first-binder ownership survives); and (2) a
//! conversation born on the Shared route cannot be re-addressed as somebody's
//! Direct DM (route re-classification is refused outright). Per-actor RESOURCE
//! isolation (memory, approvals, turn state, workspace) stays pinned by the
//! sibling scenarios.

use std::time::Duration;

use ironclaw_host_api::ids::UserId;
use ironclaw_host_api::turn::{TurnRunId, TurnScope, TurnStatus};
use ironclaw_product_contracts::binding::{ProductBindingResolver, ResolveBindingRequest};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::inbound::ProductInboundAck;
use ironclaw_turns::{GetRunStateRequest, TurnCoordinator};

use super::reborn_support::builder::{HARNESS_ACTOR_ID, binding_request};
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use super::reborn_support::test_adapter::RebornTestIngress;
use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;

const SHARED_CONVERSATION: &str = "conv-shared-channel";
const ACTOR_B: &str = "reborn-actor-b";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // Anchor harness: supplies the scenario's seams (the production per-thread
    // workflow, the shared coordinator, and owner-scoped thread reads) on a
    // plain direct conversation. The SHARED conversation below is driven
    // through this same production workflow instance — never through
    // harness-private state.
    let a = g.thread("conv-shared-route-anchor").build().await?;
    let ingress = RebornTestIngress::new("reborn-itest", "itest-install")
        .map_err(|e| format!("[ingress] {e}"))?;
    let binding_service = a.binding_service_for_test()?;

    // The pinger's Shared-route (bot mention) resolve establishes the
    // conversation binding and the acting identity the run must adopt.
    let envelope_probe = ingress.verified_text_envelope_with_trigger(
        "shared-probe-a",
        HARNESS_ACTOR_ID,
        SHARED_CONVERSATION,
        "binding probe a",
        ProductTriggerReason::BotMention,
    )?;
    let binding = binding_service
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope_probe))
        .await
        .map_err(|e| format!("[shared resolve] {e}"))?;

    // Script the pinger's thread scope: one turn. Owner == actor under
    // ephemeral-per-ping.
    let scope = TurnScope::new_with_owner(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        binding.thread_id.clone(),
        Some(binding.actor_user_id.clone()),
    );
    let _scripted_llm = g
        .register_scope_script_for_test(
            scope.clone(),
            "shared-route-thread",
            [RebornScriptedReply::text("reply-shared-alpha")],
        )
        .await?;

    // Turn: the pinger's mention, through the production surface. The run
    // acts as the pinger — never as a thread "owner" distinct from the actor.
    let workflow = a.product_surface_for_test();
    let ack = workflow
        .submit_inbound(ingress.verified_text_envelope_with_trigger(
            "shared-turn-a",
            HARNESS_ACTOR_ID,
            SHARED_CONVERSATION,
            "alpha started the deploy thread",
            ProductTriggerReason::BotMention,
        )?)
        .await
        .map_err(|e| format!("[submit] {e}"))?;
    let run = accepted_run_id(ack)?;
    let state = wait_for_completion(&a.turn_coordinator_for_test(), &scope, run).await?;
    assert_run_actor(&state, &binding.actor_user_id, "pinger")?;

    // Run-acts-as-invoker is DISCRIMINATING only with a SECOND, distinct actor:
    // the conversation was bound above by the first actor, so a run that adopts
    // the "first binder" (the retired owner-vs-actor scoping) would act as the
    // first actor. Ping the SAME conversation as a genuinely distinct actor and
    // prove that run acts as the SECOND actor — owner == actor, no first-binder
    // ownership survives.
    let envelope_b_probe = ingress.verified_text_envelope_with_trigger(
        "shared-probe-b",
        ACTOR_B,
        SHARED_CONVERSATION,
        "binding probe b",
        ProductTriggerReason::BotMention,
    )?;
    let binding_b = binding_service
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope_b_probe))
        .await
        .map_err(|e| format!("[shared resolve b] {e}"))?;
    if binding_b.actor_user_id == binding.actor_user_id {
        return Err("non-vacuity: the two pingers must resolve to distinct canonical users".into());
    }
    let scope_b = TurnScope::new_with_owner(
        binding_b.tenant_id.clone(),
        binding_b.agent_id.clone(),
        binding_b.project_id.clone(),
        binding_b.thread_id.clone(),
        Some(binding_b.actor_user_id.clone()),
    );
    let _scripted_llm_b = g
        .register_scope_script_for_test(
            scope_b.clone(),
            "shared-route-thread-b",
            [RebornScriptedReply::text("reply-shared-bravo")],
        )
        .await?;
    let ack_b = workflow
        .submit_inbound(ingress.verified_text_envelope_with_trigger(
            "shared-turn-b",
            ACTOR_B,
            SHARED_CONVERSATION,
            "bravo follows up in the channel",
            ProductTriggerReason::BotMention,
        )?)
        .await
        .map_err(|e| format!("[submit b] {e}"))?;
    let run_b = accepted_run_id(ack_b)?;
    let state_b = wait_for_completion(&a.turn_coordinator_for_test(), &scope_b, run_b).await?;
    assert_run_actor(&state_b, &binding_b.actor_user_id, "second actor")?;

    // A Direct-route probe of the SAME conversation is refused — a
    // conversation born on the Shared route may not be re-classified into
    // somebody's DM.
    let direct_probe = binding_service
        .resolve_binding(binding_request(
            &ingress.verified_text_envelope_with_trigger(
                "shared-direct-probe",
                HARNESS_ACTOR_ID,
                SHARED_CONVERSATION,
                "direct probe",
                ProductTriggerReason::DirectChat,
            )?,
        ))
        .await;
    match direct_probe {
        Err(ProductOperationFailure::BindingRequired { .. }) => {}
        other => {
            return Err(format!(
                "a Direct probe of the shared conversation must be refused, got {other:?}"
            )
            .into());
        }
    }

    Ok(())
}

fn accepted_run_id(ack: ProductInboundAck) -> HarnessResult<TurnRunId> {
    match ack {
        ProductInboundAck::Accepted {
            submitted_run_id, ..
        } => Ok(submitted_run_id),
        other => Err(format!("expected accepted inbound ack, got {other:?}").into()),
    }
}

/// Poll the group's shared coordinator for one run in the SHARED thread's
/// scope (the per-harness `wait_for_status` polls the harness's own anchor
/// scope, which is a different thread).
async fn wait_for_completion(
    coordinator: &std::sync::Arc<dyn TurnCoordinator>,
    scope: &TurnScope,
    run_id: TurnRunId,
) -> HarnessResult<ironclaw_turns::TurnRunState> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let state = coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await
            .map_err(|e| format!("[run state] {e}"))?;
        if state.status == TurnStatus::Completed {
            return Ok(state);
        }
        if state.status.is_terminal() {
            return Err(format!(
                "run reached terminal status {:?}; failure={:?}",
                state.status, state.failure
            )
            .into());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(
                format!("timed out waiting for completion; last={:?}", state.status).into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn assert_run_actor(
    state: &ironclaw_turns::TurnRunState,
    expected: &UserId,
    label: &str,
) -> HarnessResult<()> {
    let actor = state
        .actor
        .as_ref()
        .ok_or_else(|| format!("run {label} must record its acting identity"))?;
    if actor.user_id != *expected {
        return Err(format!(
            "run {label} must act as its invoker: acted as {}, expected {expected}",
            actor.user_id
        )
        .into());
    }
    Ok(())
}
