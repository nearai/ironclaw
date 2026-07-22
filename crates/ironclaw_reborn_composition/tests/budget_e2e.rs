//! End-to-end budget pipeline tests.
//!
//! These tests drive [`build_reborn_runtime`] with a stub
//! [`BudgetTestGateway`] paired with a deterministic
//! [`StaticModelCostTable`], then send user messages through
//! `RebornRuntime::send_user_message`. They assert the budget pipeline's
//! observable behavior end-to-end through **production seams only**:
//!
//! * budget limits are configured through the production
//!   [`RebornRuntimeInput::with_budget_defaults`] seam — the same
//!   composition-root path `ironclaw serve` uses — and installed on first
//!   touch by the accountant's seeding policy (so every test here also
//!   exercises that wiring);
//! * emitted [`BudgetEvent`]s are observed through the runtime's public
//!   [`RebornRuntime::broadcast_budget_event_sink`] subscription — the same
//!   fan-out the production SSE/telemetry projection drains — never a
//!   test-only in-memory sink;
//! * spend is asserted from the actual usage carried on the reconcile
//!   receipt (`BudgetEvent::Reconciled { receipt, .. }` → `receipt.actual`),
//!   which is exactly what the ledger records, rather than reaching into the
//!   governor's ledger directly.
//!
//! Scenarios covered (#3841 follow-up E2E coverage):
//!
//! | # | What it asserts |
//! |---|---|
//! | F1 | Happy path within budget — actual USD lands on the reconcile receipt |
//! | F2 | Warn threshold crossed — `BudgetEvent::Warned` emitted; run completes |
//! | F6 | Hard cap denied at `pre_model_call` — no provider call, `Denied` event, no spend |
//! | C1 | Provider tokens reconcile to actual USD (not estimate) |
//! | C2 | Unknown model uses fallback cost (fail-safe non-zero) |
//! | C3 | Free-tier model (`max_*_per_token = 0`) reconciles to zero spend |
//!
//! F7 (cancellation mid-stream) is covered by the in-crate
//! `budget_accountant::release_in_flight_drains_orphan_reservation_on_cancellation`
//! unit test, which exercises the same Drop-guard path with less
//! orchestration noise.
//!
//! F3 / F4 / F5 (approval / cancel / expire flows) and the pause/gate
//! blocking outcome live in `budget_approval_e2e.rs`.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ironclaw_host_api::ResourceUsage;
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_loop_host::{ModelCost, ModelCostTable, StaticModelCostTable};
use ironclaw_reborn_composition::test_support::{BudgetTestGateway, ScriptedReply};
use ironclaw_reborn_composition::{
    BudgetEventObserver, PollSettings, RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput,
    build_reborn_runtime,
};
use ironclaw_reborn_config::BudgetDefaults;
use ironclaw_resources::BudgetEvent;
use ironclaw_turns::TurnStatus;
use ironclaw_turns::run_profile::ModelProfileId;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

/// How long the runtime polls for a turn to complete before giving up.
/// Generous so the turn still finishes when the whole suite of full-runtime
/// tests runs concurrently and contends for CPU/disk; the poll loop exits as
/// soon as the turn is done, so the happy path is unaffected by the ceiling.
const POLL_MAX_TOTAL: Duration = Duration::from_secs(20);

/// Per-test backstop guarding `send_user_message` against a genuine hang.
/// Must be strictly larger than [`POLL_MAX_TOTAL`]: if the two are equal the
/// outer guard races the runtime's own poll budget and fires spuriously under
/// parallel load (the turn finishes right as both deadlines elapse).
const SEND_GUARD_TIMEOUT: Duration = Duration::from_secs(40);

static BUDGET_E2E_SERIAL: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();

async fn budget_e2e_serial_guard() -> tokio::sync::OwnedMutexGuard<()> {
    let gate = BUDGET_E2E_SERIAL
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    gate.lock_owned().await
}

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

/// Compiled production budget defaults with no env overlay. Seeds the
/// production-baseline $5/user, $2/project caps on first touch. Used by tests
/// that only need "some sane cap in place" and drive spend well under it.
fn compiled_defaults() -> BudgetDefaults {
    BudgetDefaults::compiled_defaults()
}

