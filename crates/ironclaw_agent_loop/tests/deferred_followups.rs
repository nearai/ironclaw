use chrono::{TimeZone, Utc};
use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, CanonicalAgentLoopExecutor},
    families,
    state::{CheckpointKind, LoopExecutionState},
    test_support::{MockAgentLoopDriverHost, MockHostCall, ScenarioScript},
};
use ironclaw_turns::{
    LoopCancelledReasonKind, LoopExit,
    run_profile::{LoopCancelReasonKind, LoopCancellationSignal, LoopRunInfoPort},
};

#[tokio::test]
async fn cancellation_accessor_short_circuits_loop_when_ws13_lands() {
    let signal = LoopCancellationSignal {
        reason_kind: LoopCancelReasonKind::UserRequested,
        requested_at: Utc.with_ymd_and_hms(2026, 5, 14, 12, 0, 0).unwrap(),
    };
    let (host, checkpoints) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("should not be requested"))
        .cancellation_signal(signal)
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should produce a cancellation exit");

    match exit {
        LoopExit::Cancelled(cancelled) => {
            assert_eq!(
                cancelled.reason_kind,
                LoopCancelledReasonKind::HostCancellation
            );
            assert!(cancelled.checkpoint_id.is_some());
        }
        other => panic!("expected cancelled exit, got {other:?}"),
    }
    assert_eq!(host.model_call_count(), 0);
    assert_eq!(
        host.call_log(),
        vec![
            MockHostCall::StageCheckpointPayload(CheckpointKind::Final),
            MockHostCall::SaveCheckpoint(CheckpointKind::Final),
        ]
    );
    checkpoints.assert_sequence(&[(CheckpointKind::Final, 0)]);
}
