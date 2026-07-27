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
//!
//! Phase 2 pins the catalog-disclosure half of the same arm: install resolves
//! the model-chosen id against the available catalog before mutating state and
//! returns one fixed generic diagnostic. The caller-supplied id must not be
//! interpolated into that diagnostic, which would disclose catalog details and
//! create an untrusted-text provenance obligation that this path does not need.

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

    model_chosen_extension_id_is_not_reflected_by_catalog_error(g).await
}

/// The fixed catalog-safe reason returned for every unknown extension id.
const GENERIC_NOT_FOUND: &str = "available extension was not found";

/// Phase 2: `builtin.extension_install` with a model-chosen `extension_id`
/// that is itself credential vocabulary (`api_key` — a
/// `CREDENTIAL_MARKERS` entry, and a legal `ExtensionId`: lowercase ASCII plus
/// `_`). Catalog resolution must return the same fixed diagnostic as every
/// other unknown id, rather than reflecting the caller-controlled value.
async fn model_chosen_extension_id_is_not_reflected_by_catalog_error(
    g: &RebornIntegrationGroup,
) -> HarnessResult<()> {
    let h = g
        .thread("ext-install-adversarial-id")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "api_key"}),
            ),
            RebornScriptedReply::text("no such extension"),
        ])
        .build()
        .await?;
    h.submit_turn("install the api_key extension").await?;
    h.assert_tool_invoked("builtin.extension_install").await?;

    h.assert_conversation_history_lacks("extension api_key is not installed")
        .await
        .map_err(|error| {
            format!("the catalog error must not interpolate the MODEL-CHOSEN extension_id: {error}")
        })?;
    h.assert_conversation_history_contains(GENERIC_NOT_FOUND)
        .await
        .map_err(|error| {
            format!("the fixed generic catalog diagnostic must reach history: {error}")
        })?;
    Ok(())
}
