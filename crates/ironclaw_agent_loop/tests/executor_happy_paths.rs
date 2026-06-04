use std::collections::VecDeque;

use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, CanonicalAgentLoopExecutor},
    families,
    state::{CheckpointKind, LoopExecutionState},
    test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedCapabilityCall,
        ScriptedCapabilityOutcome, ScriptedModelResponse, capability_descriptor, capability_id,
    },
};
use ironclaw_turns::{
    LoopBlockedKind, LoopExit,
    run_profile::{ConcurrencyHint, LoopRunInfoPort},
};

#[tokio::test]
async fn reply_only_completes() {
    let (host, checkpoints) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
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
        other => panic!("expected completed exit, got {other:?}"),
    }
    checkpoints.assert_sequence(&[(CheckpointKind::BeforeModel, 0), (CheckpointKind::Final, 0)]);
}

#[tokio::test]
async fn calls_then_reply_completes() {
    let (host, checkpoints) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::calls_then_reply("demo.echo"))
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(completed.result_refs.len(), 1);
            assert_eq!(completed.reply_message_refs.len(), 1);
        }
        other => panic!("expected completed exit, got {other:?}"),
    }
    checkpoints.assert_sequence(&[
        (CheckpointKind::BeforeModel, 0),
        (CheckpointKind::BeforeSideEffect, 0),
        (CheckpointKind::BeforeModel, 1),
        (CheckpointKind::Final, 1),
    ]);
    let calls = host.call_log();
    let append_position = calls
        .iter()
        .position(|call| matches!(call, MockHostCall::AppendCapabilityResultRef { .. }))
        .expect("completed capability result should append transcript evidence");
    let next_model_position = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| matches!(call, MockHostCall::StreamModel))
        .nth(1)
        .map(|(index, _)| index)
        .expect("model should run again after result evidence");
    assert!(append_position < next_model_position);
}

#[tokio::test]
async fn parallel_policy_batches_two_calls_in_one_iteration() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![
                ScriptedCapabilityCall::new("demo.a"),
                ScriptedCapabilityCall::new("demo.b"),
            ]),
            ScriptedModelResponse::Reply {
                text: "done".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![
            ScriptedCapabilityOutcome::completed("result:a"),
            ScriptedCapabilityOutcome::completed("result:b"),
        ]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = MockAgentLoopDriverHost::builder()
        .visible_capabilities(vec![
            capability_descriptor(capability_id("demo.a"), ConcurrencyHint::SafeForParallel),
            capability_descriptor(capability_id("demo.b"), ConcurrencyHint::SafeForParallel),
        ])
        .script(script)
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert!(host.call_log().iter().any(|call| {
        matches!(
            call,
            MockHostCall::InvokeCapabilityBatch {
                call_count: 2,
                stop_on_first_suspension: false
            }
        )
    }));
}

#[tokio::test]
async fn mixed_parallel_batch_blocks_after_recording_completed_results() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([ScriptedModelResponse::Calls(vec![
            ScriptedCapabilityCall::new("demo.a"),
            ScriptedCapabilityCall::new("demo.b"),
        ])]),
        capability_outcomes: VecDeque::from([vec![
            ScriptedCapabilityOutcome::completed("result:a"),
            ScriptedCapabilityOutcome::ApprovalRequired {
                gate_ref: "gate:approval".to_string(),
            },
        ]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, checkpoints) = MockAgentLoopDriverHost::builder()
        .visible_capabilities(vec![
            capability_descriptor(capability_id("demo.a"), ConcurrencyHint::SafeForParallel),
            capability_descriptor(capability_id("demo.b"), ConcurrencyHint::SafeForParallel),
        ])
        .script(script)
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    match exit {
        LoopExit::Blocked(blocked) => {
            assert_eq!(blocked.gate_ref.as_str(), "gate:approval");
        }
        other => panic!("expected blocked exit, got {other:?}"),
    }
    assert!(host.call_log().iter().any(|call| {
        matches!(
            call,
            MockHostCall::InvokeCapabilityBatch {
                call_count: 2,
                stop_on_first_suspension: false
            }
        )
    }));
    checkpoints.assert_sequence(&[
        (CheckpointKind::BeforeModel, 0),
        (CheckpointKind::BeforeSideEffect, 0),
        (CheckpointKind::BeforeBlock, 0),
    ]);
}

