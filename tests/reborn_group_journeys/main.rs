//! C-JOURNEY — multi-turn Reborn journeys: the deterministic twins of the live
//! canary use cases that chain a gate → resume → next turn on ONE
//! conversation/harness. Distinct from `reborn_group_approvals` /
//! `reborn_integration_auth_gate` (single-gate mechanics): the value here is the
//! CHAINING across turns and the cross-permutation matrix below.
//!
//! ## Permutation matrix (inbound source × gate class × outcome)
//!
//! | scenario                              | inbound       | gate(s)                       | outcome        |
//! |---------------------------------------|---------------|-------------------------------|----------------|
//! | interactive_approval_journey          | interactive   | approval → approval           | approve, deny, follow-up |
//! | auth_then_approval_journey            | interactive   | (approval→auth) → approval    | approve+resolve, approve, follow-up |
//! | auth_deny_then_retry_journey          | interactive   | (approval→auth) → (approval→auth) | approve+deny, approve+resolve |
//! | multi_actor_gate_isolation (IGNORED)  | interactive×2 | approval (A) / approval (B)   | blocked — see below |
//!
//! Shared turn-script helpers keep permutations from fanning out into N
//! near-identical fully-expanded scenarios (see each scenario module).
//!
//! Gate-arm discipline: a gated tool-call turn consumes exactly TWO script
//! entries (the call + one post-resume model call) regardless of approve/deny; a
//! plain follow-up turn consumes ONE.
//!
//! ## Auth→approval convergence (C-JOURNEY enabler)
//!
//! `auth_then_approval_journey` and `auth_deny_then_retry_journey` run on a
//! SECOND group, `RebornIntegrationGroup::live_auth_and_approval()` (built
//! from `HostRuntimeCapabilityHarness::file_and_github_auth_tools`), NOT
//! `live_approvals` — do not add them to the `live_approvals` group above.
//! The enabler: converge the auth gate onto the SAME `build_reborn_services`
//! runtime `live_approvals` already uses (unlike `live_auth_gate`, a separate,
//! lower-level `HostRuntimeServices` build with a hardcoded credential
//! resolver and no `run_state`/`approval_requests`/`capability_leases` stores
//! — see `reborn_integration_auth_gate.rs`'s deferred-arm note, now
//! superseded by this group for the happy-resume case). No GitHub credential
//! account is seeded at construction, so `github.get_repo` first raises a
//! real `BlockedApproval` (this harness's global auto-approve is disabled for
//! the file-tool arm, and that toggle is not capability-scoped, so it also
//! gates the WASM github capability); approving re-dispatches the
//! still-uncredentialed capability, which blocks AGAIN at a real
//! `TurnStatus::BlockedAuth`. `resolve_auth_gate` seeds a credential through
//! the REAL `ProductAuthRuntimeCredentialResolver` and resumes, letting the
//! SAME parked capability re-dispatch and complete. Making `github.*`
//! genuinely dispatchable on this runtime (not just granted/trusted) required
//! two additive `#[cfg(feature = "test-support")]` composition seams — see
//! `HostRuntimeCapabilityHarness::file_and_github_auth_tools`'s doc comment
//! (`tests/support/reborn/harness.rs`) for the mechanism (active-registry
//! publish + real asset-directory mount).
//!
//! ## Deferred / blocked permutations (findings — see the module doc of the
//! ## enabler notes and the lane report)
//!
//! - **Triggered-origin chained journey** (trigger fire → gate → resume →
//!   follow-up): `submit_triggered_turn` resolves the trigger to its OWN scope,
//!   for which no scripted model gateway is registered, so the triggered run
//!   fails benignly on a scope-miss BEFORE reaching any tool call — it cannot
//!   reach a gate on this base. The triggered path's single-turn origin coverage
//!   lives in `reborn_integration_triggered_submit` and the sibling triggered
//!   lane; a chained triggered journey needs a scripted-gateway-for-trigger-scope
//!   seam that does not exist here.
//! - **Multi-actor GATED journey** (`multi_actor_gate_isolation`, marked
//!   `#[ignore]`): a distinct actor's GATED capability turn dies with
//!   `driver_protocol_violation` because the capability harness on THIS base
//!   hardcodes ONE execution user (the group's canonical subject user), so actor
//!   B's gated dispatch is scoped to A's user and the approval-gate persistence
//!   mismatches B's run scope. Production isolates capability dispatch by run
//!   owner correctly; the missing piece is a HARNESS seam
//!   (`scope_capability_by_run_owner`) authored in the parallel C-MULTIUSER
//!   wave-3 lane and not yet merged to this base. Kept as a RED `#[ignore]`d
//!   test (`multi_actor_gate_isolation_blocked`) so the intended coverage is
//!   pinned and un-ignores the moment that seam lands. See `TODO(reborn-multiuser-gate)`.
//!   (Plain — non-gated — distinct-actor isolation already works and is covered
//!   by `reborn_group_multiuser::two_actors_own_threads`.)

