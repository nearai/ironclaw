use super::*;
use crate::{
    BlockedReason, GateRef, SanitizedFailure, TurnCheckpointId, TurnStatus,
    runner::TurnRunnerOutcome,
};
use serde_json::json;

#[test]
fn no_progress_detected_failure_kind_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_value(LoopFailureKind::NoProgressDetected).unwrap(),
        json!("no_progress_detected")
    );
}

#[test]
fn policy_denied_failure_kind_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_value(LoopFailureKind::PolicyDenied).unwrap(),
        json!("policy_denied")
    );
}

#[test]
fn await_dependent_run_blocked_kind_maps_to_dependent_run_reason() {
    let gate_ref = LoopGateRef::new("gate:dependent-run").unwrap();
    let reason = LoopBlockedKind::AwaitDependentRun
        .to_blocked_reason(gate_ref.clone())
        .unwrap();

    assert_eq!(
        reason,
        BlockedReason::DependentRun {
            gate_ref: GateRef::new(gate_ref.as_str()).unwrap()
        }
    );
    assert_eq!(reason.status(), TurnStatus::BlockedDependentRun);
}

#[test]
fn validation_policy_named_constructors_keep_fail_closed_default_and_host_verified_evidence_explicit()
 {
    let default_policy = LoopExitValidationPolicy::recovery_required();
    assert!(!default_policy.completion_refs_verified());
    assert!(!default_policy.blocked_evidence_verified());
    assert!(!default_policy.failure_evidence_verified());
    assert!(!default_policy.host_cancellation_observed());
    assert!(!default_policy.requires_final_checkpoint());
    assert!(!default_policy.allows_no_reply_completion());
    assert!(!default_policy.final_checkpoint_verified());
    assert_eq!(
        default_policy.invalid_handling(),
        LoopExitInvalidHandling::RecoveryRequired
    );

    let trusted_policy = LoopExitValidationPolicy::fail_terminal()
        .require_final_checkpoint()
        .with_allow_no_reply_completion()
        .with_host_verified_final_checkpoint()
        .with_host_verified_completion_refs()
        .with_host_verified_blocked_evidence()
        .with_host_verified_failure_evidence()
        .with_host_cancellation_observed();
    assert!(trusted_policy.completion_refs_verified());
    assert!(trusted_policy.blocked_evidence_verified());
    assert!(trusted_policy.failure_evidence_verified());
    assert!(trusted_policy.host_cancellation_observed());
    assert!(trusted_policy.requires_final_checkpoint());
    assert!(trusted_policy.allows_no_reply_completion());
    assert!(trusted_policy.final_checkpoint_verified());
    assert_eq!(
        trusted_policy.invalid_handling(),
        LoopExitInvalidHandling::FailTerminal
    );
}

#[test]
fn loop_exit_validation_policy_deserialization_cannot_mint_host_verified_evidence() {
    for trusted_field in [
        "allow_no_reply_completion",
        "final_checkpoint_verified",
        "host_cancellation_observed",
        "completion_refs_verified",
        "blocked_evidence_verified",
        "failure_evidence_verified",
    ] {
        let mut forged = json!({
            "require_final_checkpoint": false,
            "allow_no_reply_completion": false,
            "final_checkpoint_verified": false,
            "host_cancellation_observed": false,
            "invalid_handling": "recovery_required",
            "completion_refs_verified": false,
            "blocked_evidence_verified": false,
            "failure_evidence_verified": false
        });
        forged[trusted_field] = json!(true);
        assert!(
            serde_json::from_value::<LoopExitValidationPolicy>(forged).is_err(),
            "{trusted_field} should not be wire-mintable"
        );
    }

    let forged_terminal = json!({
        "require_final_checkpoint": false,
        "allow_no_reply_completion": false,
        "final_checkpoint_verified": false,
        "host_cancellation_observed": false,
        "invalid_handling": "fail_terminal",
        "completion_refs_verified": false,
        "blocked_evidence_verified": false,
        "failure_evidence_verified": false
    });
    assert!(serde_json::from_value::<LoopExitValidationPolicy>(forged_terminal).is_err());

    let strict_fail_closed = json!({
        "require_final_checkpoint": true,
        "allow_no_reply_completion": false,
        "final_checkpoint_verified": false,
        "host_cancellation_observed": false,
        "invalid_handling": "recovery_required",
        "completion_refs_verified": false,
        "blocked_evidence_verified": false,
        "failure_evidence_verified": false
    });
    let policy = serde_json::from_value::<LoopExitValidationPolicy>(strict_fail_closed).unwrap();
    assert!(policy.requires_final_checkpoint());
    assert!(!policy.allows_no_reply_completion());
    assert!(!policy.completion_refs_verified());
    assert_eq!(
        policy.invalid_handling(),
        LoopExitInvalidHandling::RecoveryRequired
    );
}

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
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
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
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
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
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::RecoveryRequired,
        completion_refs_verified: true,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
    });
    assert!(matches!(
        uncertain_decision,
        LoopExitValidationDecision {
            mapping: LoopExitMapping::RecoveryRequired { .. },
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
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
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
        LoopExitMapping::RecoveryRequired { .. }
    ));
}

