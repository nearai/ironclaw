//! Scenario 4 (W4-EXT-MANIFEST-ERR, narrowed): `builtin.extension_install`
//! with an `extension_id` not in the bundled catalog fails safely with a
//! model-visible tool error, instead of panicking or silently no-oping.
//!
//! `extension_id` resolves against a fixed, compile-time-embedded catalog
//! (`AvailableExtensionCatalog::resolve`) — every bundled manifest is
//! asset-embedded and always valid, so the originally-scoped schema/reserved-id/
//! trust-level `ManifestV2Error` arms are unreachable through this capability in
//! production. The one reachable arm is an unknown `extension_id`:
//! `catalog.resolve` returns `InvalidBindingRequest`, mapped by
//! `extension_lifecycle_capabilities.rs::lifecycle_error` to
//! `RuntimeDispatchErrorKind::InputEncode` — the `"invalid_input"` reason token,
//! a `Failed` (not `Denied`) outcome distinct from Scenario 1's success path.

use super::reborn_support::assertions::ToolErrorClass;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("ext-install-unknown-id")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "not-a-real-bundled-extension"}),
            ),
            RebornScriptedReply::text("could not install that extension"),
        ])
        .build()
        .await?;
    h.submit_turn("install the not-a-real-bundled-extension extension")
        .await?;
    h.assert_tool_invoked("builtin.extension_install").await?;

    // Proved as a *class* (not a needle-prefix convention): the same reason
    // string could in principle render under either class.
    h.assert_tool_error(ToolErrorClass::Failed, "invalid_input")
        .await?;

    // Discriminating negative arm: the same reason token under the OTHER class
    // must be absent, proving the class argument is load-bearing here.
    h.assert_no_tool_error(ToolErrorClass::Denied, "invalid_input")
        .await?;

    Ok(())
}
