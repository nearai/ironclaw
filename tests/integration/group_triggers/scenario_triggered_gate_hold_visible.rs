//! RED regression for #5886: a trigger whose active fire is parked on a
//! `BlockedApproval` gate must expose a derived `active_hold` projection
//! (reason/since/elapsed_occurrences) on BOTH read surfaces — the WebUI
//! automations list and the `builtin.trigger_list` capability output — and
//! drop it once the run reaches a terminal status.

use std::sync::Arc;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use super::reborn_support::webui_mount::{get_json, mount_webui_v2_router, webui_caller_for};
use axum::http::StatusCode;
use chrono::Duration;
use ironclaw_host_api::CapabilityId;
use ironclaw_product::RebornServices;
use ironclaw_triggers::{ClaimDueFireOutcome, ClaimDueFireRequest, FireAcceptedRequest, TriggerId};
use ironclaw_turns::TurnStatus;
use serde_json::{Value, json};

const TRIGGER_NAME: &str = "c-triggered-gate-hold-visible";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // Every-minute cron (not Once) so elapsed-occurrence counting is
    // meaningful for the hold projection (#5886).
    let creator = g
        .thread("conv-triggered-hold-create")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": TRIGGER_NAME,
                    "prompt": "write the scheduled report",
                    "schedule": {"kind": "cron", "expression": "* * * * *", "timezone": "UTC"},
                }),
            ),
            RebornScriptedReply::text("created"),
        ])
        .build()
        .await?;
    creator.submit_turn("create an every-minute report").await?;
    let created = creator.tool_result_output("builtin.trigger_create").await?;
    let trigger_id = created["trigger"]["trigger_id"]
        .as_str()
        .ok_or("trigger_create output missing trigger_id")?
        .to_string();

    let capability_harness = g
        .capability_harness()
        .ok_or("triggers_with_gated_write always uses HostRuntime")?;

    // AskEachTime beats global auto-approve (#4776 precedence), so only the
    // write gates while the trigger verbs stay auto-approved.
    capability_harness
        .set_ask_each_time_override_for_test(
            &CapabilityId::new("builtin.write_file")?,
            creator.binding.tenant_id.clone(),
            creator.binding.actor_user_id.clone(),
        )
        .await?;

    // The triggered fire parks on a REAL approval gate; it stays unresolved
    // while both read surfaces are asserted below.
    let submission = creator
        .submit_triggered_turn_scripted(
            "write the scheduled report",
            [
                RebornScriptedReply::tool_call(
                    "builtin.write_file",
                    json!({"path": "/workspace/triggered-hold.txt", "content": "hold visible"}),
                ),
                RebornScriptedReply::text("report written after approval"),
            ],
        )
        .await?;
    let blocked = creator
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::BlockedApproval,
        )
        .await?;
    let gate_ref = blocked
        .gate_ref
        .ok_or("blocked triggered run missing gate ref")?;

    // Production fire bookkeeping on the SAME repo record (the harness submit
    // wire deliberately skips it): claim the due slot, then point
    // active_run_ref at the blocked run — the same repo calls as
    // `worker/due_fire.rs`.
    let repo = capability_harness
        .trigger_repository_for_test()
        .ok_or("harness missing a captured trigger repository")?;
    let tenant_id = creator.binding.tenant_id.clone();
    let parsed_trigger_id = TriggerId::parse(&trigger_id)?;
    let record = repo
        .get_trigger(tenant_id.clone(), parsed_trigger_id)
        .await?
        .ok_or("created trigger missing from repository")?;
    let fire_slot = record.next_run_at;
    let claimed = repo
        .claim_due_fire(ClaimDueFireRequest {
            tenant_id: tenant_id.clone(),
            trigger_id: parsed_trigger_id,
            fire_slot,
            now: fire_slot,
        })
        .await?;
    if !matches!(claimed, ClaimDueFireOutcome::Claimed(_)) {
        return Err(format!("expected the due slot to claim, got {claimed:?}").into());
    }
    repo.mark_fire_accepted(FireAcceptedRequest {
        tenant_id: tenant_id.clone(),
        trigger_id: parsed_trigger_id,
        fire_slot,
        run_id: submission.run_id,
        thread_id: submission.turn_scope.thread_id.clone(),
        submitted_at: fire_slot + Duration::seconds(1),
    })
    .await?
    .ok_or("mark_fire_accepted did not find the claimed record")?;

    // Surface 1 (#5886): the automations list entry must carry active_hold
    // while the fire is gate-parked.
    let facade =
        ironclaw_reborn_composition::test_support::local_dev_automation_product_facade_for_test(
            Arc::clone(&repo),
            Arc::clone(&g.shared.turn_store),
        );
    let services = RebornServices::new(
        creator.thread_harness.service.clone(),
        creator.coordinator.clone(),
    )
    .with_automation_product_facade(facade);
    let caller = webui_caller_for(&creator.binding);
    let router = mount_webui_v2_router(Arc::new(services), caller);
    let entry = automation_entry(router.clone(), &trigger_id).await?;
    assert_active_hold(&entry, "automations list entry")?;

    // Surface 2 (#5886): the trigger_list capability output must carry the
    // same active_hold object (second thread, shared repo, same scope).
    let lister = g
        .thread("conv-triggered-hold-list")
        .script([
            RebornScriptedReply::tool_call("builtin.trigger_list", json!({})),
            RebornScriptedReply::text("listed"),
        ])
        .build()
        .await?;
    lister.submit_turn("list my automations").await?;
    let listed = lister.tool_result_output("builtin.trigger_list").await?;
    let listed_entry = listed["triggers"]
        .as_array()
        .ok_or("trigger_list output missing triggers array")?
        .iter()
        .find(|t| t["trigger_id"] == json!(trigger_id))
        .cloned()
        .ok_or_else(|| format!("created trigger absent from trigger_list: {listed}"))?;
    assert_active_hold(&listed_entry, "trigger_list entry")?;

    // Resolve the gate. active_run_ref stays set (no cleanup poller in the
    // harness), so the honest post-state is: hold derivation sees a TERMINAL
    // run and omits active_hold entirely (#5886).
    creator
        .approve_gate_in_scope(&submission.turn_scope, submission.run_id, &gate_ref)
        .await?;
    creator
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            TurnStatus::Completed,
        )
        .await?;
    let entry_after = automation_entry(router, &trigger_id).await?;
    if entry_after.get("active_hold").is_some() {
        return Err(format!(
            "#5886: active_hold must be omitted once the active run is terminal: {entry_after}"
        )
        .into());
    }

    // Surface 2 post-completion: `trigger_list` must ALSO drop active_hold
    // once the run is terminal. A fresh thread (the earlier `lister` already
    // consumed its scripted replies at `build()`).
    let lister_after = g
        .thread("conv-triggered-hold-list-after")
        .script([
            RebornScriptedReply::tool_call("builtin.trigger_list", json!({})),
            RebornScriptedReply::text("listed again"),
        ])
        .build()
        .await?;
    lister_after.submit_turn("list my automations").await?;
    let listed_after = lister_after
        .tool_result_output("builtin.trigger_list")
        .await?;
    let listed_entry_after = listed_after["triggers"]
        .as_array()
        .ok_or("trigger_list output missing triggers array")?
        .iter()
        .find(|t| t["trigger_id"] == json!(trigger_id))
        .cloned()
        .ok_or_else(|| format!("created trigger absent from trigger_list: {listed_after}"))?;
    if listed_entry_after.get("active_hold").is_some() {
        return Err(format!(
            "#5886: trigger_list active_hold must be omitted once the active run is terminal: {listed_entry_after}"
        )
        .into());
    }

    Ok(())
}