/// Budget defaults that make the **user daily** dimension the single binding
/// constraint at `max_usd` with explicit warn/pause thresholds. The project
/// dimension is set unlimited (`0.0`) so a high-cost estimate resolves against
/// the user cap deterministically, mirroring the intent of the old
/// per-account `governor.set_limit` test setup through the production
/// composition seam. Seeding installs this on the fresh user's first model
/// call (the accountant seeds before the pre-call reservation check).
fn user_cap_defaults(max_usd: f64, warn_at: f64, pause_at: f64) -> BudgetDefaults {
    let mut defaults = BudgetDefaults::compiled_defaults();
    defaults.user_daily_usd = max_usd;
    defaults.project_daily_usd = 0.0;
    defaults.warn_at = warn_at;
    defaults.pause_at = pause_at;
    defaults
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
    budget_defaults: BudgetDefaults,
) -> RebornRuntimeInput {
    RebornRuntimeInput::from_build_input(
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
        max_total: POLL_MAX_TOTAL,
    })
    .with_budget_defaults(budget_defaults)
    .with_model_gateway_override(gateway)
    .with_model_cost_table_override(cost_table)
}

/// Drain every `BudgetEvent` currently buffered on a broadcast subscriber,
/// stopping after a short idle grace window so the governor's async fan-out
/// has time to arrive without pinning the test on a fixed sleep. The caller
/// must `subscribe()` **before** sending the message so no event is missed.
async fn drain_budget_events(
    subscriber: &mut tokio::sync::broadcast::Receiver<BudgetEvent>,
) -> Vec<BudgetEvent> {
    let mut events = Vec::new();
    while let Ok(Ok(event)) =
        tokio::time::timeout(Duration::from_millis(200), subscriber.recv()).await
    {
        events.push(event);
    }
    events
}

/// The actual (post-reconcile) usage carried on every `Reconciled` event.
/// The reconcile receipt's `actual` is the same value the ledger records, so
/// asserting on it verifies the spend end-to-end without reading the governor.
fn reconciled_actuals(events: &[BudgetEvent]) -> Vec<ResourceUsage> {
    events
        .iter()
        .filter_map(|event| match event {
            BudgetEvent::Reconciled { receipt, .. } => receipt.actual.clone(),
            _ => None,
        })
        .collect()
}

/// F1: happy path — request fires, budget depletes by the gateway-reported
/// token usage × cost-table price, the reconcile receipt records exactly that.
#[tokio::test]
async fn f1_happy_path_records_actual_usd_on_receipt() {
    let _serial = budget_e2e_serial_guard().await;
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 10, 5));
    let cost_table = interactive_cost_table(dec!(0.001), dec!(0.002));
    let runtime = build_reborn_runtime(build_input(
        "f1",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
        compiled_defaults(),
    ))
    .await
    .expect("runtime builds");

    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let mut subscriber = broadcast.subscribe();

    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        SEND_GUARD_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");
    assert_eq!(reply.status, TurnStatus::Completed);
    assert_eq!(gateway.call_count(), 1, "exactly one model call expected");

    let events = drain_budget_events(&mut subscriber).await;
    let actuals = reconciled_actuals(&events);
    assert_eq!(
        actuals.len(),
        1,
        "one completed turn reconciles exactly once — got {events:?}"
    );
    // 10 × 0.001 + 5 × 0.002 = 0.020
    assert_eq!(
        actuals[0].usd,
        dec!(0.020),
        "reconciled USD must reflect provider-reported tokens × cost table"
    );
    assert_eq!(actuals[0].input_tokens, 10);
    assert_eq!(actuals[0].output_tokens, 5);

    runtime.shutdown().await.expect("shutdown");
}

