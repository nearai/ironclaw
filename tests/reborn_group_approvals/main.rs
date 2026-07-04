//! Group integration tests for the Reborn approval flow — the real gate path.
//!
//! One sequential `#[tokio::test]` drives eight scenarios over a shared
//! [`RebornIntegrationGroup::live_approvals`] group (one approval-request store,
//! one capability-lease store, one `(tenant, user)` auto-approve toggle, all
//! shared across threads). See `tests/support/reborn/CLAUDE.md` §"Group tests".
//!
//! Every scenario drives the REAL gate path end-to-end: the scripted model emits
//! a `builtin.write_file` tool call → the real first-party runtime raises a real
//! `TurnStatus::BlockedApproval` gate (auto-approve is disabled for the group at
//! construction) → the test resolves it through the real `ApprovalResolver`
//! (`approve_gate`/`deny_gate`) and `coordinator.resume_turn`. Nothing is faked
//! except the model at the vendor-SDK seam. The exception is
//! `failure_category_demasked`, which drives a genuinely-FAILED run (no gate
//! involved) to prove the group's loop-exit de-mask wiring.
//!
//! ## Scenario ordering (a state machine over the shared auto-approve store)
//!
//! 1. `gate_then_approve` — gate fires (auto-approve OFF), approve → Completed.
//! 2. `gate_then_deny` — gate fires, deny → the model sees an authorization
//!    failure, not a hang.
//! 3. `concurrent_dual_gate_resume` (HEADLINE, Option P) — two threads parked
//!    on `BlockedApproval` SIMULTANEOUSLY on the group's one shared
//!    `TurnCoordinator`, resolved independently (approve one, deny the other)
//!    — proves resume dispatch is keyed by `run_id` with zero cross-resume.
//!    Must run while auto-approve is still OFF (same control window as 1–2).
//! 4. `failure_category_demasked` — an empty-scripted thread drives a run to a
//!    genuine `TurnStatus::Failed` and asserts the TRUE failure category
//!    (`"model_error"`) survives instead of being rewritten to the masking
//!    `"driver_protocol_violation"` sentinel. Independent of the auto-approve
//!    toggle (no gate involved); ordered alongside the other independent
//!    scenarios, before the toggle is flipped.
//! 5. `gate_ref_edge_cases::stale_gate_ref_resume` (C-DENYEDGE row 7) — the
//!    local-dev approval resolve succeeds with the run's REAL gate_ref, but
//!    the coordinator resume is issued with a different, STALE gate_ref,
//!    reaching the `TurnError::InvalidRequest { reason: "gate resolution
//!    reference mismatch" }` path `approve_gate` alone cannot reach; then a
//!    non-vacuity resume with the real ref completes the run.
//! 6. `gate_ref_edge_cases::missing_gate_bare_resolve` (C-DENYEDGE row 10) — a
//!    syntactically well-formed but never-issued gate_ref is resolved on a
//!    thread that never raised any gate; pins the harness's own
//!    request-not-found rejection. Independent of the auto-approve toggle (no
//!    real gate involved).
//! 7. `approval_request_persists_after_reopen` (C-DURABLE) — reopens a FRESH
//!    `ApprovalRequestStore` at the same on-disk root and confirms the
//!    `Pending` request survives, independent of the auto-approve toggle
//!    (its own gate, resolved before returning).
//! 8. `approve_always_persists_cross_thread` (HEADLINE) — thread A flips
//!    auto-approve ON; a DIFFERENT thread B then writes with NO gate. Proves the
//!    setting persists across thread boundaries. MUST run before scenario 9 (it
//!    flips the toggle ON for the whole group), so the gate scenarios above are
//!    the control proving the gate was real before the flip.
//! 9. `ask_each_time_resumes_once` (W4-ASK-EACH-ONCE, #5306 class) — installs a
//!    persistent, group-wide `ToolPermissionOverride::AskEachTime` override on
//!    `builtin.write_file`, so it MUST run LAST: every scenario above assumes
//!    the plain default (no override) Ask-mode gate for that capability, and
//!    this override is a separate CAS store from `AutoApproveSettingStore` that
//!    force-gates fresh invocations regardless of auto-approve. Proves an
//!    approved AskEachTime-gated resume completes in ONE round trip, not an
//!    unresumable re-gate loop.

#[allow(dead_code)]
#[path = "../support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

mod scenario_approval_request_persists_after_reopen;
mod scenario_approve_always_persists_cross_thread;
mod scenario_ask_each_time_resumes_once;
mod scenario_concurrent_dual_gate_resume;
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
    // Independent gate scenarios (run while auto-approve is still OFF — they are
    // the control proving the gate is real before scenario 4 flips it ON).
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
    // C-DURABLE: independent of the auto-approve toggle (its own gate, resolved
    // before returning) — the approval-request store is always on-disk
    // regardless of the group's `StorageMode` (a separate capability-harness
    // filesystem), so this needs no `StorageMode::LibSql` variant.
    report.record(
        "approval_request_persists_after_reopen",
        scenario_approval_request_persists_after_reopen::run(&g).await,
    );
    // Dependent: must run last (flips the (tenant, user) auto-approve toggle ON).
    scenario_approve_always_persists_cross_thread::run(&g)
        .await
        .expect("approve-always persists cross-thread");
    // W4-ASK-EACH-ONCE: MUST run after every other `builtin.write_file`
    // scenario above -- it installs a persistent, group-wide
    // `ToolPermissionOverride::AskEachTime` override for `builtin.write_file`
    // (the same shared per-`(tenant, user)` CAS store `disable_outbound_target_set_tool`
    // uses), which would otherwise force-gate their plain-Ask-mode writes too.
    // Independent of the auto-approve toggle's value (`ask_each_time` always
    // gates fresh invocations regardless of auto-approve -- see the scenario's
    // module doc), so running it after the toggle flip is a stronger proof,
    // not a weaker one.
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
    // Independent gate scenarios (run while auto-approve is still OFF — they are
    // the control proving the gate is real before scenario 4 flips it ON).
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
    // W4-ASK-EACH-ONCE: MUST run after every other `builtin.write_file`
    // scenario above -- see the non-libsql variant's comment above for why.
    report.record(
        "ask_each_time_resumes_once",
        scenario_ask_each_time_resumes_once::run(&g).await,
    );
    report.assert_all_passed();
}
