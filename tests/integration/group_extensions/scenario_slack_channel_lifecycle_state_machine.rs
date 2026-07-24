//! Scenario 6 (C-SLACK-LIFECYCLE, issue #6105): the Slack channel-extension
//! lifecycle STATE MACHINE — install/setup → connect → use →
//! disconnect (via `builtin.extension_remove`, the production path: there is
//! no separate disconnect route) → reinstall/reconfigure → reconnect → use
//! again — with surface consistency asserted after every transition,
//! re-expressed onto the unified generic extension runtime.
//!
//! The three surfaces that disagreed in issue #6091 are each read through
//! their REAL implementation:
//! - **connection state**: the production `GenericChannelConnectionFacade`
//!   (extension-runtime §6.4) over the durable channel-identity store (what
//!   the Extensions page merges via `caller_channel_connections`), plus the
//!   binding store itself as durable evidence;
//! - **capability surface**: whether a scripted `slack.*` call actually
//!   dispatches through the model gateway + capability port;
//! - **lifecycle phase**: `builtin.extension_search`'s `installation_phase`.
//!
//! Key distinctions this pins (the #6091 divergence axes):
//! - setup completion publishes tools but does NOT connect (membership ≠ connected);
//! - removal runs the REAL per-caller channel disconnect — connection state,
//!   durable bindings, lifecycle phase, and tool dispatchability must ALL
//!   flip together, not drift;
//! - a reinstall + fresh reconnect restores service end to end, with no
//!   stale-binding carryover from before the removal.
//!
//! Uses the group's generic channel-connection bundle
//! (`ironclaw_reborn_composition::test_support`), whose connect drives the
//! production OAuth-callback identity-binding hook
//! (`bind_channel_identities_for_callback`) and whose facade is late-bound
//! into the same cleanup slot `extension_remove` dispatches to.
//!
//! Divergences from the retired per-vendor (slack-host-beta) expression of
//! this scenario, forced by the generic architecture:
//! - **configure is a real phase**: the generic identity bind fails closed
//!   until the extension's administrator-configuration connection-scoping values
//!   (`slack_team_id` / `slack_api_app_id`) are configured through the
//!   production configure port, so this scenario configures them explicitly
//!   (the retired lane carried them in test-bundle config instead);
//! - **connect requires an installed extension**: the generic hook binds by
//!   discovering installed channel extensions, so the post-removal reconnect
//!   happens AFTER reinstall (the retired lane's binding service was
//!   installation-config-carried and could bind while uninstalled);
//! - **no connection epochs**: the retired lane fenced reconnects with
//!   `SlackConnectionEpoch`; the generic store has no epoch vocabulary — the
//!   equivalent pin is that removal deletes the binding outright, so a fresh
//!   reconnect succeeds with no stale-state conflict (`bind_user_identity`
//!   would reject a binding held by a different user);
//! - **bindings are deleted, not tombstoned**: `has_any_active_identity_binding`
//!   reads record absence rather than tombstone state.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_auth::OAuthProviderIdentity;
use ironclaw_host_api::UserId;
use ironclaw_reborn_composition::test_support::ChannelConnectionTestBundle;
use ironclaw_secrets::SecretMaterial;
use serde_json::json;

const SLACK_BOT_TOKEN: &str = "xoxb-itest-bot-token";
const SLACK_SIGNING_SECRET: &str = "itest-slack-signing-secret";
const SLACK_INSTALLATION_ID: &str = "slack";
const SLACK_BOT_USER_ID: &str = "U-BOT";
const SLACK_OAUTH_CLIENT_ID: &str = "slack-oauth-client";
const SLACK_OAUTH_CLIENT_SECRET: &str = "slack-oauth-secret";

/// Mirrors the slack manifest's `[[tools.credentials]]` scope union
/// (`crates/ironclaw_first_party_extensions/assets/slack/manifest.toml`):
/// read scopes shared by every tool plus `chat:write` for `send_message`.
const SLACK_SCOPES: &[&str] = &[
    "search:read",
    "channels:history",
    "groups:history",
    "im:history",
    "mpim:history",
    "channels:read",
    "groups:read",
    "im:read",
    "mpim:read",
    "users:read",
    "chat:write",
];

