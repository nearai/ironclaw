//! C-BUDGET (wiring-liveness only): the harness bypass path
//! (`build_default_planned_runtime`) wires the PRODUCTION
//! `build_default_budget_accountant` into `DefaultPlannedRuntimeParts::model_budget_accountant`,
//! and that accountant fires on a real coordinator-path turn.
//!
//! Budget SEMANTICS (ledger, warn/deny thresholds, approval-unblock,
//! `BudgetEvent` cascade) are ALREADY covered at crate tier via
//! `build_reborn_runtime` (`budget_e2e.rs` / `budget_approval_e2e.rs`) — this
//! binary does NOT re-author them. It proves only the group/flat harness (which
//! bypasses the `build_reborn_services` shell) now composes the accountant live:
//! on the turn's first model call the accountant's compiled-default seeding
//! policy installs the run owner's daily USD cap, observable through the
//! retained in-memory governor.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

/// The wired accountant seeds the run owner's compiled-default daily cap on the
/// turn's first model call — the liveness proof that
/// `build_default_budget_accountant` reaches the coordinator → loop → model-port
/// path through the harness's `DefaultPlannedRuntimeParts` wiring.
#[tokio::test]
async fn budget_accountant_seeds_user_cap_on_turn_model_call() {
    let h = RebornIntegrationHarness::test_default()
        .with_budget_accounting()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("do something").await.expect("turn completes");
    h.assert_budget_user_cap_seeded()
        .await
        .expect("wired budget accountant seeded the run owner's daily cap");
}

/// Guard: without `with_budget_accounting`, no accountant is wired, so the
/// liveness assertion must FAIL (there is no governor to read). Pins that the
/// assertion above is not vacuously passing and that the default path is
/// behavior-identical (no accountant).
#[tokio::test]
async fn budget_assertion_requires_wiring() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("no budget")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("just talk").await.expect("turn completes");
    let err = h
        .assert_budget_user_cap_seeded()
        .await
        .expect_err("budget liveness assertion must fail when no accountant is wired");
    assert_eq!(
        err.to_string(),
        "harness was not built with budget accounting wired (call with_budget_accounting)",
        "expected the no-governor-wired failure, got a different harness error: {err}"
    );
}
