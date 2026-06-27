//! Text-only Reborn loop driver.
//!
//! This driver is deliberately narrow: it asks the host to build a text-only
//! prompt bundle, streams one host-managed model response, accepts only a final
//! assistant reply, and finalizes that reply through the transcript port.
//! Tool/capability calls are rejected until a tool-capable loop driver exists.

use async_trait::async_trait;
use ironclaw_turns::{
    LoopCompleted, LoopCompletionKind, LoopExit, LoopExitId, LoopFailureKind, LoopMessageRef,
    RunProfileVersion,
    run_profile::{
        AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError, AgentLoopDriverHost,
        AgentLoopDriverResumeRequest, AgentLoopDriverRunRequest, AgentLoopHostError,
        AgentLoopHostErrorKind, FinalizeAssistantMessage, LoopModelRequest,
        LoopPromptBundleRequest, ParentLoopOutput, PromptMode,
    },
};

use crate::model_failure_mapping::model_stage_failure_category;

pub(crate) const TEXT_ONLY_DRIVER_ID: &str = "reborn:text-only-model-reply";
/// Stage name for the model call site; matches the string used in `map_host_error` call sites.
const STAGE_MODEL: &str = "model";
pub(crate) const TEXT_ONLY_DRIVER_VERSION: u64 = 1;
const DEFAULT_CONTEXT_LIMIT: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOnlyModelReplyDriverConfig {
    pub context_limit: usize,
}