/// The administrator-configuration connection-scoping claims this scenario configures
/// and the proven OAuth identity must match (fail-closed on mismatch).
const SLACK_TEAM_ID: &str = "T-ITEST";
const SLACK_API_APP_ID: &str = "A-ITEST";

/// The proven vendor identity of a successful personal OAuth grant
/// (`authed_user.id` plus the workspace/app claims the token exchange
/// extracts through the manifest's `[auth.slack.identity]` pointers).
fn proven_slack_identity() -> Result<OAuthProviderIdentity, String> {
    OAuthProviderIdentity::new(
        "U-ITEST-ALPHA",
        Some(SLACK_TEAM_ID.to_string()),
        None,
        Some(SLACK_API_APP_ID.to_string()),
    )
    .map_err(|error| format!("proven slack identity: {error}"))
}

/// Seed Slack's tenant-owned connection-scoping values through the composed
/// administrator configuration service. Product callers can perform this
/// mutation only through the operator-authorized Admin Configuration API; the
/// direct seam used here is compiled for integration test support only.
async fn configure_slack_connection_scoping(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let services = g
        .capability_harness()
        .and_then(|harness| harness.reborn_services_for_test())
        .ok_or("extension_lifecycle group must expose its RebornServices bundle")?;
    services
        .configure_admin_group_for_test(
            "extension.slack",
            vec![
                ("slack_bot_token".to_string(), SLACK_BOT_TOKEN.to_string()),
                (
                    "slack_signing_secret".to_string(),
                    SLACK_SIGNING_SECRET.to_string(),
                ),
                ("slack_team_id".to_string(), SLACK_TEAM_ID.to_string()),
                ("slack_api_app_id".to_string(), SLACK_API_APP_ID.to_string()),
                (
                    "slack_installation_id".to_string(),
                    SLACK_INSTALLATION_ID.to_string(),
                ),
                (
                    "slack_bot_user_id".to_string(),
                    SLACK_BOT_USER_ID.to_string(),
                ),
                (
                    "slack_oauth_client_id".to_string(),
                    SLACK_OAUTH_CLIENT_ID.to_string(),
                ),
                (
                    "slack_oauth_client_secret".to_string(),
                    SLACK_OAUTH_CLIENT_SECRET.to_string(),
                ),
            ],
        )
        .await
        .map_err(|error| format!("slack admin configuration failed: {error}"))?;
    Ok(())
}

fn register_slack_channel_egress_credentials(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let services = g
        .capability_harness()
        .and_then(|harness| harness.reborn_services_for_test())
        .ok_or("extension_lifecycle group must expose its RebornServices bundle")?;
    if !services.register_static_channel_egress_credentials_for_test(vec![(
        "slack".to_string(),
        "slack_bot_token".to_string(),
        SecretMaterial::from(SLACK_BOT_TOKEN.to_string()),
    )]) {
        return Err("the composed runtime must expose channel-egress credential bridging".into());
    }
    Ok(())
}

