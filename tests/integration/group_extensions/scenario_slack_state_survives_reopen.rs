//! Scenario 8 (T5 of issue #6105; the #4556/#2939/#1327 failure shapes):
//! RESTART SURVIVAL of connected-channel state. After the Slack lifecycle
//! scenario (scenario 6) leaves Slack reconnected and active, this reopens
//! the durable stores at the same on-disk local-dev `storage_root` through
//! FRESH handles — independent of every live `Arc` the running group holds —
//! and asserts the state a process restart must reconstruct is actually
//! there:
//! - the active Slack identity binding (the durable "connected" evidence),
//!   read through a fresh `FilesystemSlackHostState` composed exactly the way
//!   production boot composes it;
//! - the Slack extension installation record (the durable "installed"
//!   evidence).
//!
//! Deliberately NOT claimed: the in-memory capability publication surviving —
//! production re-publishes tools on (re)activation over this same durable
//! state, and scenario 6's reinstall arm already pins that path. This
//! scenario pins the durable half a restart depends on; full process-restart
//! coverage belongs to the #6106 boot/upgrade gates.
//!
//! Ordering: depends on scenario 6 (connected end state); driven after it in
//! `main.rs`.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let slack = g
        .slack_channel_connection()
        .ok_or("extension_lifecycle group must carry the Slack channel-connection bundle")?;
    let actor = g.canonical_actor_user();
    let capability_harness = g
        .capability_harness()
        .ok_or("extension_lifecycle group always uses HostRuntime")?;
    let storage_root = capability_harness.storage_root_for_test();

    // Live-handle sanity: scenario 6 must have left Slack connected, or the
    // reopen assertions below would be vacuous.
    if !slack.caller_channel_connected(&actor).await? {
        return Err(
            "precondition: slack must still be connected after the lifecycle scenario".into(),
        );
    }

    // Durable "connected": the active identity binding reads back through a
    // FRESH host-state store over a FRESH root filesystem at the same
    // storage_root — the state a restarted process would reconstruct.
    if !slack
        .has_active_identity_binding_after_reopen(&storage_root, &actor)
        .await?
    {
        return Err(
            "the active slack identity binding must survive an independent \
             store reopen; a restart would come up disconnected (#4556/#2939 shape)"
                .into(),
        );
    }
    // Non-vacuity control: the same reopened probe must NOT report a binding
    // for a user that never connected.
    let stranger = ironclaw_host_api::UserId::new("reopen-probe-stranger")
        .map_err(|error| error.to_string())?;
    if slack
        .has_active_identity_binding_after_reopen(&storage_root, &stranger)
        .await?
    {
        return Err(
            "reopened binding probe reported a binding for a user that never connected; \
             the reopen read is not scoped correctly"
                .into(),
        );
    }

    // Durable "installed": the slack installation record reads back through a
    // fresh installation store at the same root.
    let installations = ironclaw_reborn_composition::test_support::open_local_dev_extension_installation_store_for_test(
        &storage_root,
    )
    .await?
    .list_installations()
    .await?;
    if !installations
        .iter()
        .any(|installation| installation.extension_id().as_str() == "slack")
    {
        return Err(
            "the slack installation record must survive an independent store reopen".into(),
        );
    }

    Ok(())
}