impl Default for TextOnlyModelReplyDriverConfig {
    fn default() -> Self {
        Self {
            context_limit: DEFAULT_CONTEXT_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TextOnlyModelReplyDriver {
    config: TextOnlyModelReplyDriverConfig,
}

impl TextOnlyModelReplyDriver {
    pub fn new(config: TextOnlyModelReplyDriverConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl AgentLoopDriver for TextOnlyModelReplyDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        AgentLoopDriverDescriptor::from_trusted_static(
            TEXT_ONLY_DRIVER_ID,
            RunProfileVersion::new(TEXT_ONLY_DRIVER_VERSION),
        )
        .expect("static text-only driver id must be valid") // safety: fixed validated driver id constant
    }

    async fn run(
        &self,
        request: AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        validate_run_request(&request, host, &self.descriptor())?;

        let prompt_bundle = host
            .build_prompt_bundle(LoopPromptBundleRequest {
                mode: PromptMode::TextOnly,
                context_cursor: None,
                surface_version: None,
                checkpoint_state_ref: None,
                max_messages: Some(context_limit_hint(self.config.context_limit)),
                inline_messages: Vec::new(),
                capability_view: None,
            })
            .await
            .map_err(|error| map_host_error("prompt", error))?;

        let model_response = host
            .stream_model(LoopModelRequest {
                inline_messages: Vec::new(),
                messages: prompt_bundle.messages,
                surface_version: prompt_bundle.surface_version,
                model_preference: None,
                capability_view: None,
            })
            .await
            .map_err(|error| map_host_error(STAGE_MODEL, error))?;

        let reply = match model_response.output {
            ParentLoopOutput::AssistantReply(reply) => reply,
            ParentLoopOutput::CapabilityCalls(_) => {
                return Err(AgentLoopDriverError::Failed {
                    reason_kind: loop_failure_kind_name(LoopFailureKind::InvalidModelOutput)
                        .to_string(),
                });
            }
        };

        let reply_ref = host
            .finalize_assistant_message(FinalizeAssistantMessage { reply })
            .await
            .map_err(|error| map_host_error("transcript", error))?;

        Ok(LoopExit::Completed(completed_final_reply(
            request.run_id,
            reply_ref,
        )?))
    }

    async fn resume(
        &self,
        _request: AgentLoopDriverResumeRequest,
        _host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        Err(AgentLoopDriverError::InvalidRequest {
            reason: "text-only model reply driver does not support resume".to_string(),
        })
    }
}

fn validate_run_request(
    request: &AgentLoopDriverRunRequest,
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    descriptor: &AgentLoopDriverDescriptor,
) -> Result<(), AgentLoopDriverError> {
    let context = host.run_context();
    if request.turn_id != context.turn_id || request.run_id != context.run_id {
        return Err(AgentLoopDriverError::InvalidRequest {
            reason: "driver request does not match loop host run context".to_string(),
        });
    }
    if request.resolved_run_profile != context.resolved_run_profile {
        return Err(AgentLoopDriverError::InvalidRequest {
            reason: "driver request profile does not match loop host run context".to_string(),
        });
    }
    if request.resolved_run_profile.loop_driver != *descriptor {
        return Err(AgentLoopDriverError::InvalidRequest {
            reason: "driver request profile is not assigned to the text-only model reply driver"
                .to_string(),
        });
    }
    Ok(())
}

fn completed_final_reply(
    run_id: ironclaw_turns::TurnRunId,
    reply_ref: LoopMessageRef,
) -> Result<LoopCompleted, AgentLoopDriverError> {
    let exit_id = LoopExitId::new(format!("exit:{run_id}-final-reply")).map_err(|_| {
        AgentLoopDriverError::Failed {
            reason_kind: loop_failure_kind_name(LoopFailureKind::DriverBug).to_string(),
        }
    })?;
    Ok(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs: vec![reply_ref],
        result_refs: Vec::new(),
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id,
    })
}

fn context_limit_hint(context_limit: usize) -> u32 {
    u32::try_from(context_limit.max(1)).unwrap_or(u32::MAX)
}

fn map_host_error(stage: &'static str, error: AgentLoopHostError) -> AgentLoopDriverError {
    tracing::warn!(
        stage,
        kind = ?error.kind,
        reason_kind = ?error.reason_kind,
        diagnostic_ref = ?error.diagnostic_ref,
        safe_summary = %error.safe_summary,
        "loop host port returned sanitized error"
    );

    if let Some(category) =
        model_stage_failure_category(stage == STAGE_MODEL, error.kind, error.reason_kind)
    {
        return AgentLoopDriverError::Failed {
            reason_kind: category.to_string(),
        };
    }

    match error.kind {
        AgentLoopHostErrorKind::InvalidInvocation
        | AgentLoopHostErrorKind::Invalid
        | AgentLoopHostErrorKind::ScopeMismatch => AgentLoopDriverError::InvalidRequest {
            reason: format!("{stage}: {}", error.kind.as_str()),
        },
        AgentLoopHostErrorKind::Unavailable | AgentLoopHostErrorKind::Cancelled => {
            AgentLoopDriverError::Unavailable {
                reason: format!("{stage}: {}", error.kind.as_str()),
            }
        }
        AgentLoopHostErrorKind::InvalidOutput => AgentLoopDriverError::Failed {
            reason_kind: loop_failure_kind_name(LoopFailureKind::InvalidModelOutput).to_string(),
        },
        AgentLoopHostErrorKind::Internal => AgentLoopDriverError::Unavailable {
            reason: format!("{stage}: unavailable"),
        },
        AgentLoopHostErrorKind::TranscriptWriteFailed => AgentLoopDriverError::Failed {
            reason_kind: loop_failure_kind_name(LoopFailureKind::TranscriptWriteFailed).to_string(),
        },
        AgentLoopHostErrorKind::BudgetExceeded
        | AgentLoopHostErrorKind::BudgetApprovalRequired
        | AgentLoopHostErrorKind::BudgetAccountingFailed
        | AgentLoopHostErrorKind::PolicyDenied => AgentLoopDriverError::Failed {
            reason_kind: loop_failure_kind_name(LoopFailureKind::ModelError).to_string(),
        },
        AgentLoopHostErrorKind::CredentialUnavailable => AgentLoopDriverError::Failed {
            reason_kind: loop_failure_kind_name(LoopFailureKind::ModelError).to_string(),
        },
        AgentLoopHostErrorKind::CheckpointRejected => AgentLoopDriverError::Failed {
            reason_kind: loop_failure_kind_name(LoopFailureKind::CheckpointRejected).to_string(),
        },
        AgentLoopHostErrorKind::Unauthorized | AgentLoopHostErrorKind::StaleSurface => {
            AgentLoopDriverError::Failed {
                reason_kind: loop_failure_kind_name(LoopFailureKind::DriverBug).to_string(),
            }
        }
    }
}

fn loop_failure_kind_name(kind: LoopFailureKind) -> &'static str {
    match kind {
        LoopFailureKind::ModelError => "model_error",
        LoopFailureKind::ContextBuildFailed => "context_build_failed",
        LoopFailureKind::CapabilityProtocolError => "capability_protocol_error",
        LoopFailureKind::IterationLimit => "iteration_limit",
        LoopFailureKind::InvalidModelOutput => "invalid_model_output",
        LoopFailureKind::CheckpointRejected => "checkpoint_rejected",
        LoopFailureKind::CheckpointUnavailable => "checkpoint_unavailable",
        LoopFailureKind::TranscriptWriteFailed => "transcript_write_failed",
        LoopFailureKind::DriverBug => "driver_bug",
        LoopFailureKind::InterruptedUnexpectedly => "interrupted_unexpectedly",
        LoopFailureKind::NoProgressDetected => "no_progress_detected",
        LoopFailureKind::PolicyDenied => "policy_denied",
        // LoopFailureKind is `#[non_exhaustive]`; fail closed if a new variant
        // lands in `ironclaw_turns` ahead of this matcher being updated.
        _ => "driver_bug",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure_categories::{
        MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY, MODEL_CREDITS_EXHAUSTED_CATEGORY,
        MODEL_CREDITS_EXHAUSTED_REASON_KIND,
    };

    #[test]
    fn host_internal_errors_map_to_sanitized_unavailable() {
        let mapped = map_host_error(
            "model",
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "RAW_PROVIDER_ERROR invalid api key sk-provider-secret /host/path tool_input",
            ),
        );

        assert_eq!(
            mapped,
            AgentLoopDriverError::Unavailable {
                reason: "model: unavailable".to_string()
            }
        );
        assert_driver_error_hides_raw_payloads(&mapped);
    }

    #[test]
    fn model_credit_exhaustion_maps_to_sanitized_failure_category() {
        let mapped = map_host_error(
            "model",
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::CredentialUnavailable,
                "safe summary wording is display-only",
            )
            .with_reason_kind(MODEL_CREDITS_EXHAUSTED_REASON_KIND),
        );

        assert_eq!(
            mapped,
            AgentLoopDriverError::Failed {
                reason_kind: MODEL_CREDITS_EXHAUSTED_CATEGORY.to_string()
            }
        );
    }

