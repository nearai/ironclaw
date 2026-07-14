use std::collections::VecDeque;

use chrono::{TimeZone, Utc};
use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, CanonicalAgentLoopExecutor},
    families,
    state::{CheckpointKind, LoopExecutionState},
    test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedCapabilityCall,
        ScriptedCapabilityOutcome, ScriptedModelResponse, capability_id, surface_version,
    },
};
use ironclaw_turns::{
    CapabilityActivityId, LoopExit, LoopFailureKind,
    run_profile::{
        AgentLoopHostErrorKind, CapabilityBatchInvocation, CapabilityInputRef,
        CapabilityInvocation, ContentDigest, LoopCancelReasonKind, LoopCancellationPort,
        LoopCancellationSignal, LoopCapabilityPort, LoopRunInfoPort,
    },
};

#[tokio::test(start_paused = true)]
async fn cancel_after_capability_batch_is_consumed_once() {
    let first_signal = LoopCancellationSignal {
        reason_kind: LoopCancelReasonKind::UserRequested,
        requested_at: Utc.with_ymd_and_hms(2026, 6, 12, 10, 0, 0).unwrap(),
    };
    let second_signal = LoopCancellationSignal {
        reason_kind: LoopCancelReasonKind::Policy,
        requested_at: Utc.with_ymd_and_hms(2026, 6, 12, 10, 1, 0).unwrap(),
    };
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript {
            model_responses: VecDeque::new(),
            capability_outcomes: VecDeque::from([
                vec![ScriptedCapabilityOutcome::completed("result:first")],
                vec![ScriptedCapabilityOutcome::completed("result:second")],
            ]),
            single_call_retry_outcomes: VecDeque::new(),
            pending_inputs: VecDeque::new(),
        })
        .cancel_after_capability_batch(first_signal.clone())
        .build();
    let request = CapabilityBatchInvocation {
        invocations: vec![CapabilityInvocation {
            surface_version: surface_version(),
            capability_id: capability_id("demo.echo"),
            activity_id: CapabilityActivityId::new(),
            input_ref: CapabilityInputRef::new("input:one-shot").unwrap(),
            approval_resume: None,
            auth_resume: None,
        }],
        stop_on_first_suspension: false,
    };

    host.invoke_capability_batch(request.clone()).await.unwrap();
    assert_eq!(host.observe_cancellation(), Some(first_signal));

    host.set_cancellation_signal(second_signal.clone());
    host.invoke_capability_batch(request).await.unwrap();
    assert_eq!(host.observe_cancellation(), Some(second_signal));
}

#[tokio::test(start_paused = true)]
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

#[tokio::test(start_paused = true)]
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
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::NoProgressDetected);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected typed no-progress failure, got {other:?}"),
    }
    assert_no_progress_typed_failure(&host);
    assert_eq!(host.model_call_count(), 4);
    assert_eq!(repeated_call_warning_prompt_count(&host), 1);
}

#[tokio::test(start_paused = true)]
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