#[test]
fn final_checkpoint_policy_rejects_terminal_exit_without_checkpoint() {
    let cases = [
        LoopExit::Completed(LoopCompleted {
            completion_kind: LoopCompletionKind::FinalReply,
            reply_message_refs: vec![message_ref("msg:assistant-final")],
            result_refs: vec![],
            final_checkpoint_id: None,
            usage_summary_ref: None,
            exit_id: exit_id("exit:no-final-checkpoint-completed"),
        }),
        LoopExit::Cancelled(LoopCancelled {
            reason_kind: LoopCancelledReasonKind::HostCancellation,
            checkpoint_id: None,
            interrupted_message_refs: vec![],
            exit_id: exit_id("exit:no-final-checkpoint-cancelled"),
        }),
        LoopExit::failed(
            LoopFailureKind::DriverBug,
            exit_id("exit:no-final-checkpoint-failed"),
        ),
    ];

    for exit in cases {
        let decision = exit.validate(LoopExitValidationPolicy {
            require_final_checkpoint: true,
            allow_no_reply_completion: false,
            final_checkpoint_verified: false,
            host_cancellation_observed: true,
            invalid_handling: LoopExitInvalidHandling::FailTerminal,
            completion_refs_verified: true,
            blocked_evidence_verified: false,
            failure_evidence_verified: true,
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
}

#[test]
fn validation_policy_requires_final_checkpoint_only_when_configured() {
    let cases = [
        (
            false,
            None,
            TurnRunnerOutcome::Completed.into(),
            "relaxed policy should accept durable completion refs without a final checkpoint",
        ),
        (
            true,
            Some(LoopExitViolationKind::MissingFinalCheckpoint),
            TurnRunnerOutcome::Failed {
                failure: SanitizedFailure::new("driver_protocol_violation").unwrap(),
            }
            .into(),
            "strict policy should reject terminal exits without a final checkpoint",
        ),
    ];

    for (require_final_checkpoint, expected_violation, expected_mapping, context) in cases {
        let decision = LoopExit::Completed(LoopCompleted {
            completion_kind: LoopCompletionKind::FinalReply,
            reply_message_refs: vec![message_ref("msg:assistant-final")],
            result_refs: vec![],
            final_checkpoint_id: None,
            usage_summary_ref: None,
            exit_id: exit_id("exit:checkpoint-policy"),
        })
        .validate(LoopExitValidationPolicy {
            require_final_checkpoint,
            allow_no_reply_completion: false,
            final_checkpoint_verified: false,
            host_cancellation_observed: false,
            invalid_handling: LoopExitInvalidHandling::FailTerminal,
            completion_refs_verified: true,
            blocked_evidence_verified: false,
            failure_evidence_verified: false,
        });

        assert_eq!(
            decision
                .violation
                .as_ref()
                .map(|violation| violation.kind()),
            expected_violation,
            "{context}"
        );
        assert_eq!(decision.mapping, expected_mapping, "{context}");
    }
}

#[test]
fn blocked_exit_maps_to_block_run_outcome_with_verified_checkpoint_and_gate_ref() {
    let checkpoint_id = TurnCheckpointId::new();
    let loop_gate_ref = loop_gate_ref("gate:approval-gate");
    let gate_ref = GateRef::new(loop_gate_ref.as_str()).unwrap();
    let state_ref = checkpoint_state_ref();
    let decision = LoopExit::Blocked(LoopBlocked {
        kind: LoopBlockedKind::Approval,
        gate_ref: loop_gate_ref,
        checkpoint_id,
        state_ref: state_ref.clone(),
        exit_id: exit_id("exit:blocked"),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
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
            state_ref,
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
        state_ref: checkpoint_state_ref(),
        exit_id: exit_id("exit:unverified-blocked"),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
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
        LoopExitMapping::RecoveryRequired { .. }
    ));
}

#[test]
fn cancelled_exit_requires_observed_host_cancellation() {
    let exit = LoopExit::cancelled_for_observed_interrupt(exit_id("exit:cancelled"));

    let rejected = exit.clone().validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
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
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
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
        LoopExitMapping::RecoveryRequired { .. }
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

// --- Gap coverage tests (KB-037) ---

#[test]
fn no_reply_with_empty_refs_requires_explicit_policy_permission() {
    let exit = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::NoReply,
        reply_message_refs: vec![],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id("exit:no-reply-empty"),
    });

    let decision = exit.validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
        completion_refs_verified: true,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
    });

    assert_eq!(
        decision.violation.as_ref().unwrap().kind(),
        LoopExitViolationKind::NoReplyNotAllowed,
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
fn no_reply_with_empty_refs_maps_to_completed_when_policy_allows_it() {
    let decision = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::NoReply,
        reply_message_refs: vec![],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id("exit:no-reply-allowed"),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        allow_no_reply_completion: true,
        final_checkpoint_verified: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
        completion_refs_verified: false,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
    });

    assert_eq!(decision.violation, None);
    assert_eq!(decision.mapping, TurnRunnerOutcome::Completed.into());
}

