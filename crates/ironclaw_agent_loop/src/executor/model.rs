use super::*;

pub(super) enum ModelStep {
    Response(
        Box<LoopExecutionState>,
        ironclaw_turns::run_profile::LoopModelResponse,
    ),
    Exit(LoopExit),
}

impl CanonicalAgentLoopExecutor {
    pub(super) async fn stream_model_with_recovery(
        &self,
        planner: &dyn AgentLoopPlannerInternal,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
        request: LoopModelRequest,
    ) -> Result<ModelStep, AgentLoopExecutorError> {
        let mut recorded_failure = false;
        for _ in 0..MAX_MODEL_RETRIES {
            match host.stream_model(request.clone()).await {
                Ok(response) => {
                    state.recovery_state = state.recovery_state.cleared_attempts();
                    return Ok(ModelStep::Response(Box::new(state), response));
                }
                Err(error) => {
                    if error.kind == AgentLoopHostErrorKind::Cancelled {
                        return Err(AgentLoopExecutorError::Cancelled);
                    }
                    let Some(class) = model_error_class(&error) else {
                        return Err(AgentLoopExecutorError::HostUnavailable {
                            stage: HostStage::Model,
                        });
                    };
                    if !recorded_failure {
                        state.recent_failure_kinds.push(LoopFailureKind::ModelError);
                        recorded_failure = true;
                    }
                    let summary = ModelErrorSummary {
                        class,
                        safe_summary: sanitized_strategy_summary(error.safe_summary)?,
                        diagnostic_ref: error.diagnostic_ref,
                    };
                    match planner.recovery().on_model_error(&state, &summary).await {
                        RecoveryOutcome::Retry {
                            recovery, alter, ..
                        } => {
                            state.recovery_state = recovery;
                            match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                                CancelCheck::Continue(next) => state = *next,
                                CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                            }
                            honor_retry_alteration(alter.as_ref())?;
                            self.emit_progress(
                                host,
                                LoopProgressEvent::driver_note(
                                    LoopDriverNoteKind::Retrying,
                                    "retrying model request",
                                )
                                .map_err(|_| {
                                    AgentLoopExecutorError::PlannerContract {
                                        detail: "retry progress summary was invalid",
                                    }
                                })?,
                            )
                            .await;
                        }
                        RecoveryOutcome::SkipResult { .. } => {
                            return Err(AgentLoopExecutorError::PlannerContract {
                                detail: "SkipResult on model error",
                            });
                        }
                        RecoveryOutcome::Abort {
                            recovery,
                            failure_kind,
                        } => {
                            state.recovery_state = recovery;
                            match self.checkpoint_and_exit_if_cancelled(host, state).await? {
                                CancelCheck::Continue(next) => state = *next,
                                CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                            }
                            let checked =
                                self.checkpoint(host, state, CheckpointKind::Final).await?;
                            return Ok(ModelStep::Exit(failed_exit(
                                host,
                                checked.state,
                                failure_kind,
                                Some(checked.checkpoint_id),
                            )?));
                        }
                    }
                }
            }
        }

        let checked = self.checkpoint(host, state, CheckpointKind::Final).await?;
        Ok(ModelStep::Exit(failed_exit(
            host,
            checked.state,
            LoopFailureKind::DriverBug,
            Some(checked.checkpoint_id),
        )?))
    }
}
