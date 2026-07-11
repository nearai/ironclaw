use super::{
    AgentLoopExecutor, AgentLoopExecutorError, AgentLoopHostError, AgentLoopHostErrorKind,
    CanonicalAgentLoopExecutor, CapabilityFailureKind, CapabilityOutcome, CapabilityResultMessage,
    CheckpointKind, DefaultCompactionStrategy, FixedReplyAdmissionPolicy, GateOutcome, HostStage,
    LoopCheckpointKind, LoopCompactionError, LoopExecutionState, LoopExit, LoopFailureKind,
    LoopGateRef, LoopResultRef, LoopSafeSummary, MockHost, active_task_preserving_compaction_index,
    calls_response, empty_gate_state, family_with_compaction_strategy, family_with_gate_outcome,
    family_with_iteration_limit, family_with_reply_admission, final_staged_state, reply_response,
    reply_response_with_text,
};
use ironclaw_turns::run_profile::LoopRunInfoPort;

#[derive(Clone, Copy)]
struct MatrixRow {
    label: &'static str,
    setup: FailureSetup,
    expected_kind: ExpectedTerminal,
    expects_explanation: bool,
}

#[derive(Debug, Clone, Copy)]
enum FailureSetup {
    ModelError,
    CapabilityProtocolError,
    CapabilityInvalidInputRecoverable,
    CapabilityInvalidOutputRecoverable,
    IterationLimit,
    InvalidModelOutput,
    DriverBugApprovalSkip,
    NoProgressDetected,
    PolicyDenied,
    CapabilityPolicyDeniedRecoverable,
    CompactionUnavailable,
    TranscriptWriteFailed,
    CheckpointRejected,
}

#[derive(Debug, Clone, Copy)]
enum ExpectedTerminal {
    Failed {
        kind: LoopFailureKind,
        safe_summary: Option<&'static str>,
    },
    Error {
        error: ExpectedError,
    },
    CompletedDivergence {
        planned_kind: LoopFailureKind,
    },
}

#[derive(Debug, Clone, Copy)]
enum ExpectedError {
    HostUnavailable { stage: HostStage },
    CheckpointFailed { stage: CheckpointKind },
}

#[derive(Debug)]
struct ObservedTerminal {
    terminal: Terminal,
    final_assistant_refs: Option<Vec<ironclaw_turns::LoopMessageRef>>,
    finalized_assistant_messages: Vec<String>,
    model_request_count: usize,
}

#[derive(Debug)]
enum Terminal {
    Exit(LoopExit),
    Error(AgentLoopExecutorError),
}

