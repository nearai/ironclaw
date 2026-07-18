use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{DispatchInputIssueCode, InvocationId, UserId};
use ironclaw_loop_host::{CapabilityResultWrite, DurablePersistence};
use ironclaw_threads::{
    MessageKind, MessageStatus, ReadToolResultRecordRequest, SessionThreadError,
    SessionThreadService, TOOL_RESULT_RECORD_READ_MAX_BYTES, ThreadHistoryRequest,
    ToolResultReferenceEnvelope,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailure, CapabilityFailureDetail,
    CapabilityFailureKind, CapabilityInputIssue, CapabilityOutcome, CapabilityProgress,
    CapabilityResultMessage, ConcurrencyHint, MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
    ModelVisibleArtifact, ModelVisibleToolObservation, ObservationTrust, ToolObservationDetail,
    ToolObservationStatus, sanitize_model_visible_text,
};

use super::{
    local_dev_thread_scope_for_run,
    synthetic_capability::{
        SyntheticCapability, SyntheticCapabilityDescriptor, SyntheticCapabilityHandler,
        SyntheticCapabilityInvocation,
    },
};

/// Test-support wrap: layers the synthetic `result_read` capability onto
/// `inner`, mirroring how `refreshing_capability_port.rs`'s `build_inner`
/// wires it in production (unconditionally, via `wrap_synthetic_capabilities`).
/// `input_resolver`/`result_writer` MUST be the SAME shared io object the
/// harness's capability port already uses -- see
/// `RefreshingCapabilityPortTestParts::input_resolver` in
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
    super::synthetic_capability::wrap_synthetic_capabilities(
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
) -> Result<SyntheticCapability, AgentLoopHostError> {
    Ok(SyntheticCapability::new(
        SyntheticCapabilityDescriptor::new(
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
impl SyntheticCapabilityHandler for ResultReadHandler {
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
        invocation: SyntheticCapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let input = match parse_result_read_input(&invocation.input) {
            Ok(input) => input,
            Err(failure) => return Ok(CapabilityOutcome::Failed(failure)),
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
            // `content` here is always a paged text chunk, never array-shaped.
            item_count: None,
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

/// Builds the `InvalidInput` `CapabilityFailure` every
/// `parse_result_read_input` error arm returns, carrying one structured
/// repair issue.
fn invalid_input_failure(safe_summary: &str, issue: CapabilityInputIssue) -> CapabilityFailure {
    CapabilityFailure {
        error_kind: CapabilityFailureKind::InvalidInput,
        safe_summary: safe_summary.to_string(),
        detail: Some(CapabilityFailureDetail::InvalidInput {
            issues: vec![issue],
        }),
    }
}

/// JSON type name for a `CapabilityInputIssue::received` value, distinct from
/// `serde_json::Value`'s numeric `Display` used for out-of-range values.
fn json_value_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Model-controlled text echoed into a `CapabilityInputIssue` must be
/// secret-redacted first, or the persistence-side content scan drops the
/// whole observation for exactly the inputs that need repair guidance most.
fn sanitized_issue_text(value: impl Into<String>) -> String {
    sanitize_model_visible_text(value)
}

/// A model-authored field name may only reach the model-visible issue `path`
/// when identifier-shaped (1..=64 chars of `[A-Za-z0-9_.-]`); anything else
/// gets a fixed placeholder so instruction-shaped names cannot be echoed.
fn safe_issue_path(key: &str) -> String {
    let identifier_shaped = (1..=64).contains(&key.len())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if identifier_shaped {
        sanitized_issue_text(key)
    } else {
        "unexpected_field".to_string()
    }
}

fn parse_result_read_input(
    value: &serde_json::Value,
) -> Result<ResultReadInput, CapabilityFailure> {
    let object = value.as_object().ok_or_else(|| {
        invalid_input_failure(
            "result_read arguments must be an object",
            CapabilityInputIssue {
                path: "root".to_string(),
                code: DispatchInputIssueCode::TypeMismatch,
                expected: Some("object".to_string()),
                received: Some(json_value_kind(value).to_string()),
                schema_path: Some("root".to_string()),
            },
        )
    })?;
    if let Some(unexpected) = object
        .keys()
        .find(|key| *key != "result_ref" && *key != "offset" && *key != "max_bytes")
    {
        return Err(invalid_input_failure(
            "result_read arguments contain an unsupported field",
            CapabilityInputIssue {
                path: safe_issue_path(unexpected),
                code: DispatchInputIssueCode::UnexpectedField,
                expected: Some("declared field".to_string()),
                received: Some("unexpected field".to_string()),
                schema_path: Some("additionalProperties".to_string()),
            },
        ));
    }
    let result_ref_value = object.get("result_ref");
    let result_ref = match result_ref_value.and_then(serde_json::Value::as_str) {
        Some(value) => value.to_string(),
        None => {
            let (code, expected, received) = match result_ref_value {
                None => (
                    DispatchInputIssueCode::MissingRequired,
                    Some("required field".to_string()),
                    None,
                ),
                Some(other) => (
                    DispatchInputIssueCode::TypeMismatch,
                    Some("string".to_string()),
                    Some(json_value_kind(other).to_string()),
                ),
            };
            return Err(invalid_input_failure(
                "result_read requires a result_ref string",
                CapabilityInputIssue {
                    path: "result_ref".to_string(),
                    code,
                    expected,
                    received,
                    schema_path: Some("properties/result_ref".to_string()),
                },
            ));
        }
    };
    ToolResultReferenceEnvelope::validate_result_ref(&result_ref).map_err(|error| {
        tracing::debug!(validation_error = %error, "result reader result reference validation failed");
        invalid_input_failure(
            "result_read result_ref is invalid",
            CapabilityInputIssue {
                path: "result_ref".to_string(),
                code: DispatchInputIssueCode::InvalidValue,
                expected: Some("valid result reference format".to_string()),
                received: Some(sanitized_issue_text(result_ref.clone())),
                schema_path: Some("properties/result_ref".to_string()),
            },
        )
    })?;
    let offset_value = object.get("offset");
    let offset = match offset_value.and_then(serde_json::Value::as_u64) {
        Some(value) => value,
        None => {
            let (code, expected, received) = match offset_value {
                None => (
                    DispatchInputIssueCode::MissingRequired,
                    Some("required field".to_string()),
                    None,
                ),
                // A number that isn't a u64 (negative, float) is an
                // InvalidValue; any other JSON type is a TypeMismatch echoing
                // only the type name (mirrors the result_ref arm).
                Some(other) if other.is_number() => (
                    DispatchInputIssueCode::InvalidValue,
                    Some("non-negative integer".to_string()),
                    Some(sanitized_issue_text(other.to_string())),
                ),
                Some(other) => (
                    DispatchInputIssueCode::TypeMismatch,
                    Some("integer".to_string()),
                    Some(json_value_kind(other).to_string()),
                ),
            };
            return Err(invalid_input_failure(
                "result_read requires a non-negative offset",
                CapabilityInputIssue {
                    path: "offset".to_string(),
                    code,
                    expected,
                    received,
                    schema_path: Some("properties/offset".to_string()),
                },
            ));
        }
    };
    let max_bytes_value = object.get("max_bytes");
    let Some(max_bytes_value) = max_bytes_value else {
        return Err(invalid_input_failure(
            "result_read requires a max_bytes integer",
            CapabilityInputIssue {
                path: "max_bytes".to_string(),
                code: DispatchInputIssueCode::MissingRequired,
                expected: Some("required field".to_string()),
                received: None,
                schema_path: Some("properties/max_bytes".to_string()),
            },
        ));
    };
    if !max_bytes_value.is_number() {
        return Err(invalid_input_failure(
            "result_read requires a max_bytes integer",
            CapabilityInputIssue {
                path: "max_bytes".to_string(),
                code: DispatchInputIssueCode::TypeMismatch,
                expected: Some("integer".to_string()),
                received: Some(json_value_kind(max_bytes_value).to_string()),
                schema_path: Some("properties/max_bytes".to_string()),
            },
        ));
    }
    let max_bytes = match max_bytes_value
        .as_u64()
        .filter(|value| (RESULT_READ_MIN_BYTES..=RESULT_READ_MAX_BYTES).contains(value))
    {
        Some(value) => value,
        None => {
            return Err(invalid_input_failure(
                "result_read max_bytes is outside the allowed range",
                CapabilityInputIssue {
                    path: "max_bytes".to_string(),
                    code: DispatchInputIssueCode::InvalidValue,
                    expected: Some(format!("{RESULT_READ_MIN_BYTES}..={RESULT_READ_MAX_BYTES}")),
                    received: Some(sanitized_issue_text(max_bytes_value.to_string())),
                    schema_path: Some("properties/max_bytes".to_string()),
                },
            ));
        }
    };
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
