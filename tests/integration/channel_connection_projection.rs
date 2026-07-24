//! Caller-level regression: channel connection render guidance must not leak
//! into the model-visible `builtin.extension_search` result.
//!
//! The descriptor still identifies the package as a channel so the model can
//! reason about its surface, while WebUI-only setup copy stays on the display
//! preview path.

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
async fn extension_search_omits_ui_only_connection_copy_from_model_output() {
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
        telegram.get("channel_connection").is_none(),
        "model-visible search must omit UI-only connection guidance: {telegram}"
    );
}