const ROWS: &[MatrixRow] = &[
    MatrixRow {
        label: "ModelError <- with_model_errors exhausting retries",
        setup: FailureSetup::ModelError,
        expected_kind: ExpectedTerminal::Failed {
            kind: LoopFailureKind::ModelError,
            safe_summary: Some("model_unavailable"),
        },
        expects_explanation: false,
    },
    MatrixRow {
        label: "CapabilityProtocolError <- batch outcome Failed(Permanent)",
        setup: FailureSetup::CapabilityProtocolError,
        expected_kind: ExpectedTerminal::Failed {
            kind: LoopFailureKind::CapabilityProtocolError,
            safe_summary: Some("capability_permanent"),
        },
        expects_explanation: true,
    },
    MatrixRow {
        label: "ModelError <- capability Failed(InvalidInput)",
        setup: FailureSetup::CapabilityInvalidInputRecoverable,
        // stack #5389 makes this recoverable; matrix asserts recovery.
        expected_kind: ExpectedTerminal::CompletedDivergence {
            planned_kind: LoopFailureKind::ModelError,
        },
        expects_explanation: false,
    },
    MatrixRow {
        label: "CapabilityProtocolError <- capability Failed(InvalidOutput)",
        setup: FailureSetup::CapabilityInvalidOutputRecoverable,
        // stack #5389 makes this recoverable; matrix asserts recovery.
        expected_kind: ExpectedTerminal::CompletedDivergence {
            planned_kind: LoopFailureKind::CapabilityProtocolError,
        },
        expects_explanation: false,
    },
    MatrixRow {
        label: "IterationLimit <- family_with_iteration_limit(0)",
        setup: FailureSetup::IterationLimit,
        expected_kind: ExpectedTerminal::Failed {
            kind: LoopFailureKind::IterationLimit,
            safe_summary: None,
        },
        expects_explanation: true,
    },
    MatrixRow {
        label: "InvalidModelOutput <- RejectAlways reply admission",
        setup: FailureSetup::InvalidModelOutput,
        expected_kind: ExpectedTerminal::Failed {
            kind: LoopFailureKind::InvalidModelOutput,
            safe_summary: None,
        },
        expects_explanation: true,
    },
    MatrixRow {
        label: "DriverBug <- Approval gate SkipAndContinue",
        setup: FailureSetup::DriverBugApprovalSkip,
        // matrix-divergence: GateOutcome::validate_for_gate_kind marks
        // Approval+SkipAndContinue as DriverBug, but GateStage does not enforce
        // that validator today; the planned executor skips the gate and can
        // continue to a normal completion.
        expected_kind: ExpectedTerminal::CompletedDivergence {
            planned_kind: LoopFailureKind::DriverBug,
        },
        expects_explanation: false,
    },
    MatrixRow {
        label: "NoProgressDetected <- repeated identical no-change calls",
        setup: FailureSetup::NoProgressDetected,
        expected_kind: ExpectedTerminal::Failed {
            kind: LoopFailureKind::NoProgressDetected,
            safe_summary: None,
        },
        // matrix-divergence: NoProgressDetected is listed as explainable, but
        // the StopKind::NoProgressDetected exit path writes the failed exit
        // directly instead of calling attach_failure_explanation.
        expects_explanation: false,
    },
    MatrixRow {
        label: "PolicyDenied <- scripted Denied capability outcome",
        setup: FailureSetup::PolicyDenied,
        // matrix-divergence: DefaultRecoveryStrategy turns policy-denied
        // capability outcomes into model-visible tool-error results; with a
        // follow-up model reply the planned executor completes instead of
        // surfacing LoopFailureKind::PolicyDenied.
        expected_kind: ExpectedTerminal::CompletedDivergence {
            planned_kind: LoopFailureKind::PolicyDenied,
        },
        expects_explanation: false,
    },
    MatrixRow {
        label: "PolicyDenied <- capability Failed(PolicyDenied)",
        setup: FailureSetup::CapabilityPolicyDeniedRecoverable,
        // stack #5389 makes this recoverable; matrix asserts recovery.
        expected_kind: ExpectedTerminal::CompletedDivergence {
            planned_kind: LoopFailureKind::PolicyDenied,
        },
        expects_explanation: false,
    },
    MatrixRow {
        label: "CompactionUnavailable <- compaction port returns Err",
        setup: FailureSetup::CompactionUnavailable,
        // stack #5838 makes best-effort compaction failures recoverable; matrix
        // asserts the prompt path continues instead of failing the whole run.
        expected_kind: ExpectedTerminal::CompletedDivergence {
            planned_kind: LoopFailureKind::CompactionUnavailable,
        },
        expects_explanation: false,
    },
    MatrixRow {
        label: "TranscriptWriteFailed <- fail_transcript_with",
        setup: FailureSetup::TranscriptWriteFailed,
        // matrix-divergence: TranscriptWriteFailed enum origin is legacy
        // text_loop_driver; planned executor maps assistant transcript finalize
        // failure to HostUnavailable { stage: Transcript } before any LoopExit.
        expected_kind: ExpectedTerminal::Error {
            error: ExpectedError::HostUnavailable {
                stage: HostStage::Transcript,
            },
        },
        expects_explanation: false,
    },
    MatrixRow {
        label: "CheckpointRejected <- fail_checkpoint(BeforeModel)",
        setup: FailureSetup::CheckpointRejected,
        // matrix-divergence: CheckpointRejected enum origin is legacy
        // text_loop_driver; planned executor maps host checkpoint rejection to
        // AgentLoopExecutorError::CheckpointFailed rather than LoopExit::Failed.
        expected_kind: ExpectedTerminal::Error {
            error: ExpectedError::CheckpointFailed {
                stage: CheckpointKind::BeforeModel,
            },
        },
        expects_explanation: false,
    },
];