#[tokio::test(start_paused = true)]
async fn repeated_identical_output_digest_trips_no_progress() {
    // PR3: the same call producing the SAME output (identical content digest)
    // every turn is genuine no-progress — the guard fires (typed
    // NoProgressDetected, nudge gate off), even though the host tags each
    // completed result MadeProgress. This is the load-bearing output-aware case.
    let digest = ContentDigest(7);
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "done after identical output digests".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([
            vec![ScriptedCapabilityOutcome::completed_with_output_digest(
                "result:repeat-digest-1",
                digest,
            )],
            vec![ScriptedCapabilityOutcome::completed_with_output_digest(
                "result:repeat-digest-2",
                digest,
            )],
            vec![ScriptedCapabilityOutcome::completed_with_output_digest(
                "result:repeat-digest-3",
                digest,
            )],
            vec![ScriptedCapabilityOutcome::completed_with_output_digest(
                "result:repeat-digest-4",
                digest,
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
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::NoProgressDetected);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected typed no-progress failure, got {other:?}"),
    }
    assert_no_progress_typed_failure(&host);
}

#[tokio::test(start_paused = true)]
async fn changing_output_digests_do_not_trip_no_progress() {
    // PR3 counterpart: the SAME call returning DIFFERENT output each turn
    // (polling / pagination that advances) is real progress — every new digest
    // is MadeProgress, so the guard never fires and the run completes normally,
    // even though the call signature repeats. This is exactly the false positive
    // the output-aware signal removes.
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "done after advancing output".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([
            vec![ScriptedCapabilityOutcome::completed_with_output_digest(
                "result:poll-1",
                ContentDigest(1),
            )],
            vec![ScriptedCapabilityOutcome::completed_with_output_digest(
                "result:poll-2",
                ContentDigest(2),
            )],
            vec![ScriptedCapabilityOutcome::completed_with_output_digest(
                "result:poll-3",
                ContentDigest(3),
            )],
            vec![ScriptedCapabilityOutcome::completed_with_output_digest(
                "result:poll-4",
                ContentDigest(4),
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
        other => panic!("changing output is progress; expected completion, got {other:?}"),
    }
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["done after advancing output"]
    );
}

#[tokio::test(start_paused = true)]
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
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::NoProgressDetected);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected typed no-progress failure, got {other:?}"),
    }
    assert_no_progress_typed_failure(&host);
    assert_eq!(host.model_call_count(), 3);
}

#[tokio::test(start_paused = true)]
async fn typed_blocked_results_do_not_escape_via_no_progress() {
    // PR3: blocked/failed results are NOT no-progress (only a repeated identical
    // output is). Three blocked batches (distinct inputs, so no repeated-call
    // signature either) do not fire NoProgressDetected; the run continues and
    // completes once the model recovers with a reply.
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![call_with_input("input:blocked-1")]),
            ScriptedModelResponse::Calls(vec![call_with_input("input:blocked-2")]),
            ScriptedModelResponse::Calls(vec![call_with_input("input:blocked-3")]),
            ScriptedModelResponse::Reply {
                text: "recovered after blocked tools".to_string(),
            },
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
        other => {
            panic!("blocked failures must not trip no-progress; expected completion, got {other:?}")
        }
    }
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["recovered after blocked tools"]
    );
}

#[tokio::test(start_paused = true)]
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

#[tokio::test(start_paused = true)]
async fn chaos_repeated_model_service_drops_report_model_error() {
    let script = ScenarioScript {
        model_responses: (0..8)
            .map(|_| ScriptedModelResponse::Error {
                kind: AgentLoopHostErrorKind::Unavailable,
            })
            .collect(),
        capability_outcomes: VecDeque::new(),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should produce controlled failed exit");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::ModelError);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected model-error failed exit, got {other:?}"),
    }
    assert!(
        host.model_call_count() >= 3,
        "model recovery should retry before returning a controlled failure"
    );
    assert!(host.finalized_assistant_messages().is_empty());
}

#[tokio::test(start_paused = true)]
async fn invalid_model_output_is_retried_before_accepting_next_valid_reply() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::ErrorWithSummary {
                kind: AgentLoopHostErrorKind::Unavailable,
                safe_summary: "model output was structurally invalid",
            },
            ScriptedModelResponse::Reply {
                text: "recovered after invalid model output".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::new(),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = MockAgentLoopDriverHost::builder().script(script).build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should recover from retryable invalid model output");

    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(completed.reply_message_refs.len(), 1);
            assert!(completed.final_checkpoint_id.is_some());
        }
        other => panic!("expected completion after retrying invalid model output, got {other:?}"),
    }
    assert_eq!(host.model_call_count(), 2);
    assert_eq!(
        host.finalized_assistant_messages(),
        vec!["recovered after invalid model output"]
    );
    assert_eq!(
        host.prompt_requests().len(),
        2,
        "model recovery must rebuild the prompt before retrying"
    );
}

#[tokio::test(start_paused = true)]
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

fn assert_no_progress_typed_failure(host: &MockAgentLoopDriverHost) {
    // A no-progress stop with the nudge gate off finalizes NO assistant reply —
    // the run ends as a typed `NoProgressDetected` failure, not a canned
    // "I stopped" message masquerading as a completed turn.
    assert!(
        host.finalized_assistant_messages().is_empty(),
        "no-progress failure must not finalize an assistant reply, got {:?}",
        host.finalized_assistant_messages()
    );
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