#[tokio::test]
async fn attested_signing_required_blocks_with_expected_tx_hash() {
    // The primary user-visible attested-signing flow: a request_signature
    // capability returns AttestedSigningRequired, and the loop must block with
    // LoopBlockedKind::Attested carrying the expected_tx_hash the resume path
    // verifies the user's signature against.
    let script = ScenarioScript {
        model_responses: VecDeque::from([ScriptedModelResponse::Calls(vec![
            ScriptedCapabilityCall::new("demo.sign"),
        ])]),
        capability_outcomes: VecDeque::from([vec![
            ScriptedCapabilityOutcome::AttestedSigningRequired {
                gate_ref: "gate:attested-abc".to_string(),
                expected_tx_hash: "deadbeef".to_string(),
            },
        ]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, checkpoints) = MockAgentLoopDriverHost::builder()
        .visible_capabilities(vec![capability_descriptor(
            capability_id("demo.sign"),
            ConcurrencyHint::SafeForParallel,
        )])
        .script(script)
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    match exit {
        LoopExit::Blocked(blocked) => {
            assert_eq!(blocked.kind, LoopBlockedKind::Attested);
            assert_eq!(blocked.gate_ref.as_str(), "gate:attested-abc");
            assert_eq!(
                blocked
                    .expected_tx_hash
                    .as_ref()
                    .expect("attested block carries expected_tx_hash")
                    .as_str(),
                "deadbeef"
            );
        }
        other => panic!("expected blocked exit, got {other:?}"),
    }
    checkpoints.assert_sequence(&[
        (CheckpointKind::BeforeModel, 0),
        (CheckpointKind::BeforeSideEffect, 0),
        (CheckpointKind::BeforeBlock, 0),
    ]);
}

#[tokio::test]
async fn sequential_batch_when_exclusive_present() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![
                ScriptedCapabilityCall::new("demo.safe"),
                ScriptedCapabilityCall::new("demo.exclusive"),
            ]),
            ScriptedModelResponse::Reply {
                text: "done".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![
            ScriptedCapabilityOutcome::completed("result:safe"),
            ScriptedCapabilityOutcome::completed("result:exclusive"),
        ]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = MockAgentLoopDriverHost::builder()
        .visible_capabilities(vec![
            capability_descriptor(capability_id("demo.safe"), ConcurrencyHint::SafeForParallel),
            capability_descriptor(capability_id("demo.exclusive"), ConcurrencyHint::Exclusive),
        ])
        .script(script)
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert!(host.call_log().iter().any(|call| {
        matches!(
            call,
            MockHostCall::InvokeCapabilityBatch {
                call_count: 2,
                stop_on_first_suspension: true
            }
        )
    }));
}

#[tokio::test]
async fn multiple_turns_complete_after_final_reply() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            ScriptedModelResponse::Reply {
                text: "done".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([
            vec![ScriptedCapabilityOutcome::completed("result:first")],
            vec![ScriptedCapabilityOutcome::completed("result:second")],
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

    assert!(matches!(exit, LoopExit::Completed(_)));
    checkpoints.assert_sequence(&[
        (CheckpointKind::BeforeModel, 0),
        (CheckpointKind::BeforeSideEffect, 0),
        (CheckpointKind::BeforeModel, 1),
        (CheckpointKind::BeforeSideEffect, 1),
        (CheckpointKind::BeforeModel, 2),
        (CheckpointKind::Final, 2),
    ]);
}
