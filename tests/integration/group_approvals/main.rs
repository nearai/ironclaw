//! Group integration tests for the Reborn approval flow — the real gate path.
//!
//! `approvals_group_e2e` is one sequential `#[tokio::test]` that drives
//! several scenarios over a shared [`RebornIntegrationGroup::live_approvals`]
//! group (one approval-request store, one capability-lease store, one
//! `(tenant, user)` auto-approve toggle, all shared across threads). See
//! `tests/integration/CLAUDE.md` §"Group tests".
//!
//! `concurrent_dual_gate_resume_parallel` (#5466) runs in both
//! `approvals_group_e2e` and `approvals_group_libsql_e2e` -- #5751 fixed the
//! libsql SIGABRT root cause; see the scenario's own module doc for the
//! cycle-3 fix lane's verification (50 libsql + 40 in-memory runs, 0 flakes).
//!
//! Every scenario in that test drives the REAL gate path: scripted
//! `builtin.write_file` call → real `TurnStatus::BlockedApproval` gate
//! (auto-approve disabled for the group at construction) → real
//! `ApprovalResolver` (`approve_gate`/`deny_gate`) → `coordinator.resume_turn`.
//! Only the model is faked. Exception: `failure_category_demasked` drives a
//! genuinely-FAILED run (no gate) to prove the loop-exit de-mask wiring. The
//! discard-tombstone invariant is pinned at the store contract tier and
//! through the real `CapabilityHost` rollback caller instead of here, to keep
//! this suite's execution model turn-driven.
//!
//! `approvals_group_real_gate_dispatch_e2e` is a separate group/test proving
//! the `submit_inbound(ApprovalResolution)` dispatch arm instead: it wires the
//! real interaction services over the group's own shared turn-state store so
//! resolution reaches the literal dispatch arm a real adapter's "approve"/
//! "deny" reply hits, rather than resuming the turn directly.
//!
//! ## Ordering (state machine over the shared auto-approve store)
//!
//! Independent gate scenarios run first while auto-approve is OFF (the control
//! proving gates are real): `gate_then_approve`, `gate_then_deny`,
//! `concurrent_dual_gate_resume` (HEADLINE, Option P — two threads parked on
//! `BlockedApproval` simultaneously on the shared `TurnCoordinator`, resolved
//! independently by `run_id`), `failure_category_demasked`,
//! `gate_ref_edge_cases::{stale_gate_ref_resume, missing_gate_bare_resolve}`
//! (C-DENYEDGE rows 7 & 10), `approval_request_persists_after_reopen`
//! (C-DURABLE). Then `approve_always_persists_cross_thread` (HEADLINE) flips
//! the toggle ON and MUST run before `ask_each_time_resumes_once`
//! (W4-ASK-EACH-ONCE, #5306 class), which installs a persistent group-wide
//! `AskEachTime` override on `builtin.write_file` and so must run LAST.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

mod scenario_approval_request_persists_after_reopen;
mod scenario_approve_always_persists_cross_thread;
mod scenario_ask_each_time_resumes_once;
mod scenario_concurrent_dual_gate_resume;
mod scenario_concurrent_dual_gate_resume_parallel;
mod scenario_failure_category_demasked;
mod scenario_gate_ref_edge_cases;
mod scenario_gate_then_approve;
mod scenario_gate_then_deny;
mod scenario_submit_inbound_approval_resolution;

use reborn_support::builder::StorageMode;
use reborn_support::group::{RebornIntegrationGroup, ScenarioReport};