/// F2: warn threshold crossed but pause not — reservation succeeds, run
/// completes, and a `Warned` event lands on the broadcast alongside the
/// `Reserved` and `Reconciled` for this turn.
#[tokio::test]
async fn f2_crossing_warn_threshold_emits_warned_event() {
    let _serial = budget_e2e_serial_guard().await;
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 10, 10));
    // Cost-table entry with explicit `max_output_tokens` so the reservation
    // estimate is deterministic and lands between warn=0.5 and pause=0.95
    // against the seeded $10 user cap:
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
        user_cap_defaults(10.00, 0.5, 0.95),
    ))
    .await
    .expect("runtime builds");

    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let mut subscriber = broadcast.subscribe();

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        SEND_GUARD_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    let events = drain_budget_events(&mut subscriber).await;
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
        "Reserved must still fire alongside the warning — got {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Reconciled { .. })),
        "successful run reconciles — got {events:?}"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// F6: hard cap denied — estimate alone exceeds the seeded limit; the
/// accountant emits `Denied` and refuses the reservation before any provider
/// call.
#[tokio::test]
async fn f6_hard_cap_denied_before_provider_call() {
    let _serial = budget_e2e_serial_guard().await;
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("should not reach", 10, 10));
    // High prices overflow the tiny seeded user cap on the pre-call estimate.
    let cost_table = interactive_cost_table(dec!(0.10), dec!(0.10));
    let runtime = build_reborn_runtime(build_input(
        "f6",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
        // Not "unlimited" (that is `0.0`): a real, tiny cap the estimate blows.
        user_cap_defaults(0.000_001, 0.5, 0.95),
    ))
    .await
    .expect("runtime builds");

    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let mut subscriber = broadcast.subscribe();

    let conversation = runtime.new_conversation().await.expect("conversation");
    let outcome = tokio::time::timeout(
        SEND_GUARD_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes");
    // The send either errors or returns a non-Completed status; either counts
    // as "denied before provider call". What MUST hold: zero gateway calls and
    // a Denied event on the broadcast.
    assert_eq!(
        gateway.call_count(),
        0,
        "hard-cap denial must short-circuit before any model call"
    );
    let events = drain_budget_events(&mut subscriber).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Denied { .. })),
        "hard cap must emit Denied — got {events:?}"
    );
    #[allow(clippy::let_underscore_must_use)]
    // outcome intentionally unused; the assertions above check the side effects
    let _ = outcome;

    runtime.shutdown().await.expect("shutdown");
}

/// C1: provider tokens reconcile to actual USD via the cost table, not to the
/// (conservative) reservation estimate.
#[tokio::test]
async fn c1_provider_tokens_reconcile_to_actual_usd() {
    let _serial = budget_e2e_serial_guard().await;
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 3, 7));
    let cost_table = interactive_cost_table(dec!(0.05), dec!(0.10));
    // Raise the user cap well above the pre-call estimate (~$6.24 at these
    // prices) with thresholds effectively disabled (warn/pause at 100%) so the
    // turn neither warns nor pauses.
    let runtime = build_reborn_runtime(build_input(
        "c1",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
        user_cap_defaults(1_000.00, 1.0, 1.0),
    ))
    .await
    .expect("runtime builds");

    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let mut subscriber = broadcast.subscribe();

    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        SEND_GUARD_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");
    assert_eq!(reply.status, TurnStatus::Completed);

    let events = drain_budget_events(&mut subscriber).await;
    let actuals = reconciled_actuals(&events);
    assert_eq!(
        actuals.len(),
        1,
        "one turn reconciles once — got {events:?}"
    );
    // 3 × $0.05 + 7 × $0.10 = $0.85 — exact, not the overestimate.
    assert_eq!(actuals[0].usd, dec!(0.85));
    assert_eq!(actuals[0].input_tokens, 3);
    assert_eq!(actuals[0].output_tokens, 7);

    runtime.shutdown().await.expect("shutdown");
}

