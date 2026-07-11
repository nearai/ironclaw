//! A filesystem-discovered generic `ExternalChannel` extension installs and
//! removes through the real lifecycle capabilities without a Slack connection
//! facade. The fixture is copied before `build_reborn_services`, so discovery,
//! install, removal, and durable-store read-back all use production wiring.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_extensions::ExtensionInstallationId;
use ironclaw_host_api::ExtensionId;
use serde_json::json;

const EXTENSION_ID: &str = "channel-ext";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let installer = g
        .thread("ext-generic-channel-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": EXTENSION_ID}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer
        .submit_turn("install the generic external channel")
        .await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;

    let remover = g
        .thread("ext-generic-channel-remove")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({"extension_id": EXTENSION_ID}),
            ),
            RebornScriptedReply::text("removed"),
        ])
        .build()
        .await?;
    remover
        .submit_turn("remove the generic external channel")
        .await?;
    remover
        .assert_tool_invoked("builtin.extension_remove")
        .await?;
    remover
        .assert_tool_result_contains("\"removed\":true")
        .await?;

    let capability_harness = g
        .capability_harness()
        .ok_or("extension lifecycle group must use the host-runtime harness")?;
    let storage_root = capability_harness.storage_root_for_test();
    let package_root = storage_root.join("system/extensions").join(EXTENSION_ID);
    if package_root.exists() {
        return Err(format!(
            "removed generic channel package still exists at {}",
            package_root.display()
        )
        .into());
    }

    let store = ironclaw_reborn_composition::test_support::open_local_dev_extension_installation_store_for_test(
        &storage_root,
    )
    .await?;
    let extension_id = ExtensionId::new(EXTENSION_ID)?;
    if store.get_manifest(&extension_id).await?.is_some() {
        return Err("removed generic channel manifest record still exists".into());
    }
    let installation_id = ExtensionInstallationId::new(EXTENSION_ID)?;
    if store.get_installation(&installation_id).await?.is_some() {
        return Err("removed generic channel installation record still exists".into());
    }

    Ok(())
}
