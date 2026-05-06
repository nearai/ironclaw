use ironclaw_turns::{
    BlockedReason, GateRef, LoopBlocked, LoopBlockedKind, LoopCompleted, LoopCompletionKind,
    LoopExit, LoopExitId, LoopExitInvalidHandling, LoopExitValidationDecision,
    LoopExitValidationPolicy, LoopFailureKind, LoopGateRef, LoopMessageRef, LoopResultRef,
    SanitizedFailure, TurnCheckpointId, runner::TurnRunnerOutcome,
};
use serde_json::json;

#[test]
fn completed_ask_user_exit_maps_to_trusted_completed_outcome_without_final_checkpoint() {
    let exit_id = exit_id("exit:completed");
    let decision = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::AskUserReply,
        reply_message_refs: vec![message_ref("msg:assistant-question")],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id.clone(),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
        completion_refs_verified: true,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
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
        exit_id: exit_id("exit:missing-refs"),
    });

    let safe_decision = exit.clone().validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
        completion_refs_verified: true,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
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
        completion_refs_verified: true,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
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
fn completed_exit_requires_host_verified_completion_refs_before_trusted_mapping() {
    let exit = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs: vec![message_ref("msg:assistant-final")],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id("exit:unverified-completion"),
    });

    let decision = exit.validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::RecoveryRequired,
        completion_refs_verified: false,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
    });

    assert_eq!(
        decision.violation.unwrap().category(),
        "unverified_completion_reference"
    );
    assert!(matches!(
        decision.mapping,
        ironclaw_turns::LoopExitMapping::RecoveryRequired { .. }
    ));
}

#[test]
fn final_checkpoint_policy_rejects_terminal_exit_without_checkpoint() {
    let decision = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs: vec![message_ref("msg:assistant-final")],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id("exit:no-final-checkpoint"),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: true,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
        completion_refs_verified: true,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
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
fn blocked_exit_maps_to_block_run_outcome_with_verified_checkpoint_and_gate_ref() {
    let checkpoint_id = TurnCheckpointId::new();
    let loop_gate_ref = loop_gate_ref("gate:approval-gate");
    let gate_ref = GateRef::new(loop_gate_ref.as_str()).unwrap();
    let decision = LoopExit::Blocked(LoopBlocked {
        kind: LoopBlockedKind::Approval,
        gate_ref: loop_gate_ref,
        checkpoint_id,
        exit_id: exit_id("exit:blocked"),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::RecoveryRequired,
        completion_refs_verified: false,
        blocked_evidence_verified: true,
        failure_evidence_verified: false,
    });

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
fn blocked_exit_requires_host_verified_gate_and_checkpoint_before_trusted_mapping() {
    let decision = LoopExit::Blocked(LoopBlocked {
        kind: LoopBlockedKind::Approval,
        gate_ref: loop_gate_ref("gate:approval-gate"),
        checkpoint_id: TurnCheckpointId::new(),
        exit_id: exit_id("exit:unverified-blocked"),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::RecoveryRequired,
        completion_refs_verified: false,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
    });

    assert_eq!(
        decision.violation.unwrap().category(),
        "unverified_blocked_evidence"
    );
    assert!(matches!(
        decision.mapping,
        ironclaw_turns::LoopExitMapping::RecoveryRequired { .. }
    ));
}

#[test]
fn cancelled_exit_requires_observed_host_cancellation() {
    let exit = LoopExit::cancelled_for_observed_interrupt(exit_id("exit:cancelled"));

    let rejected = exit.clone().validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
        completion_refs_verified: false,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
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
        completion_refs_verified: false,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
    });
    assert_eq!(accepted.mapping, TurnRunnerOutcome::Cancelled.into());
    assert_eq!(accepted.violation, None);
}

#[test]
fn iteration_limit_failure_maps_to_stable_sanitized_runner_failure_after_host_verification() {
    let decision = LoopExit::failed(
        LoopFailureKind::IterationLimit,
        exit_id("exit:max-iterations"),
    )
    .validate(LoopExitValidationPolicy {
        failure_evidence_verified: true,
        ..LoopExitValidationPolicy::default()
    });

    assert_eq!(
        decision.mapping,
        TurnRunnerOutcome::Failed {
            failure: SanitizedFailure::new("iteration_limit").unwrap(),
        }
        .into()
    );
}

