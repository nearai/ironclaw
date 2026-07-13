use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{InvocationId, UserId};
use ironclaw_loop_host::{CapabilityResultWrite, DurablePersistence};
use ironclaw_threads::{
    MessageKind, MessageStatus, ReadToolResultRecordRequest, SessionThreadError,
    SessionThreadService, TOOL_RESULT_RECORD_READ_MAX_BYTES, ThreadHistoryRequest,
    ToolResultReferenceEnvelope,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailure, CapabilityFailureKind,
    CapabilityOutcome, CapabilityProgress, CapabilityResultMessage, ConcurrencyHint,
    MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleArtifact,
    ModelVisibleToolObservation, ObservationTrust, ToolObservationDetail, ToolObservationStatus,
    sanitize_model_visible_text,
};

use super::{
    local_dev_thread_scope_for_run,
    synthetic_capability::{
        LocalDevSyntheticCapability, LocalDevSyntheticCapabilityDescriptor,
        LocalDevSyntheticCapabilityHandler, LocalDevSyntheticCapabilityInvocation,
    },
};

/// Test-support wrap: layers the synthetic `result_read` capability onto
/// `inner`, mirroring how `refreshing_capability_port.rs`'s `build_inner`
/// wires it in production (unconditionally, via `wrap_local_dev_synthetic_capabilities`).
/// `input_resolver`/`result_writer` MUST be the SAME shared io object the
/// harness's capability port already uses -- see
/// `RefreshingLocalDevCapabilityPortTestParts::input_resolver` in
/// `test_support/refreshing_capability_port.rs` for the identical
/// same-object requirement. Tests only -- gated behind `test-support`,
/// ships zero bytes in production builds.
#[cfg(feature = "test-support")]
pub(crate) fn wrap_result_read_capability_for_test(
    inner: Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    thread_service: Arc<dyn SessionThreadService>,
    fallback_user_id: UserId,
    run_context: ironclaw_turns::run_profile::LoopRunContext,
    input_resolver: Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver>,
    result_writer: Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter>,
) -> Result<Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>, AgentLoopHostError> {
    super::synthetic_capability::wrap_local_dev_synthetic_capabilities(
        inner,
        vec![result_read_capability(thread_service, fallback_user_id)?],
        run_context,
        input_resolver,
        result_writer,
        // trajectory_observer: None — not wired in the integration-test harness.
        None,
    )
}

/// Test-support export of the capability id, so integration tests can script
/// a `result_read` tool call without hand-copying the literal.
#[cfg(feature = "test-support")]
pub(crate) const RESULT_READ_CAPABILITY_ID_FOR_TEST: &str = RESULT_READ_CAPABILITY_ID;

pub(super) const RESULT_READ_CAPABILITY_ID: &str = "builtin.result_read";
const RESULT_READ_PROVIDER_TOOL_NAME: &str = "builtin__result_read";
const RESULT_READ_MIN_BYTES: u64 = 4;
const RESULT_READ_MAX_BYTES: u64 = TOOL_RESULT_RECORD_READ_MAX_BYTES as u64;

pub(super) fn result_read_capability(
    thread_service: Arc<dyn SessionThreadService>,
    fallback_user_id: UserId,
) -> Result<LocalDevSyntheticCapability, AgentLoopHostError> {
    Ok(LocalDevSyntheticCapability::new(
        LocalDevSyntheticCapabilityDescriptor::new(
            RESULT_READ_CAPABILITY_ID,
            RESULT_READ_PROVIDER_TOOL_NAME,
            "Read a bounded continuation of a previously completed tool result by result reference.",
            ConcurrencyHint::SafeForParallel,
            result_read_input_schema(),
        )?,
        Arc::new(ResultReadHandler {
            thread_service,
            fallback_user_id,
        }),
    ))
}

struct ResultReadHandler {
    thread_service: Arc<dyn SessionThreadService>,
    fallback_user_id: UserId,
}

#[async_trait]
impl LocalDevSyntheticCapabilityHandler for ResultReadHandler {
    fn validate_provider_arguments(
        &self,
        _arguments: &serde_json::Value,
    ) -> Result<(), AgentLoopHostError> {
        // Provider-call registration must not terminalize a turn for a
        // model-correctable result_read mistake. `invoke` returns that shape
        // as a model-visible InvalidInput failure instead.
        Ok(())
    }

