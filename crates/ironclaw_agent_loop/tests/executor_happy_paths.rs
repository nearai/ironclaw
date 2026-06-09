use std::collections::VecDeque;

use ironclaw_agent_loop::{
    executor::{AgentLoopExecutor, CanonicalAgentLoopExecutor},
    families,
    state::{CheckpointKind, LoopExecutionState},
    test_support::{
        MockAgentLoopDriverHost, MockHostCall, ScenarioScript, ScriptedCapabilityCall,
        ScriptedCapabilityOutcome, ScriptedModelResponse, capability_descriptor, capability_id,
        compaction::{active_task_preserving_compaction_index, compaction_metadata},
    },
};
use ironclaw_turns::{
    LoopBlockedKind, LoopExit, LoopFailureKind, TurnRunId,
    run_profile::{
        CompactionInitiator, ConcurrencyHint, LoopCompactionResponse, LoopContextCompactionKind,
        LoopProgressEvent, LoopRunInfoPort, LoopSummaryArtifactId,
    },
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
async fn compaction_failure_returns_failed_exit() {
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .prompt_compaction_index(active_task_preserving_compaction_index())
        .build();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should produce failed exit");

    match exit {
        LoopExit::Failed(failed) => {
            assert_eq!(failed.reason_kind, LoopFailureKind::CompactionUnavailable);
            assert!(failed.checkpoint_id.is_some());
        }
        other => panic!("expected failed exit, got {other:?}"),
    }
}

#[tokio::test]
async fn compaction_success_updates_state_and_emits_progress() {
    let (host, _) = MockAgentLoopDriverHost::builder()
        .script(ScenarioScript::reply_only("hi"))
        .prompt_compaction_indexes(vec![
            active_task_preserving_compaction_index(),
            vec![compaction_metadata(
                2,
                LoopContextCompactionKind::Assistant,
                10,
            )],
        ])
        .compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-1").unwrap(),
            compression_ratio_ppm: 250_000,
        }))
        .build();
    let mut state = LoopExecutionState::initial_for_run(host.run_context());
    state.compaction_state.force_compact_on_next_iteration = true;

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed after compaction");

    assert!(matches!(exit, LoopExit::Completed(_)));
    let progress_events = host.progress_events();
    assert!(progress_events.iter().any(|event| {
        matches!(
            event,
            LoopProgressEvent::CompactionStarted {
                initiator: ironclaw_turns::run_profile::CompactionInitiator::Auto,
                ..
            }
        )
    }));
    assert!(progress_events.iter().any(|event| {
        matches!(
            event,
            LoopProgressEvent::CompactionCompleted {
                compression_ratio_ppm: 250_000,
                ..
            }
        )
    }));
    assert!(
        host.call_log()
            .contains(&MockHostCall::FinalizeAssistantMessage)
    );
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
    assert!(host.call_log().iter().any(|call| {
        matches!(
            call,
            MockHostCall::AppendCapabilityResultRef { result_ref, .. }
                if result_ref.as_str() == "result:a"
        )
    }));
}

