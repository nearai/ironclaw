//! Approval-flow E2E budget tests (#3841 follow-ups F3 / F4 / F5).
//!
//! These tests drive a `send_user_message` past the pause threshold so
//! the accountant returns `BudgetApprovalRequired`, then resolve the
//! pending gate three different ways:
//!
//! | # | Resolution | Expectation |
//! |---|---|---|
//! | F3 | `Approve { increased_limit }` | Subsequent send succeeds with the new cap |
//! | F4 | `Cancel { by }` | Gate terminal; subsequent send still fails the same way |
//! | F5 | Expiry via `expire_pending_older_than(cutoff)` | Same as cancel |
//!
//! The gate store is wired into the accountant by
//! `build_reborn_runtime` (local-dev composition); tests exercise the
//! flow without standing up a separate gate-handler service.

use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::TenantId;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_loop_support::{ModelCost, ModelCostTable, StaticModelCostTable};
use ironclaw_reborn_composition::test_support::{BudgetTestGateway, RebornRuntimeInputTestExt};
use ironclaw_reborn_composition::{
    PollSettings, RebornBuildInput, RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput,
    build_reborn_runtime,
};
use ironclaw_resources::{
    BudgetGateOutcome, BudgetGateStatus, BudgetPeriod, BudgetThresholds, ResourceAccount,
    ResourceLimits,
};
use ironclaw_turns::run_profile::ModelProfileId;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
    EffectiveRuntimePolicy {
        deployment: DeploymentMode::LocalSingleUser,
        requested_profile: RuntimeProfile::LocalDev,
        resolved_profile: RuntimeProfile::LocalDev,
        filesystem_backend: FilesystemBackendKind::HostWorkspace,
        process_backend: ProcessBackendKind::LocalHost,
        network_mode: NetworkMode::DirectLogged,
        secret_mode: SecretMode::ScrubbedEnv,
        approval_policy: ApprovalPolicy::AskDestructive,
        audit_mode: AuditMode::LocalMinimal,
    }
}

/// Cost table tuned so a default-size reservation lands just above
/// `pause_at` against the test user's $1.00 cap:
///   estimate = 64 input × $0.05 + 20 output × $0.10 = $5.20 × 1.20 = $6.24
/// → 624% utilization, well above pause(0.95) → ApprovalRequired.
fn pause_inducing_cost_table() -> Arc<dyn ModelCostTable> {
    let mut table = StaticModelCostTable::new();
    table.insert(
        ModelProfileId::new("interactive_model").unwrap(),
        ModelCost {
            input_per_token: dec!(0.05),
            output_per_token: dec!(0.10),
            max_output_tokens: 20,
        },
    );
    Arc::new(table)
}

async fn build_runtime_with_pause_inducing_setup(
    tag: &str,
    root: std::path::PathBuf,
) -> (RebornRuntime, Arc<BudgetTestGateway>) {
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 5, 5));
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev(format!("{tag}-owner"), root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: format!("{tag}-tenant"),
        agent_id: format!("{tag}-agent"),
        source_binding_id: format!("{tag}-source"),
        reply_target_binding_id: format!("{tag}-reply"),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_test_model_gateway(gateway.clone())
    .with_test_model_cost_table(pause_inducing_cost_table());
    let runtime = build_reborn_runtime(input).await.expect("runtime builds");

    // Tight user cap so the estimate crosses pause but not the hard
    // ceiling — that's how the cascade lands on RequiresApproval rather
    // than LimitExceeded.
    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account = ResourceAccount::user(
        TenantId::new(format!("{tag}-tenant")).unwrap(),
        ironclaw_host_api::UserId::new(format!("{tag}-owner")).unwrap(),
    );
    governor
        .set_limit(
            user_account,
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                period: BudgetPeriod::Rolling24h,
                thresholds: BudgetThresholds {
                    warn_at: 0.2,
                    pause_at: 0.5,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    (runtime, gateway)
}

/// Drive the runtime past the pause threshold, then return the pending
/// gate. The send must fail (no gateway call, no completion).
async fn pump_until_pending_gate(
    runtime: &RebornRuntime,
    gateway: &BudgetTestGateway,
) -> ironclaw_resources::BudgetGateId {
    let conversation = runtime.new_conversation().await.expect("conversation");
    let outcome = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "first try"),
    )
    .await
    .expect("send finishes");
    let _ = outcome; // we don't care about Err vs non-Completed shape here
    assert_eq!(
        gateway.call_count(),
        0,
        "pause threshold must short-circuit before any model call"
    );

    let store = runtime.budget_gate_store().expect("gate store");
    let pending = store.list_pending().expect("list pending");
    assert_eq!(
        pending.len(),
        1,
        "exactly one pending gate expected after pause; got {pending:?}"
    );
    pending[0].id
}

