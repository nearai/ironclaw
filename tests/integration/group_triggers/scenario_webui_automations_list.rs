//! W5-WEBUI-API-1: cold LIST over a real `RebornAutomationProductFacade`
//! wired from this group's shared, live trigger repository (Enabler B).
//! Reuses the group's ONE repository so the facade's real visibility-filter/
//! run-history-join logic is under test, not a hand-rolled double.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use super::reborn_support::webui_mount::{get_json, mount_webui_v2_router, webui_caller_for};
use axum::http::StatusCode;
use ironclaw_product_workflow::RebornServices;
use serde_json::json;
use std::sync::Arc;

const ONCE_AT: &str = "2999-06-02T00:00:00";
const TRIGGER_NAME: &str = "c-webui-automations-list-trigger";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-webui-automations-list")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": TRIGGER_NAME,
                    "prompt": "remind me once (webui automations list check)",
                    "schedule": {"kind": "once", "at": ONCE_AT, "timezone": "UTC"},
                }),
            ),
            RebornScriptedReply::text("created"),
        ])
        .build()
        .await?;
    h.submit_turn("create a one-time trigger").await?;

    let created = h.tool_result_output("builtin.trigger_create").await?;
    let trigger_id = created["trigger"]["trigger_id"]
        .as_str()
        .ok_or("trigger_create output missing trigger_id")?
        .to_string();

    let capability_harness = g
        .capability_harness()
        .ok_or("triggers group always uses HostRuntime")?;
    let trigger_repository = capability_harness
        .trigger_repository_for_test()
        .ok_or("triggers group harness missing a captured trigger repository")?;
    let facade =
        ironclaw_reborn_composition::test_support::local_dev_automation_product_facade_for_test(
            trigger_repository,
        );

    let services = RebornServices::new(h.thread_harness.service.clone(), h.coordinator.clone())
        .with_automation_product_facade(facade);
    // `triggers()` group's capability harness uses a fixed constructor user,
    // not the thread's binding subject — trigger creator_user_id is that
    // user, so the WebUI caller must match it for list_scoped_triggers's
    // caller-scoped filter to see the trigger.
    let mut caller = webui_caller_for(&h.binding);
    caller.user_id = capability_harness.user_id().clone();
    let router = mount_webui_v2_router(Arc::new(services), caller);

    let (status, body) = get_json(router, "/api/webchat/v2/automations").await;
    if status != StatusCode::OK {
        return Err(
            format!("expected 200 from cold automations LIST, got {status}: {body}").into(),
        );
    }
    let automations = body["automations"]
        .as_array()
        .ok_or("automations response missing 'automations' array")?;
    let found = automations
        .iter()
        .any(|automation| automation["automation_id"] == trigger_id);
    if !found {
        return Err(
            format!("expected automation_id {trigger_id:?} in cold LIST response: {body}").into(),
        );
    }
    Ok(())
}
