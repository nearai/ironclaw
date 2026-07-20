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
//! Phase 2 pins the PROVENANCE half of the same arm: `InvalidBindingRequest`'s
//! `reason` interpolates the MODEL-CHOSEN `extension_id` verbatim, so it must
//! ride the UNTRUSTED diagnostic channel. Routing it onto the trusted
//! host-remediation channel would stamp
//! `ObservationTrust::HostAuthored` on attacker-influenced text and thereby skip
//! the credential-vocabulary scan `ironclaw_threads` applies to untrusted
//! output.

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

    model_chosen_extension_id_is_not_labelled_host_authored(g).await
}

/// The exact string an UNTRUSTED diagnostic collapses to at the host_api
/// boundary. Its PRESENCE is the proof the text was scanned; the adversarial
/// id's absence is the proof it was not waved through as host-authored.
const DEGRADED_PLACEHOLDER: &str = "capability summary unavailable";

/// Phase 2: `builtin.extension_activate` with a model-chosen `extension_id`
/// that is itself credential vocabulary (`api_key` — a
/// `CREDENTIAL_MARKERS` entry, and a legal `ExtensionId`: lowercase ASCII plus
/// `_`). The reason interpolates it verbatim into "extension api_key is not
/// installed".
///
/// On the untrusted channel that whole string fails the `SafeSummary`
/// credential-vocabulary scan and collapses to the placeholder, which is
/// correct. On the trusted host-remediation channel the scan is skipped by
/// provenance and the attacker-influenced text reaches thread history intact —
/// the defect this pins.
async fn model_chosen_extension_id_is_not_labelled_host_authored(
    g: &RebornIntegrationGroup,
) -> HarnessResult<()> {
    let h = g
        .thread("ext-activate-adversarial-id")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "api_key"}),
            ),
            RebornScriptedReply::text("no such extension"),
        ])
        .build()
        .await?;
    h.submit_turn("activate the api_key extension").await?;
    h.assert_tool_invoked("builtin.extension_activate").await?;

    h.assert_conversation_history_lacks("extension api_key is not installed")
        .await
        .map_err(|error| {
            format!(
                "a reason interpolating the MODEL-CHOSEN extension_id must ride the untrusted \
                 diagnostic channel and be scanned, not be stamped HostAuthored and waved \
                 through: {error}"
            )
        })?;
    h.assert_conversation_history_contains(DEGRADED_PLACEHOLDER)
        .await
        .map_err(|error| {
            format!("the scanned-and-collapsed placeholder must be what reaches history: {error}")
        })?;
    Ok(())
}
