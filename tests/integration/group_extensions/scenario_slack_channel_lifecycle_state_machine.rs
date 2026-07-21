//! Scenario 6 (C-SLACK-LIFECYCLE, issue #6105): the Slack channel-extension
//! lifecycle STATE MACHINE — install → activate → connect → use → disconnect
//! (via `builtin.extension_remove`, the production path: there is no separate
//! disconnect route) → reconnect → reinstall/activate → use again — with
//! surface consistency asserted after every transition.
//!
//! The three surfaces that disagreed in issue #6091 are each read through
//! their REAL implementation:
//! - **connection state**: the production `SlackChannelConnectionFacade`
//!   over durable host-state identity bindings (what the Extensions page
//!   merges via `caller_channel_connections`), plus the binding store itself
//!   as durable evidence;
//! - **capability surface**: whether a scripted `slack.*` call actually
//!   dispatches through the model gateway + capability port;
//! - **lifecycle phase**: `builtin.extension_search`'s `installation_phase`.
//!
//! Key distinctions this pins (the #6091 divergence axes):
//! - activation publishes tools but does NOT connect (installed ≠ connected);
//! - removal runs the REAL personal-connection cleanup — connection state,
//!   durable bindings, lifecycle phase, and tool dispatchability must ALL
//!   flip together, not drift;
//! - a reconnect (new OAuth epoch) + reinstall restores service end to end.
//!
//! Uses the group's Slack channel-connection bundle
//! (`ironclaw_reborn_composition::test_support`), whose connect mirrors the
//! `slack_personal` OAuth callback and whose facade is late-bound into the
//! same cleanup slot `extension_remove` dispatches to.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

/// Mirrors the `slack_personal` scope union the extension-lifecycle profile
/// seeds (`profiles/extension.rs`): read scopes for `search_messages` plus
/// `chat:write` for `send_message`.
const SLACK_PERSONAL_SCOPES: &[&str] = &[
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

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let slack = g
        .slack_channel_connection()
        .ok_or("extension_lifecycle group must carry the Slack channel-connection bundle")?;
    // Direct-chat bindings resolve subject == actor, so this one identity is
    // both the capability dispatch user and the authenticated actor removal
    // cleanup disconnects.
    let actor = g.canonical_actor_user();

    // ── Phase 0: nothing installed, nothing connected ────────────────────────
    if slack.caller_channel_connected(&actor).await? {
        return Err("slack must not report connected before any OAuth connect".into());
    }
    if slack.has_any_active_identity_binding(&actor).await? {
        return Err("no active slack identity binding may exist before connect".into());
    }

    // ── Phase 1: install + activate publishes tools, but does NOT connect ───
    let lifecycle = g
        .thread("slack-lifecycle-install")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("slack ready"),
        ])
        .build()
        .await?;
    // Activation's credential gate and dispatch-time staging select a
    // `slack_personal` account under the capability dispatch scope.
    lifecycle
        .seed_capability_credential_account("slack_personal", "itest slack", SLACK_PERSONAL_SCOPES)
        .await?;
    lifecycle.submit_turn("install and activate slack").await?;
    lifecycle
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    lifecycle
        .assert_tool_result_contains("\"activated\":true")
        .await?;
    // Activation published the tool surface…
    lifecycle
        .assert_tool_result_contains(r#""slack.send_message""#)
        .await?;
    // …but activation is NOT connection (the #6091 distinction).
    if slack.caller_channel_connected(&actor).await? {
        return Err("slack must not report connected after activate alone; \
             activation published tools without any OAuth connect"
            .into());
    }

    // ── Phase 2: connect (OAuth-callback-shaped) flips connection state ─────
    slack.connect_personal_user(&actor, "U-ITEST-ALPHA").await?;
    if !slack.caller_channel_connected(&actor).await? {
        return Err("slack must report connected after the personal OAuth connect".into());
    }
    if !slack.has_any_active_identity_binding(&actor).await? {
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
    remover.submit_turn("remove slack").await?;
    remover
        .assert_tool_result_contains("\"removed\":true")
        .await?;
    // Every surface flips together: connection facade…
    if slack.caller_channel_connected(&actor).await? {
        return Err("slack still reports connected after extension_remove; \
             removal did not run the personal-connection cleanup (issue #6091 shape)"
            .into());
    }
    // …durable identity bindings (tombstoned out of the active state)…
    if slack.has_any_active_identity_binding(&actor).await? {
        return Err("extension_remove must deactivate durable slack identity bindings".into());
    }
    // …and the lifecycle phase seen by extension_search (fresh viewer thread
    // so earlier `"activated":true` results can't satisfy the negative check).
    let viewer = g
        .thread("slack-lifecycle-viewer-after-remove")
        .script([
            RebornScriptedReply::tool_call("builtin.extension_search", json!({"query": "slack"})),
            RebornScriptedReply::text("searched catalog"),
        ])
        .build()
        .await?;
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

    // ── Phase 5: use after remove — the identical call no longer dispatches ─
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

    // ── Phase 6: reconnect (new epoch) + reinstall/activate restores service ─
    // A real reconnect is a fresh OAuth grant: new credential, new epoch.
    let restorer = g
        .thread("slack-lifecycle-restore")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("slack restored"),
        ])
        .build()
        .await?;
    restorer
        .seed_capability_credential_account(
            "slack_personal",
            "itest slack reconnect",
            SLACK_PERSONAL_SCOPES,
        )
        .await?;
    slack.connect_personal_user(&actor, "U-ITEST-ALPHA").await?;
    if !slack.caller_channel_connected(&actor).await? {
        return Err("slack must report connected after reconnect; \
             a completed disconnect must not fence out a NEW connection epoch (issue #6092 shape)"
            .into());
    }
    restorer.submit_turn("reinstall and activate slack").await?;
    restorer
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    restorer
        .assert_tool_result_contains("\"activated\":true")
        .await?;

    // ── Phase 7: use again — the same call that was rejected now dispatches ─
    caller.submit_turn("send the slack message again").await?;
    caller.assert_tool_invoked("slack.send_message").await?;
    caller.assert_reply_contains("sent slack message").await?;

    Ok(())
}
