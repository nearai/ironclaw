//! Regression coverage for bug #5416: `builtin.extension_search` must NOT render
//! the model-visible "already configured or active … do not ask the user for
//! credentials" message for an extension that is `Enabled` in lifecycle state
//! but has NO credential account configured — and must STILL render it for an
//! extension that is legitimately ready (no credential required).
//!
//! Root cause (`extension_lifecycle.rs::search_installation_phase`): once an
//! installation's `activation_state()` maps to `LifecyclePhase::Active`, the
//! function returns that phase immediately — `search_credentials_configured`
//! (which consults the `RuntimeExtensionActivationCredentialGate`) is only ever
//! called for the `LifecyclePhase::Installed` case. An `Enabled` installation
//! therefore never has its credential state checked, so
//! `extension_search_has_ready_result` (which looks at `installation_phase` +
//! the always-cleared `credential_requirements` field — see
//! `suppress_search_credential_onboarding`) reports it as ready regardless of
//! whether a credential account exists.
//!
//! `builtin.extension_activate` cannot reach `Enabled` without a configured
//! credential account for a credentialed extension (it raises `AuthRequired`
//! first). So each buggy case installs through the real `builtin.extension_install`
//! tool call (install needs no credentials) and then flips JUST the activation
//! state to `Enabled` directly on the shared installation store via
//! `set_activation_state` (E-EXTSTORE seam) — bypassing the credential gate,
//! with NO credential account created anywhere.
//!
//! ## Coverage — not just Google OAuth
//!
//! | Extension    | Credential kind        | Expected ready message |
//! |--------------|------------------------|------------------------|
//! | `gmail`      | Google OAuth           | ABSENT (needs auth)    |
//! | `github`     | GitHub OAuth (≠ google)| ABSENT (needs auth)    |
//! | `notion`     | Notion OAuth + **MCP** | ABSENT (needs auth)    |
//! | `web-access` | none required          | PRESENT (control)      |
//!
//! The three credentialed cases pin the bug (message must be absent). The
//! `web-access` control pins that the fix does NOT over-suppress a genuinely
//! ready, credential-free extension.
//!
//! Shared-store preconditions from earlier scenarios in this group:
//! `github` installed (Scenario 1), `notion` removed (Scenario 2), `web-access`
//! Enabled (Scenario 3), `gmail` untouched. This scenario seeds/installs
//! accordingly and does not disturb their recorded assertions.
//!
//! SHAPE NOTE (Phase 0 of the #5416 fix, LANDED): the fix replaced the
//! model-facing `installation_phase` field on the search summary with a
//! collapsed `availability` field. The `assert_active_in_result` guards below
//! now assert `"availability":"needs_auth"` to prove the seeded `Enabled`
//! state actually reached the search result as a credentialed-but-unauthenticated
//! extension — without that guard a failed seed would leave the extension
//! `Installed`, which is NOT the buggy Active branch, and the message-absence
//! assertion would pass vacuously.

use super::reborn_support::builder::RebornIntegrationHarness;
use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_extensions::{ExtensionActivationState, ExtensionInstallationId};
use serde_json::json;