/// GET the automations list and return this scenario's entry.
async fn automation_entry(router: axum::Router, trigger_id: &str) -> HarnessResult<Value> {
    let (status, body) = get_json(router, "/api/webchat/v2/automations").await;
    if status != StatusCode::OK {
        return Err(format!("expected 200 from automations LIST, got {status}: {body}").into());
    }
    body["automations"]
        .as_array()
        .ok_or("automations response missing 'automations' array")?
        .iter()
        .find(|automation| automation["automation_id"] == json!(trigger_id))
        .cloned()
        .ok_or_else(|| {
            format!("automation {trigger_id:?} absent from LIST response: {body}").into()
        })
}

/// The #5886 hold contract on one listed entry: an `active_hold` object with
/// reason "approval", a `since` timestamp, and an `elapsed_occurrences` count.
fn assert_active_hold(entry: &Value, surface: &str) -> HarnessResult<()> {
    let hold = entry.get("active_hold").ok_or_else(|| {
        format!("#5886: {surface} missing \"active_hold\" for a gate-parked fire: {entry}")
    })?;
    if hold["reason"] != json!("approval") {
        return Err(
            format!("#5886: {surface} active_hold.reason must be \"approval\": {entry}").into(),
        );
    }
    if hold.get("since").is_none() {
        return Err(format!("#5886: {surface} active_hold missing \"since\": {entry}").into());
    }
    if hold.get("elapsed_occurrences").is_none() {
        return Err(format!(
            "#5886: {surface} active_hold missing \"elapsed_occurrences\": {entry}"
        )
        .into());
    }
    Ok(())
}
