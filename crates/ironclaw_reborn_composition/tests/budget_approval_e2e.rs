//! Budget pause / approval-gate E2E tests (#3841 follow-ups).
//!
//! These tests drive a `send_user_message` past the pause threshold so the
//! accountant returns `BudgetApprovalRequired`, opens a budget gate, and the
//! turn parks in `BlockedResource`. They assert that **observable** blocking
//! outcome through production seams only:
//!
//! * the budget cap is configured through the production
//!   [`RebornRuntimeInput::with_budget_defaults`] seam and seeded on the fresh
//!   user's first model call;
//! * the pause is observed through `send_user_message_until_gate`, which
//!   surfaces the same `RebornTurnDriveOutcome::BlockedOnGate { gate_ref, .. }`
//!   a real caller (the WebUI product surface) sees when a run parks on a gate;
//! * the `BudgetEvent::GateOpened` carrying the real persisted gate id is
//!   observed through the public
//!   [`RebornRuntime::broadcast_budget_event_sink`] subscription — the same
//!   fan-out the production projection drains.
//!
//! ## Not covered here — no production resolution seam
//!
//! The former F3 (approve-with-increased-limit → retry succeeds), F4 (cancel →
//! retry stays blocked), and F5 (expire → retry stays blocked) scenarios drove
//! budget-gate **resolution** directly through the runtime's
//! `budget_gate_store()` + `apply_resolved_budget_gate()` test accessors.
//! Those accessors were removed with the runtime store unification, and budget
//! gate resolution (approve / cancel / expire) is **not** wired to any
//! production caller today (the gate store is written only by the accountant
//! opening gates; nothing resolves them outside the deleted test hooks). There
//! is therefore no observable seam through which to drive resolution, so those
//! scenarios are intentionally not reproduced here. See the accompanying report
//! for the STOP rationale; they should return once a real gate-resolution route
//! exists (the runtime comment pointed at a future web-gateway resolution
//! surface).

use std::sync::Arc;
use std::time::Duration;

use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_loop_host::{ModelCost, ModelCostTable, StaticModelCostTable};
use ironclaw_reborn_composition::test_support::BudgetTestGateway;
use ironclaw_reborn_composition::{
    PollSettings, RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput, RebornTurnDriveOutcome,
    build_reborn_runtime,
};
use ironclaw_reborn_config::BudgetDefaults;
use ironclaw_resources::BudgetGateId;
use ironclaw_turns::GateRef;

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

/// Assert a turn-drive outcome is the expected budget pause, returning the
/// budget gate id embedded in the surfaced `gate_ref`. Mirrors what a real
/// caller reads off `BlockedOnGate` without touching the gate store.
fn assert_budget_blocked_outcome(outcome: RebornTurnDriveOutcome) -> BudgetGateId {
    match outcome {
        RebornTurnDriveOutcome::BlockedOnGate {
            status,
            gate_ref,
            partial_text,
            ..
        } => {
            assert_eq!(
                status,
                ironclaw_turns::TurnStatus::BlockedResource,
                "unexpected budget approval status"
            );
            assert_eq!(partial_text, None);
            budget_gate_id_from_ref(&gate_ref)
        }
        RebornTurnDriveOutcome::Terminal(reply) => {
            panic!("budget approval should block instead of terminal reply: {reply:?}");
        }
    }
}

/// Parse the `gate:budget-<uuid>` ref a paused budget run surfaces back into
/// the typed [`BudgetGateId`], so the id can be compared against the
/// `GateOpened` broadcast event without reading the gate store.
fn budget_gate_id_from_ref(gate_ref: &GateRef) -> BudgetGateId {
    let raw_id = gate_ref
        .as_str()
        .strip_prefix("gate:budget-")
        .expect("budget approval should block on a budget gate ref");
    BudgetGateId::from_uuid(
        uuid::Uuid::parse_str(raw_id).expect("budget gate ref should contain a UUID"),
    )
}

/// Cost table tuned so a default-size reservation lands above `pause_at`
/// against the seeded $10 user cap:
///   estimate = 64 input × $0.05 + 20 output × $0.10 = $5.20 × 1.20 = $6.24
/// → 62.4% utilization, above pause(0.5) but below the hard ceiling → the
/// cascade returns ApprovalRequired and opens a gate rather than denying.
fn pause_inducing_cost_table() -> Arc<dyn ModelCostTable> {
    let mut table = StaticModelCostTable::new();
    table.insert(
        ironclaw_turns::run_profile::ModelProfileId::new("interactive_model").unwrap(),
        ModelCost {
            input_per_token: rust_decimal_macros::dec!(0.05),
            output_per_token: rust_decimal_macros::dec!(0.10),
            max_output_tokens: 20,
        },
    );
    Arc::new(table)
}

