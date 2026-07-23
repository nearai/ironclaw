//! C-BUDGET (wiring-liveness only): the harness bypass path
//! (`build_default_planned_runtime`) wires the PRODUCTION
//! `build_default_budget_accountant` into `DefaultPlannedRuntimeParts::model_budget_accountant`,
//! and that accountant fires on a real coordinator-path turn.
//!
//! Budget SEMANTICS (ledger, thresholds, approval-unblock, `BudgetEvent`
//! cascade) are covered at crate tier via `build_ironclaw_runtime`
//! (`budget_e2e.rs` / `budget_approval_e2e.rs`) — not re-authored here. This
//! proves only that the accountant is live: on the turn's first model call it
//! seeds the run owner's daily USD cap, observable via the in-memory governor.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::builder::IronClawIntegrationHarness;
use ironclaw_support::reply::IronClawScriptedReply;

/// Liveness proof: the wired accountant seeds the run owner's daily cap on
/// the turn's first model call, reaching coordinator → loop → model-port via
/// `DefaultPlannedRuntimeParts` wiring.
#[tokio::test]
async fn budget_accountant_seeds_user_cap_on_turn_model_call() {
    let h = IronClawIntegrationHarness::test_default()
        .with_budget_accounting()
        .script([IronClawScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("do something").await.expect("turn completes");
    h.assert_budget_user_cap_seeded()
        .await
        .expect("wired budget accountant seeded the run owner's daily cap");
}

/// Guard: without `with_budget_accounting` wired, no governor exists, so the
/// liveness assertion above must fail — not vacuously passing — and the
/// default path stays behavior-identical (no accountant).
#[tokio::test]
async fn budget_assertion_requires_wiring() {
    let h = IronClawIntegrationHarness::test_default()
        .script([IronClawScriptedReply::text("no budget")])
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