/// F3: pause → user approves with an increased limit → retry succeeds.
#[tokio::test]
async fn f3_approval_with_increased_limit_unblocks_retry() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, gateway) =
        build_runtime_with_pause_inducing_setup("f3", root.path().to_path_buf()).await;

    let gate_id = pump_until_pending_gate(&runtime, &gateway).await;

    // Resolve: approve with a much larger cap so the next reservation
    // succeeds.
    let store = runtime.budget_gate_store().expect("gate store");
    let approver = ironclaw_host_api::UserId::new("f3-approver").unwrap();
    let increased = ResourceLimits {
        max_usd: Some(dec!(1_000.00)),
        period: BudgetPeriod::Rolling24h,
        thresholds: BudgetThresholds::DISABLED,
        ..ResourceLimits::default()
    };
    let resolved = store
        .resolve(
            gate_id,
            BudgetGateOutcome::Approve {
                increased_limit: increased.clone(),
                by: approver,
            },
            chrono::Utc::now(),
        )
        .expect("resolve approve");
    assert!(matches!(resolved.status, BudgetGateStatus::Approved { .. }));

    // Apply the resolution to the governor — production wires this
    // through a gate-resolution handler; the test-only accessor mimics
    // that surface.
    runtime
        .apply_resolved_budget_gate(gate_id)
        .expect("apply resolved gate");

    // Now retry. With the larger cap in place, the reservation
    // succeeds, the model call runs, and the budget event sink sees
    // Reserved + Reconciled.
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "approved retry"),
    )
    .await
    .expect("retry send finishes")
    .expect("retry send succeeds");
    assert_eq!(reply.status, ironclaw_turns::TurnStatus::Completed);
    assert_eq!(
        gateway.call_count(),
        1,
        "retry should issue exactly one model call"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// F4: pause → user cancels → retry still fails the same way.
#[tokio::test]
async fn f4_cancel_keeps_budget_blocked_on_retry() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, gateway) =
        build_runtime_with_pause_inducing_setup("f4", root.path().to_path_buf()).await;
    let gate_id = pump_until_pending_gate(&runtime, &gateway).await;

    let store = runtime.budget_gate_store().expect("gate store");
    let canceller = ironclaw_host_api::UserId::new("f4-canceller").unwrap();
    let resolved = store
        .resolve(
            gate_id,
            BudgetGateOutcome::Cancel { by: canceller },
            chrono::Utc::now(),
        )
        .expect("resolve cancel");
    assert!(matches!(
        resolved.status,
        BudgetGateStatus::Cancelled { .. }
    ));

    // Applying a cancel is a no-op on the governor (the limit stays
    // tight); calling the helper just confirms it doesn't panic.
    runtime
        .apply_resolved_budget_gate(gate_id)
        .expect("apply resolved gate (cancel is a no-op)");

    // Retry — the same pause threshold fires, gateway still untouched.
    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "retry after cancel"),
    )
    .await
    .expect("retry send finishes");
    assert_eq!(
        gateway.call_count(),
        0,
        "cancellation must NOT unblock the budget; gateway stays untouched"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// F5: pause → no user action → admin expires stale gates → terminal
/// state is `Expired`; retry remains blocked exactly like F4.
#[tokio::test]
async fn f5_expiry_marks_gate_terminal_and_keeps_budget_blocked() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, gateway) =
        build_runtime_with_pause_inducing_setup("f5", root.path().to_path_buf()).await;
    let gate_id = pump_until_pending_gate(&runtime, &gateway).await;

    let store = runtime.budget_gate_store().expect("gate store");
    // Expire every pending gate whose `expires_at` is at or before
    // 365 days in the future — covers the default 24h expiry window
    // without us having to sleep or inject a clock.
    let cutoff = chrono::Utc::now() + chrono::Duration::days(365);
    let expired = store
        .expire_pending_older_than(cutoff)
        .expect("expire pending");
    assert_eq!(expired.len(), 1, "exactly one gate should have expired");
    assert!(matches!(
        expired[0].status,
        BudgetGateStatus::Expired { .. }
    ));
    assert_eq!(expired[0].id, gate_id);

    // Confirm the expired gate is no longer pending — before doing
    // a retry that would itself open a fresh gate.
    let pending_after_expiry = store.list_pending().expect("list pending");
    assert!(
        pending_after_expiry.iter().all(|g| g.id != gate_id),
        "the expired gate must drop out of the pending list — got {pending_after_expiry:?}"
    );

    // Retry — same as cancel, the budget is still tight.
    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "retry after expiry"),
    )
    .await
    .expect("retry send finishes");
    assert_eq!(
        gateway.call_count(),
        0,
        "expired gate must NOT unblock the budget; gateway stays untouched"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// Bonus: opening a gate is idempotent on the same run — repeated
/// approval-required errors don't pile up duplicate pending gates.
///
/// Today the accountant rejects a second concurrent reservation for the
/// same `TurnRunId`, so the second send_user_message that pauses lands
/// in a *fresh* run — and therefore SHOULD produce a new gate. This
/// test documents that shape: two failed sends produce two pending
/// gates (one per run), which is the expected behavior because each
/// run is a separate user-visible attempt that may want its own
/// approval decision.
#[tokio::test]
async fn pause_in_distinct_runs_produces_distinct_pending_gates() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, gateway) =
        build_runtime_with_pause_inducing_setup("dup", root.path().to_path_buf()).await;

    // First send → first gate.
    let _gate_a = pump_until_pending_gate(&runtime, &gateway).await;

    // Second send (fresh conversation, fresh run) → second gate.
    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "second"),
    )
    .await
    .expect("send finishes");

    let store = runtime.budget_gate_store().expect("gate store");
    let pending = store.list_pending().expect("list pending");
    assert_eq!(
        pending.len(),
        2,
        "two distinct paused runs must produce two pending gates"
    );
    let _ = Decimal::ZERO; // keep the rust_decimal import live across compile shapes

    runtime.shutdown().await.expect("shutdown");
}
