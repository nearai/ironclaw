//! Scenario 3 (HEADLINE): install an extension in thread A, ACTIVATE it in
//! thread B (a DIFFERENT conversation), and confirm thread C (yet another
//! conversation) observes the extension as ACTIVE — not merely installed — over
//! the shared store. This closes the `extension_activate` int-tier gap: install,
//! search, and remove already have cross-thread coverage; activation did not.
//!
//! Uses "web-access" (NOT "github"/"notion") for two reasons. First, it is the
//! only bundled extension that activates WITHOUT credentials / without raising an
//! auth gate, so activation reaches a SUCCESS result (`activated:true`) in this
//! harness rather than blocking on a credential gate. "github" et al. require
//! credentials and would return an auth gate (confirmed by the unit test
//! `local_dev_extension_activate_returns_auth_gate_for_missing_extension_credentials`
//! in `extension_lifecycle_capabilities.rs`). Second, it is untouched by Scenario
//! 1 ("github") and Scenario 2 ("notion"), so a fresh install→activate cycle here
//! observes a real lifecycle transition over the shared store rather than an
//! already-installed/activated no-op.
//!
//! Key behaviours asserted (from `extension_lifecycle.rs::commit_activation`): a
//! successful activate yields `"activated":true` plus a `visible_capability_ids`
//! array of the now-published capability ids; and `extension_search` renders a
//! ready, credential-free extension as `"availability":"available"`.
//!
//! Because all three conversations use different conversation IDs but the same
//! `Arc<HostRuntimeCapabilityHarness>`, asserting that thread C's search sees
//! `availability:available` proves cross-thread install+activate persistence:
//! an activate in thread B is durably visible to thread C.
//!
//! SHAPE NOTE (post-#5416 Phase 0): `availability` is the collapsed
//! `installed? × credential-readiness` projection — for a credential-free
//! extension like "web-access", `Installed` and `Active` BOTH project to
//! `available` (by design: activation vs mere installation is a lifecycle
//! detail the model does not need, see the 5416 plan §4.1). So
//! `extension_search`'s `availability` field can no longer discriminate
//! "installed but not yet activated" from "active" the way the old
//! `installation_phase` field did. To still prove that THREAD B's
//! **activation** specifically (not just thread A's install) is durably
//! visible to thread C, this scenario also reads the shared durable
//! installation store directly (the same E-EXTSTORE seam the credential-gap
//! regression test uses) and asserts `ExtensionActivationState::Enabled`.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_extensions::{ExtensionActivationState, ExtensionInstallationId};
use serde_json::json;

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Thread A: installer ─────────────────────────────────────────────────
    // Install "web-access" so there is an installed-but-inactive extension to
    // activate. The install persists to the shared HostRuntimeCapabilityHarness
    // filesystem so the activator thread sees it immediately.
    let installer = g
        .thread("ext-activate-phase-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "web-access"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer.submit_turn("install web-access").await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    // Confirm the install succeeded: output carries `"installed":true`.
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;

    // ── Thread B: activator (DIFFERENT conversation, SAME shared store) ──────
    // A distinct conversation_id → distinct binding/thread scope, but the same
    // `HostRuntimeCapabilityHarness`, so the activator can see and activate the
    // installation Thread A just wrote. "web-access" needs no credentials, so
    // activation reaches a SUCCESS result instead of an auth gate.
    let activator = g
        .thread("ext-activate-phase-activate")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "web-access"}),
            ),
            RebornScriptedReply::text("activated"),
        ])
        .build()
        .await?;
    activator.submit_turn("activate web-access").await?;
    activator
        .assert_tool_invoked("builtin.extension_activate")
        .await?;
    // Direct effect: activation succeeded → output carries `"activated":true`.
    // Assert the VALUE, not just the key, so an `activated:false` / auth-gate
    // outcome cannot satisfy this.
    activator
        .assert_tool_result_contains("\"activated\":true")
        .await?;
    // Capability surfaces: the activate payload's `visible_capability_ids` array
    // carries the now-published capability ids. `web-access.search` coming online
    // is the observable proof that activation published the extension's tool
    // surface (mere install does NOT publish capabilities).
    activator
        .assert_tool_result_contains(r#""web-access.search""#)
        .await?;

    // ── Thread C: viewer (DIFFERENT conversation, SAME shared store) ─────────
    // A third distinct conversation_id over the Arc-cloned store. Searching for
    // "web-access" must now report it ready — observing Thread A's install and
    // Thread B's activation across threads.
    let viewer = g
        .thread("ext-activate-phase-viewer")
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
        .submit_turn("search web-access after activation")
        .await?;
    viewer
        .assert_tool_invoked("builtin.extension_search")
        .await?;
    // Cross-thread install+activate persistence: the search result carries
    // `availability:"available"` because both Thread A's install and Thread B's
    // activation reached the shared store (web-access needs no credentials, so
    // `available` fires as soon as it's installed — see the module SHAPE NOTE).
    viewer
        .assert_tool_result_contains(r#""availability":"available""#)
        .await?;

    // Non-vacuity guard: "web-access" must still appear in the catalog search
    // result, proving the search actually ran and returned a catalog entry. The
    // presence of `availability:available` is therefore meaningful — not a
    // symptom of an empty or errored result.
    if viewer
        .assert_tool_result_contains("\"web-access\"")
        .await
        .is_err()
    {
        return Err(
            "non-vacuity guard failed: web-access catalog entry must appear in search results \
             after activation; the availability assertion would otherwise be vacuous"
                .into(),
        );
    }

    // `availability` cannot discriminate "installed" from "active" for a
    // credential-free extension (see module SHAPE NOTE), so read the shared
    // durable installation store directly to prove Thread B's ACTIVATION
    // specifically — not just Thread A's install — is durably visible: the
    // store must report `ExtensionActivationState::Enabled`, not `Installed`.
    let installation_id = ExtensionInstallationId::new("web-access")?;
    let store = g
        .capability_harness()
        .ok_or("extension_lifecycle group missing HostRuntime capability harness")?
        .extension_installation_store_for_test()
        .ok_or("harness missing extension installation store (E-EXTSTORE seam)")?;
    let installation = store
        .get_installation(&installation_id)
        .await?
        .ok_or("web-access installation missing from the shared store after activation")?;
    if installation.activation_state() != ExtensionActivationState::Enabled {
        return Err(format!(
            "web-access installation state is {:?}, not Enabled, after a cross-thread activate; \
             builtin.extension_activate did not durably advance the lifecycle through the shared store",
            installation.activation_state()
        )
        .into());
    }

    Ok(())
}
