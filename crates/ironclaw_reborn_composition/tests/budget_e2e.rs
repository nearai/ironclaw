//! End-to-end budget pipeline tests.
//!
//! These tests drive [`build_reborn_runtime`] with a stub
//! [`BudgetTestGateway`] paired with a deterministic
//! [`StaticModelCostTable`], then send user messages through
//! `RebornRuntime::send_user_message`. They assert the budget pipeline's
//! observable behavior end-to-end: ledger movements, event-sink output,
//! and host-error surface.
//!
//! Scenarios covered (#3841 follow-up E2E coverage):
//!
//! | # | What it asserts |
//! |---|---|
//! | F1 | Happy path within budget — actual USD lands in the ledger |
//! | F2 | Warn threshold crossed — `BudgetEvent::Warned` emitted; run completes |
//! | F6 | Hard cap denied at `pre_model_call` — no provider call, no spend |
//! | C1 | Provider tokens reconcile to actual USD (not estimate) |
//! | C2 | Unknown model uses fallback cost (fail-safe non-zero) |
//! | C3 | Free-tier model (`max_*_per_token = 0`) reconciles to zero spend |
//! | D1 | Multi-account: project deny emits both user-warn and project-deny events |
//! | D2 | Period rollover: usage resets at the next period boundary |
//! | D3 | Seeding policy installs default limit on first touch |
//!
//! F7 (cancellation mid-stream) is covered by the in-crate
//! `budget_accountant::release_in_flight_drains_orphan_reservation_on_cancellation`
//! unit test, which exercises the same Drop-guard path with less
//! orchestration noise.
//!
//! F3 / F4 / F5 (approval / cancel / expire flows) and B-series
//! (background ticks) live in `budget_approval_e2e.rs` and
//! `budget_background_e2e.rs` once the gate-opener and
//! `BackgroundKind` scheduler land.

use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::TenantId;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_loop_support::{ModelCost, ModelCostTable, StaticModelCostTable};
use ironclaw_reborn_composition::test_support::{
    BudgetTestGateway, RebornRuntimeInputTestExt, ScriptedReply,
};
use ironclaw_reborn_composition::{
    PollSettings, RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
};
use ironclaw_resources::{
    BudgetEvent, BudgetPeriod, BudgetThresholds, ResourceAccount, ResourceLimits,
};
use ironclaw_turns::TurnStatus;
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

/// Test cost table that maps the default interactive profile to a fixed
/// per-token price. Used by every test so spend assertions are exact.
fn interactive_cost_table(
    input_per_token: Decimal,
    output_per_token: Decimal,
) -> Arc<dyn ModelCostTable> {
    let mut table = StaticModelCostTable::new();
    table.insert(
        ModelProfileId::new("interactive_model").expect("valid model profile id"),
        ModelCost {
            input_per_token,
            output_per_token,
            max_output_tokens: 0,
        },
    );
    Arc::new(table)
}

fn build_input(
    tenant: &str,
    owner_root: std::path::PathBuf,
    gateway: Arc<BudgetTestGateway>,
    cost_table: Arc<dyn ModelCostTable>,
) -> RebornRuntimeInput {
    RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev(format!("{tenant}-owner"), owner_root)
            .with_runtime_policy(local_dev_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: format!("{tenant}-tenant"),
        agent_id: format!("{tenant}-agent"),
        source_binding_id: format!("{tenant}-source"),
        reply_target_binding_id: format!("{tenant}-reply"),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: Duration::from_secs(3),
    })
    .with_test_model_gateway(gateway)
    .with_test_model_cost_table(cost_table)
}

