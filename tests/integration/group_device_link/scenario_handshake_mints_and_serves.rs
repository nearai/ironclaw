use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::harness::profiles::device_link::ScriptedDeviceLinkCall;
use super::reborn_support::harness::profiles::device_link::{
    LINKED_EXTENSION_ID, LINKED_VENDOR_ID, LINKED_WHOAMI_CAPABILITY_ID, LinkedFixtureHandles,
};
use super::reborn_support::reply::RebornScriptedReply;

/// This scenario's own actor — see the note at its first thread.
const HANDSHAKE_ACTOR_ID: &str = "device-link-handshake-actor";

pub async fn run(
    group: &RebornIntegrationGroup,
    handles: &LinkedFixtureHandles,
) -> HarnessResult<()> {
    // Deployment configuration and installation already happened: this group's
    // first (dependent) scenario does both, and admin configuration is
    // deployment-scoped and revision-gated, so re-writing it here would be a
    // conflicting second write rather than a no-op.
    // A distinct actor, so the account this handshake mints lands in its own
    // credential-owner scope. Group stores are shared: a second `telegram`
    // account under an actor the other scenarios also use would make their
    // dispatch-time selection ambiguous, which is a real product behavior
    // (`AccountSelectionRequired`) and not something to discover by ordering.
    // Deployment configuration is already in place (this group's first,
    // dependent scenario writes it, and it is deployment-scoped and
    // revision-gated).
    //
    // Order matters and matches the real user story: **link first, then
    // install**. The device-link flow resolves its adapter from the
    // deployment-wide active snapshot, so it needs no per-user install; the
    // per-user install, by contrast, gates on this user having a credential
    // for the extension's declared auth surface. Installing first would park
    // that turn on the very gate the link is there to satisfy.
    let linker = group
        .thread("conv-device-link-handshake")
        .with_actor_id(HANDSHAKE_ACTOR_ID)
        .script([RebornScriptedReply::text("ready to link")])
        .build()
        .await?;
    linker.submit_turn("ready to link").await?;

    // ── The handshake ─────────────────────────────────────────────────────
    let account = linker
        .link_device_through_product_auth(LINKED_VENDOR_ID, LINKED_EXTENSION_ID, "cloud-password")
        .await?;
    let actor = linker.binding.actor_user_id.clone();
    let channel_connection = group
        .channel_connection()
        .ok_or("device-link group has no production channel-connection service")?;
    if channel_connection
        .has_any_active_identity_binding(LINKED_VENDOR_ID, &actor)
        .await?
    {
        return Err(
            "personal device-link completion persisted a workspace-bot channel identity".into(),
        );
    }
    if channel_connection
        .caller_channel_connected(LINKED_EXTENSION_ID, &actor)
        .await?
    {
        return Err("personal device-link completion connected the Telegram bot channel".into());
    }

    // PROPOSAL §4.5's ownership pin, asserted on the account the production
    // mint actually produced. Every clause matters: a reusable account would
    // be reachable by every installed extension AND would survive uninstall
    // with its live vendor device authorization attached.
    if !account.is_linked_device() {
        return Err("a completed link must carry a live link revision".into());
    }
    if !account.linked_device_ownership_is_pinned() {
        return Err(format!(
            "the minted account is not ownership-pinned (ownership {:?}, owner {:?}, grants {:?})",
            account.ownership, account.owner_extension, account.granted_extensions
        )
        .into());
    }
    if account.owner_extension.as_ref().map(|id| id.as_str()) != Some(LINKED_EXTENSION_ID) {
        return Err("the minted account is owned by the wrong extension".into());
    }

    // The adapter's session write during the handshake reached custody rather
    // than the fail-closed store: this is what the profile header used to
    // record as impossible.
    let custody = handles.device_link.custody_outcomes();
    if !custody.iter().any(|outcome| outcome == "ok") {
        return Err(format!(
            "the handshake never persisted a session through custody: {custody:?}"
        )
        .into());
    }

    // ── The link makes the extension installable, and its tools usable ────
    //
    // The per-user install gates on this user holding a credential for the
    // extension's declared auth surface; the link is what supplies it.
    let installer = group
        .thread("conv-device-link-handshake-install")
        .with_actor_id(HANDSHAKE_ACTOR_ID)
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                serde_json::json!({ "extension_id": LINKED_EXTENSION_ID }),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await?;
    installer.submit_turn("install telegram for me").await?;
    installer
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    if channel_connection
        .has_any_active_identity_binding(LINKED_VENDOR_ID, &actor)
        .await?
        || channel_connection
            .caller_channel_connected(LINKED_EXTENSION_ID, &actor)
            .await?
    {
        return Err(
            "installing personal-account tools created a workspace-bot channel binding".into(),
        );
    }

    //
    // A tool call now resolves the caller to the account the handshake minted,
    // through the host's bind-time resolver. The fixture adapter echoes the
    // grant it was handed, so this asserts the identity actually flowed rather
    // than that a call merely succeeded.
    let caller = group
        .thread("conv-device-link-handshake-call")
        .with_actor_id(HANDSHAKE_ACTOR_ID)
        .script([
            RebornScriptedReply::tool_call(LINKED_WHOAMI_CAPABILITY_ID, serde_json::json!({})),
            RebornScriptedReply::text("read the linked account"),
        ])
        .build()
        .await?;
    caller.submit_turn("who am i on telegram").await?;
    caller
        .assert_tool_invoked(LINKED_WHOAMI_CAPABILITY_ID)
        .await?;
    caller
        .assert_tool_result_contains(account.id.to_string().as_str())
        .await?;
    if !handles
        .device_link
        .calls()
        .iter()
        .any(|call| matches!(call, ScriptedDeviceLinkCall::Finalize(_)))
    {
        return Err("completed Telegram link did not finalize its vendor session".into());
    }

    // Remove through the product lifecycle, which must invoke the same
    // disconnect coordinator the extensions page uses: vendor/device revoke
    // first, then target and identity cleanup, before the installation is
    // removed. This runs last in the group because removal is deployment-wide.
    let remover = group
        .thread("conv-device-link-handshake-remove")
        .with_actor_id(HANDSHAKE_ACTOR_ID)
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_remove",
                serde_json::json!({ "extension_id": LINKED_EXTENSION_ID }),
            ),
            RebornScriptedReply::text("removed"),
        ])
        .build()
        .await?;
    remover.submit_turn("remove telegram for me").await?;
    remover
        .assert_tool_invoked("builtin.extension_remove")
        .await?;
    if channel_connection
        .caller_channel_connected(LINKED_EXTENSION_ID, &actor)
        .await?
    {
        return Err("Telegram remained connected after extension removal".into());
    }
    if channel_connection
        .has_any_active_identity_binding(LINKED_VENDOR_ID, &actor)
        .await?
    {
        return Err("Telegram identity binding survived extension removal".into());
    }
    if !handles
        .device_link
        .calls()
        .iter()
        .any(|call| matches!(call, ScriptedDeviceLinkCall::Revoke(_)))
    {
        return Err(
            "extension removal cleared local state without revoking the linked device".into(),
        );
    }

    Ok(())
}
