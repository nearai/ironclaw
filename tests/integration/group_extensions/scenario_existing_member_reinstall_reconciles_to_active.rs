//! The public Activate action is gone (#6520): `setup_needed -> active` is
//! reconciled by an EXISTING member re-entering the idempotent install action
//! after their personal setup completes (`extension_lifecycle.rs`'s
//! `Some(existing)` same-caller arm). This scenario drives that successor
//! path on the shared store — install, observe `setup_needed` cross-thread,
//! complete setup, re-install the SAME membership with no remove in between,
//! observe `active` cross-thread. It is also the only scenario that
//! positively observes the intermediate `setup_needed` phase at this tier;
//! both arms were retired alongside the Activate vocabulary.
//!
//! Runs as a DISTINCT actor (`with_actor_id`, E-MULTIUSER seam) so github is
//! uninstalled and un-credentialed for THIS caller regardless of Scenario 1's
//! default-actor github install on the same shared store — membership and
//! credentials are per-user (#5459 P1). That kills scenario-order coupling
//! both ways: earlier scenarios cannot pre-credential this caller, and this
//! caller's private membership stays invisible to the default actor.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const ACTOR: &str = "reconcile-member-actor";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    // ── Phase 1: first install for THIS caller — parks the normal
    // per-account credential gate; denial leaves the joined membership
    // resting at setup_needed (removal is the sole reset action). ───────────
    let installer = g
        .thread("ext-reconcile-phase-install")
        .with_actor_id(ACTOR)
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("github needs a credential"),
        ])
        .build()
        .await?;
    let (run_id, gate_ref) = installer
        .submit_turn_until_auth_blocked("install github")
        .await?;
    installer.deny_auth_gate(run_id, &gate_ref).await?;
    installer
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Completed)
        .await?;

    // ── Phase 2: cross-thread view — the membership positively reads
    // setup_needed. Only this caller's github entry can carry a phase (their
    // sole installation), so the value assert is entry-precise. ─────────────
    let pending_viewer = g
        .thread("ext-reconcile-phase-pending-viewer")
        .with_actor_id(ACTOR)
        .script([
            RebornScriptedReply::tool_call("builtin.extension_search", json!({"query": "github"})),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    pending_viewer
        .submit_turn("search github before setup")
        .await?;
    pending_viewer
        .assert_tool_invoked("builtin.extension_search")
        .await?;
    pending_viewer
        .assert_tool_result_contains(r#""installation_phase":"setup_needed""#)
        .await?;

    // ── Phase 3: personal setup completes out-of-band for this caller. ─────
    installer
        .seed_capability_credential_account("github", "itest github reconcile", &[])
        .await?;

    // ── Phase 4: the SAME member re-enters the idempotent install — the
    // existing-caller retry arm reconciles the completed setup to active
    // without any remove. ───────────────────────────────────────────────────
    let retrier = g
        .thread("ext-reconcile-phase-retry")
        .with_actor_id(ACTOR)
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("github reconciled"),
        ])
        .build()
        .await?;
    retrier.submit_turn("install github again").await?;
    retrier
        .assert_tool_invoked("builtin.extension_install")
        .await?;
    retrier
        .assert_tool_result_contains("\"installed\":true")
        .await?;
    retrier
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await?;

    // ── Phase 5: cross-thread view — the reconciliation propagated. ────────
    let active_viewer = g
        .thread("ext-reconcile-phase-active-viewer")
        .with_actor_id(ACTOR)
        .script([
            RebornScriptedReply::tool_call("builtin.extension_search", json!({"query": "github"})),
            RebornScriptedReply::text("searched"),
        ])
        .build()
        .await?;
    active_viewer
        .submit_turn("search github after setup")
        .await?;
    active_viewer
        .assert_tool_invoked("builtin.extension_search")
        .await?;
    active_viewer
        .assert_tool_result_contains(r#""installation_phase":"active""#)
        .await?;
    Ok(())
}