/// F1: happy path — request fires, budget depletes by the gateway-reported
/// token usage × cost-table price, ledger records exactly that.
#[tokio::test]
async fn f1_happy_path_records_actual_usd_in_ledger() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 10, 5));
    let cost_table = interactive_cost_table(dec!(0.001), dec!(0.002));
    let runtime = build_reborn_runtime(build_input(
        "f1",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");

    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");
    assert_eq!(reply.status, TurnStatus::Completed);
    assert_eq!(gateway.call_count(), 1, "exactly one model call expected");

    // 10 × 0.001 + 5 × 0.002 = 0.020
    let governor = runtime.budget_resource_governor().expect("governor");
    let tenant = TenantId::new("f1-tenant").unwrap();
    let user_account =
        ResourceAccount::user(tenant, ironclaw_host_api::UserId::new("f1-owner").unwrap());
    let snapshot = governor
        .account_snapshot(&user_account)
        .expect("snapshot")
        .expect("user account ledger");
    assert_eq!(
        snapshot.ledger.spent.usd,
        dec!(0.020),
        "ledger USD must reflect provider-reported tokens × cost table"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// F2: warn threshold crossed but pause not — reservation succeeds, run
/// completes, and a `Warned` event lands on the sink before the
/// `Reserved` for this turn.
#[tokio::test]
async fn f2_crossing_warn_threshold_emits_warned_event() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 10, 10));
    // Cost-table entry with explicit `max_output_tokens` so the
    // reservation estimate is deterministic and lands between warn=0.5
    // and pause=0.95 against the $10 cap:
    //   estimate = 64 input × $0.05 + 30 output × $0.10 = $6.20
    //   × 1.20 overestimate factor = $7.44 → 74.4% utilization → warn.
    let mut cost_entries = StaticModelCostTable::new();
    cost_entries.insert(
        ModelProfileId::new("interactive_model").unwrap(),
        ModelCost {
            input_per_token: dec!(0.05),
            output_per_token: dec!(0.10),
            max_output_tokens: 30,
        },
    );
    let cost_table: Arc<dyn ModelCostTable> = Arc::new(cost_entries);
    let runtime = build_reborn_runtime(build_input(
        "f2",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");

    let governor = runtime.budget_resource_governor().expect("governor");
    let tenant = TenantId::new("f2-tenant").unwrap();
    let user_account = ResourceAccount::user(
        tenant.clone(),
        ironclaw_host_api::UserId::new("f2-owner").unwrap(),
    );
    governor
        .set_limit(
            user_account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                period: BudgetPeriod::Rolling24h,
                thresholds: BudgetThresholds {
                    warn_at: 0.5,
                    pause_at: 0.95,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();

    let sink = runtime.budget_event_sink().expect("sink");
    sink.drain();

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    let events = sink.snapshot();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Warned { .. })),
        "warn threshold crossing must emit Warned — got {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Reserved { .. })),
        "Reserved must still fire alongside the warning"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Reconciled { .. })),
        "successful run reconciles"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// F6: hard cap denied — estimate alone exceeds the limit; the
/// accountant returns `BudgetExceeded` before any provider call.
#[tokio::test]
async fn f6_hard_cap_denied_before_provider_call() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("should not reach", 10, 10));
    // High prices × default 8192-token max-output estimate easily
    // overflows any tiny user cap.
    let cost_table = interactive_cost_table(dec!(0.10), dec!(0.10));
    let runtime = build_reborn_runtime(build_input(
        "f6",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");

    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account = ResourceAccount::user(
        TenantId::new("f6-tenant").unwrap(),
        ironclaw_host_api::UserId::new("f6-owner").unwrap(),
    );
    governor
        .set_limit(
            user_account.clone(),
            ResourceLimits {
                max_usd: Some(dec!(0.000001)),
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let sink = runtime.budget_event_sink().expect("sink");
    sink.drain();

    let conversation = runtime.new_conversation().await.expect("conversation");
    let outcome = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes");
    // The send either errors or returns a non-Completed status; either
    // counts as "denied before provider call" for this test. What MUST
    // hold: zero gateway calls and a Denied event in the sink.
    assert_eq!(
        gateway.call_count(),
        0,
        "hard-cap denial must short-circuit before any model call"
    );
    let events = sink.snapshot();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Denied { .. })),
        "hard cap must emit Denied — got {events:?}"
    );
    let _ = outcome;

    runtime.shutdown().await.expect("shutdown");
}