macro_rules! matrix_row_test {
    ($name:ident, $row_index:expr) => {
        // The executor future outgrows the default 2MiB test-thread stack in
        // debug builds on the in-run recovery rows (capability failure ->
        // model-visible result -> second model turn). Production loop threads
        // run with 8MiB stacks (`ironclaw_reborn_cli` serve runtime), so run
        // each row on a big-stack thread — the repo's standard pattern
        // (`traces/tests.rs`, `process_port.rs`) — instead of shrinking the row.
        #[test]
        fn $name() {
            std::thread::Builder::new()
                .stack_size(16 * 1024 * 1024)
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("matrix row runtime")
                        .block_on(run_matrix_row(&ROWS[$row_index]))
                })
                .expect("matrix row thread should spawn")
                .join()
                .expect("matrix row thread should not panic");
        }
    };
}

matrix_row_test!(matrix_model_error_exhausting_retries, 0);
matrix_row_test!(matrix_capability_protocol_error_permanent, 1);
matrix_row_test!(matrix_capability_invalid_input_recovers, 2);
matrix_row_test!(matrix_capability_invalid_output_recovers, 3);
matrix_row_test!(matrix_iteration_limit, 4);
matrix_row_test!(matrix_invalid_model_output, 5);
matrix_row_test!(matrix_driver_bug_approval_skip_diverges, 6);
matrix_row_test!(matrix_no_progress_detected, 7);
matrix_row_test!(matrix_policy_denied_outcome_diverges, 8);
matrix_row_test!(matrix_capability_policy_denied_recovers, 9);
matrix_row_test!(matrix_compaction_unavailable, 10);
matrix_row_test!(matrix_transcript_write_failed, 11);
matrix_row_test!(matrix_checkpoint_rejected, 12);

async fn run_matrix_row(row: &MatrixRow) {
    let observed = run_setup(row.setup).await;
    assert_expected_terminal(row, &observed);
}

