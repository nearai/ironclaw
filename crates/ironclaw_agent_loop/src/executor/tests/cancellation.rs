use super::{
    AgentLoopExecutor, AgentLoopExecutorError, AgentLoopHostError, AgentLoopHostErrorKind,
    CanonicalAgentLoopExecutor, CapabilityFailureKind, CapabilityOutcome, CapabilityResultMessage,
    CheckpointKind, LoopCancelReasonKind, LoopCancelledReasonKind, LoopCheckpointKind,
    LoopExecutionState, LoopExit, LoopGateRef, LoopInput, LoopInputAckToken, LoopInputBatch,
    LoopInputCursor, LoopInterruptKind, LoopResultRef, LoopRunInfoPort, MockHost, calls_response,
    family_with_drain, final_staged_state, input_ack, input_cursor, message_ref, reply_response,
    surface_version,
};

#[tokio::test]
async fn steering_drain_does_not_ack_cancel_before_user_message() {
    let host = MockHost::new(Vec::new());
    let run_context = host.run_context().clone();
    let next_cursor = input_cursor(&run_context, "input-cursor:after-cancel");
    let host = host.with_input_batches(vec![LoopInputBatch {
        inputs: vec![
            LoopInput::Cancel {
                reason_kind: LoopCancelReasonKind::UserRequested,
            },
            LoopInput::UserMessage {
                message_ref: message_ref("msg:after-cancel"),
            },
        ],
        input_acks: vec![
            input_ack(&run_context, "input-cursor:cancel", "input-ack:cancel"),
            input_ack(
                &run_context,
                "input-cursor:after-cancel",
                "input-ack:after-cancel",
            ),
        ],
        next_cursor,
    }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let next = executor
        .drain_user_inputs(&host, state)
        .await
        .expect("drain");

    assert_eq!(
        next.state.input_cursor,
        input_cursor(&run_context, "input-cursor:cancel")
    );
    assert!(!next.drained);
    assert_eq!(
        next.ack_tokens,
        vec![LoopInputAckToken::new("input-ack:cancel").expect("valid ack token")]
    );
    assert_eq!(
        next.cancelled_reason_kind,
        Some(LoopCancelledReasonKind::HostCancellation)
    );
    assert!(host.acked_input_tokens().is_empty());
}

#[tokio::test]
async fn queued_cancel_exits_before_prompt_or_model_call() {
    let host = MockHost::new(Vec::new());
    let run_context = host.run_context().clone();
    let host = host.with_input_batches(vec![LoopInputBatch {
        inputs: vec![
            LoopInput::Cancel {
                reason_kind: LoopCancelReasonKind::UserRequested,
            },
            LoopInput::UserMessage {
                message_ref: message_ref("msg:after-cancel"),
            },
        ],
        input_acks: vec![
            input_ack(&run_context, "input-cursor:cancel", "input-ack:cancel"),
            input_ack(
                &run_context,
                "input-cursor:after-cancel",
                "input-ack:after-cancel",
            ),
        ],
        next_cursor: input_cursor(&run_context, "input-cursor:after-cancel"),
    }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("queued cancel should produce a loop exit");

    match exit {
        LoopExit::Cancelled(cancelled) => {
            assert_eq!(
                cancelled.reason_kind,
                LoopCancelledReasonKind::HostCancellation
            );
            assert!(cancelled.checkpoint_id.is_some());
        }
        other => panic!("expected queued cancel to return Cancelled, got {other:?}"),
    }
    assert!(host.model_requests().is_empty());
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    assert_eq!(
        host.acked_input_tokens(),
        vec![LoopInputAckToken::new("input-ack:cancel").expect("valid ack token")]
    );
    assert_eq!(
        host.events(),
        vec!["checkpoint:final".to_string(), "ack_inputs".to_string()]
    );
}

#[tokio::test]
async fn queued_cancel_after_user_prefix_exits_before_model_call() {
    let host = MockHost::new(Vec::new());
    let run_context = host.run_context().clone();
    let host = host.with_input_batches(vec![LoopInputBatch {
        inputs: vec![
            LoopInput::UserMessage {
                message_ref: message_ref("msg:before-cancel"),
            },
            LoopInput::Cancel {
                reason_kind: LoopCancelReasonKind::UserRequested,
            },
            LoopInput::UserMessage {
                message_ref: message_ref("msg:after-cancel"),
            },
        ],
        input_acks: vec![
            input_ack(
                &run_context,
                "input-cursor:before-cancel",
                "input-ack:before-cancel",
            ),
            input_ack(&run_context, "input-cursor:cancel", "input-ack:cancel"),
            input_ack(
                &run_context,
                "input-cursor:after-cancel",
                "input-ack:after-cancel",
            ),
        ],
        next_cursor: input_cursor(&run_context, "input-cursor:after-cancel"),
    }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("queued cancel should produce a loop exit");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert!(host.model_requests().is_empty());
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
    assert_eq!(
        host.acked_input_tokens(),
        vec![
            LoopInputAckToken::new("input-ack:before-cancel").expect("valid"),
            LoopInputAckToken::new("input-ack:cancel").expect("valid"),
        ]
    );
    assert_eq!(
        host.events(),
        vec!["checkpoint:final".to_string(), "ack_inputs".to_string()]
    );
}

#[tokio::test]
async fn steering_drain_leaves_unhandled_control_at_head_unacked() {
    let host = MockHost::new(Vec::new());
    let run_context = host.run_context().clone();
    let host = host.with_input_batches(vec![LoopInputBatch {
        inputs: vec![
            LoopInput::GateResolved {
                gate_ref: LoopGateRef::new("gate:resolved").expect("valid"),
            },
            LoopInput::CapabilitySurfaceChanged {
                version: surface_version(),
            },
            LoopInput::UserMessage {
                message_ref: message_ref("msg:after-control"),
            },
        ],
        input_acks: vec![
            input_ack(&run_context, "input-cursor:gate", "input-ack:gate"),
            input_ack(&run_context, "input-cursor:surface", "input-ack:surface"),
            input_ack(
                &run_context,
                "input-cursor:after-control",
                "input-ack:after-control",
            ),
        ],
        next_cursor: input_cursor(&run_context, "input-cursor:after-control"),
    }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let next = executor
        .drain_user_inputs(&host, state)
        .await
        .expect("drain");

    assert!(!next.drained);
    assert!(next.ack_tokens.is_empty());
    assert!(next.cancelled_reason_kind.is_none());
    assert_eq!(
        next.state.input_cursor,
        LoopInputCursor::origin_for_run(&run_context)
    );
    assert!(host.acked_input_tokens().is_empty());
}

#[tokio::test]
async fn followup_drain_does_not_ack_interrupt_before_followup() {
    let host = MockHost::new(Vec::new());
    let run_context = host.run_context().clone();
    let next_cursor = input_cursor(&run_context, "input-cursor:after-interrupt");
    let host = host.with_input_batches(vec![LoopInputBatch {
        inputs: vec![
            LoopInput::Interrupt {
                kind: LoopInterruptKind::UserInterrupt,
            },
            LoopInput::FollowUp {
                message_ref: message_ref("msg:after-interrupt"),
            },
        ],
        input_acks: vec![
            input_ack(
                &run_context,
                "input-cursor:interrupt",
                "input-ack:interrupt",
            ),
            input_ack(
                &run_context,
                "input-cursor:after-interrupt",
                "input-ack:after-interrupt",
            ),
        ],
        next_cursor,
    }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let next = executor.drain_followup(&host, state).await.expect("drain");

    assert!(!next.drained);
    assert_eq!(
        next.state.input_cursor,
        input_cursor(&run_context, "input-cursor:interrupt")
    );
    assert_eq!(
        next.ack_tokens,
        vec![LoopInputAckToken::new("input-ack:interrupt").expect("valid ack token")]
    );
    assert_eq!(
        next.cancelled_reason_kind,
        Some(LoopCancelledReasonKind::HostInterrupt)
    );
    assert!(host.acked_input_tokens().is_empty());
}

#[tokio::test]
async fn steering_drain_acks_only_after_cursor_checkpoint_is_durable() {
    let host = MockHost::new(vec![reply_response()]);
    let run_context = host.run_context().clone();
    let next_cursor = input_cursor(&run_context, "input-cursor:after-user");
    let host = host.with_input_batches(vec![LoopInputBatch {
        inputs: vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user-drained"),
        }],
        input_acks: vec![input_ack(
            &run_context,
            "input-cursor:after-user",
            "input-ack:after-user",
        )],
        next_cursor: next_cursor.clone(),
    }]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Completed(_)));
    assert_eq!(
        host.acked_input_tokens(),
        vec![LoopInputAckToken::new("input-ack:after-user").expect("valid")]
    );
    assert_eq!(
        host.events(),
        vec![
            "checkpoint:before_model".to_string(),
            "ack_inputs".to_string(),
            "checkpoint:final".to_string(),
        ]
    );
}

#[tokio::test]
async fn cancellation_after_steering_drain_flushes_pending_input_ack() {
    let host = MockHost::new(vec![reply_response()]);
    let run_context = host.run_context().clone();
    let next_cursor = input_cursor(&run_context, "input-cursor:after-user-before-cancel");
    let host = host
        .with_input_batches(vec![LoopInputBatch {
            inputs: vec![LoopInput::UserMessage {
                message_ref: message_ref("msg:user-drained-before-cancel"),
            }],
            input_acks: vec![input_ack(
                &run_context,
                "input-cursor:after-user-before-cancel",
                "input-ack:after-user-before-cancel",
            )],
            next_cursor: next_cursor.clone(),
        }])
        .cancel_after_poll_inputs();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert_eq!(
        host.acked_input_tokens(),
        vec![LoopInputAckToken::new("input-ack:after-user-before-cancel").expect("valid")]
    );
    assert_eq!(
        host.events(),
        vec!["checkpoint:final".to_string(), "ack_inputs".to_string()]
    );
}

#[tokio::test]
async fn cancellation_after_pending_input_ack_strict_profile_propagates_checkpoint_error() {
    // Strict-profile checkpoint failure via the pending-input-ack helper must
    // propagate `CheckpointFailed` (mirroring the non-ack helper), not return
    // `Ok(LoopExit::failed)`. Returning `Ok` would mask the failure and bypass
    // the strict require-final-checkpoint contract.
    let host = MockHost::new(vec![reply_response()]);
    let run_context = host.run_context().clone();
    let next_cursor = input_cursor(&run_context, "input-cursor:after-user-before-cancel");
    let host = host
        .with_input_batches(vec![LoopInputBatch {
            inputs: vec![LoopInput::UserMessage {
                message_ref: message_ref("msg:user-drained-before-cancel"),
            }],
            input_acks: vec![input_ack(
                &run_context,
                "input-cursor:after-user-before-cancel",
                "input-ack:after-user-before-cancel",
            )],
            next_cursor: next_cursor.clone(),
        }])
        .cancel_after_poll_inputs()
        .with_require_final_checkpoint(true)
        .fail_checkpoint(LoopCheckpointKind::Final);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let err = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("expected executor error on strict-profile checkpoint failure");

    assert_eq!(
        err,
        AgentLoopExecutorError::CheckpointFailed {
            stage: CheckpointKind::Final
        }
    );
}

#[tokio::test]
async fn cancellation_after_followup_drain_flushes_pending_input_ack() {
    let host = MockHost::new(vec![reply_response()]);
    let run_context = host.run_context().clone();
    let next_cursor = input_cursor(&run_context, "input-cursor:after-followup-before-cancel");
    let host = host
        .with_input_batches(vec![LoopInputBatch {
            inputs: vec![LoopInput::FollowUp {
                message_ref: message_ref("msg:followup-drained-before-cancel"),
            }],
            input_acks: vec![input_ack(
                &run_context,
                "input-cursor:after-followup-before-cancel",
                "input-ack:after-followup-before-cancel",
            )],
            next_cursor: next_cursor.clone(),
        }])
        .cancel_after_poll_inputs();
    let family = family_with_drain(false, true);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&family, &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert_eq!(
        host.acked_input_tokens(),
        vec![LoopInputAckToken::new("input-ack:after-followup-before-cancel").expect("valid")]
    );
    assert_eq!(
        host.events(),
        vec![
            "checkpoint:before_model".to_string(),
            "checkpoint:final".to_string(),
            "ack_inputs".to_string(),
        ]
    );
}

#[tokio::test]
async fn model_cancelled_returns_cancelled_without_retry() {
    let host = MockHost::new(Vec::new()).with_model_errors(vec![AgentLoopHostError::new(
        AgentLoopHostErrorKind::Cancelled,
        "model cancelled",
    )]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let result = executor
        .execute_family(&crate::families::default(), &host, state)
        .await;

    assert!(matches!(result, Err(AgentLoopExecutorError::Cancelled)));
    assert_eq!(host.model_requests().len(), 1);
}

#[tokio::test]
async fn cancellation_after_retry_prompt_rebuild_skips_second_model_call() {
    let host = MockHost::new(vec![reply_response()])
        .with_model_errors(vec![AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            "model unavailable",
        )])
        .cancel_after_prompt_bundle(2);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert_eq!(host.prompt_requests().len(), 2);
    assert_eq!(host.model_requests().len(), 1);
}

#[tokio::test]
async fn capability_cancelled_returns_cancelled_exit_without_retry() {
    let host = MockHost::new(vec![calls_response()]).with_batch_outcomes(vec![
        ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Failed(
                ironclaw_turns::run_profile::CapabilityFailure {
                    error_kind: CapabilityFailureKind::Cancelled,
                    safe_summary: "capability cancelled".to_string(),
                },
            )],
            stopped_on_suspension: false,
        },
    ]);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

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
    assert!(host.single_invocations().is_empty());
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeSideEffect,
            LoopCheckpointKind::Final,
        ]
    );
}

#[tokio::test]
async fn cancellation_before_first_iteration_exits_with_final_checkpoint() {
    let host = MockHost::new(vec![reply_response()]);
    host.request_cancellation(LoopCancelReasonKind::UserRequested);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Cancelled(cancelled) => {
            assert_eq!(
                cancelled.reason_kind,
                LoopCancelledReasonKind::HostCancellation
            );
            assert!(cancelled.checkpoint_id.is_some());
        }
        other => panic!("expected cancelled, got {other:?}"),
    }
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}

#[tokio::test]
async fn cancellation_after_boundary_skips_next_model_call() {
    let host = MockHost::new(vec![reply_response()])
        .cancel_after_checkpoint(LoopCheckpointKind::BeforeModel);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert_eq!(
        host.checkpoint_kinds(),
        vec![LoopCheckpointKind::BeforeModel, LoopCheckpointKind::Final]
    );
}

#[tokio::test]
async fn cancellation_after_model_response_preserves_assistant_reply() {
    let host = MockHost::new(vec![reply_response()]).cancel_after_model_response();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert_eq!(host.model_requests().len(), 1);
    assert_eq!(
        final_staged_state(&host).assistant_refs,
        vec![message_ref("msg:assistant")]
    );
}

#[tokio::test]
async fn cancellation_after_before_side_effect_checkpoint_skips_capability_call() {
    let host = MockHost::new(vec![calls_response()])
        .cancel_after_checkpoint(LoopCheckpointKind::BeforeSideEffect);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert!(host.batch_invocations().is_empty());
    assert_eq!(
        host.checkpoint_kinds(),
        vec![
            LoopCheckpointKind::BeforeModel,
            LoopCheckpointKind::BeforeSideEffect,
            LoopCheckpointKind::Final,
        ]
    );
}

#[tokio::test]
async fn cancellation_after_capability_batch_preserves_completed_result() {
    let result_ref = LoopResultRef::new("result:late-cancel").expect("valid");
    let host = MockHost::new(vec![calls_response()])
        .with_batch_outcomes(vec![ironclaw_turns::run_profile::CapabilityBatchOutcome {
            outcomes: vec![CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: result_ref.clone(),
                safe_summary: "completed before cancellation".to_string(),
                terminate_hint: true,
            })],
            stopped_on_suspension: false,
        }])
        .cancel_after_batch_invocation();
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    assert!(matches!(exit, LoopExit::Cancelled(_)));
    assert_eq!(host.batch_invocations().len(), 1);
    assert_eq!(final_staged_state(&host).result_refs, vec![result_ref]);
}

