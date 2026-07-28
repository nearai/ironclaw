//! Scenario 3 (HEADLINE): install a no-setup extension in thread A and
//! confirm thread B observes ACTIVE over the shared store. Installation owns
//! every internal readiness/publication checkpoint; no public Activate action
//! exists.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let installer = g
        .thread("ext-install-active-phase")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "web-access"}),
            ),
            RebornScriptedReply::text("installed and ready"),
        ])
        .build()
        .await?;
    installer.submit_turn("install web-access").await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    installer
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;
    installer
        .assert_tool_result_contains(r#""web-access.search""#)
        .await?;

    let viewer = g
        .thread("ext-install-active-phase-viewer")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_search",
                json!({"query": "web-access"}),
            ),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    viewer
        .submit_turn("search web-access after install")
        .await?;
    viewer
        .assert_tool_invoked("builtin.extension_search")
        .await?;
    viewer
        .assert_tool_result_contains(r#""installation_phase":"active""#)
        .await?;
    viewer
        .assert_model_message_content_contains(r#"\"id\":\"web-access\""#)
        .await?;
    viewer
        .assert_model_message_content_contains(r#"\"installation_phase\":\"active\""#)
        .await?;

    if viewer
        .assert_tool_result_contains(r#""installation_phase":"setup_needed""#)
        .await
        .is_ok()
    {
        return Err(
            "web-access rested at installation_phase:setup_needed after a no-setup install; \
             builtin.extension_install did not complete readiness"
                .into(),
        );
    }
    if viewer
        .assert_tool_result_contains("\"web-access\"")
        .await
        .is_err()
    {
        return Err("non-vacuity guard failed: web-access must remain discoverable".into());
    }

    Ok(())
}
