//! A channel's descriptor-declared surface must remain discoverable through
//! `builtin.extension_search` without exposing UI-only connection chrome to
//! the model.
//!
//! This is intentionally a caller-level test. Both the catalog projection and
//! the account-setup registry are compiled from the resolved manifest; checking
//! either in isolation would not catch drift in the sanitized model-visible
//! projection.

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
async fn extension_search_preserves_channel_kind_without_connection_chrome() {
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
        "extension_search must still identify Telegram as a channel: {telegram}"
    );
    assert!(
        telegram.get("channel_connection").is_none(),
        "model-visible extension_search must omit UI-only connection chrome: {telegram}"
    );

    let rendered = telegram.to_string().to_ascii_lowercase();
    assert!(
        !rendered.contains("/pair"),
        "model-visible extension_search must not expose unsupported /pair guidance: {telegram}"
    );
    assert!(
        !rendered.contains("get the pairing code from")
            && !rendered.contains("get the pairing code"),
        "model-visible extension_search must not expose UI-only pairing instructions: {telegram}"
    );
}