/// Budget defaults that seed a $10 user cap with a low warn(0.2)/pause(0.5)
/// band — the production composition seam that replaces the old per-account
/// `governor.set_limit`. The project dimension is unlimited so the user cap is
/// the single binding constraint.
fn pause_inducing_budget_defaults() -> BudgetDefaults {
    let mut defaults = BudgetDefaults::compiled_defaults();
    defaults.user_daily_usd = 10.00;
    defaults.project_daily_usd = 0.0;
    defaults.warn_at = 0.2;
    defaults.pause_at = 0.5;
    defaults
}

async fn build_runtime_with_pause_inducing_setup(
    tag: &str,
    root: std::path::PathBuf,
) -> (RebornRuntime, Arc<BudgetTestGateway>) {
    let gateway = Arc::new(BudgetTestGateway::with_constant("ok", 5, 5));
    let input = RebornRuntimeInput::from_build_input(
        ironclaw_reborn_composition::local_dev_build_input(format!("{tag}-owner"), root)
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
    .with_budget_defaults(pause_inducing_budget_defaults())
    .with_model_gateway_override(gateway.clone())
    .with_model_cost_table_override(pause_inducing_cost_table());
    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    (runtime, gateway)
}

/// Drive the runtime past the pause threshold and return the budget gate id
/// surfaced on the blocked outcome. The send must park (no gateway call, no
/// completion).
async fn pump_until_pause(runtime: &RebornRuntime, gateway: &BudgetTestGateway) -> BudgetGateId {
    let conversation = runtime.new_conversation().await.expect("conversation");
    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.send_user_message_until_gate(&conversation, "first try"),
    )
    .await
    .expect("send finishes")
    .expect("budget approval should return a blocked gate outcome");
    let gate_id = assert_budget_blocked_outcome(reply);
    assert_eq!(
        gateway.call_count(),
        0,
        "pause threshold must short-circuit before any model call"
    );
    gate_id
}

/// Crossing the pause threshold parks the turn on a budget gate: the run
/// reaches `BlockedResource`, surfaces a `gate:budget-<uuid>` ref, and never
/// dispatches the model. This is the observable core the former F3/F4/F5
/// approval scenarios all began with; the resolution half of those scenarios
/// has no production seam (see module docs).
#[tokio::test]
async fn budget_pause_blocks_run_on_budget_gate() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, gateway) =
        build_runtime_with_pause_inducing_setup("pause", root.path().to_path_buf()).await;

    let gate_id = pump_until_pause(&runtime, &gateway).await;
    // A real, parseable budget gate id was surfaced (not a placeholder).
    assert_ne!(gate_id, BudgetGateId::from_uuid(uuid::Uuid::nil()));

    runtime.shutdown().await.expect("shutdown");
}

/// Regression for the invented-gate-id bug: when the cascade pauses, the
/// accountant emits `BudgetEvent::GateOpened` with the *real* `BudgetGateId`
/// it just persisted. The broadcast must carry that same id the paused run
/// surfaces in its `gate_ref`, so a subscriber can resolve the gate it was
/// notified about — asserted here through the public broadcast subscription
/// and the blocked-outcome gate ref, no gate-store peek.
#[tokio::test]
async fn gate_opened_event_id_matches_blocked_outcome_gate_ref() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, gateway) =
        build_runtime_with_pause_inducing_setup("gate-id", root.path().to_path_buf()).await;

    // Subscribe BEFORE the send so we don't miss the GateOpened event.
    let broadcast = runtime
        .broadcast_budget_event_sink()
        .expect("broadcast sink");
    let mut subscriber = broadcast.subscribe();

    let blocked_gate_id = pump_until_pause(&runtime, &gateway).await;

    // Drain the broadcast and find the GateOpened event's id.
    let mut received_gate_id = None;
    while let Ok(Ok(event)) =
        tokio::time::timeout(Duration::from_millis(200), subscriber.recv()).await
    {
        if let ironclaw_resources::BudgetEvent::GateOpened { gate_id, .. } = event {
            received_gate_id = Some(gate_id);
            break;
        }
    }
    let received = received_gate_id.expect("GateOpened reached the broadcast");
    assert_eq!(
        received, blocked_gate_id,
        "the GateOpened event's gate_id must match the gate id the paused run surfaced"
    );

    runtime.shutdown().await.expect("shutdown");
}

/// Two distinct paused runs (each a fresh conversation/run) produce two
/// distinct budget gates. The accountant rejects a second concurrent
/// reservation for the same `TurnRunId`, so each user-visible attempt gets its
/// own gate. Asserted through the distinct gate refs each blocked outcome
/// surfaces, rather than counting rows in the gate store.
#[tokio::test]
async fn pause_in_distinct_runs_produces_distinct_gates() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, gateway) =
        build_runtime_with_pause_inducing_setup("dup", root.path().to_path_buf()).await;

    let gate_a = pump_until_pause(&runtime, &gateway).await;
    let gate_b = pump_until_pause(&runtime, &gateway).await;
    assert_ne!(
        gate_a, gate_b,
        "two distinct paused runs must open two distinct budget gates"
    );

    runtime.shutdown().await.expect("shutdown");
}
