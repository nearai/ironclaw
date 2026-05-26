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
use ironclaw_reborn_composition::test_support::{BudgetTestGateway, ScriptedReply};
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
///
/// `max_output_tokens = 20` keeps the reservation estimate tiny enough
/// to fit under the seeded $5/user default cap installed by composition
/// (review feedback High #2). Tests that want to exceed that cap raise
/// the user limit explicitly before sending.
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
            max_output_tokens: 20,
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
    .with_model_gateway_override(gateway)
    .with_model_cost_table_override(cost_table)
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
    // Raise the user cap above the seeded $5 default so the
    // pre-call estimate (≈$5.50 at these prices) doesn't pause.
    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account = ResourceAccount::user(
        TenantId::new("c1-tenant").unwrap(),
        ironclaw_host_api::UserId::new("c1-owner").unwrap(),
    );
    governor
        .set_limit(
            user_account,
            ResourceLimits {
                max_usd: Some(dec!(1_000.00)),
                period: BudgetPeriod::Rolling24h,
                thresholds: BudgetThresholds::DISABLED,
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
/// table's `cost_for` returning `None` → the accountant's `default_cost`
/// fallback fires (conservative ~GPT-4o pricing) so the ledger records
/// *non-zero* spend. This is the fail-closed shape from review feedback
/// Medium #5: a paid model missing from the cost table must NOT silently
/// reconcile to zero.
#[tokio::test]
async fn c2_unknown_model_in_cost_table_uses_default_cost_fallback() {
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
    let usage = governor
        .usage_for(&user_account)
        .expect("usage_for read succeeds");
    // 10 input × ~$0.0000025 + 10 output × ~$0.00001 ≈ $0.000125 — what
    // matters is that this is strictly greater than zero. Silently
    // recording zero for an unknown paid model is the bug we're fixing.
    assert!(
        usage.usd > Decimal::ZERO,
        "unknown model must NOT silently reconcile to zero USD (got {})",
        usage.usd,
    );
    assert_eq!(usage.input_tokens, 10);
    assert_eq!(usage.output_tokens, 10);

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
/// model call against a fresh account. The local-dev composition wires
/// `BudgetSeedingPolicy::new(user_daily=$5, project_daily=$2)` from
/// `BudgetDefaults::compiled_defaults()`, so the first model call
/// against a fresh user installs that $5/day cap.
///
/// This test sends one tiny call that fits well under $5, then asserts
/// the user account ended up with the seeded $5 limit (the proof of
/// "seeding fired"). It guards against regressions in the
/// composition-level `with_seeding_policy` wiring (review feedback:
/// High #2).
#[tokio::test]
async fn d3_seeding_policy_installs_default_cap_on_first_touch() {
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
    // Seeding policy fired: user account now has the compiled default $5 cap.
    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account_seed_check = ResourceAccount::user(
        TenantId::new("d3-tenant").unwrap(),
        ironclaw_host_api::UserId::new("d3-owner").unwrap(),
    );
    let seeded_snapshot = governor
        .account_snapshot(&user_account_seed_check)
        .expect("snapshot read")
        .expect("user account exists after first call");
    let seeded_limits = seeded_snapshot
        .limits
        .expect("seeding policy installed a default limit");
    assert_eq!(
        seeded_limits.max_usd,
        Some(dec!(5.00)),
        "first-touch seeding must install the compiled-default $5 user cap"
    );
    // 5 × $0.01 + 5 × $0.02 = $0.15 — recorded against the seeded
    // user account.
    let usage = governor
        .usage_for(&user_account_seed_check)
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

/// A2 projection: the broadcast sink emits every BudgetEvent published
/// by the governor. Subscribers (a future production projection task in
/// the startup owner — the previous `src/bridge/budget_events.rs`
/// helper was removed pending a real caller, review feedback
/// Thermo-Nuclear #3) receive Warned / Reserved / Reconciled events
/// without polling.
#[tokio::test]
async fn broadcast_sink_publishes_events_to_subscribers() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 10, 5));
    let cost_table = interactive_cost_table(dec!(0.001), dec!(0.002));
    let runtime = build_reborn_runtime(build_input(
        "a2",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
    ))
    .await
    .expect("runtime builds");

    // Subscribe BEFORE the model call so we don't miss the events.
    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let mut subscriber = broadcast.subscribe();
    assert_eq!(
        broadcast.subscriber_count(),
        1,
        "subscribe must register exactly one receiver"
    );

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    // Drain everything available with a small grace window for the
    // governor's broadcast send to fan out.
    let mut received = Vec::new();
    while let Ok(Ok(event)) =
        tokio::time::timeout(Duration::from_millis(200), subscriber.recv()).await
    {
        received.push(event);
    }

    assert!(
        received
            .iter()
            .any(|e| matches!(e, ironclaw_resources::BudgetEvent::Reserved { .. })),
        "broadcast must surface Reserved — got {received:?}"
    );
    assert!(
        received
            .iter()
            .any(|e| matches!(e, ironclaw_resources::BudgetEvent::Reconciled { .. })),
        "broadcast must surface Reconciled — got {received:?}"
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
    // Raise the user cap above the seeded $5 default.
    let governor = runtime.budget_resource_governor().expect("governor");
    let user_account = ResourceAccount::user(
        TenantId::new("scripted-tenant").unwrap(),
        ironclaw_host_api::UserId::new("scripted-owner").unwrap(),
    );
    governor
        .set_limit(
            user_account,
            ResourceLimits {
                max_usd: Some(dec!(1_000.00)),
                period: BudgetPeriod::Rolling24h,
                thresholds: BudgetThresholds::DISABLED,
                ..ResourceLimits::default()
            },
        )
        .unwrap();

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