    #[test]
    fn model_credential_unavailable_maps_to_sanitized_failure_category() {
        let mapped = map_host_error(
            "model",
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::CredentialUnavailable,
                "model credentials are unavailable",
            ),
        );

        assert_eq!(
            mapped,
            AgentLoopDriverError::Failed {
                reason_kind: MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY.to_string()
            }
        );
    }

    #[test]
    fn non_model_stage_with_credit_reason_does_not_map_to_credits_category() {
        const CREDIT_SUMMARY: &str = "model provider account is out of credits";
        let mapped = map_host_error(
            "prompt",
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::CredentialUnavailable,
                CREDIT_SUMMARY,
            )
            .with_reason_kind(MODEL_CREDITS_EXHAUSTED_REASON_KIND),
        );

        assert_eq!(
            mapped,
            AgentLoopDriverError::Failed {
                reason_kind: loop_failure_kind_name(LoopFailureKind::ModelError).to_string()
            }
        );
    }

    #[test]
    fn non_model_stage_with_credential_unavailable_maps_to_model_error() {
        let mapped = map_host_error(
            "prompt",
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::CredentialUnavailable,
                "model credentials are unavailable",
            ),
        );

        assert_eq!(
            mapped,
            AgentLoopDriverError::Failed {
                reason_kind: loop_failure_kind_name(LoopFailureKind::ModelError).to_string()
            }
        );
    }

    fn assert_driver_error_hides_raw_payloads(error: &AgentLoopDriverError) {
        let debug = format!("{error:?}");
        for forbidden in [
            "RAW_PROVIDER_ERROR",
            "invalid api key",
            "sk-provider-secret",
            "/host/path",
            "tool_input",
        ] {
            assert!(
                !debug.contains(forbidden),
                "driver error leaked {forbidden}"
            );
        }
    }
}
