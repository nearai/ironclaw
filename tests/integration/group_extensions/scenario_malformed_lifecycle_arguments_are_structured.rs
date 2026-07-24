//! RED: malformed lifecycle-tool arguments must preserve field-level repair
//! information through the real capability and model-observation path.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("ext-install-malformed-arguments")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                // The JSON schema only requires a string. An empty id reaches
                // the strong lifecycle id validator, so this catches the
                // handler-to-observation mapping rather than the upstream
                // schema validator's already-structured type mismatch path.
                json!({"extension_id": ""}),
            ),
            RebornScriptedReply::text("correct the extension id"),
        ])
        .build()
        .await?;
    h.submit_turn("install this extension with malformed arguments")
        .await?;
    h.assert_tool_invoked("builtin.extension_install").await?;
    h.assert_conversation_history_lacks("the tool input could not be encoded")
        .await?;
    h.assert_conversation_history_contains(r#""path":"extension_id""#)
        .await?;
    h.assert_conversation_history_contains(r#""code":"invalid_value""#)
        .await
}
