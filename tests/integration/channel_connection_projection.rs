//! Caller-level regression for the model-visible `builtin.extension_search`
//! channel-connection contract (#6618, #7715): Telegram advertises generated
//! code pairing for the workspace bot while its personal tools remain visible
//! and independently protected by device-link setup.

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
async fn extension_search_separates_telegram_bot_pairing_from_personal_tools() {
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
    assert_eq!(
        telegram["channel_connection"]["strategy"], "web_generated_code",
        "Telegram must advertise the workspace-bot pairing ceremony independently: {telegram}"
    );
    assert!(
        telegram["visible_capability_ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id == "telegram.whoami")),
        "linked-account tools must remain model-visible: {telegram}"
    );
}
