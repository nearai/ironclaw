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
async fn repeated_signature_warns_before_allowing_final_reply() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "done after warning".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([
            vec![ScriptedCapabilityOutcome::completed("result:repeat-1")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-2")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-3")],
        ]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, checkpoints) = MockAgentLoopDriverHost::builder().script(script).build();
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
        other => panic!("expected final reply completion, got {other:?}"),
    }
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["done after warning"]
    );
    assert_eq!(host.model_call_count(), 4);
    assert_eq!(repeated_call_warning_prompt_count(&host), 1);
    assert_eq!(
        checkpoints.kinds(),
        vec![
            CheckpointKind::BeforeModel,
            CheckpointKind::BeforeSideEffect,
            CheckpointKind::BeforeModel,
            CheckpointKind::BeforeSideEffect,
            CheckpointKind::BeforeModel,
            CheckpointKind::BeforeSideEffect,
            CheckpointKind::BeforeModel,
            CheckpointKind::Final,
        ]
    );
}

#[tokio::test]
async fn repeated_signature_stops_after_rendered_warning_and_no_progress_result() {
    let script =
        ScenarioScript::same_calls_repeated("demo.echo", 4).with_capability_outcomes(vec![
            vec![ScriptedCapabilityOutcome::completed("result:repeat-1")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-2")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-3")],
            vec![ScriptedCapabilityOutcome::completed_no_change(
                "result:repeat-4",
            )],
        ]);
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
    assert_eq!(host.model_call_count(), 4);
    assert_eq!(repeated_call_warning_prompt_count(&host), 1);
}

#[tokio::test]
async fn repeated_signature_made_progress_after_warning_clears_warning_and_continues() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "done after progress".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([
            vec![ScriptedCapabilityOutcome::completed("result:repeat-1")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-2")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-3")],
            vec![ScriptedCapabilityOutcome::completed("result:repeat-4")],
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
        other => panic!("expected final reply completion, got {other:?}"),
    }
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["done after progress"]
    );
    assert_eq!(host.model_call_count(), 5);
    assert_eq!(repeated_call_warning_prompt_count(&host), 1);
    assert!(
        host.prompt_requests()
            .last()
            .expect("final prompt request")
            .inline_messages
            .is_empty(),
        "warning should be cleared before the final reply prompt"
    );
}

#[tokio::test]
async fn typed_no_progress_results_escape_without_repeated_call_signature() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![call_with_input("input:no-change-1")]),
            ScriptedModelResponse::Calls(vec![call_with_input("input:no-change-2")]),
            ScriptedModelResponse::Calls(vec![call_with_input("input:no-change-3")]),
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
            ScriptedModelResponse::Calls(vec![call_with_input("input:blocked-1")]),
            ScriptedModelResponse::Calls(vec![call_with_input("input:blocked-2")]),
            ScriptedModelResponse::Calls(vec![call_with_input("input:blocked-3")]),
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
async fn repeated_failure_kind_does_not_trigger_no_progress_escape() {
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
            assert_eq!(failed.reason_kind, LoopFailureKind::ModelError);
        }
        other => panic!("expected model exhaustion failure after continuing, got {other:?}"),
    }
    assert!(host.finalized_assistant_messages().is_empty());
    assert!(
        host.model_call_count() > 3,
        "coarse repeated failure kinds must not stop the run at the old threshold"
    );
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

fn call_with_input(input_ref: &str) -> ScriptedCapabilityCall {
    ScriptedCapabilityCall {
        name: "demo.echo".to_string(),
        input_ref: input_ref.to_string(),
    }
}

fn repeated_call_warning_prompt_count(host: &MockAgentLoopDriverHost) -> usize {
    host.prompt_requests()
        .iter()
        .filter(|request| {
            request.inline_messages.iter().any(|message| {
                message.safe_body.as_str()
                    == "loop control repeated capability call detected change strategy explain new evidence or answer from current evidence"
            })
        })
        .count()
}
