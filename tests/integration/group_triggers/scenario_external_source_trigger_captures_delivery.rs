//! RED journey: creating a trigger in an external product conversation must
//! preserve a host-validated route for the eventual result back to that
//! conversation.
//!
//! `RebornIntegrationGroup` turns enter through the real product-adapter
//! envelope and persist a sealed `reply_target_binding_ref` on the run. The
//! model deliberately omits `delivery_target_id`, reproducing the normal user
//! request that currently creates an automation whose fires are visible only
//! in WebUI. The assertion is on the persisted trigger, not model prose.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::{GetRunStateRequest, TurnStateStore};
use serde_json::json;

const ONCE_AT: &str = "2999-01-01T00:00:00";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let creator = g
        .thread("external-source-trigger-captures-delivery")
        .script([
            RebornScriptedReply::text("ready"),
            RebornScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": "external-source-delivery-red",
                    "prompt": "summarize the latest BTC news",
                    "schedule": {
                        "kind": "once",
                        "at": ONCE_AT,
                        "timezone": "UTC"
                    }
                }),
            ),
            RebornScriptedReply::text("scheduled"),
        ])
        .build()
        .await?;

    // Establish the external conversation first, then expose that exact
    // host-sealed reply binding through the caller-owned outbound registry.
    // The trigger-creation turn below never sends this value to the model.
    let setup_run_id = creator.submit_turn("hello from the source chat").await?;
    let setup_run = creator
        .turn_state_store_for_test()
        .get_run_state(GetRunStateRequest {
            scope: creator.turn_scope.clone(),
            run_id: setup_run_id,
        })
        .await?;
    g.register_source_delivery_target_for_test(
        "external-source-trigger-captures-delivery",
        "external:test-source-chat",
        setup_run.reply_target_binding_ref,
    )?;

    let run_id = creator
        .submit_turn("send the latest BTC news back here later")
        .await?;
    let source_run = creator
        .turn_state_store_for_test()
        .get_run_state(GetRunStateRequest {
            scope: creator.turn_scope.clone(),
            run_id,
        })
        .await?;
    let created = creator.tool_result_output("builtin.trigger_create").await?;
    let persisted_target = created["trigger"]["delivery_target_id"].as_str();

    if persisted_target.is_none() {
        return Err(format!(
            "trigger_create silently dropped the originating product reply target {}; \
             a future fire has no host-owned route back to the source channel: {created}",
            source_run.reply_target_binding_ref.as_str(),
        )
        .into());
    }
    Ok(())
}
