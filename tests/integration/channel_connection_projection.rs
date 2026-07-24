//! Caller-level regression for the model-visible `builtin.extension_search`
//! channel-connection contract (#6618): a generated-code channel's setup
//! guidance IS model-visible (the model needs it to explain the next step),
//! while UI-only chrome — the static pairing failure copy — stays on the
//! display preview path and never reads as live state.

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
async fn extension_search_retains_generated_code_guidance_without_ui_failure_copy() {
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
    let connection = &telegram["channel_connection"];
    assert_eq!(
        connection["strategy"], "web_generated_code",
        "generated-code connection guidance must remain model-visible: {telegram}"
    );
    assert!(
        connection["instructions"]
            .as_str()
            .is_some_and(|instructions| instructions.contains("IronClaw pairing panel")),
        "manifest-authored connection guidance must survive catalog projection: {connection}"
    );
    assert_eq!(
        connection["error_message"], "",
        "static pairing failure copy is UI-only, not live model state: {connection}"
    );
}