#[tokio::test]
async fn cancellation_checkpoint_failure_still_cancels_for_permissive_profile() {
    let host = MockHost::new(vec![reply_response()]).fail_checkpoint(LoopCheckpointKind::Final);
    host.request_cancellation(LoopCancelReasonKind::UserRequested);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let exit = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect("execute");

    match exit {
        LoopExit::Cancelled(cancelled) => assert!(cancelled.checkpoint_id.is_none()),
        other => panic!("expected cancelled, got {other:?}"),
    }
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}

#[tokio::test]
async fn cancellation_checkpoint_failure_propagates_executor_error_for_strict_profile() {
    // Strict profiles require a verified final checkpoint. When the final checkpoint
    // write itself fails during cooperative cancellation, the executor cannot produce
    // a trustworthy LoopExit — it must surface CheckpointFailed rather than returning
    // a LoopExit::Failed with no checkpoint_id, which would fail strict-profile validation.
    let host = MockHost::new(vec![reply_response()])
        .with_require_final_checkpoint(true)
        .fail_checkpoint(LoopCheckpointKind::Final);
    host.request_cancellation(LoopCancelReasonKind::UserRequested);
    let executor = CanonicalAgentLoopExecutor;
    let state = LoopExecutionState::initial_for_run(host.run_context());

    let err = executor
        .execute_family(&crate::families::default(), &host, state)
        .await
        .expect_err("expected executor error on strict-profile checkpoint failure");

    assert_eq!(
        err,
        AgentLoopExecutorError::CheckpointFailed {
            stage: CheckpointKind::Final
        }
    );
    assert_eq!(host.checkpoint_kinds(), vec![LoopCheckpointKind::Final]);
}