/// C2: unknown model profile in the cost table → accountant uses the table's
/// `cost_for` returning `None` → the accountant's `default_cost` fallback fires
/// (conservative ~GPT-4o pricing) so the reconcile receipt records *non-zero*
/// spend. Fail-closed shape from review feedback Medium #5: a paid model
/// missing from the cost table must NOT silently reconcile to zero.
#[tokio::test]
async fn c2_unknown_model_in_cost_table_uses_default_cost_fallback() {
    let _serial = budget_e2e_serial_guard().await;
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 10, 10));
    // Empty cost table — no entry for "interactive_model".
    let cost_table: Arc<dyn ModelCostTable> = Arc::new(StaticModelCostTable::new());
    let runtime = build_reborn_runtime(build_input(
        "c2",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
        compiled_defaults(),
    ))
    .await
    .expect("runtime builds");

    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let mut subscriber = broadcast.subscribe();

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        SEND_GUARD_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    let events = drain_budget_events(&mut subscriber).await;
    let actuals = reconciled_actuals(&events);
    assert_eq!(
        actuals.len(),
        1,
        "one turn reconciles once — got {events:?}"
    );
    // ~$0.000125 at the fallback price — what matters is strictly greater than
    // zero. Silently recording zero for an unknown paid model is the bug.
    assert!(
        actuals[0].usd > Decimal::ZERO,
        "unknown model must NOT silently reconcile to zero USD (got {})",
        actuals[0].usd,
    );
    assert_eq!(actuals[0].input_tokens, 10);
    assert_eq!(actuals[0].output_tokens, 10);

    runtime.shutdown().await.expect("shutdown");
}

/// C3: zero-cost model (Ollama / free-tier) — every turn reconciles to $0.00
/// even with high token counts.
#[tokio::test]
async fn c3_zero_cost_model_records_zero_spend() {
    let _serial = budget_e2e_serial_guard().await;
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 1000, 2000));
    let cost_table = interactive_cost_table(Decimal::ZERO, Decimal::ZERO);
    let runtime = build_reborn_runtime(build_input(
        "c3",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
        compiled_defaults(),
    ))
    .await
    .expect("runtime builds");

    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let mut subscriber = broadcast.subscribe();

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        SEND_GUARD_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    let events = drain_budget_events(&mut subscriber).await;
    let actuals = reconciled_actuals(&events);
    assert_eq!(
        actuals.len(),
        1,
        "one turn reconciles once — got {events:?}"
    );
    assert_eq!(
        actuals[0].usd,
        Decimal::ZERO,
        "free model reconciles to zero USD"
    );
    assert_eq!(actuals[0].input_tokens, 1000);
    assert_eq!(actuals[0].output_tokens, 2000);

    runtime.shutdown().await.expect("shutdown");
}

/// A2 projection: the broadcast sink emits every BudgetEvent published by the
/// governor. Subscribers (the production projection task wired by
/// `build_reborn_runtime`, plus any additional consumer that subscribes
/// directly) receive Warned / Reserved / Reconciled events without polling.
#[tokio::test]
async fn broadcast_sink_publishes_events_to_subscribers() {
    let _serial = budget_e2e_serial_guard().await;
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 10, 5));
    let cost_table = interactive_cost_table(dec!(0.001), dec!(0.002));
    let runtime = build_reborn_runtime(build_input(
        "a2",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
        compiled_defaults(),
    ))
    .await
    .expect("runtime builds");

    // The runtime always spawns its own projection task, which holds one
    // receiver. Subscribe BEFORE the model call so we don't miss the events
    // and confirm the test subscriber is additive to the production projection
    // (count goes 1 -> 2).
    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let baseline_subscribers = broadcast.subscriber_count();
    let mut subscriber = broadcast.subscribe();
    assert_eq!(
        broadcast.subscriber_count(),
        baseline_subscribers + 1,
        "subscribe must register exactly one receiver"
    );
    assert!(
        baseline_subscribers >= 1,
        "the runtime's own projection task must already be subscribed before the test \
         subscriber attaches — got baseline={baseline_subscribers}"
    );

    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        SEND_GUARD_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    let received = drain_budget_events(&mut subscriber).await;
    assert!(
        received
            .iter()
            .any(|e| matches!(e, BudgetEvent::Reserved { .. })),
        "broadcast must surface Reserved — got {received:?}"
    );
    assert!(
        received
            .iter()
            .any(|e| matches!(e, BudgetEvent::Reconciled { .. })),
        "broadcast must surface Reconciled — got {received:?}"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// Scripted multi-turn smoke: two messages, two replies with different token
/// counts → the reconcile receipts sum to the total. Exercises the
/// script-queue path of `BudgetTestGateway::push`.
#[tokio::test]
async fn budget_test_gateway_scripted_replies_drive_per_turn_costs() {
    let _serial = budget_e2e_serial_guard().await;
    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::new());
    gateway.push(ScriptedReply::new("turn-1", 4, 6));
    gateway.push(ScriptedReply::new("turn-2", 2, 8));
    let cost_table = interactive_cost_table(dec!(0.05), dec!(0.10));
    // Raise the user cap above the estimate with thresholds disabled.
    let runtime = build_reborn_runtime(build_input(
        "scripted",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
        user_cap_defaults(1_000.00, 1.0, 1.0),
    ))
    .await
    .expect("runtime builds");

    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let mut subscriber = broadcast.subscribe();

    let conversation = runtime.new_conversation().await.expect("conversation");
    for prompt in ["first", "second"] {
        let _ = tokio::time::timeout(
            SEND_GUARD_TIMEOUT,
            runtime.send_user_message(&conversation, prompt),
        )
        .await
        .expect("send finishes")
        .expect("send succeeds");
    }
    assert_eq!(gateway.call_count(), 2);

    let events = drain_budget_events(&mut subscriber).await;
    let actuals = reconciled_actuals(&events);
    assert_eq!(
        actuals.len(),
        2,
        "two completed turns reconcile twice — got {events:?}"
    );
    let total_usd: Decimal = actuals.iter().map(|usage| usage.usd).sum();
    let total_input: u64 = actuals.iter().map(|usage| usage.input_tokens).sum();
    let total_output: u64 = actuals.iter().map(|usage| usage.output_tokens).sum();
    // Turn 1: 4 × $0.05 + 6 × $0.10 = $0.80
    // Turn 2: 2 × $0.05 + 8 × $0.10 = $0.90
    // Total: $1.70
    assert_eq!(total_usd, dec!(1.70));
    assert_eq!(total_input, 6);
    assert_eq!(total_output, 14);

    runtime.shutdown().await.expect("shutdown");
}

