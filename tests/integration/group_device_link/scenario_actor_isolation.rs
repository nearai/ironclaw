//! Two users, two linked accounts: a second actor's call resolves the SECOND
//! actor's own linked account, never the first binder's.
//!
//! Driven end to end — scripted model tool call → `CapabilityHost` →
//! `ToolAdapter::invoke` — not at the auth selector seam, because the axis
//! under test is whether *dispatch* re-resolves the caller's linked account per
//! run owner. The adapter echoes the dispatching scope's user id into its
//! canonical output, so the assertion reads the identity the linked-account
//! adapter actually acted as, at the adapter boundary itself.
//!
//! The negative half of the claim — one user's link never unlocks a caller who
//! has none — is pinned by `scenario_link_call_unlink`'s unlink step, which
//! proves a caller without a live linked account parks before the adapter is
//! reached.

use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::harness::profiles::device_link::{
    LINKED_EXTENSION_ID, LINKED_LIST_CONVERSATIONS_CAPABILITY_ID, LINKED_VENDOR_ID,
    LinkedFixtureHandles,
};
use super::reborn_support::reply::RebornScriptedReply;

/// A distinct actor from the group's canonical binder.
const SECOND_ACTOR_ID: &str = "device-link-second-actor";

pub async fn run(
    group: &RebornIntegrationGroup,
    handles: &LinkedFixtureHandles,
) -> HarnessResult<()> {
    // ── Actor one (the group's canonical binder) links and reads ────────────
    let first = group
        .thread("conv-device-link-actor-one")
        .script([
            RebornScriptedReply::tool_call(LINKED_LIST_CONVERSATIONS_CAPABILITY_ID, json!({})),
            RebornScriptedReply::text("here are your chats"),
        ])
        .build()
        .await?;
    first
        .seed_capability_credential_account(LINKED_VENDOR_ID, "actor one linked account", &[])
        .await?;
    first.submit_turn("list my telegram chats").await?;
    first
        .assert_tool_invoked(LINKED_LIST_CONVERSATIONS_CAPABILITY_ID)
        .await?;
    let first_caller = last_caller(handles, LINKED_LIST_CONVERSATIONS_CAPABILITY_ID)?;

    // ── Actor two: their OWN installation and their OWN linked account ──────
    //
    // Membership and credentials are per-user, so the second actor installs the
    // package for themselves rather than inheriting the first actor's install.
    let second_install = group
        .thread("conv-device-link-actor-two-install")
        .with_actor_id(SECOND_ACTOR_ID)
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": LINKED_EXTENSION_ID}),
            ),
            RebornScriptedReply::text("linked account ready"),
        ])
        .build()
        .await?;
    // Non-vacuity: a regressed `with_actor_id` would collapse both callers onto
    // one owner and turn the comparison below into a tautology.
    if first.binding.actor_user_id == second_install.binding.actor_user_id {
        return Err("with_actor_id seam no-op: both callers resolved the same owner".into());
    }
    second_install
        .seed_capability_credential_account(LINKED_VENDOR_ID, "actor two linked account", &[])
        .await?;
    second_install
        .submit_turn("link my telegram account")
        .await?;
    second_install
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;

    let second = group
        .thread("conv-device-link-actor-two-call")
        .with_actor_id(SECOND_ACTOR_ID)
        .script([
            RebornScriptedReply::tool_call(LINKED_LIST_CONVERSATIONS_CAPABILITY_ID, json!({})),
            RebornScriptedReply::text("here are your chats"),
        ])
        .build()
        .await?;
    second.submit_turn("list my telegram chats").await?;
    second
        .assert_tool_invoked(LINKED_LIST_CONVERSATIONS_CAPABILITY_ID)
        .await?;
    let second_caller = last_caller(handles, LINKED_LIST_CONVERSATIONS_CAPABILITY_ID)?;

    if first_caller == second_caller {
        return Err(format!(
            "the second actor's dispatch must resolve its own linked account, but both resolved {first_caller}"
        )
        .into());
    }
    Ok(())
}

/// The dispatching caller of the most recent adapter invocation of `capability_id`.
fn last_caller(handles: &LinkedFixtureHandles, capability_id: &str) -> HarnessResult<String> {
    handles
        .tools
        .calls()
        .into_iter()
        .rev()
        .find(|(id, _)| id == capability_id)
        .map(|(_, caller)| caller)
        .ok_or_else(|| format!("no linked-account dispatch recorded for {capability_id}").into())
}