#[test]
fn delegated_result_with_result_refs_maps_to_trusted_completed() {
    let decision = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::DelegatedResult,
        reply_message_refs: vec![],
        result_refs: vec![result_ref("result:delegated-job-1")],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id("exit:delegated"),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
        completion_refs_verified: true,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
    });

    assert_eq!(decision.violation, None);
    assert_eq!(decision.mapping, TurnRunnerOutcome::Completed.into());
}

#[test]
fn result_only_with_result_refs_maps_to_trusted_completed() {
    let decision = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::ResultOnly,
        reply_message_refs: vec![],
        result_refs: vec![result_ref("result:tool-output-1")],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: exit_id("exit:result-only"),
    })
    .validate(LoopExitValidationPolicy {
        require_final_checkpoint: false,
        allow_no_reply_completion: false,
        final_checkpoint_verified: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
        completion_refs_verified: true,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
    });

    assert_eq!(decision.violation, None);
    assert_eq!(decision.mapping, TurnRunnerOutcome::Completed.into());
}

#[test]
fn completion_kind_must_match_durable_reference_shape() {
    let policy = LoopExitValidationPolicy {
        require_final_checkpoint: false,
        allow_no_reply_completion: true,
        final_checkpoint_verified: false,
        host_cancellation_observed: false,
        invalid_handling: LoopExitInvalidHandling::FailTerminal,
        completion_refs_verified: true,
        blocked_evidence_verified: false,
        failure_evidence_verified: false,
    };

    for (completion_kind, reply_message_refs, result_refs, expected_violation) in [
        (
            LoopCompletionKind::FinalReply,
            vec![],
            vec![result_ref("result:tool-output-only")],
            LoopExitViolationKind::MissingCompletionReference,
        ),
        (
            LoopCompletionKind::ResultOnly,
            vec![message_ref("msg:unexpected-assistant")],
            vec![result_ref("result:tool-output-1")],
            LoopExitViolationKind::MismatchedCompletionReferenceKind,
        ),
        (
            LoopCompletionKind::DelegatedResult,
            vec![message_ref("msg:unexpected-assistant")],
            vec![result_ref("result:delegated-job-1")],
            LoopExitViolationKind::MismatchedCompletionReferenceKind,
        ),
        (
            LoopCompletionKind::NoReply,
            vec![message_ref("msg:unexpected-assistant")],
            vec![],
            LoopExitViolationKind::MismatchedCompletionReferenceKind,
        ),
        (
            LoopCompletionKind::NoReply,
            vec![],
            vec![result_ref("result:unexpected-output")],
            LoopExitViolationKind::MismatchedCompletionReferenceKind,
        ),
    ] {
        let decision = LoopExit::Completed(LoopCompleted {
            completion_kind,
            reply_message_refs,
            result_refs,
            final_checkpoint_id: None,
            usage_summary_ref: None,
            exit_id: exit_id("exit:mismatched-completion-kind"),
        })
        .validate(policy);

        assert_eq!(
            decision.violation.as_ref().map(LoopExitViolation::kind),
            Some(expected_violation),
            "completion kind {completion_kind:?} should reject mismatched refs"
        );
        assert_eq!(
            decision.mapping,
            TurnRunnerOutcome::Failed {
                failure: SanitizedFailure::new("driver_protocol_violation").unwrap(),
            }
            .into()
        );
    }
}