/// Regression for #3841 A2 / Thermo-Nuclear #3 (now wired): the runtime's
/// budget-event broadcast sink must actually deliver events to a
/// [`BudgetEventObserver`] installed through
/// [`RebornRuntimeInput::with_budget_event_observer`]. This goes through the
/// full runtime caller (build → send → shutdown) and asserts the observer
/// sees the same `Reserved` / `Reconciled` shape the broadcast surfaces.
#[tokio::test]
async fn projection_delivers_budget_events_to_installed_observer() {
    let _serial = budget_e2e_serial_guard().await;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct CapturingObserver {
        events: Mutex<Vec<BudgetEvent>>,
    }

    impl BudgetEventObserver for CapturingObserver {
        fn observe(&self, event: BudgetEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    let root = tempfile::tempdir().unwrap();
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 3, 7));
    let cost_table = interactive_cost_table(dec!(0.001), dec!(0.001));
    let observer = Arc::new(CapturingObserver::default());

    let input = build_input(
        "proj",
        root.path().to_path_buf(),
        gateway.clone(),
        cost_table,
        compiled_defaults(),
    )
    .with_budget_event_observer(Arc::clone(&observer) as Arc<dyn BudgetEventObserver>);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");
    let _ = tokio::time::timeout(
        SEND_GUARD_TIMEOUT,
        runtime.send_user_message(&conversation, "ping"),
    )
    .await
    .expect("send finishes")
    .expect("send succeeds");

    // Give the projection task a small window to drain. The broadcast is
    // non-blocking on emit; the projection task observes on its own tokio task
    // and may not have run yet when send_user_message returns.
    let saw_reconciled = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let reconciled = {
                let events = observer.events.lock().unwrap();
                events
                    .iter()
                    .any(|event| matches!(event, BudgetEvent::Reconciled { .. }))
            };
            if reconciled {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false);
    assert!(
        saw_reconciled,
        "observer must receive Reconciled after a successful turn"
    );

    runtime.shutdown().await.expect("shutdown");

    // After shutdown the observer must have seen at minimum the turn's
    // Reserved + Reconciled pair from the model call. Any additional events
    // (Warned at low-default threshold, etc.) are tolerated — the contract is
    // "no events get silently dropped".
    let events = observer.events.lock().unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Reserved { .. })),
        "observer must receive Reserved — got {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BudgetEvent::Reconciled { .. })),
        "observer must receive Reconciled — got {events:?}"
    );
}
