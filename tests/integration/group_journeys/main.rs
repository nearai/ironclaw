//! C-JOURNEY — multi-turn Reborn journeys: deterministic twins of the live
//! canary use cases that chain gate → resume → next turn on ONE
//! conversation/harness. Distinct from `reborn_group_approvals` /
//! `reborn_integration_auth_gate` (single-gate mechanics): the value here is
//! the chaining across turns.
//!
//! | scenario | inbound | gate(s) | outcome |
//! |---|---|---|---|
//! | interactive_approval_journey | interactive | approval → approval | approve, deny, follow-up |
//! | auth_then_approval_journey | interactive | (approval→auth) → approval | approve+resolve, approve, follow-up |
//! | auth_deny_then_retry_journey | interactive | (approval→auth) → (approval→auth) | approve+deny, approve+resolve |
//! | multi_actor_gate_isolation | interactive×2 | approval (A) / approval (B) | per-actor gate + resume isolation |
//!
//! Gate-arm discipline: a gated tool-call turn consumes exactly TWO script
//! entries regardless of approve/deny; a plain follow-up turn consumes ONE.
//!
//! `auth_then_approval_journey` and `auth_deny_then_retry_journey` run on a
//! SECOND group, `RebornIntegrationGroup::live_auth_and_approval()` (built
//! from `HostRuntimeCapabilityHarness::file_and_github_auth_tools`), NOT
//! `live_approvals` above — it converges the auth gate onto the SAME
//! `build_reborn_services` runtime (unlike `live_auth_gate`'s separate,
//! lower-level `HostRuntimeServices` build with no `run_state`/
//! `approval_requests`/`capability_leases` stores). No GitHub credential is
//! seeded at construction, so `github.get_repo` raises `BlockedApproval` then,
//! post-approve, a real `BlockedAuth`; `resolve_auth_gate` seeds a credential
//! through the real `ProductAuthRuntimeCredentialResolver` and resumes the
//! same parked capability. See
//! `HostRuntimeCapabilityHarness::file_and_github_auth_tools`'s doc comment
//! for the composition-seam mechanism this required.
//!
//! ## Deferred / blocked permutations
//!
//! - **Triggered-origin chained journey**: needs the scripted-gateway seam
//!   (`RebornIntegrationHarness::submit_triggered_turn_scripted`) to reconcile
//!   the trigger's minted owner scope with the journey approval helpers'
//!   binding scope. Single-turn triggered-origin coverage already exists (see
//!   `reborn_integration_triggered_submit`).
//! - **Multi-actor GATED journey** (`multi_actor_gate_isolation`): runs on
//!   `RebornIntegrationGroup::multiuser_approvals()`, whose C-MULTIUSER
//!   `scope_capability_by_run_owner` seam scopes each actor's gated write to
//!   its own run owner. Plain (non-gated) distinct-actor isolation is covered
//!   by `reborn_group_multiuser::two_actors_own_threads`.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
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
/// `live_auth_and_approval` groups (not the `live_approvals` group above).
///
/// ONE GROUP PER SCENARIO: `resolve_auth_gate` seeds a `UserReusable` GitHub
/// credential under the group's canonical scope, so a shared group would let
/// scenario 2's `github.get_repo` resolve immediately instead of raising the
/// fresh `BlockedAuth` gate it pins (verified: shared-group variant fails
/// with "expected BlockedAuth but run reached terminal status Completed").
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

/// The multi-actor GATED journey — see the module doc's "Multi-actor GATED
/// journey" note for the `scope_capability_by_run_owner` seam this requires.
#[tokio::test]
async fn multi_actor_gate_isolation() {
    let g = RebornIntegrationGroup::multiuser_approvals()
        .await
        .expect("group builds");
    scenario_multi_actor_gate_isolation::run(&g)
        .await
        .expect("multi-actor gated journey");
}