#[test]
fn blocked_variants_map_to_correct_blocked_reason() {
    for kind in [
        LoopBlockedKind::Approval,
        LoopBlockedKind::Auth,
        LoopBlockedKind::Resource,
        LoopBlockedKind::AwaitDependentRun,
    ] {
        let checkpoint_id = TurnCheckpointId::new();
        let lg = loop_gate_ref("gate:test-gate");
        let gate_ref = GateRef::new(lg.as_str()).unwrap();
        let state_ref = checkpoint_state_ref();

        let decision = LoopExit::Blocked(LoopBlocked {
            kind,
            gate_ref: lg,
            checkpoint_id,
            state_ref: state_ref.clone(),
            exit_id: exit_id("exit:blocked-variant"),
        })
        .validate(LoopExitValidationPolicy {
            blocked_evidence_verified: true,
            ..LoopExitValidationPolicy::default()
        });

        let expected_reason = match kind {
            LoopBlockedKind::Approval => BlockedReason::Approval { gate_ref },
            LoopBlockedKind::Auth => BlockedReason::Auth { gate_ref },
            LoopBlockedKind::Resource => BlockedReason::Resource { gate_ref },
            LoopBlockedKind::AwaitDependentRun => BlockedReason::DependentRun { gate_ref },
        };

        assert_eq!(decision.violation, None);
        assert_eq!(
            decision.mapping,
            TurnRunnerOutcome::Blocked {
                checkpoint_id,
                state_ref,
                reason: expected_reason,
            }
            .into()
        );
    }
}

