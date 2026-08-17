//! Caller-level regression for the model-visible `builtin.extension_search`
//! channel-connection contract (#6618): Telegram's linked-device flow must not
//! regress to the retired generated proof-code recipe. Its channel and linked
//! tools remain discoverable while device-link configuration stays on the
//! extension's authenticated setup surface.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[tokio::test]
async fn extension_search_omits_retired_proof_code_guidance_for_linked_device_telegram() {
    let group = RebornIntegrationGroup::extension_delivery()
        .await
        .expect("extension-delivery group builds with the Telegram manifest");
    let search = group
        .thread("channel-connection-projection")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_search",
                json!({"query": "telegram"}),
            ),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await
        .expect("search thread builds");

    search
        .submit_turn("how do I connect Telegram?")
        .await
        .expect("extension search completes");

    let output = search
        .tool_result_output("builtin.extension_search")
        .await
        .expect("extension_search result");
    let telegram = output["payload"]["extensions"]
        .as_array()
        .expect("extensions array")
        .iter()
        .find(|entry| entry["package_ref"]["id"] == "telegram")
        .unwrap_or_else(|| panic!("Telegram catalog result in {output}"));
    assert!(
        telegram["surface_kinds"]
            .as_array()
            .is_some_and(|kinds| kinds.iter().any(|kind| kind == "channel")),
        "model-visible search must still identify Telegram as a channel: {telegram}"
    );
    assert!(
        telegram["channel_connection"].is_null(),
        "linked-device Telegram must not advertise the retired proof-code recipe: {telegram}"
    );
    assert!(
        telegram["visible_capability_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id == "telegram.whoami")),
        "linked-account tools must remain model-visible: {telegram}"
    );
}
