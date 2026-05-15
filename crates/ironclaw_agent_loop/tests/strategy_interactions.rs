use std::collections::VecDeque;

use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, CanonicalAgentLoopExecutor},
    families,
    state::LoopExecutionState,
    test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedCapabilityCall,
        ScriptedCapabilityOutcome, ScriptedModelResponse,
    },
};
use ironclaw_turns::{LoopExit, LoopFailureKind, run_profile::LoopRunInfoPort};

#[tokio::test]
async fn gate_blocks_before_recovery_budget_exhausts() {
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::approval_required("demo.echo"))
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    assert!(matches!(exit, LoopExit::Blocked(_)));
    assert_eq!(
        host.call_log()
            .iter()
            .filter(|call| matches!(call, MockHostCall::InvokeCapability { .. }))
            .count(),
        0,
        "gate handling should not enter the retry path"
    );
}

#[tokio::test]
async fn terminate_hint_after_batch_stops_without_extra_model_call() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([ScriptedModelResponse::Calls(vec![
            ScriptedCapabilityCall::new("demo.echo"),
        ])]),
        capability_outcomes: VecDeque::from([vec![
            ScriptedCapabilityOutcome::completed_with_terminate_hint("result:done"),
        ]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(host.model_call_count(), 1);
}

#[tokio::test]
async fn denied_call_skips_and_repetition_net_catches_stuck_denials() {
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
    assert_eq!(host.model_call_count(), 3);
}

#[tokio::test]
async fn retries_do_not_push_signatures_again() {
    let script = ScenarioScript::same_failure_repeated("demo.echo", "transient", 1)
        .with_single_call_retry_outcomes(vec![
            ScriptedCapabilityOutcome::completed_with_terminate_hint("result:retry"),
        ]);
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.call_log()
            .iter()
            .filter(|call| matches!(call, MockHostCall::InvokeCapability { .. }))
            .count(),
        1
    );
}