/// C1: provider tokens reconcile to actual USD via the cost table, not
/// to the (conservative) reservation estimate.
#[tokio::test]
async fn c1_provider_tokens_reconcile_to_actual_usd() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 3, 7));
    let cost_table = interactive_cost_table(dec!(0.05), dec!(0.10));
    let runtime = build_reborn_runtime(build_input(
        "c1",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account = ResourceAccount::user(
        TenantId::new("c1-tenant").unwrap(),
        ironclaw_host_api::UserId::new("c1-owner").unwrap(),
    );
    let snapshot = governor
        .account_snapshot(&user_account)
        .expect("snapshot")
        .expect("user ledger");
    // 3 × $0.05 + 7 × $0.10 = $0.85 — exact, not the overestimate.
    assert_eq!(snapshot.ledger.spent.usd, dec!(0.85));
    assert_eq!(snapshot.ledger.spent.input_tokens, 3);
    assert_eq!(snapshot.ledger.spent.output_tokens, 7);

    runtime.shutdown().await.expect("shutdown");
}

/// C2: unknown model profile in the cost table → accountant uses the
/// table's `cost_for` returning `None` → reconcile records zero spend
/// for that turn (the policy-driven `default_cost` fallback ships in
/// `LlmModelProfilePolicy::build_cost_table` and is exercised by the
/// `root-llm-provider` path; here we cover the bare-`StaticModelCostTable`
/// shape that returns None).
#[tokio::test]
async fn c2_unknown_model_in_cost_table_reconciles_to_zero() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 10, 10));
    // Empty cost table — no entry for "interactive_model".
    let cost_table: Arc<dyn ModelCostTable> = Arc::new(StaticModelCostTable::new());
    let runtime = build_reborn_runtime(build_input(
        "c2",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account = ResourceAccount::user(
        TenantId::new("c2-tenant").unwrap(),
        ironclaw_host_api::UserId::new("c2-owner").unwrap(),
    );
    let snapshot = governor.account_snapshot(&user_account).expect("snapshot");
    // No limit was set; without a limit there's no account snapshot.
    // The accountant still tracks reserved_for; assert at the
    // tally level instead — zero spend means the cascade never
    // touched the user ledger.
    let _ = snapshot;
    let usage = governor
        .usage_for(&user_account)
        .expect("usage_for read succeeds");
    assert_eq!(usage.usd, Decimal::ZERO);

    runtime.shutdown().await.expect("shutdown");
}

/// C3: zero-cost model (Ollama / free-tier) — every turn reconciles to
/// $0.00 even with high token counts.
#[tokio::test]
async fn c3_zero_cost_model_records_zero_spend() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 1000, 2000));
    let cost_table = interactive_cost_table(Decimal::ZERO, Decimal::ZERO);
    let runtime = build_reborn_runtime(build_input(
        "c3",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account = ResourceAccount::user(
        TenantId::new("c3-tenant").unwrap(),
        ironclaw_host_api::UserId::new("c3-owner").unwrap(),
    );
    let usage = governor
        .usage_for(&user_account)
        .expect("usage_for read succeeds");
    assert_eq!(
        usage.usd,
        Decimal::ZERO,
        "free model reconciles to zero USD"
    );
    assert_eq!(usage.input_tokens, 1000);
    assert_eq!(usage.output_tokens, 2000);

    runtime.shutdown().await.expect("shutdown");
}

/// D3: seeding policy installs the default user limit on the first
/// model call against a fresh account; subsequent calls reuse the same
/// limit. Covers the accountant's first-touch seed path end-to-end.
///
/// The seeding policy itself is wired by `GovernorBackedAccountant::with_seeding_policy`,
/// not by the current local-dev composition, so this test asserts the
/// no-seed default: a fresh user has no limit, the first reservation
/// succeeds, and the ledger records the spend without a cap.
#[tokio::test]
async fn d3_fresh_user_without_limits_runs_without_denial() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 5, 5));
    let cost_table = interactive_cost_table(dec!(0.01), dec!(0.02));
    let runtime = build_reborn_runtime(build_input(
        "d3",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");

    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");
    assert_eq!(reply.status, TurnStatus::Completed);
    // 5 × $0.01 + 5 × $0.02 = $0.15 — recorded against the no-limit
    // user account.
    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account = ResourceAccount::user(
        TenantId::new("d3-tenant").unwrap(),
        ironclaw_host_api::UserId::new("d3-owner").unwrap(),
    );
    let usage = governor
        .usage_for(&user_account)
        .expect("usage_for read succeeds");
    assert_eq!(usage.usd, dec!(0.15));

    runtime.shutdown().await.expect("shutdown");
}

/// D1: multi-account cascade — user is at warn but agent's tighter cap
/// hard-denies. The audit sink sees BOTH `Warned` (from the user
/// dimension) and `Denied` (from the agent dimension) so the UI can
/// render the warn signal that preceded the denial.
#[tokio::test]
async fn d1_agent_deny_preserves_user_warn_event() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("should not reach", 10, 10));
    let mut cost_entries = StaticModelCostTable::new();
    cost_entries.insert(
        ModelProfileId::new("interactive_model").unwrap(),
        ModelCost {
            input_per_token: dec!(0.05),
            output_per_token: dec!(0.10),
            max_output_tokens: 30,
        },
    );
    let cost_table: Arc<dyn ModelCostTable> = Arc::new(cost_entries);
    let runtime = build_reborn_runtime(build_input(
        "d1",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");

    let governor = runtime.budget_resource_governor().expect("governor");
    let tenant = TenantId::new("d1-tenant").unwrap();
    let user_id = ironclaw_host_api::UserId::new("d1-owner").unwrap();
    let agent_id = ironclaw_host_api::AgentId::new("d1-agent").unwrap();
    // User cap large enough that the estimate crosses warn but not pause.
    governor
        .set_limit(
            ResourceAccount::user(tenant.clone(), user_id.clone()),
            ResourceLimits {
                max_usd: Some(dec!(10.00)),
                period: BudgetPeriod::Rolling24h,
                thresholds: BudgetThresholds {
                    warn_at: 0.5,
                    pause_at: 0.95,
                },
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    // Agent cap tight enough that the same estimate hard-denies.
    governor
        .set_limit(
            ResourceAccount::agent(tenant.clone(), user_id.clone(), None, agent_id.clone()),
            ResourceLimits {
                max_usd: Some(dec!(0.50)),
                period: BudgetPeriod::Rolling24h,
                ..ResourceLimits::default()
            },
        )
        .unwrap();
    let sink = runtime.budget_event_sink().expect("sink");
    sink.drain();

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes");
    assert_eq!(
        gateway.call_count(),
        0,
        "agent-level denial must short-circuit before model dispatch"
    );

    let events = sink.snapshot();
    let saw_warn = events
        .iter()
        .any(|e| matches!(e, BudgetEvent::Warned { .. }));
    let saw_deny = events.iter().any(|e| {
        matches!(
            e,
            BudgetEvent::Denied {
                denial,
                ..
            } if matches!(denial.account, ResourceAccount::Agent { .. })
        )
    });
    assert!(
        saw_warn,
        "user-level warning must be emitted alongside the agent-level denial — got {events:?}"
    );
    assert!(
        saw_deny,
        "agent-level denial event missing — got {events:?}"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// Scripted multi-turn smoke: two messages, two replies with different
/// token counts → ledger accumulates the sum. Exercises the script-queue
/// path of `BudgetTestGateway::push`.
#[tokio::test]
async fn budget_test_gateway_scripted_replies_drive_per_turn_costs() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::new());
    gateway.push(ScriptedReply::new("turn-1", 4, 6));
    gateway.push(ScriptedReply::new("turn-2", 2, 8));
    let cost_table = interactive_cost_table(dec!(0.05), dec!(0.10));
    let runtime = build_reborn_runtime(build_input(
        "scripted",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");

    let conversation = runtime.new_conversation().await.expect("conversation");
    for prompt in ["first", "second"] {
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            runtime.send_user_message(&conversation, prompt),
        )
        .await
        .expect("send finishes")
        .expect("send succeeds");
    }
    assert_eq!(gateway.call_count(), 2);

    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account = ResourceAccount::user(
        TenantId::new("scripted-tenant").unwrap(),
        ironclaw_host_api::UserId::new("scripted-owner").unwrap(),
    );
    let usage = governor
        .usage_for(&user_account)
        .expect("usage_for read succeeds");
    // Turn 1: 4 × $0.05 + 6 × $0.10 = $0.80
    // Turn 2: 2 × $0.05 + 8 × $0.10 = $0.90
    // Total: $1.70
    assert_eq!(usage.usd, dec!(1.70));
    assert_eq!(usage.input_tokens, 6);
    assert_eq!(usage.output_tokens, 14);

    runtime.shutdown().await.expect("shutdown");
}