const READY_MESSAGE_MARKER: &str = "already configured or active";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let mut bugs: Vec<String> = Vec::new();

    // ── gmail — Google OAuth, fresh install then seed Enabled ───────────────
    install_extension(g, "gmail", "ready-cred-gmail-install").await?;
    seed_enabled(g, "gmail").await?;
    let gmail = search(g, "gmail", "ready-cred-gmail-view").await?;
    gmail.assert_tool_result_contains("\"gmail\"").await?;
    assert_active_in_result(&gmail).await?; // SHAPE-COUPLED (see module note)
    if ready_message_present(&gmail).await {
        bugs.push("gmail (Google OAuth)".into());
    }

    // ── github — GitHub OAuth (different provider). Scenario 1 already
    //    installed it; seed Enabled directly on the shared store. ────────────
    seed_enabled(g, "github").await?;
    let github = search(g, "github", "ready-cred-github-view").await?;
    github.assert_tool_result_contains("\"github\"").await?;
    assert_active_in_result(&github).await?; // SHAPE-COUPLED
    if ready_message_present(&github).await {
        bugs.push("github (GitHub OAuth)".into());
    }

    // ── notion — Notion OAuth + MCP. Scenario 2 removed it; re-install. ─────
    install_extension(g, "notion", "ready-cred-notion-install").await?;
    seed_enabled(g, "notion").await?;
    let notion = search(g, "notion", "ready-cred-notion-view").await?;
    notion.assert_tool_result_contains("\"notion\"").await?;
    assert_active_in_result(&notion).await?; // SHAPE-COUPLED
    if ready_message_present(&notion).await {
        bugs.push("notion (Notion OAuth / MCP)".into());
    }

    // ── web-access CONTROL — no credential required, already Enabled by
    //    Scenario 3. The ready message SHOULD be present; the fix must not
    //    over-suppress a genuinely ready, credential-free extension. ─────────
    let web = search(g, "web-access", "ready-cred-web-view").await?;
    web.assert_tool_result_contains("\"web-access\"").await?;
    if !ready_message_present(&web).await {
        bugs.push(
            "CONTROL web-access: ready message wrongly ABSENT (over-suppression of a \
             credential-free ready extension)"
                .into(),
        );
    }

    if !bugs.is_empty() {
        return Err(format!(
            "bug #5416: `builtin.extension_search` mis-reported credential readiness for: {bugs:?}. \
             Credentialed Enabled-but-unauthenticated extensions must NOT be reported \"already \
             configured or active / do not ask the user for credentials\", and a credential-free \
             ready extension must still be. `search_installation_phase` must consult the credential \
             gate for `LifecyclePhase::Active`, not only `LifecyclePhase::Installed`."
        )
        .into());
    }
    Ok(())
}

/// Install an extension through the real `builtin.extension_install` tool call
/// (needs no credentials; writes the manifest + `Installed` installation row).
async fn install_extension(
    g: &RebornIntegrationGroup,
    extension_id: &str,
    conversation: &str,
) -> HarnessResult<()> {
    let installer = g
        .thread(conversation)
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({ "extension_id": extension_id }),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer
        .submit_turn(&format!("install {extension_id}"))
        .await?;
    installer
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    Ok(())
}

/// Flip an already-installed extension straight to `Enabled` on the shared
/// installation store (E-EXTSTORE seam) — the only way to construct the buggy
/// Active-but-uncredentialed state, since `builtin.extension_activate` gates on
/// credentials for a credentialed extension. No credential account is created.
async fn seed_enabled(g: &RebornIntegrationGroup, extension_id: &str) -> HarnessResult<()> {
    let store = g
        .capability_harness()
        .ok_or("extension_lifecycle group missing HostRuntime capability harness")?
        .extension_installation_store_for_test()
        .ok_or("harness missing extension installation store (E-EXTSTORE seam)")?;
    let installation_id = ExtensionInstallationId::new(extension_id)?;
    store
        .set_activation_state(&installation_id, ExtensionActivationState::Enabled)
        .await?;
    Ok(())
}

/// Run `builtin.extension_search` for `query` on a fresh thread over the shared
/// store and return the viewer harness for result assertions.
async fn search(
    g: &RebornIntegrationGroup,
    query: &str,
    conversation: &str,
) -> HarnessResult<RebornIntegrationHarness> {
    let viewer = g
        .thread(conversation)
        .script([
            RebornScriptedReply::tool_call("builtin.extension_search", json!({ "query": query })),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    viewer.submit_turn(&format!("search {query}")).await?;
    viewer
        .assert_tool_invoked("builtin.extension_search")
        .await?;
    Ok(viewer)
}

/// Whether the model-visible "already configured or active … do not ask for
/// credentials" ready message appears in the search tool result.
async fn ready_message_present(viewer: &RebornIntegrationHarness) -> bool {
    viewer
        .assert_tool_result_contains(READY_MESSAGE_MARKER)
        .await
        .is_ok()
}

/// Non-vacuity guard that the seeded `Enabled` state actually reached the
/// search result (the buggy branch only fires for an Active installation).
///
/// Post-Phase-0: an `Enabled`-but-uncredentialed extension projects
/// `availability: needs_auth` — see module-level SHAPE NOTE.
async fn assert_active_in_result(viewer: &RebornIntegrationHarness) -> HarnessResult<()> {
    viewer
        .assert_tool_result_contains(r#""availability":"needs_auth""#)
        .await
}
