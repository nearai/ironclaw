//! W5-WEBUI-API-2: rename through the real WebUI automations route, then
//! list through the same facade to prove the trigger repository mutation is
//! scoped and persisted.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use super::reborn_support::webui_mount::{
    get_json, mount_webui_v2_router, post_json, webui_caller_for,
};
use axum::http::StatusCode;
use ironclaw_product_workflow::RebornServices;
use serde_json::json;
use std::sync::Arc;

const ONCE_AT: &str = "2999-06-02T00:00:00";
const TRIGGER_NAME: &str = "c-webui-automations-rename-trigger";
const RENAMED_TRIGGER_NAME: &str = "c-webui-automations-renamed-trigger";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-webui-automations-rename")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.trigger_create",
                json!({
                    "name": TRIGGER_NAME,
                    "prompt": "remind me once (webui automations rename check)",
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
    // The production capability port resolves the execution user from the
    // run's binding owner, so the trigger creator is the binding subject —
    // the default caller already matches it for the scoped automation facade.
    // (The pre-port harness dispatched under a fixed constructor user and
    // needed a caller override here.)
    let caller = webui_caller_for(&h.binding);
    let router = mount_webui_v2_router(Arc::new(services), caller);

    let rename_path = format!("/api/webchat/v2/automations/{trigger_id}");
    let (rename_status, rename_body) = post_json(
        router.clone(),
        &rename_path,
        json!({ "name": RENAMED_TRIGGER_NAME }),
    )
    .await;
    if rename_status != StatusCode::OK {
        return Err(format!(
            "expected 200 from automation rename, got {rename_status}: {rename_body}"
        )
        .into());
    }
    if rename_body["updated"] != true {
        return Err(format!("expected rename updated=true, got {rename_body}").into());
    }
    if rename_body["automation"]["name"] != RENAMED_TRIGGER_NAME {
        return Err(format!(
            "expected rename response name {RENAMED_TRIGGER_NAME:?}, got {rename_body}"
        )
        .into());
    }

    let (list_status, list_body) = get_json(router, "/api/webchat/v2/automations").await;
    if list_status != StatusCode::OK {
        return Err(format!(
            "expected 200 from automations LIST after rename, got {list_status}: {list_body}"
        )
        .into());
    }
    let automations = list_body["automations"]
        .as_array()
        .ok_or("automations response missing 'automations' array")?;
    let renamed = automations.iter().any(|automation| {
        automation["automation_id"] == trigger_id && automation["name"] == RENAMED_TRIGGER_NAME
    });
    if !renamed {
        return Err(format!(
            "expected automation_id {trigger_id:?} with renamed name in LIST response: {list_body}"
        )
        .into());
    }

    Ok(())
}
