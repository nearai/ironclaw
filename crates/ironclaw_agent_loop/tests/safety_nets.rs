use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, CanonicalAgentLoopExecutor},
    families,
    state::{CheckpointKind, LoopExecutionState},
    test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedCapabilityOutcome,
    },
};
use ironclaw_turns::{LoopExit, LoopFailureKind, run_profile::LoopRunInfoPort};

#[tokio::test]
async fn repetition_escape_after_three_iterations() {
    let (host, checkpoints) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::same_calls_repeated("demo.echo", 6))
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::NoProgressDetected);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected no-progress failure, got {other:?}"),
    }
    assert_eq!(host.model_call_count(), 3);
    assert_eq!(
        checkpoints.kinds(),
        vec![
            CheckpointKind::BeforeModel,
            CheckpointKind::BeforeSideEffect,
            CheckpointKind::BeforeModel,
            CheckpointKind::BeforeSideEffect,
            CheckpointKind::BeforeModel,
            CheckpointKind::BeforeSideEffect,
            CheckpointKind::Final,
        ]
    );
}

#[tokio::test]
async fn failure_run_length_escape() {
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::same_failure_repeated(
            "demo.echo",
            "policy_denied",
            3,
        ))
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::NoProgressDetected);
        }
        other => panic!("expected no-progress failure, got {other:?}"),
    }
}

#[tokio::test]
async fn recovery_budget_exhaustion_uses_single_call_retry() {
    let script = ScenarioScript::same_failure_repeated("demo.echo", "transient", 1)
        .with_single_call_retry_outcomes(vec![
            ScriptedCapabilityOutcome::failed("transient"),
            ScriptedCapabilityOutcome::failed("transient"),
        ]);
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::CapabilityProtocolError);
        }
        other => panic!("expected failed exit, got {other:?}"),
    }

    let calls = host.call_log();
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, MockHostCall::InvokeCapabilityBatch { .. }))
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| matches!(call, MockHostCall::InvokeCapability { .. }))
            .count(),
        2
    );
}
