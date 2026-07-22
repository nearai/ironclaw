use super::{
    AgentLoopExecutor, AgentLoopExecutorError, AgentLoopHostError, AgentLoopHostErrorKind,
    CanonicalAgentLoopExecutor, CapabilityFailureKind, CheckpointKind, DefaultCompactionStrategy,
    FixedReplyAdmissionPolicy, GateOutcome, HostStage, LoopCheckpointKind, LoopCompactionError,
    LoopExecutionState, LoopExit, LoopFailureKind, LoopGateRef, LoopResultRef, LoopSafeSummary,
    MockHost, TerminalWarningObservation, active_task_preserving_compaction_index, calls_response,
    empty_gate_state, family_with_compaction_strategy, family_with_gate_outcome,
    family_with_iteration_limit, family_with_reply_admission, final_staged_state,
    provider_calls_response, reply_response, reply_response_with_text, resolution,
};
use ironclaw_turns::run_profile::{
    AppendCapabilityResultRef, CapabilityFailureDetail, LoopRunInfoPort, ToolObservationDetail,
    ToolObservationStatus,
};

const PERMANENT_CAPABILITY_DETAIL: &str = "permanent dispatch failure code 47";
const INVALID_OUTPUT_CAPABILITY_SUMMARY: &str = "MCP dispatch failed at /tmp/{socket}";
const INVALID_OUTPUT_CAPABILITY_DETAIL: &str = "MCP dispatch failed with transport error";

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
    CapabilityAuthorizationRecoverable,
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
    appended_result_refs: Vec<AppendCapabilityResultRef>,
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
        // §5a.1 closed: GateStage now enforces
        // GateOutcome::validate_for_gate_kind — a SkipAndContinue outcome on an
        // Approval gate is a strategy-contract violation and fails the run as
        // DriverBug instead of silently skipping the gated call and completing.
        expected_kind: ExpectedTerminal::Failed {
            kind: LoopFailureKind::DriverBug,
            safe_summary: None,
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
        // §5a.2 closed: the StopKind::NoProgressDetected failed branch now
        // attaches a failure explanation (same path as other explainable
        // kinds) after the final-answer nudge declines — the nudge itself is
        // untouched (a successful nudge still completes with no explanation).
        expects_explanation: true,
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
        label: "PolicyDenied <- capability Failed(Authorization)",
        setup: FailureSetup::CapabilityAuthorizationRecoverable,
        // Regression: the auto-built card-summary prefix ("capability failed
        // with authorization: ") used to trip the summary validator's own
        // "authorization:" marker ban and terminally bork the run before
        // handle_capability_error could fire. A 401/expired-scope tool failure
        // is model-recoverable and must stay that way.
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
                        .start_paused(true)
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
matrix_row_test!(matrix_driver_bug_approval_skip_fails_as_driver_bug, 6);
matrix_row_test!(matrix_no_progress_detected, 7);
matrix_row_test!(matrix_policy_denied_outcome_diverges, 8);
matrix_row_test!(matrix_capability_policy_denied_recovers, 9);
matrix_row_test!(matrix_capability_authorization_recovers, 10);
matrix_row_test!(matrix_compaction_unavailable, 11);
matrix_row_test!(matrix_transcript_write_failed, 12);
matrix_row_test!(matrix_checkpoint_rejected, 13);

async fn run_matrix_row(row: &MatrixRow) {
    let observed = run_setup(row.setup).await;
    assert_expected_terminal(row, &observed);
}

async fn run_setup(setup: FailureSetup) -> ObservedTerminal {
    match setup {
        FailureSetup::ModelError => {
            // One more error than the availability retry budget so the abort
            // (and its category) is driven by the Unavailable class rather
            // than the mock's script-exhausted Internal fallback.
            let unavailable_error_count = crate::strategies::DefaultRecoveryStrategy::default()
                .max_model_availability_attempts as usize
                + 1;
            let host = MockHost::new(Vec::new()).with_model_errors(
                (0..unavailable_error_count)
                    .map(|_| {
                        AgentLoopHostError::new(
                            AgentLoopHostErrorKind::Unavailable,
                            "model unavailable",
                        )
                    })
                    .collect(),
            );
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::CapabilityProtocolError => {
            let host = MockHost::new(vec![
                calls_response(),
                reply_response_with_text("explanation"),
            ])
            .with_batch_outcomes(vec![batch_outcome(resolution::failed(
                CapabilityFailureKind::Permanent,
                "permanent protocol failure".to_string(),
                Some(CapabilityFailureDetail::Diagnostic {
                    text: PERMANENT_CAPABILITY_DETAIL.to_string(),
                }),
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
                provider_calls_response(),
                reply_response_with_text("completed after invalid output"),
            ])
            .with_batch_outcomes(vec![batch_outcome(resolution::failed(
                CapabilityFailureKind::InvalidOutput,
                INVALID_OUTPUT_CAPABILITY_SUMMARY.to_string(),
                Some(CapabilityFailureDetail::Diagnostic {
                    text: INVALID_OUTPUT_CAPABILITY_DETAIL.to_string(),
                }),
            ))]);
            run_local(crate::families::default(), host, None).await
        }
        FailureSetup::IterationLimit => {
            let host = MockHost::new(vec![reply_response_with_text("iteration explanation")]);
            let mut state = LoopExecutionState::initial_for_run(host.run_context());
            assert!(
                state
                    .terminal_warning_state
                    .schedule(TerminalWarningObservation::iteration_limit(0))
            );
            state.terminal_warning_state.clear_pending();
            run_local(family_with_iteration_limit(0), host, Some(state)).await
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
                resolution::approval_required(
                    LoopGateRef::new("gate:approval-skip").expect("valid"),
                    "approval required".to_string(),
                    None,
                )
                .resolution,
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
            // The 4th scripted response feeds the failure-explanation model
            // call that fires after the no-progress stop (nudges are disabled
            // in this profile, so the nudge path declines without a model call).
            let host = MockHost::new(vec![
                calls_response(),
                calls_response(),
                calls_response(),
                reply_response_with_text("no progress explanation"),
            ])
            .with_batch_outcomes(vec![
                batch_outcome(no_change_result("result:no-progress-1")),
                batch_outcome(no_change_result("result:no-progress-2")),
                batch_outcome(no_change_result("result:no-progress-3")),
            ]);
            let mut state = LoopExecutionState::initial_for_run(host.run_context());
            assert!(
                state
                    .terminal_warning_state
                    .schedule(TerminalWarningObservation::no_progress(None, None))
            );
            state.terminal_warning_state.clear_pending();
            run_local(crate::families::default(), host, Some(state)).await
        }
        FailureSetup::PolicyDenied => {
            let host = MockHost::new(vec![
                calls_response(),
                reply_response_with_text("completed"),
            ])
            .with_batch_outcomes(vec![batch_outcome(
                resolution::denied(
                    ironclaw_turns::run_profile::CapabilityDeniedReasonKind::EmptySurface,
                    "provider call denied".to_string(),
                )
                .resolution,
            )]);
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
        FailureSetup::CapabilityAuthorizationRecoverable => {
            let host = MockHost::new(vec![
                calls_response(),
                reply_response_with_text("completed after authorization failure"),
            ])
            .with_batch_outcomes(vec![batch_outcome(failed_capability(
                CapabilityFailureKind::Authorization,
                "the provider token has expired",
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
        appended_result_refs: host.appended_result_refs(),
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
            if matches!(row.setup, FailureSetup::CapabilityProtocolError) {
                assert_eq!(
                    failed
                        .safe_summary
                        .as_ref()
                        .and_then(|failure| failure.detail()),
                    Some(PERMANENT_CAPABILITY_DETAIL),
                    "{}: terminal capability cause should survive on detail",
                    row.label
                );
            }
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
            if matches!(row.setup, FailureSetup::CapabilityInvalidOutputRecoverable) {
                // Phase 1: the capability failure's safe_summary carries `/`
                // and `{` and fails strict validation. The run must NOT bork —
                // it recovers (two model requests) and the real cause survives
                // on the model-visible observation detail.
                assert_eq!(
                    observed.model_request_count, 2,
                    "{}: malformed summary must remain recoverable",
                    row.label
                );
                let observation = observed
                    .appended_result_refs
                    .iter()
                    .find_map(|result| result.model_observation.as_ref())
                    .expect("provider result should carry a model-visible observation");
                assert_eq!(observation.status, ToolObservationStatus::Error);
                assert!(matches!(
                    &observation.detail,
                    ToolObservationDetail::GenericFailure {
                        failure_kind: CapabilityFailureKind::InvalidOutput,
                        detail: Some(detail),
                    } if detail == INVALID_OUTPUT_CAPABILITY_DETAIL
                ));
            }
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

fn batch_outcome(outcome: ironclaw_host_api::Resolution) -> ironclaw_host_api::ResolutionBatch {
    ironclaw_host_api::ResolutionBatch {
        resolutions: vec![outcome],
        stopped_on_suspension: false,
    }
}

fn failed_capability(
    error_kind: CapabilityFailureKind,
    safe_summary: &str,
) -> ironclaw_host_api::Resolution {
    resolution::failed(error_kind, safe_summary.to_string(), None)
}

fn batch_outcome_stopped(
    outcome: ironclaw_host_api::Resolution,
) -> ironclaw_host_api::ResolutionBatch {
    ironclaw_host_api::ResolutionBatch {
        resolutions: vec![outcome],
        stopped_on_suspension: true,
    }
}

fn no_change_result(result_ref: &str) -> ironclaw_host_api::Resolution {
    resolution::completed(
        LoopResultRef::new(result_ref).expect("valid"),
        "completed without progress".to_string(),
        ironclaw_turns::run_profile::CapabilityProgress::NoChange,
        false,
        0,
        None,
        None,
    )
}
