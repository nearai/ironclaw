//! Group integration tests for the Reborn approval flow — the real gate path.
//!
//! One sequential `#[tokio::test]` drives three scenarios over a shared
//! [`RebornIntegrationGroup::live_approvals`] group (one approval-request store,
//! one capability-lease store, one `(tenant, user)` auto-approve toggle, all
//! shared across threads). See `tests/support/reborn/CLAUDE.md` §"Group tests".
//!
//! Every scenario drives the REAL gate path end-to-end: the scripted model emits
//! a `builtin.write_file` tool call → the real first-party runtime raises a real
//! `TurnStatus::BlockedApproval` gate (auto-approve is disabled for the group at
//! construction) → the test resolves it through the real `ApprovalResolver`
//! (`approve_gate`/`deny_gate`) and `coordinator.resume_turn`. Nothing is faked
//! except the model at the vendor-SDK seam.
//!
//! ## Scenario ordering (a state machine over the shared auto-approve store)
//!
//! 1. `gate_then_approve` — gate fires (auto-approve OFF), approve → Completed.
//! 2. `gate_then_deny` — gate fires, deny → the model sees an authorization
//!    failure, not a hang.
//! 3. `approve_always_persists_cross_thread` (HEADLINE) — thread A flips
//!    auto-approve ON; a DIFFERENT thread B then writes with NO gate. Proves the
//!    setting persists across thread boundaries. MUST run last (it flips the
//!    toggle ON for the whole group), so the gate scenarios above are the control
//!    proving the gate was real before the flip.

#[allow(dead_code)]
#[path = "../support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

mod scenario_approve_always_persists_cross_thread;
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
    // Independent gate scenarios (run while auto-approve is still OFF — they are
    // the control proving the gate is real before scenario 3 flips it ON).
    report.record(
        "gate_then_approve",
        scenario_gate_then_approve::run(&g).await,
    );
    report.record("gate_then_deny", scenario_gate_then_deny::run(&g).await);
    // Dependent: must run last (flips the (tenant, user) auto-approve toggle ON).
    scenario_approve_always_persists_cross_thread::run(&g)
        .await
        .expect("approve-always persists cross-thread");
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
    // Independent gate scenarios (run while auto-approve is still OFF — they are
    // the control proving the gate is real before scenario 3 flips it ON).
    report.record(
        "gate_then_approve",
        scenario_gate_then_approve::run(&g).await,
    );
    report.record("gate_then_deny", scenario_gate_then_deny::run(&g).await);
    // Dependent: must run last (flips the (tenant, user) auto-approve toggle ON).
    scenario_approve_always_persists_cross_thread::run(&g)
        .await
        .expect("approve-always persists cross-thread");
    report.assert_all_passed();
}
