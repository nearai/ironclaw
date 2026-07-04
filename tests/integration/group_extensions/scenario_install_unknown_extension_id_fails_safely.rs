//! Scenario 4 (W4-EXT-MANIFEST-ERR, narrowed): `builtin.extension_install`
//! with an `extension_id` that is not in the bundled catalog fails safely with
//! a model-visible tool error, instead of panicking or silently no-oping.
//!
//! `builtin.extension_install`'s only input is `extension_id: String`, resolved
//! against a FIXED, compile-time-embedded catalog
//! (`AvailableExtensionCatalog::resolve`, `available_extensions.rs`) — there is
//! no live path for a model/user to submit raw manifest TOML through this
//! capability (every bundled manifest is asset-embedded and always valid), so
//! the originally-scoped "schema mismatch / reserved id / forbidden trust
//! level" arms (`ironclaw_extensions::v2::ManifestV2Error` variants) are not
//! reachable through `extension_install` in production. The one genuinely
//! reachable, wired error arm through this capability is an unknown
//! `extension_id`: `catalog.resolve` returns
//! `ProductWorkflowError::InvalidBindingRequest`, which
//! `extension_lifecycle_capabilities.rs::lifecycle_error` maps to
//! `RuntimeDispatchErrorKind::InputEncode`, rendered by the executor as the
//! `"invalid_input"` reason token — a `Failed` (not `Denied`) capability
//! outcome, distinct from the `"installed":true` success path Scenario 1
//! already covers.

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

    // The capability outcome is `Failed{invalid_input}` — proved as a *class*
    // (not a needle-prefix convention): the same reason string can in
    // principle render under either class, so asserting the class
    // discriminates structurally, not just by text convention.
    h.assert_tool_error(ToolErrorClass::Failed, "invalid_input")
        .await?;

    // Discriminating negative arm: the SAME reason token under the OTHER
    // class must be absent, proving `assert_tool_error`'s class argument is
    // load-bearing here rather than a convention `assert_tool_result_contains`
    // would also satisfy.
    h.assert_no_tool_error(ToolErrorClass::Denied, "invalid_input")
        .await?;

    Ok(())
}
