use std::collections::VecDeque;

use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, CanonicalAgentLoopExecutor},
    families,
    state::{CheckpointKind, LoopExecutionState},
    test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedCapabilityCall,
        ScriptedCapabilityOutcome, ScriptedModelResponse,
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
        LoopExit::Completed(completed) => {
            assert_eq!(completed.reply_message_refs.len(), 1);
            assert!(completed.final_checkpoint_id.is_some());
        }
        other => panic!("expected no-progress fallback completion, got {other:?}"),
    }
    assert_no_progress_fallback(&host);
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
async fn typed_no_progress_results_escape_without_repeated_call_signature() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo_1")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo_2")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo_3")]),
        ]),
        capability_outcomes: VecDeque::from([
            vec![ScriptedCapabilityOutcome::completed_no_change(
                "result:no-change-1",
            )],
            vec![ScriptedCapabilityOutcome::completed_no_change(
                "result:no-change-2",
            )],
            vec![ScriptedCapabilityOutcome::completed_no_change(
                "result:no-change-3",
            )],
        ]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(completed.reply_message_refs.len(), 1);
            assert!(completed.final_checkpoint_id.is_some());
        }
        other => panic!("expected no-progress fallback completion, got {other:?}"),
    }
    assert_no_progress_fallback(&host);
    assert_eq!(host.model_call_count(), 3);
}

#[tokio::test]
async fn typed_blocked_results_escape_without_repeated_call_signature() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo_1")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo_2")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo_3")]),
        ]),
        capability_outcomes: VecDeque::from([
            vec![ScriptedCapabilityOutcome::completed_blocked(
                "result:blocked-1",
            )],
            vec![ScriptedCapabilityOutcome::completed_blocked(
                "result:blocked-2",
            )],
            vec![ScriptedCapabilityOutcome::completed_blocked(
                "result:blocked-3",
            )],
        ]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(completed.reply_message_refs.len(), 1);
            assert!(completed.final_checkpoint_id.is_some());
        }
        other => panic!("expected no-progress fallback completion, got {other:?}"),
    }
    assert_no_progress_fallback(&host);
    assert_eq!(host.model_call_count(), 3);
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
        LoopExit::Completed(completed) => {
            assert_eq!(completed.reply_message_refs.len(), 1);
            assert!(completed.final_checkpoint_id.is_some());
        }
        other => panic!("expected no-progress fallback completion, got {other:?}"),
    }
    assert_no_progress_fallback(&host);
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
            assert_eq!(failed.reason_kind, LoopFailureKind::ModelError);
        }
        other => panic!("expected failed exit, got {other:?}"),
    }

    let calls = host.call_log();
    assert!(
        calls.starts_with(&[
            MockHostCall::PollInputs,
            MockHostCall::VisibleCapabilities,
            MockHostCall::BuildPromptBundle,
            MockHostCall::StageCheckpointPayload(CheckpointKind::BeforeModel),
            MockHostCall::SaveCheckpoint(CheckpointKind::BeforeModel),
            MockHostCall::StreamModel,
            MockHostCall::StageCheckpointPayload(CheckpointKind::BeforeSideEffect),
            MockHostCall::SaveCheckpoint(CheckpointKind::BeforeSideEffect),
        ]),
        "retry result ordering should stay on the wire; got {calls:?}"
    );
    assert!(matches!(
        calls.get(8),
        Some(MockHostCall::InvokeCapabilityBatch { .. })
    ));
    assert!(matches!(
        calls.get(9),
        Some(MockHostCall::InvokeCapability { .. })
    ));
    assert!(matches!(
        calls.get(10),
        Some(MockHostCall::InvokeCapability { .. })
    ));
    let final_calls = &calls[calls.len().saturating_sub(2)..];
    assert_eq!(
        final_calls,
        [
            MockHostCall::StageCheckpointPayload(CheckpointKind::Final),
            MockHostCall::SaveCheckpoint(CheckpointKind::Final)
        ]
    );
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

fn assert_no_progress_fallback(host: &MockAgentLoopDriverHost) {
    let messages = host.finalized_assistant_messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("repeating the same step without making progress"));
    assert!(messages[0].contains("repeated calls, results, and any failure summaries"));
}