#[tokio::test]
async fn await_dependent_run_blocks_with_dependent_gate_kind() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([ScriptedModelResponse::Calls(vec![
            ScriptedCapabilityCall::new("demo.spawn"),
        ])]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::AwaitDependentRun {
            gate_ref: "gate:child-wait".to_string(),
            result_ref: "result:child-wait".to_string(),
            byte_len: 0,
        }]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, checkpoints) = MockAgentLoopDriverHost::builder()
        .visible_capabilities(vec![capability_descriptor(
            capability_id("demo.spawn"),
            ConcurrencyHint::Exclusive,
        )])
        .script(script)
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should block on dependent run");

    match exit {
        LoopExit::Blocked(blocked) => {
            assert_eq!(blocked.kind, LoopBlockedKind::AwaitDependentRun);
            assert_eq!(blocked.gate_ref.as_str(), "gate:child-wait");
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
async fn spawned_child_run_appends_result_ref_and_continues() {
    let child_run_id = TurnRunId::new();
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.spawn")]),
            ScriptedModelResponse::Reply {
                text: "done".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![ScriptedCapabilityOutcome::SpawnedChildRun {
            child_run_id,
            result_ref: "result:child-run".to_string(),
            byte_len: 0,
        }]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };
    let (host, _) = MockAgentLoopDriverHost::builder()
        .visible_capabilities(vec![capability_descriptor(
            capability_id("demo.spawn"),
            ConcurrencyHint::Exclusive,
        )])
        .script(script)
        .build();
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should continue after child spawn result");

    match exit {
        LoopExit::Completed(completed) => {
            assert_eq!(completed.result_refs.len(), 1);
            assert_eq!(completed.result_refs[0].as_str(), "result:child-run");
        }
        other => panic!("expected completed exit, got {other:?}"),
    }
    assert!(host.call_log().iter().any(|call| {
        matches!(
            call,
            MockHostCall::AppendCapabilityResultRef { result_ref, .. }
                if result_ref.as_str() == "result:child-run"
        )
    }));
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

// ---------------------------------------------------------------------------
// F14 — End-to-end caller-level test for the proactive byte-cap → SkipModel
//        → compaction → continue flow (per the "Test Through the Caller" rule)
// ---------------------------------------------------------------------------

/// Caller-level test through `CanonicalAgentLoopExecutor` covering the full
/// proactive byte-cap → SkipModel → compaction → continue chain.
///
/// This test exercises the multi-stage chain that `ByteCapStrategy +
/// PostCapabilityStage + PromptStage skip_model + canonical SkipModel arm`
/// form end-to-end. Per `.claude/rules/testing.md` "Test Through the Caller,
/// Not Just the Helper", testing each stage in isolation is insufficient:
/// a unit test on the helper alone cannot detect a regression where the
/// state-threaded initiator is dropped between PostCapabilityStage and
/// PromptCompactionStep.
///
/// Iteration 1: model → `SpawnedChildRun` capability outcome with
///   `byte_len = 49 001` (exceeds the 32 000-byte default cap).
///   → `push_completed_result` accumulates bytes → `PostCapabilityStage`
///   trips `ByteCapStrategy` → sets `force_compact_on_next_iteration`,
///   `skip_model_this_iteration`, and
///   `force_compact_initiator = CapabilityResultOverflow` → clears byte map.
///
/// Iteration 2: `PromptStage` detects `skip_model_this_iteration` →
///   `PromptCompactionStep` runs (compaction index is non-empty) → emits
///   `CompactionStarted { initiator: CapabilityResultOverflow }` (taken from
///   state) → returns `PromptStep::SkipModel` → canonical `SkipModel` arm
///   bypasses Model + Capability + PostCapability → `stop.observe +
///   stop.decide` → `iter++`.
///
/// Iteration 3: model → reply → `GracefulStop`.
///
/// Asserts:
///   - `model_call_count() == 2` (no model call on iter 2)
///   - `host.progress_events()` contains exactly one `CompactionStarted`
///     with `initiator == CapabilityResultOverflow`
///   - final state has `force_compact_on_next_iteration == false` and
///     `force_compact_initiator == None` (all consumed/cleared)
///   - `turns_completed == 3` (CompactionOnly counts per Step 9 finding)
#[tokio::test]
async fn executor_proactive_byte_cap_drives_full_compaction_cycle() {
    let script = ScenarioScript {
        model_responses: VecDeque::from([
            // Iteration 1: capability call → SpawnedChildRun with large byte_len.
            ScriptedModelResponse::Calls(vec![ScriptedCapabilityCall::new("demo.echo")]),
            // Iteration 3: final reply (iteration 2 is SkipModel, no model call).
            ScriptedModelResponse::Reply {
                text: "done".to_string(),
            },
        ]),
        capability_outcomes: VecDeque::from([vec![
            // SpawnedChildRun with byte_len > 32 000 trips ByteCapStrategy.
            ScriptedCapabilityOutcome::SpawnedChildRun {
                child_run_id: TurnRunId::new(),
                result_ref: "result:big-spawn-f14".to_string(),
                // Exceeds the default 32 000-byte fallback cap; same value
                // used in spawned_child_run_byte_len_accumulates_and_trips_policy.
                byte_len: 49_001,
            },
        ]]),
        single_call_retry_outcomes: VecDeque::new(),
        pending_inputs: VecDeque::new(),
    };

    // prompt_compaction_indexes is consumed once per build_prompt_bundle call.
    // The SkipModel path (iteration 2) does NOT call build_prompt_bundle —
    // PromptStage short-circuits before surface/bundle building and uses
    // state.compaction_prompt.message_index that was populated by iteration 1's
    // build_prompt_bundle call. So only TWO builds happen:
    //   call 1 → iteration 1 (capability prompt build)
    //   call 2 → iteration 3 (reply prompt build after compaction)
    // We supply a non-empty index for call 1 so state.compaction_prompt.message_index
    // is populated; PromptCompactionStep on the SkipModel turn (iter 2) reads this
    // stored index and DefaultCompactionStrategy returns Trigger.
    let (host, _checkpoints) = MockAgentLoopDriverHost::builder()
        .script(script)
        .prompt_compaction_indexes(vec![
            // Iteration 1 prompt build: non-empty — seeds state.compaction_prompt.
            // message_index, which PromptCompactionStep reads on iter 2 (SkipModel).
            active_task_preserving_compaction_index(),
            // Iteration 3 prompt build (post-compaction reply): empty.
            vec![],
        ])
        .compaction_result(Ok(LoopCompactionResponse {
            summary_artifact_id: LoopSummaryArtifactId::new("summary-f14").unwrap(),
            compression_ratio_ppm: 250_000,
        }))
        .build();

    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = CanonicalAgentLoopExecutor
        .execute_family(&families::default(), &host, state)
        .await
        .expect("loop execution should succeed after byte-cap compaction cycle");

    assert!(
        matches!(exit, LoopExit::Completed(_)),
        "loop must complete after byte-cap → compaction → continue cycle"
    );

    // Model must be called exactly twice (iteration 1 and 3); iteration 2 is
    // SkipModel and must never reach ModelStage.
    assert_eq!(
        host.model_call_count(),
        2,
        "model must be called exactly twice (capability turn + reply turn); \
         SkipModel iteration must bypass ModelStage"
    );

    // The progress events must contain exactly one CompactionStarted event
    // with initiator == CapabilityResultOverflow — proving the D-A threaded
    // initiator survived the PostCapabilityStage → state → PromptCompactionStep
    // boundary.
    let progress_events = host.progress_events();
    let compaction_started_events: Vec<_> = progress_events
        .iter()
        .filter(|event| matches!(event, LoopProgressEvent::CompactionStarted { .. }))
        .collect();
    assert_eq!(
        compaction_started_events.len(),
        1,
        "exactly one CompactionStarted event must be emitted on the SkipModel iteration"
    );
    match compaction_started_events[0] {
        LoopProgressEvent::CompactionStarted { initiator, .. } => {
            assert_eq!(
                initiator,
                &CompactionInitiator::CapabilityResultOverflow,
                "CompactionStarted initiator must be CapabilityResultOverflow; \
                 if it is Auto the D-A state-threaded initiator was dropped before \
                 PromptCompactionStep emitted the event"
            );
        }
        other => panic!("expected CompactionStarted event, got {:?}", other),
    }

    // Verify the run is clean after compaction: no lingering compaction flags.
    // (We verify via progress events rather than inspecting internal state,
    // since MockAgentLoopDriverHost does not expose staged checkpoint payloads.
    // The initiator-emission / flag-clearing verification is also covered by the
    // F12 unit-level test in src/executor/tests.rs which inspects checkpoint state
    // directly.)
    let compaction_completed = progress_events
        .iter()
        .any(|event| matches!(event, LoopProgressEvent::CompactionCompleted { .. }));
    assert!(
        compaction_completed,
        "CompactionCompleted must be emitted after successful compaction on the SkipModel iteration"
    );
}