async fn wait_for_slack_connected(
    slack: &ChannelConnectionTestBundle,
    actor: &UserId,
    label: &str,
) -> HarnessResult<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if slack.caller_channel_connected("slack", actor).await? {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("slack must report connected after {label}").into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn assert_slack_installation_phase(
    g: &RebornIntegrationGroup,
    expected_phase: &str,
    label: &str,
) -> HarnessResult<()> {
    let viewer = g
        .thread(format!("slack-lifecycle-viewer-{label}"))
        .script([
            RebornScriptedReply::tool_call("builtin.extension_search", json!({"query": "slack"})),
            RebornScriptedReply::text("searched slack lifecycle"),
        ])
        .build()
        .await?;
    let prompt = format!("search slack {label}");
    viewer.submit_turn(&prompt).await?;
    viewer
        .assert_tool_invoked("builtin.extension_search")
        .await?;
    let output = viewer
        .tool_result_output("builtin.extension_search")
        .await?;
    let output = output.to_string();
    if !output.contains("\"slack\"") {
        return Err(format!("slack search result missing slack entry: {output}").into());
    }
    let expected = format!(r#""installation_phase":"{expected_phase}""#);
    if !output.contains(&expected) {
        return Err(format!(
            "slack search result missing expected phase {expected_phase}: {output}"
        )
        .into());
    }
    Ok(())
}

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let slack = g
        .channel_connection()
        .ok_or("extension_lifecycle group must carry the channel-connection bundle")?;
    // Direct-chat bindings resolve subject == actor, so this one identity is
    // both the capability dispatch user and the authenticated actor removal
    // cleanup disconnects.
    let actor = g.canonical_actor_user();

    // ── Phase 0: nothing installed, nothing connected ────────────────────────
    if slack.caller_channel_connected("slack", &actor).await? {
        return Err("slack must not report connected before any OAuth connect".into());
    }
    if slack
        .has_any_active_identity_binding("slack", &actor)
        .await?
    {
        return Err("no active slack identity binding may exist before connect".into());
    }

    // ── Phase 1: install auto-publishes tools, but does NOT connect ─────
    let lifecycle = g
        .thread("slack-lifecycle-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("slack ready"),
        ])
        .build()
        .await?;
    // Install reconciliation's credential gate and dispatch-time staging select a slack
    // account under the capability dispatch scope.
    lifecycle
        .seed_capability_credential_account("slack", "itest slack", SLACK_SCOPES)
        .await?;
    eprintln!("SLACK-LIFECYCLE PHASE1-install begin");
    lifecycle.submit_turn("install slack").await?;
    lifecycle
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;
    lifecycle
        .assert_model_message_content_contains(r#"\"installed\":true"#)
        .await?;
    lifecycle
        .assert_model_message_content_contains(r#"\"phase\":\"active\""#)
        .await?;
    // Reaching the active lifecycle state published the tool surface…
    lifecycle
        .assert_tool_result_contains(r#""slack.send_message""#)
        .await?;
    // Activation published the lifecycle phase and tool surface.
    assert_slack_installation_phase(g, "active", "after-activate").await?;
    // …and the §6.5 configure step binds the connection-scoping values the
    // generic identity bind validates proven identities against.
    eprintln!("SLACK-LIFECYCLE PHASE1b-configure begin");
    configure_slack_connection_scoping(g).await?;
    register_slack_channel_egress_credentials(g)?;
    // …but neither activation nor configuration is connection (the #6091
    // distinction).
    if slack.caller_channel_connected("slack", &actor).await? {
        return Err(
            "slack must not report connected after install + configure alone; \
             activation published tools without any OAuth connect"
                .into(),
        );
    }

    // ── Phase 2: connect (OAuth-callback-shaped) flips connection state ─────
    slack
        .connect_provider_user(&actor, "slack", proven_slack_identity()?)
        .await?;
    wait_for_slack_connected(&slack, &actor, "the personal OAuth connect").await?;
    if !slack
        .has_any_active_identity_binding("slack", &actor)
        .await?
    {
        return Err("connect must persist an active slack identity binding".into());
    }

    // ── Phase 3: use — a slack.* call dispatches through the real port ──────
    let caller = g
        .thread("slack-lifecycle-caller")
        .script([
            RebornScriptedReply::tool_call(
                "slack.search_messages",
                json!({"query": "from:me lifecycle"}),
            ),
            RebornScriptedReply::text("searched slack"),
            RebornScriptedReply::tool_call(
                "slack.send_message",
                json!({"channel": "C-ITEST", "text": "hello after remove?"}),
            ),
            RebornScriptedReply::text("slack unavailable"),
            RebornScriptedReply::tool_call(
                "slack.send_message",
                json!({"channel": "C-ITEST", "text": "hello again"}),
            ),
            RebornScriptedReply::text("sent slack message"),
        ])
        .build()
        .await?;
    eprintln!("SLACK-LIFECYCLE PHASE3-search begin");
    caller.submit_turn("search my slack messages").await?;
    caller.assert_tool_invoked("slack.search_messages").await?;
    caller.assert_reply_contains("searched slack").await?;

    // ── Phase 4: disconnect = extension_remove runs the REAL cleanup ────────
    let remover = g
        .thread("slack-lifecycle-remove")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("slack removed"),
        ])
        .build()
        .await?;
    eprintln!("SLACK-LIFECYCLE PHASE4-remove begin");
    remover.submit_turn("remove slack").await?;
    remover
        .assert_tool_invoked("builtin.extension_remove")
        .await?;
    // Every surface flips together: connection facade…
    if slack.caller_channel_connected("slack", &actor).await? {
        return Err("slack still reports connected after extension_remove; \
             removal did not run the per-caller channel disconnect (issue #6091 shape)"
            .into());
    }
    // …durable identity bindings (deleted by the generic disconnect)…
    if slack
        .has_any_active_identity_binding("slack", &actor)
        .await?
    {
        return Err("extension_remove must delete durable slack identity bindings".into());
    }
    // …and the lifecycle phase seen by extension_search (fresh viewer thread
    // so earlier `"phase":"active"` results can't satisfy the negative check).
    let viewer = g
        .thread("slack-lifecycle-viewer-after-remove")
        .script([
            RebornScriptedReply::tool_call("builtin.extension_search", json!({"query": "slack"})),
            RebornScriptedReply::text("searched catalog"),
        ])
        .build()
        .await?;
    eprintln!("SLACK-LIFECYCLE PHASE4c-viewer begin");
    viewer.submit_turn("search slack after removal").await?;
    if viewer
        .assert_tool_result_contains(r#""installation_phase":"active""#)
        .await
        .is_ok()
    {
        return Err(
            "slack still shows installation_phase:active after extension_remove; \
             lifecycle phase and connection state have drifted apart"
                .into(),
        );
    }
    // Non-vacuity: the catalog entry itself must still be discoverable.
    viewer.assert_tool_result_contains("\"slack\"").await?;
    viewer
        .assert_model_message_content_contains(r#"\"id\":\"slack\""#)
        .await?;

    // ── Phase 5: use after remove — the identical call no longer dispatches ─
    eprintln!("SLACK-LIFECYCLE PHASE5-send-after-remove begin");
    caller.submit_turn("send a slack message").await?;
    if caller
        .assert_tool_invoked("slack.send_message")
        .await
        .is_ok()
    {
        return Err("slack.send_message dispatched after extension_remove; \
             removal must unpublish the capability surface"
            .into());
    }
    caller.assert_reply_contains("slack unavailable").await?;

    // ── Phase 6: reinstall + reconfigure + reconnect restores service
    // A real reconnect is a fresh OAuth grant against a fresh install. The
    // generic identity bind discovers INSTALLED channel extensions
    // (fail-closed while slack is removed), so — unlike the retired
    // per-vendor lane, which could bind while uninstalled — the reconnect
    // runs after reinstall + reconfigure.
    let restorer = g
        .thread("slack-lifecycle-restore")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("slack restored"),
        ])
        .build()
        .await?;
    // Removal revoked the caller's personal slack credential; a reconnect
    // mints a fresh account, exactly like a fresh OAuth grant.
    restorer
        .seed_capability_credential_account("slack", "itest slack reconnect", SLACK_SCOPES)
        .await?;
    eprintln!("SLACK-LIFECYCLE PHASE6-reinstall begin");
    restorer.submit_turn("reinstall slack").await?;
    restorer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    restorer
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;
    restorer
        .assert_model_message_content_contains(r#"\"installed\":true"#)
        .await?;
    restorer
        .assert_model_message_content_contains(r#"\"phase\":\"active\""#)
        .await?;
    // Tenant administrator configuration survives a user's removal; only the
    // caller-owned membership and personal OAuth binding were cleared.
    slack
        .connect_provider_user(&actor, "slack", proven_slack_identity()?)
        .await?;
    wait_for_slack_connected(
        &slack,
        &actor,
        "reconnect; a completed removal must not fence out a NEW connection (issue #6092 shape)",
    )
    .await?;

    // ── Phase 7: use again — the same call that was rejected now dispatches ─
    eprintln!("SLACK-LIFECYCLE PHASE7-send-again begin");
    caller.submit_turn("send the slack message again").await?;
    caller.assert_tool_invoked("slack.send_message").await?;
    caller.assert_reply_contains("sent slack message").await?;

    Ok(())
}