#[tokio::test]
async fn approvals_group_e2e() {
    let g = RebornIntegrationGroup::live_approvals()
        .await
        .expect("group builds");

    let mut report = ScenarioReport::new();
    // Independent gate scenarios, run while auto-approve is still OFF (see
    // module doc's ordering section).
    report.record(
        "gate_then_approve",
        scenario_gate_then_approve::run(&g).await,
    );
    report.record("gate_then_deny", scenario_gate_then_deny::run(&g).await);
    report.record(
        "concurrent_dual_gate_resume",
        scenario_concurrent_dual_gate_resume::run(&g).await,
    );
    // #5466/#5751: libsql SIGABRT root-cause fixed; see main.rs module doc.
    report.record(
        "concurrent_dual_gate_resume_parallel",
        scenario_concurrent_dual_gate_resume_parallel::run(&g).await,
    );
    report.record(
        "failure_category_demasked",
        scenario_failure_category_demasked::run(&g).await,
    );
    report.record(
        "stale_gate_ref_resume",
        scenario_gate_ref_edge_cases::stale_gate_ref_resume(&g).await,
    );
    report.record(
        "missing_gate_bare_resolve",
        scenario_gate_ref_edge_cases::missing_gate_bare_resolve(&g).await,
    );
    // C-DURABLE: approval-request store is always on-disk regardless of the
    // group's `StorageMode`, so no `StorageMode::LibSql` variant is needed.
    report.record(
        "approval_request_persists_after_reopen",
        scenario_approval_request_persists_after_reopen::run(&g).await,
    );
    // Dependent: must run last (flips the (tenant, user) auto-approve toggle ON).
    scenario_approve_always_persists_cross_thread::run(&g)
        .await
        .expect("approve-always persists cross-thread");
    // W4-ASK-EACH-ONCE: must run after every other `builtin.write_file`
    // scenario above -- installs a persistent group-wide `AskEachTime`
    // override that would otherwise force-gate their plain-Ask-mode writes.
    report.record(
        "ask_each_time_resumes_once",
        scenario_ask_each_time_resumes_once::run(&g).await,
    );
    report.assert_all_passed();
}

/// Proof-of-seam group — the harness mid-stack bypass that resolved
/// approval gates via `TurnCoordinator::resume_turn` directly, never through
/// `ApprovalInteractionService::resolve`, is removed for this group only.
/// `.with_real_gate_dispatch_services()` wires the REAL interaction services
/// over the group's own shared turn-state store, so
/// `submit_approval_resolution` reaches the literal `submit_inbound` dispatch
/// arm a real adapter's "approve"/"deny" reply hits.
#[tokio::test]
async fn approvals_group_real_gate_dispatch_e2e() {
    let g = RebornIntegrationGroup::builder()
        .with_real_gate_dispatch_services()
        .live_approvals()
        .await
        .expect("group builds");

    let mut report = ScenarioReport::new();
    report.record(
        "submit_inbound_approval_resolution_approve",
        scenario_submit_inbound_approval_resolution::approve(&g).await,
    );
    report.record(
        "submit_inbound_approval_resolution_deny",
        scenario_submit_inbound_approval_resolution::deny(&g).await,
    );
    report.assert_all_passed();
}

#[tokio::test]
async fn approvals_group_libsql_e2e() {
    let g = RebornIntegrationGroup::builder()
        .storage(StorageMode::LibSql)
        .live_approvals()
        .await
        .expect("group builds");

    let mut report = ScenarioReport::new();
    // Independent gate scenarios, run while auto-approve is still OFF (see
    // module doc's ordering section).
    report.record(
        "gate_then_approve",
        scenario_gate_then_approve::run(&g).await,
    );
    report.record("gate_then_deny", scenario_gate_then_deny::run(&g).await);
    report.record(
        "concurrent_dual_gate_resume",
        scenario_concurrent_dual_gate_resume::run(&g).await,
    );
    // #5466/#5751: libsql SIGABRT root-cause fixed; see main.rs module doc.
    report.record(
        "concurrent_dual_gate_resume_parallel",
        scenario_concurrent_dual_gate_resume_parallel::run(&g).await,
    );
    report.record(
        "failure_category_demasked",
        scenario_failure_category_demasked::run(&g).await,
    );
    report.record(
        "stale_gate_ref_resume",
        scenario_gate_ref_edge_cases::stale_gate_ref_resume(&g).await,
    );
    report.record(
        "missing_gate_bare_resolve",
        scenario_gate_ref_edge_cases::missing_gate_bare_resolve(&g).await,
    );
    // Dependent: must run last (flips the (tenant, user) auto-approve toggle ON).
    scenario_approve_always_persists_cross_thread::run(&g)
        .await
        .expect("approve-always persists cross-thread");
    // W4-ASK-EACH-ONCE: must run after every other `builtin.write_file`
    // scenario above because it flips approval preferences.
    report.record(
        "ask_each_time_resumes_once",
        scenario_ask_each_time_resumes_once::run(&g).await,
    );
    report.assert_all_passed();
}
