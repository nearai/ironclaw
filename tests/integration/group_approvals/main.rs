//! Group integration tests for the Reborn approval flow — the real gate path.
//!
//! One sequential `#[tokio::test]` drives several scenarios over a shared
//! [`RebornIntegrationGroup::live_approvals`] group (one approval-request store,
//! one capability-lease store, one `(tenant, user)` auto-approve toggle, all
//! shared across threads). See `tests/integration/CLAUDE.md` §"Group tests".
//!
//! `concurrent_dual_gate_resume_parallel` (#5466) is `approvals_group_e2e`
//! only (InMemory) -- never add a `StorageMode::LibSql` variant, see its
//! own module doc (libsql SIGABRTs the whole test binary, #5466).
//!
//! Every scenario drives the REAL gate path: scripted `builtin.write_file` call
//! → real `TurnStatus::BlockedApproval` gate (auto-approve disabled for the
//! group at construction) → real `ApprovalResolver` (`approve_gate`/`deny_gate`)
//! → `coordinator.resume_turn`. Only the model is faked. Exceptions:
//! `failure_category_demasked` drives a genuinely-FAILED run (no gate) to prove
//! the loop-exit de-mask wiring; `discard_then_resubmit` (#5467) drives the
//! approval-request store directly (no `submit_turn`/gate at all) since no
//! harness fault-injection seam exists for the mid-turn discard race it covers.
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
mod scenario_discard_then_resubmit;
mod scenario_failure_category_demasked;
mod scenario_gate_ref_edge_cases;
mod scenario_gate_then_approve;
mod scenario_gate_then_deny;

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
    // #5466: InMemory only, see this scenario's own module doc + main.rs doc.
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
    // #5467: store-direct, independent of `StorageMode` (same as C-DURABLE above).
    report.record(
        "discard_then_resubmit",
        scenario_discard_then_resubmit::run(&g).await,
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
    // scenario above -- see the non-libsql variant's comment above.
    report.record(
        "ask_each_time_resumes_once",
        scenario_ask_each_time_resumes_once::run(&g).await,
    );
    report.assert_all_passed();
}