#[allow(dead_code)]
#[path = "../support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

mod scenario_auth_deny_then_retry_journey;
mod scenario_auth_then_approval_journey;
mod scenario_interactive_approval_journey;
mod scenario_multi_actor_gate_isolation;

use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn journeys_group_e2e() {
    let g = RebornIntegrationGroup::live_approvals()
        .await
        .expect("group builds");

    let mut report = ScenarioReport::new();
    report.record(
        "interactive_approval_journey",
        scenario_interactive_approval_journey::run(&g).await,
    );
    report.assert_all_passed();
}

/// C-JOURNEY: auth→approval convergence journeys, on SEPARATE
/// `live_auth_and_approval` groups (not the `live_approvals` group above —
/// see the module-level "Auth→approval convergence" note).
///
/// ONE GROUP PER SCENARIO (not shared): `resolve_auth_gate` seeds a
/// `UserReusable` GitHub credential account under the group's canonical
/// `(tenant, user, agent, project)` scope — exactly like production, where a
/// submitted token persists for the user. On a SHARED group, scenario 1's
/// seeded credential would make scenario 2's `github.get_repo` resolve
/// immediately instead of raising the fresh `BlockedAuth` gate the scenario
/// pins (verified: the shared-group variant fails with "expected BlockedAuth
/// but run reached terminal status Completed"). Each journey needs a
/// pristine no-credential runtime, so each builds its own group.
#[tokio::test]
async fn journeys_group_auth_convergence_e2e() {
    let mut report = ScenarioReport::new();

    let g = RebornIntegrationGroup::live_auth_and_approval()
        .await
        .expect("auth+approval group builds");
    report.record(
        "auth_then_approval_journey",
        scenario_auth_then_approval_journey::run(&g).await,
    );

    let g_deny = RebornIntegrationGroup::live_auth_and_approval()
        .await
        .expect("auth+approval deny group builds");
    report.record(
        "auth_deny_then_retry_journey",
        scenario_auth_deny_then_retry_journey::run(&g_deny).await,
    );
    report.assert_all_passed();
}

/// RED, `#[ignore]`d: the multi-actor GATED journey. Currently fails with
/// `driver_protocol_violation` on actor B's gated turn because this base's
/// capability harness hardcodes one execution user, so a distinct actor's gated
/// dispatch cannot be scoped to its own run owner. Un-ignore once the
/// C-MULTIUSER `scope_capability_by_run_owner` harness seam merges to main.
/// See the module-level "Deferred / blocked permutations" note.
/// TODO(reborn-multiuser-gate): un-ignore after the capability-owner-scoping seam lands.
#[tokio::test]
#[ignore = "blocked: needs unmerged C-MULTIUSER scope_capability_by_run_owner harness seam"]
async fn multi_actor_gate_isolation_blocked() {
    let g = RebornIntegrationGroup::live_approvals()
        .await
        .expect("group builds");
    scenario_multi_actor_gate_isolation::run(&g)
        .await
        .expect("multi-actor gated journey");
}