    async fn invoke(
        &self,
        invocation: LocalDevSyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let input = match parse_result_read_input(&invocation.input) {
            Ok(input) => input,
            Err(error) => {
                return Ok(CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    safe_summary: error.safe_summary,
                    detail: None,
                }));
            }
        };
        let scope = local_dev_thread_scope_for_run(&invocation.run_context, &self.fallback_user_id)
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "result reader requires an agent-scoped thread",
                )
            })?;
        let reference_is_available = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: scope.clone(),
                thread_id: invocation.run_context.thread_id.clone(),
            })
            .await
            .map(|history| {
                history.messages.iter().any(|message| {
                    message.kind == MessageKind::ToolResultReference
                        && message.status == MessageStatus::Finalized
                        && message.tool_result_ref.as_deref() == Some(input.result_ref.as_str())
                })
            });
        let reference_is_available = match reference_is_available {
            Ok(available) => available,
            Err(SessionThreadError::UnknownThread { .. }) => false,
            Err(error) => {
                return Err(storage_unavailable_error(error, "history lookup"));
            }
        };
        if !reference_is_available {
            return Ok(unavailable_result_reference());
        }

        let chunk = match self
            .thread_service
            .read_tool_result_record(ReadToolResultRecordRequest {
                scope,
                thread_id: invocation.run_context.thread_id.clone(),
                result_ref: input.result_ref.clone(),
                offset: input.offset,
                max_bytes: input.max_bytes as usize,
            })
            .await
        {
            Ok(Some(chunk)) => chunk,
            Ok(None) | Err(SessionThreadError::UnknownThread { .. }) => {
                return Ok(unavailable_result_reference());
            }
            Err(error) => {
                return Err(storage_unavailable_error(error, "record lookup"));
            }
        };
        let ironclaw_threads::ToolResultRecordChunk {
            content: chunk_content,
            total_bytes,
            next_offset,
        } = chunk;
        let content = match String::from_utf8(chunk_content) {
            Ok(content) => content,
            Err(_) => return Ok(non_text_result_content()),
        };
        let output = serde_json::json!({
            "result_ref": input.result_ref.clone(),
            "offset": input.offset,
            "content": content,
            "total_bytes": total_bytes,
            "next_offset": next_offset,
        });
        // `InlineOnly` (see `DurablePersistence` doc comment): this chunk is
        // already fully delivered to the model inline via
        // `result_read_observation`'s `preview`. The ORIGINAL result this
        // chunk was paged from stays durable and untouched.
        let mut write = invocation
            .result_writer
            .write_capability_result(CapabilityResultWrite {
                run_context: &invocation.run_context,
                input_ref: &invocation.request.input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &invocation.request.capability_id,
                output,
                display_preview: None,
                durable_persistence: DurablePersistence::InlineOnly,
            })
            .await?;
        write.model_observation = Some(result_read_observation(
            &input.result_ref,
            write.byte_len,
            total_bytes,
            next_offset,
            sanitize_model_visible_text(content),
        ));
        Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: write.result_ref,
            safe_summary: "result chunk returned".to_string(),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: write.byte_len,
            output_digest: write.output_digest,
            model_observation: write.model_observation,
        }))
    }
}

fn result_read_observation(
    result_ref: &str,
    byte_len: u64,
    total_bytes: u64,
    next_offset: Option<u64>,
    content: String,
) -> ModelVisibleToolObservation {
    ModelVisibleToolObservation {
        schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        summary: "Requested tool-result chunk returned.".to_string(),
        detail: ToolObservationDetail::ResultReference {
            result_ref: result_ref.to_string(),
            byte_len,
            preview: Some(content),
            total_bytes: Some(total_bytes),
            next_offset,
        },
        artifacts: vec![ModelVisibleArtifact {
            artifact_ref: result_ref.to_string(),
            summary: "Stored result-read response".to_string(),
        }],
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    }
}

fn unavailable_result_reference() -> CapabilityOutcome {
    CapabilityOutcome::Failed(CapabilityFailure {
        error_kind: CapabilityFailureKind::InvalidInput,
        safe_summary: "result reference is unavailable in this thread".to_string(),
        detail: None,
    })
}

fn non_text_result_content() -> CapabilityOutcome {
    CapabilityOutcome::Failed(CapabilityFailure {
        error_kind: CapabilityFailureKind::InvalidInput,
        safe_summary: "stored tool result cannot be returned as text".to_string(),
        detail: None,
    })
}

fn storage_unavailable_error(
    error: SessionThreadError,
    operation: &'static str,
) -> AgentLoopHostError {
    tracing::debug!(error = %error, operation, "result reader storage lookup failed");
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "result reader storage is unavailable",
    )
}

struct ResultReadInput {
    result_ref: String,
    offset: u64,
    max_bytes: u64,
}

fn parse_result_read_input(
    value: &serde_json::Value,
) -> Result<ResultReadInput, AgentLoopHostError> {
    let object = value.as_object().ok_or_else(|| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "result_read arguments must be an object",
        )
    })?;
    if object
        .keys()
        .any(|key| key != "result_ref" && key != "offset" && key != "max_bytes")
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "result_read arguments contain an unsupported field",
        ));
    }
    let result_ref = object
        .get("result_ref")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "result_read requires a result_ref string",
            )
        })?
        .to_string();
    ToolResultReferenceEnvelope::validate_result_ref(&result_ref).map_err(|error| {
        tracing::debug!(validation_error = %error, "result reader result reference validation failed");
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "result_read result_ref is invalid",
        )
    })?;
    let offset = object
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "result_read requires a non-negative offset",
            )
        })?;
    let max_bytes = object
        .get("max_bytes")
        .and_then(serde_json::Value::as_u64)
        .filter(|value| (RESULT_READ_MIN_BYTES..=RESULT_READ_MAX_BYTES).contains(value))
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "result_read max_bytes is outside the allowed range",
            )
        })?;
    Ok(ResultReadInput {
        result_ref,
        offset,
        max_bytes,
    })
}

fn result_read_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["result_ref", "offset", "max_bytes"],
        "properties": {
            "result_ref": {"type": "string", "description": "Opaque result reference from a prior tool result."},
            "offset": {"type": "integer", "minimum": 0},
            "max_bytes": {"type": "integer", "minimum": RESULT_READ_MIN_BYTES, "maximum": RESULT_READ_MAX_BYTES}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_failures_remain_terminal_and_model_safe() {
        let error = storage_unavailable_error(
            SessionThreadError::Backend("result reader storage test failure".to_string()),
            "record lookup",
        );

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(error.safe_summary, "result reader storage is unavailable");
        assert!(error.detail.is_none());
    }
}