#[test]
fn all_failure_kinds_produce_stable_sanitized_category_strings() {
    let variants: &[(LoopFailureKind, &str)] = &[
        (LoopFailureKind::ModelError, "model_error"),
        (LoopFailureKind::ContextBuildFailed, "context_build_failed"),
        (
            LoopFailureKind::CapabilityProtocolError,
            "capability_protocol_error",
        ),
        (LoopFailureKind::IterationLimit, "iteration_limit"),
        (LoopFailureKind::InvalidModelOutput, "invalid_model_output"),
        (LoopFailureKind::CheckpointRejected, "checkpoint_rejected"),
        (
            LoopFailureKind::TranscriptWriteFailed,
            "transcript_write_failed",
        ),
        (LoopFailureKind::DriverBug, "driver_bug"),
        (
            LoopFailureKind::InterruptedUnexpectedly,
            "interrupted_unexpectedly",
        ),
        (LoopFailureKind::NoProgressDetected, "no_progress_detected"),
        (LoopFailureKind::PolicyDenied, "policy_denied"),
    ];

    for (kind, expected_category) in variants {
        let decision = LoopExit::failed(*kind, exit_id("exit:failure-variant")).validate(
            LoopExitValidationPolicy {
                failure_evidence_verified: true,
                ..LoopExitValidationPolicy::default()
            },
        );

        assert_eq!(
            decision.violation, None,
            "unexpected violation for {kind:?}"
        );
        assert_eq!(
            decision.mapping,
            TurnRunnerOutcome::Failed {
                failure: SanitizedFailure::new(*expected_category).unwrap(),
            }
            .into(),
            "wrong category for {kind:?}"
        );
    }
}

#[test]
fn recovery_required_is_not_a_loop_exit_variant() {
    // Attempt various shapes that might be confused with a recovery_required variant
    for payload in [
        json!("recovery_required"),
        json!({"type": "recovery_required"}),
        json!({"type": "recovery_required", "failure": {"category": "test"}}),
        json!({"recovery_required": {}}),
        json!({"recovery_required": {"failure": {"category": "test"}}}),
        json!({"recovery_required": null}),
    ] {
        assert!(
            serde_json::from_value::<LoopExit>(payload.clone()).is_err(),
            "LoopExit accepted recovery_required variant: {payload}"
        );
    }
}

#[test]
fn cancelled_with_checkpoint_and_interrupted_refs_maps_to_cancelled_outcome() {
    let checkpoint_id = TurnCheckpointId::new();
    let exit = LoopExit::Cancelled(LoopCancelled {
        reason_kind: LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: Some(checkpoint_id),
        interrupted_message_refs: vec![message_ref("msg:partial-1"), message_ref("msg:partial-2")],
        exit_id: exit_id("exit:cancelled-with-checkpoint"),
    });

    let decision = exit.validate(LoopExitValidationPolicy {
        host_cancellation_observed: true,
        ..LoopExitValidationPolicy::default()
    });

    assert_eq!(decision.violation, None);
    assert_eq!(decision.mapping, TurnRunnerOutcome::Cancelled.into());
}

#[test]
fn terminal_statuses_release_lock_and_non_terminal_keep_it() {
    for status in [
        TurnStatus::Queued,
        TurnStatus::Running,
        TurnStatus::BlockedApproval,
        TurnStatus::BlockedAuth,
        TurnStatus::BlockedResource,
        TurnStatus::BlockedDependentRun,
        TurnStatus::CancelRequested,
        TurnStatus::Cancelled,
        TurnStatus::Completed,
        TurnStatus::Failed,
        TurnStatus::RecoveryRequired,
    ] {
        let (expected_terminal, expected_keeps_lock) = match status {
            TurnStatus::Queued => (false, true),
            TurnStatus::Running => (false, true),
            TurnStatus::BlockedApproval => (false, true),
            TurnStatus::BlockedAuth => (false, true),
            TurnStatus::BlockedResource => (false, true),
            TurnStatus::BlockedDependentRun => (false, true),
            TurnStatus::CancelRequested => (false, true),
            TurnStatus::Cancelled => (true, false),
            TurnStatus::Completed => (true, false),
            TurnStatus::Failed => (true, false),
            TurnStatus::RecoveryRequired => (false, true),
        };

        assert_eq!(
            status.is_terminal(),
            expected_terminal,
            "{status:?} terminal classification changed"
        );
        assert_eq!(
            status.keeps_active_lock(),
            expected_keeps_lock,
            "{status:?} lock retention changed"
        );
    }
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

fn checkpoint_state_ref() -> LoopCheckpointStateRef {
    LoopCheckpointStateRef::new("checkpoint:blocked-state").unwrap()
}

fn result_ref(value: &str) -> LoopResultRef {
    LoopResultRef::new(value).unwrap()
}
