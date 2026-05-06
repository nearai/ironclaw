use ironclaw_turns::{
    BlockedReason, GateRef, LoopBlocked, LoopBlockedKind, LoopCompleted, LoopCompletionKind,
    LoopExit, LoopExitId, LoopExitInvalidHandling, LoopExitValidationDecision,
    LoopExitValidationPolicy, LoopFailureKind, LoopMessageRef, LoopResultRef, SanitizedFailure,
    TurnCheckpointId, runner::TurnRunnerOutcome,
};
use serde_json::json;

#[test]
fn completed_ask_user_exit_maps_to_trusted_completed_outcome_without_final_checkpoint() {
    let exit_id = exit_id("exit-completed");
    let decision = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::AskUserReply,
        reply_message_refs: vec![message_ref("assistant-question")],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id.clone(),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
    });

    assert_eq!(decision.exit_id, exit_id);
    assert_eq!(decision.violation, None);
    assert_eq!(decision.mapping, TurnRunnerOutcome::Completed.into());
}

#[test]
fn completed_exit_without_durable_refs_maps_to_protocol_failure_or_recovery() {
    let exit = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs: vec![],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id("exit-missing-refs"),
    });

    let safe_decision = exit.clone().validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
    });
    assert_eq!(
        safe_decision.mapping,
        TurnRunnerOutcome::Failed {
            failure: SanitizedFailure::new("driver_protocol_violation").unwrap(),
        }
        .into()
    );
    assert_eq!(
        safe_decision.violation.unwrap().category(),
        "missing_completion_reference"
    );

    let uncertain_decision = exit.validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::RecoveryRequired,
    });
    assert!(matches!(
        uncertain_decision,
        LoopExitValidationDecision {
            mapping: ironclaw_turns::LoopExitMapping::RecoveryRequired { .. },
            ..
        }
    ));
}

#[test]
fn final_checkpoint_policy_rejects_terminal_exit_without_checkpoint() {
    let decision = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs: vec![message_ref("assistant-final")],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id("exit-no-final-checkpoint"),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: true,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
    });

    assert_eq!(
        decision.violation.unwrap().category(),
        "missing_final_checkpoint"
    );
    assert_eq!(
        decision.mapping,
        TurnRunnerOutcome::Failed {
            failure: SanitizedFailure::new("driver_protocol_violation").unwrap(),
        }
        .into()
    );
}

#[test]
fn blocked_exit_maps_to_block_run_outcome_with_checkpoint_and_gate_ref() {
    let checkpoint_id = TurnCheckpointId::new();
    let gate_ref = GateRef::new("approval-gate").unwrap();
    let decision = LoopExit::Blocked(LoopBlocked {
        kind: LoopBlockedKind::Approval,
        gate_ref: gate_ref.clone(),
        checkpoint_id,
        exit_id: exit_id("exit-blocked"),
    })
    .validate(LoopExitValidationPolicy::default());

    assert_eq!(decision.violation, None);
    assert_eq!(
        decision.mapping,
        TurnRunnerOutcome::Blocked {
            checkpoint_id,
            reason: BlockedReason::Approval { gate_ref },
        }
        .into()
    );
}

#[test]
fn cancelled_exit_requires_observed_host_cancellation() {
    let exit = LoopExit::cancelled_for_observed_interrupt(exit_id("exit-cancelled"));

    let rejected = exit.clone().validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
    });
    assert_eq!(
        rejected.mapping,
        TurnRunnerOutcome::Failed {
            failure: SanitizedFailure::new("interrupted_unexpectedly").unwrap(),
        }
        .into()
    );
    assert_eq!(
        rejected.violation.unwrap().category(),
        "cancellation_not_observed"
    );

    let accepted = exit.validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: true,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
    });
    assert_eq!(accepted.mapping, TurnRunnerOutcome::Cancelled.into());
    assert_eq!(accepted.violation, None);
}

#[test]
fn iteration_limit_failure_maps_to_stable_sanitized_runner_failure() {
    let decision = LoopExit::failed(
        LoopFailureKind::IterationLimit,
        exit_id("exit-max-iterations"),
    )
    .validate(LoopExitValidationPolicy::default());

    assert_eq!(
        decision.mapping,
        TurnRunnerOutcome::Failed {
            failure: SanitizedFailure::new("iteration_limit").unwrap(),
        }
        .into()
    );
}

#[test]
fn loop_exit_wire_shape_rejects_raw_payload_fields_and_recovery_required_variant() {
    let raw_completed = json!({
        "completed": {
            "completion_kind": "final_reply",
            "reply_message_refs": ["assistant-final"],
            "result_refs": [],
            "final_checkpoint_id": null,
            "usage_summary_ref": null,
            "exit_id": "exit-raw",
            "raw_reply_text": "secret prompt-adjacent content"
        }
    });
    assert!(serde_json::from_value::<LoopExit>(raw_completed).is_err());

    let raw_blocked = json!({
        "blocked": {
            "kind": "approval",
            "gate_ref": "approval-gate",
            "checkpoint_id": TurnCheckpointId::new(),
            "exit_id": "exit-raw-blocked",
            "raw_approval_payload": {"tool_input": "secret"}
        }
    });
    assert!(serde_json::from_value::<LoopExit>(raw_blocked).is_err());

    assert!(serde_json::from_value::<LoopExit>(json!({"recovery_required": {}})).is_err());
}

fn exit_id(value: &str) -> LoopExitId {
    LoopExitId::new(value).unwrap()
}

fn message_ref(value: &str) -> LoopMessageRef {
    LoopMessageRef::new(value).unwrap()
}

#[allow(dead_code)]
fn result_ref(value: &str) -> LoopResultRef {
    LoopResultRef::new(value).unwrap()
}
