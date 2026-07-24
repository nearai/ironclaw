//! Caller-level regression: descriptor-declared channel connection guidance
//! must survive in the model-visible `builtin.extension_search` result.
//!
//! The model needs the strategy and instructions to avoid inventing a setup
//! flow, while browser input/action/error chrome stays on the display-preview
//! path.

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
async fn extension_search_preserves_guidance_and_omits_ui_only_connection_chrome() {
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
        "extension_search must preserve the descriptor's WebGeneratedCode strategy: {connection}"
    );
    assert!(
        connection["instructions"]
            .as_str()
            .is_some_and(|instructions| instructions.contains("IronClaw pairing panel")),
        "manifest-authored connection guidance must survive catalog projection: {connection}"
    );
    assert_eq!(
        connection["input_placeholder"], "",
        "model-visible search must clear browser input chrome: {connection}"
    );
    assert_eq!(
        connection["submit_label"], "",
        "model-visible search must clear browser action chrome: {connection}"
    );
    assert_eq!(
        connection["error_message"], "",
        "model-visible search must clear UI-only failure copy: {connection}"
    );

    let rendered = connection.to_string().to_ascii_lowercase();
    assert!(
        !rendered.contains("/pair"),
        "the generic connection contract must never invent an unsupported /pair command: {connection}"
    );
    assert!(
        !rendered.contains("get the pairing code from")
            && !rendered.contains("get the pairing code"),
        "WebGeneratedCode means IronClaw mints the code/deep link; the bot does not issue it: {connection}"
    );
}