async fn run_setup(setup: FailureSetup) -> ObservedTerminal {
    match setup {
        FailureSetup::ModelError => {
            let host = MockHost::new(Vec::new()).with_model_errors(vec![
                AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "model unavailable"),
                AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "model unavailable"),
                AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, "model unavailable"),
            ]);
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::CapabilityProtocolError => {
            let host = MockHost::new(vec![
                calls_response(),
                reply_response_with_text("explanation"),
            ])
            .with_batch_outcomes(vec![batch_outcome(CapabilityOutcome::Failed(
                ironclaw_turns::run_profile::CapabilityFailure {
                    error_kind: CapabilityFailureKind::Permanent,
                    safe_summary: "permanent protocol failure".to_string(),
                    detail: None,
                },
            ))]);
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::CapabilityInvalidInputRecoverable => {
            let host = MockHost::new(vec![
                calls_response(),
                reply_response_with_text("completed after invalid input"),
            ])
            .with_batch_outcomes(vec![batch_outcome(failed_capability(
                CapabilityFailureKind::InvalidInput,
                "invalid input supplied by model",
            ))]);
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::CapabilityInvalidOutputRecoverable => {
            let host = MockHost::new(vec![
                calls_response(),
                reply_response_with_text("completed after invalid output"),
            ])
            .with_batch_outcomes(vec![batch_outcome(failed_capability(
                CapabilityFailureKind::InvalidOutput,
                "invalid tool output",
            ))]);
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::IterationLimit => {
            let host = MockHost::new(vec![reply_response_with_text("iteration explanation")]);
            run_local(family_with_iteration_limit(0), host, None).await
        }
        FailureSetup::InvalidModelOutput => {
            let host = MockHost::new(vec![
                reply_response(),
                reply_response(),
                reply_response(),
                reply_response_with_text("invalid model output explanation"),
            ]);
            run_local(
                family_with_reply_admission(FixedReplyAdmissionPolicy::RejectAlways),
                host,
                None,
            )
            .await
        }
        FailureSetup::DriverBugApprovalSkip => {
            let host = MockHost::new(vec![
                calls_response(),
                reply_response_with_text("completed"),
            ])
            .with_batch_outcomes(vec![batch_outcome_stopped(
                CapabilityOutcome::ApprovalRequired {
                    gate_ref: LoopGateRef::new("gate:approval-skip").expect("valid"),
                    safe_summary: "approval required".to_string(),
                    approval_resume: None,
                },
            )]);
            run_local(
                family_with_gate_outcome(GateOutcome::SkipAndContinue {
                    gate: empty_gate_state(),
                }),
                host,
                None,
            )
            .await
        }
        FailureSetup::NoProgressDetected => {
            let host = MockHost::new(vec![calls_response(), calls_response(), calls_response()])
                .with_batch_outcomes(vec![
                    batch_outcome(no_change_result("result:no-progress-1")),
                    batch_outcome(no_change_result("result:no-progress-2")),
                    batch_outcome(no_change_result("result:no-progress-3")),
                ]);
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::PolicyDenied => {
            let host = MockHost::new(vec![
                calls_response(),
                reply_response_with_text("completed"),
            ])
            .with_batch_outcomes(vec![batch_outcome(CapabilityOutcome::Denied(
                ironclaw_turns::run_profile::CapabilityDenied {
                    reason_kind:
                        ironclaw_turns::run_profile::CapabilityDeniedReasonKind::EmptySurface,
                    safe_summary: "provider call denied".to_string(),
                },
            ))]);
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::CapabilityPolicyDeniedRecoverable => {
            let host = MockHost::new(vec![
                calls_response(),
                reply_response_with_text("completed after policy denial"),
            ])
            .with_batch_outcomes(vec![batch_outcome(failed_capability(
                CapabilityFailureKind::PolicyDenied,
                "capability policy denied",
            ))]);
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::CompactionUnavailable => {
            let host = MockHost::new(vec![reply_response_with_text("compaction explanation")])
                .with_prompt_compaction_index(active_task_preserving_compaction_index())
                .with_compaction_outcome(Err(LoopCompactionError::SecurityRejected {
                    safe_summary: LoopSafeSummary::new("security rejected").expect("safe"),
                }));
            let mut state = LoopExecutionState::initial_for_run(host.run_context());
            state.compaction_state.force_compact_on_next_iteration = true;
            run_local(
                family_with_compaction_strategy(DefaultCompactionStrategy {
                    deadline_ms: 100,
                    ..Default::default()
                }),
                host,
                Some(state),
            )
            .await
        }
        FailureSetup::TranscriptWriteFailed => {
            let host = MockHost::new(vec![reply_response_with_text("reply should not finalize")])
                .fail_transcript_with(AgentLoopHostErrorKind::TranscriptWriteFailed);
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::CheckpointRejected => {
            let host = MockHost::new(vec![reply_response_with_text("unused")])
                .fail_checkpoint(LoopCheckpointKind::BeforeModel);
            run_local(crate::families::default(), host, None).await
        }
    }
}

async fn run_local(
    family: crate::family::LoopFamily,
    host: MockHost,
    initial_state: Option<LoopExecutionState>,
) -> ObservedTerminal {
    let executor = CanonicalAgentLoopExecutor;
    let state =
        initial_state.unwrap_or_else(|| LoopExecutionState::initial_for_run(host.run_context()));
    let terminal = match executor.execute_family(&family, &host, state).await {
        Ok(exit) => Terminal::Exit(exit),
        Err(error) => Terminal::Error(error),
    };
    let final_assistant_refs = match &terminal {
        Terminal::Exit(LoopExit::Completed(_)) | Terminal::Exit(LoopExit::Failed(_)) => {
            Some(final_staged_state(&host).assistant_refs)
        }
        Terminal::Exit(_) | Terminal::Error(_) => None,
    };
    ObservedTerminal {
        terminal,
        final_assistant_refs,
        finalized_assistant_messages: host.finalized_assistant_messages(),
        model_request_count: host.model_requests().len(),
    }
}

fn assert_expected_terminal(row: &MatrixRow, observed: &ObservedTerminal) {
    match (&row.expected_kind, &observed.terminal) {
        (
            ExpectedTerminal::Failed { kind, safe_summary },
            Terminal::Exit(LoopExit::Failed(failed)),
        ) => {
            assert_eq!(
                failed.reason_kind, *kind,
                "{}: failed exit reason kind",
                row.label
            );
            assert_eq!(
                failed.reason_kind.as_str(),
                kind.as_str(),
                "{}: sanitized failure category",
                row.label
            );
            assert_eq!(
                failed
                    .safe_summary
                    .as_ref()
                    .map(|summary| summary.category()),
                *safe_summary,
                "{}: safe summary category",
                row.label
            );
            assert_explanation_refs(row, &failed.explanation_message_refs);
            let final_refs = observed
                .final_assistant_refs
                .as_ref()
                .expect("failed exits should have a final checkpoint state");
            assert_eq!(
                final_refs, &failed.explanation_message_refs,
                "{}: no fabricated final assistant reply",
                row.label
            );
        }
        (ExpectedTerminal::Error { error }, Terminal::Error(actual)) => {
            assert_expected_error(row, *error, actual);
            assert!(
                !row.expects_explanation,
                "{}: executor errors cannot carry explanation refs",
                row.label
            );
            match error {
                ExpectedError::HostUnavailable {
                    stage: HostStage::Transcript,
                } => {
                    assert_eq!(
                        observed.model_request_count, 1,
                        "{}: transcript row should reach exactly one model reply attempt",
                        row.label
                    );
                    assert!(
                        observed.finalized_assistant_messages.is_empty(),
                        "{}: no assistant message should be finalized after transcript write failure",
                        row.label
                    );
                }
                ExpectedError::CheckpointFailed { .. } => {
                    assert_eq!(
                        observed.model_request_count, 0,
                        "{}: checkpoint failure before model should not fabricate a reply",
                        row.label
                    );
                }
                ExpectedError::HostUnavailable { .. } => {}
            }
        }
        (
            ExpectedTerminal::CompletedDivergence { planned_kind },
            Terminal::Exit(LoopExit::Completed(completed)),
        ) => {
            assert!(
                !row.expects_explanation,
                "{}: completed divergence rows do not carry failure explanations",
                row.label
            );
            assert_eq!(
                completed.reply_message_refs.len(),
                1,
                "{}: matrix-divergence for planned {:?} currently completes with one real reply",
                row.label,
                planned_kind
            );
            assert!(
                completed.final_checkpoint_id.is_some(),
                "{}: completed divergence row should still checkpoint final state",
                row.label
            );
        }
        _ => panic!(
            "{}: expected {:?}, observed {:?}",
            row.label, row.expected_kind, observed.terminal
        ),
    }
}

fn assert_expected_error(
    row: &MatrixRow,
    expected: ExpectedError,
    actual: &AgentLoopExecutorError,
) {
    match expected {
        ExpectedError::HostUnavailable { stage } => {
            assert_eq!(
                actual,
                &AgentLoopExecutorError::HostUnavailable { stage },
                "{}: executor error",
                row.label
            );
        }
        ExpectedError::CheckpointFailed { stage } => {
            assert_eq!(
                actual,
                &AgentLoopExecutorError::CheckpointFailed { stage },
                "{}: executor error",
                row.label
            );
        }
    }
}

fn assert_explanation_refs(row: &MatrixRow, refs: &[ironclaw_turns::LoopMessageRef]) {
    if row.expects_explanation {
        assert!(
            !refs.is_empty(),
            "{}: expected a persisted failure explanation message ref",
            row.label
        );
    } else {
        assert!(
            refs.is_empty(),
            "{}: expected no failure explanation message refs",
            row.label
        );
    }
}

fn batch_outcome(
    outcome: CapabilityOutcome,
) -> ironclaw_turns::run_profile::CapabilityBatchOutcome {
    ironclaw_turns::run_profile::CapabilityBatchOutcome {
        outcomes: vec![outcome],
        stopped_on_suspension: false,
    }
}

fn failed_capability(error_kind: CapabilityFailureKind, safe_summary: &str) -> CapabilityOutcome {
    CapabilityOutcome::Failed(ironclaw_turns::run_profile::CapabilityFailure {
        error_kind,
        safe_summary: safe_summary.to_string(),
        detail: None,
    })
}

fn batch_outcome_stopped(
    outcome: CapabilityOutcome,
) -> ironclaw_turns::run_profile::CapabilityBatchOutcome {
    ironclaw_turns::run_profile::CapabilityBatchOutcome {
        outcomes: vec![outcome],
        stopped_on_suspension: true,
    }
}

fn no_change_result(result_ref: &str) -> CapabilityOutcome {
    CapabilityOutcome::Completed(CapabilityResultMessage {
        result_ref: LoopResultRef::new(result_ref).expect("valid"),
        safe_summary: "completed without progress".to_string(),
        progress: ironclaw_turns::run_profile::CapabilityProgress::NoChange,
        terminate_hint: false,
        byte_len: 0,
        output_digest: None,
        model_observation: None,
    })
}