#[test]
fn failed_exit_requires_host_verified_failure_evidence_before_trusted_mapping() {
    let decision = LoopExit::failed(LoopFailureKind::DriverBug, exit_id("exit:driver-bug"))
        .validate(LoopExitValidationPolicy::default());

    assert_eq!(
        decision.violation.unwrap().category(),
        "unverified_failure_evidence"
    );
    assert!(matches!(
        decision.mapping,
        ironclaw_turns::LoopExitMapping::RecoveryRequired { .. }
    ));
}

#[test]
fn loop_exit_wire_shape_rejects_raw_payload_fields_and_recovery_required_variant() {
    let raw_completed = json!({
        "completed": {
            "completion_kind": "final_reply",
            "reply_message_refs": ["msg:assistant-final"],
            "result_refs": [],
            "final_checkpoint_id": null,
            "usage_summary_ref": null,
            "exit_id": "exit:raw",
            "raw_reply_text": "secret prompt-adjacent content"
        }
    });
    assert!(serde_json::from_value::<LoopExit>(raw_completed).is_err());

    let raw_blocked = json!({
        "blocked": {
            "kind": "approval",
            "gate_ref": "gate:approval-gate",
            "checkpoint_id": TurnCheckpointId::new(),
            "exit_id": "exit:raw-blocked",
            "raw_approval_payload": {"tool_input": "secret"}
        }
    });
    assert!(serde_json::from_value::<LoopExit>(raw_blocked).is_err());

    assert!(serde_json::from_value::<LoopExit>(json!({"recovery_required": {}})).is_err());
}

#[test]
fn loop_exit_rejects_oversized_or_duplicate_ref_vectors() {
    let oversized_messages = (0..65)
        .map(|index| format!("msg:item-{index}"))
        .collect::<Vec<_>>();
    let raw_completed = json!({
        "completed": {
            "completion_kind": "final_reply",
            "reply_message_refs": oversized_messages,
            "result_refs": [],
            "final_checkpoint_id": null,
            "usage_summary_ref": null,
            "exit_id": "exit:oversized"
        }
    });
    assert!(serde_json::from_value::<LoopExit>(raw_completed).is_err());

    let duplicate_refs = json!({
        "completed": {
            "completion_kind": "final_reply",
            "reply_message_refs": ["msg:dup", "msg:dup"],
            "result_refs": [],
            "final_checkpoint_id": null,
            "usage_summary_ref": null,
            "exit_id": "exit:duplicates"
        }
    });
    assert!(serde_json::from_value::<LoopExit>(duplicate_refs).is_err());
}

#[test]
fn loop_refs_reject_raw_payload_like_values_inside_ref_strings() {
    for raw in [
        "plain assistant text",
        "secret prompt-adjacent content",
        "/tmp/host/path",
        "Error: provider leaked stack",
        "tool_input={\"secret\":true}",
    ] {
        assert!(
            LoopMessageRef::new(raw).is_err(),
            "message ref accepted {raw:?}"
        );
        assert!(
            LoopResultRef::new(raw).is_err(),
            "result ref accepted {raw:?}"
        );
        assert!(LoopGateRef::new(raw).is_err(), "gate ref accepted {raw:?}");
    }

    assert!(LoopMessageRef::new("msg:assistant-final").is_ok());
    assert!(LoopResultRef::new("result:delegated-job-1").is_ok());
    assert!(LoopGateRef::new("gate:approval-gate").is_ok());
}

fn exit_id(value: &str) -> LoopExitId {
    LoopExitId::new(value).unwrap()
}

fn message_ref(value: &str) -> LoopMessageRef {
    LoopMessageRef::new(value).unwrap()
}

fn loop_gate_ref(value: &str) -> LoopGateRef {
    LoopGateRef::new(value).unwrap()
}

#[allow(dead_code)]
fn result_ref(value: &str) -> LoopResultRef {
    LoopResultRef::new(value).unwrap()
}
