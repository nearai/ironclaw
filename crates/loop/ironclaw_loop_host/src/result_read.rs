//! Loop-facing capability result reader.

use std::sync::Arc;

use crate::{
    CapabilityResultWrite, DurablePersistence, SyntheticCapability, SyntheticCapabilityDescriptor,
    SyntheticCapabilityHandler, SyntheticCapabilityInvocation,
};
use async_trait::async_trait;
use ironclaw_host_api::{
    dispatch::DispatchInputIssueCode,
    ids::{InvocationId, UserId},
    model_result_preview::{MODEL_RESULT_PREVIEW_MAX_BYTES, ModelResultJsonPage},
    resolution::Resolution,
    result_meta::FailureKind,
    turn::LoopResultRef,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilityFailureDetail, CapabilityInputIssue,
    CapabilityProgress, MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleArtifact,
    ModelVisibleToolObservation, ObservationTrust, ToolObservationDetail, ToolObservationStatus,
    resolution, sanitize_model_visible_text,
};
use ironclaw_threads::{
    MessageKind, MessageStatus, ReadToolResultRecordRequest, SessionThreadError,
    SessionThreadService, TOOL_RESULT_JSON_MAX_LIMIT, ThreadHistoryRequest, ThreadScope,
    ToolResultRecordRead, ToolResultRecordReadError, ToolResultRecordSelection,
    ToolResultReferenceEnvelope, effective_tool_result_read_max_bytes,
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
pub fn wrap_result_read_capability_for_test(
    inner: Arc<dyn ironclaw_loop_contracts::LoopCapabilityPort>,
    thread_service: Arc<dyn SessionThreadService>,
    fallback_user_id: UserId,
    run_context: ironclaw_loop_contracts::LoopRunContext,
    input_resolver: Arc<dyn crate::LoopCapabilityInputResolver>,
    result_writer: Arc<dyn crate::LoopCapabilityResultWriter>,
) -> Result<Arc<dyn ironclaw_loop_contracts::LoopCapabilityPort>, AgentLoopHostError> {
    crate::wrap_synthetic_capabilities(
        inner,
        vec![result_read_capability(
            thread_service,
            fallback_user_id.clone(),
        )?],
        run_context,
        fallback_user_id,
        input_resolver,
        result_writer,
        // trajectory_observer: None — not wired in the integration-test harness.
        None,
        // `result_read` never raises an approval gate, so its resume path never
        // loads a replay payload; an in-memory store keeps the seam wired.
        Arc::new(ironclaw_capabilities::ReplayPayloadStore::new(
            replay_payload_filesystem()?,
        )),
    )
}

/// Test-support export of the capability id, so integration tests can script
/// a `result_read` tool call without hand-copying the literal.
#[cfg(feature = "test-support")]
pub const RESULT_READ_CAPABILITY_ID_FOR_TEST: &str = RESULT_READ_CAPABILITY_ID;

pub const RESULT_READ_CAPABILITY_ID: &str = "builtin.result_read";
const RESULT_READ_PROVIDER_TOOL_NAME: &str = "builtin__result_read";
const RESULT_READ_MIN_BYTES: u64 = 4;
const RESULT_READ_JSON_POINTER_MAX_BYTES: usize = 4096;
/// The largest `max_bytes` a caller may request, resolved per request.
///
/// NOT the compile-time `TOOL_RESULT_RECORD_READ_MAX_BYTES`. This gate sits UPSTREAM of
/// `validate_tool_result_record_read`, so pinning it to the constant made
/// `IRONCLAW_TOOL_RESULT_READ_MAX_BYTES` inert: the downstream validator was widened while this
/// one still rejected anything over 24 KiB and the advertised schema still said `maximum: 24576`,
/// so no larger read was ever issued and the knob changed nothing.
fn result_read_max_bytes() -> u64 {
    effective_tool_result_read_max_bytes() as u64
}

fn thread_scope_for_run(
    run_context: &ironclaw_loop_contracts::LoopRunContext,
    fallback_user_id: &UserId,
) -> Option<ThreadScope> {
    let resource = run_context.scope.to_resource_scope();
    let base = ThreadScope {
        tenant_id: resource.tenant_id,
        agent_id: resource.agent_id?,
        project_id: resource.project_id,
        owner_user_id: Some(fallback_user_id.clone()),
        mission_id: resource.mission_id,
    };
    Some(crate::ThreadScopeResolver::resolve_for_turn(
        &base,
        &run_context.scope,
        run_context.actor(),
    ))
}

#[cfg(feature = "test-support")]
fn replay_payload_filesystem() -> Result<
    Arc<ironclaw_filesystem::ScopedFilesystem<ironclaw_filesystem::InMemoryBackend>>,
    AgentLoopHostError,
> {
    use ironclaw_host_api::{
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };

    let invalid_mount = || {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "test replay payload filesystem configuration is invalid",
        )
    };
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/replay-payloads").map_err(|_| invalid_mount())?,
        VirtualPath::new("/tenants/test/users/test/replay-payloads")
            .map_err(|_| invalid_mount())?,
        MountPermissions::read_write_list_delete(),
    )])
    .map_err(|_| invalid_mount())?;
    Ok(Arc::new(
        ironclaw_filesystem::ScopedFilesystem::with_fixed_view(
            Arc::new(ironclaw_filesystem::InMemoryBackend::new()),
            mounts,
        ),
    ))
}

pub fn result_read_capability(
    thread_service: Arc<dyn SessionThreadService>,
    fallback_user_id: UserId,
) -> Result<SyntheticCapability, AgentLoopHostError> {
    Ok(SyntheticCapability::new(
        SyntheticCapabilityDescriptor::new(
            RESULT_READ_CAPABILITY_ID,
            RESULT_READ_PROVIDER_TOOL_NAME,
            "Read a bounded continuation of a prior tool result, or select a JSON node with an RFC 6901 json_pointer.",
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
    ) -> Result<Resolution, AgentLoopHostError> {
        let input = match parse_result_read_input(&invocation.input) {
            Ok(input) => input,
            Err(resolution) => return Ok(*resolution),
        };
        let scope = thread_scope_for_run(&invocation.run_context, &self.fallback_user_id)
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

        let selection = match &input.json_pointer {
            Some(pointer) => ToolResultRecordSelection::Json {
                pointer: pointer.clone(),
                limit: input.limit.map(|limit| limit as usize),
            },
            None => ToolResultRecordSelection::Bytes,
        };
        let read = match self
            .thread_service
            .read_tool_result_record(ReadToolResultRecordRequest {
                scope: scope.clone(),
                thread_id: invocation.run_context.thread_id.clone(),
                result_ref: input.result_ref.clone(),
                offset: input.offset,
                max_bytes: input.max_bytes as usize,
                selection: selection.clone(),
            })
            .await
        {
            Ok(Some(chunk)) => chunk,
            Ok(None) | Err(SessionThreadError::UnknownThread { .. }) => {
                return Ok(unavailable_result_reference());
            }
            Err(SessionThreadError::ToolResultRecordRead(error)) => {
                return Ok(tool_result_read_failure(error));
            }
            Err(error) => {
                return Err(storage_unavailable_error(error, "record lookup"));
            }
        };
        let (output, preview, structured_json_view, total_bytes, next_offset) = match read {
            ToolResultRecordRead::Bytes(chunk) => {
                let content = match String::from_utf8(chunk.content) {
                    Ok(content) => content,
                    Err(_) => return Ok(non_text_result_content()),
                };
                let output = serde_json::json!({
                    "result_ref": input.result_ref.clone(),
                    "offset": input.offset,
                    "content": content,
                    "total_bytes": chunk.total_bytes,
                    "next_offset": chunk.next_offset,
                });
                (
                    output,
                    sanitize_model_visible_text(content),
                    false,
                    Some(chunk.total_bytes),
                    chunk.next_offset,
                )
            }
            ToolResultRecordRead::Json(page) => {
                let original_total_bytes = page.total_bytes;
                let (output, preview) = match model_visible_json_page(
                    page,
                    input.max_bytes as usize,
                ) {
                    Ok(visible) => visible,
                    Err(ModelVisibleJsonPageError::TooLarge { cause }) => {
                        // Credential placeholders can be longer than the values
                        // they replace. Retry once with fixed headroom rather
                        // than letting post-read redaction terminalize the run.
                        tracing::debug!(%cause, "JSON result page exceeds visible budget; retrying with headroom");
                        let retry_max_bytes =
                            (input.max_bytes / 3).max(RESULT_READ_MIN_BYTES) as usize;
                        let page = match self
                            .thread_service
                            .read_tool_result_record(ReadToolResultRecordRequest {
                                scope,
                                thread_id: invocation.run_context.thread_id.clone(),
                                result_ref: input.result_ref.clone(),
                                offset: input.offset,
                                max_bytes: retry_max_bytes,
                                selection,
                            })
                            .await
                        {
                            Ok(Some(ToolResultRecordRead::Json(page))) => page,
                            Ok(Some(ToolResultRecordRead::Bytes(_))) => {
                                return Err(AgentLoopHostError::new(
                                    AgentLoopHostErrorKind::Internal,
                                    "JSON result retry returned a byte page",
                                ));
                            }
                            Ok(None) | Err(SessionThreadError::UnknownThread { .. }) => {
                                return Ok(unavailable_result_reference());
                            }
                            Err(SessionThreadError::ToolResultRecordRead(error)) => {
                                return Ok(tool_result_read_failure(error));
                            }
                            Err(error) => {
                                return Err(storage_unavailable_error(error, "record retry"));
                            }
                        };
                        match model_visible_json_page(page, retry_max_bytes) {
                            Ok(visible) => visible,
                            Err(error) => {
                                tracing::debug!(
                                    ?error,
                                    "JSON result page remains unavailable after bounded retry"
                                );
                                return Ok(json_page_visibility_failure());
                            }
                        }
                    }
                    Err(ModelVisibleJsonPageError::Invalid { cause }) => {
                        tracing::debug!(%cause, "JSON result page is invalid after redaction");
                        return Ok(json_page_visibility_failure());
                    }
                };
                // Structured pages carry selection continuation and durable
                // byte totals inside the page itself. Do not project their
                // item/string offsets into the legacy outer byte fields.
                (output, preview, true, Some(original_total_bytes), None)
            }
        };
        // `parse_result_read_input` already validated this value against the
        // durable result-reference grammar. Preserve that pageable identity as
        // the completed outcome's origin so the transcript and replay surface
        // exactly the ref the next `result_read` call can use.
        let continuation_result_ref =
            LoopResultRef::new(input.result_ref.clone()).map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "validated result reference could not be represented",
                )
                .with_detail(format!("loop result reference validation failed: {error}"))
            })?;
        // `InlineOnly` (see `DurablePersistence` doc comment): this chunk is
        // already fully delivered to the model inline via
        // `result_read_observation`'s `preview`. The ORIGINAL result this
        // chunk was paged from stays durable and untouched. The writer still
        // mints an internal staging/display ref for byte accounting and output
        // evidence, but that inline-only ref is not continuation authority.
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
            preview,
            structured_json_view,
        ));
        Ok(resolution::completed(
            continuation_result_ref,
            "result chunk returned".to_string(),
            CapabilityProgress::MadeProgress,
            false,
            write.byte_len,
            write.output_digest,
            write.model_observation,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelVisibleJsonPageError {
    TooLarge { cause: String },
    Invalid { cause: String },
}

fn model_visible_json_page(
    page: ModelResultJsonPage,
    max_bytes: usize,
) -> Result<(serde_json::Value, String), ModelVisibleJsonPageError> {
    let encoded = serde_json::to_string(&page).map_err(|error| {
        let cause = sanitized_issue_text(error.to_string());
        tracing::debug!(%cause, "JSON result page serialization failed");
        ModelVisibleJsonPageError::Invalid { cause }
    })?;
    if encoded.len() > max_bytes || encoded.len() > MODEL_RESULT_PREVIEW_MAX_BYTES {
        return Err(ModelVisibleJsonPageError::TooLarge {
            cause: format!(
                "serialized JSON result page is {} bytes, over the {}-byte preview budget",
                encoded.len(),
                max_bytes.min(MODEL_RESULT_PREVIEW_MAX_BYTES)
            ),
        });
    }
    let preview = ironclaw_threads::model_result_preview_from_json_page(&page)
        // A threads-rendered page is structurally valid. Redaction is the only
        // step that can grow it after the renderer applied `max_bytes`, so a
        // refusal here is retried with a smaller source page before failing.
        .map_err(|error| {
            let cause = sanitized_issue_text(error);
            tracing::debug!(%cause, "JSON result page redaction failed");
            ModelVisibleJsonPageError::TooLarge { cause }
        })?
        .into_inner();
    if preview.len() > max_bytes || preview.len() > MODEL_RESULT_PREVIEW_MAX_BYTES {
        return Err(ModelVisibleJsonPageError::TooLarge {
            cause: format!(
                "redacted JSON result page is {} bytes, over the {}-byte preview budget",
                preview.len(),
                max_bytes.min(MODEL_RESULT_PREVIEW_MAX_BYTES)
            ),
        });
    }
    let output = serde_json::from_str(&preview).map_err(|error| {
        let cause = sanitized_issue_text(error.to_string());
        tracing::debug!(%cause, "redacted JSON result page could not be decoded");
        ModelVisibleJsonPageError::Invalid { cause }
    })?;
    Ok((output, preview))
}

fn json_page_visibility_failure() -> Resolution {
    diagnostic_failure(
        FailureKind::OutputDecode,
        "JSON result page cannot be made model-visible within the preview budget".to_string(),
    )
}

fn result_read_observation(
    result_ref: &str,
    byte_len: u64,
    total_bytes: Option<u64>,
    next_offset: Option<u64>,
    content: String,
    structured_json_view: bool,
) -> ModelVisibleToolObservation {
    ModelVisibleToolObservation {
        schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        summary: "Requested tool-result chunk returned.".to_string(),
        detail: ToolObservationDetail::ResultReference {
            result_ref: result_ref.to_string(),
            byte_len,
            preview: Some(content),
            structured_json_view,
            total_bytes,
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

/// The named result ref does not exist in this thread — a domain failure of
/// the read operation, not an encoding fault (`InputEncode`) and not a
/// retryable host outage (`Unavailable` would quietly retry a ref that can
/// never appear). Model-visible and non-retryable.
fn unavailable_result_reference() -> Resolution {
    diagnostic_failure(
        FailureKind::OperationFailed,
        "result reference is unavailable in this thread".to_string(),
    )
}

/// The stored result exists but cannot be decoded as text — an output-decode
/// failure, not an input-encoding fault.
fn non_text_result_content() -> Resolution {
    diagnostic_failure(
        FailureKind::OutputDecode,
        "stored tool result cannot be returned as text".to_string(),
    )
}

fn tool_result_read_failure(error: ToolResultRecordReadError) -> Resolution {
    let cause = sanitized_issue_text(error.to_string());
    tracing::debug!(%cause, "typed result-read failure is model-visible");
    let (summary, path, expected) = match error {
        ToolResultRecordReadError::MalformedStoredJson { .. } => {
            return diagnostic_failure(
                FailureKind::OutputDecode,
                "stored tool result is not valid JSON".to_string(),
            );
        }
        ToolResultRecordReadError::StoredResultTooLarge => {
            return diagnostic_failure(
                FailureKind::OutputTooLarge,
                "stored tool result exceeds the durable storage limit".to_string(),
            );
        }
        ToolResultRecordReadError::InvalidJsonPointer { .. } => (
            "result_read json_pointer is invalid",
            "json_pointer",
            "RFC 6901 JSON Pointer",
        ),
        ToolResultRecordReadError::JsonPointerNotFound { .. } => (
            "result_read json_pointer does not select a value",
            "json_pointer",
            "pointer to an existing JSON value",
        ),
        ToolResultRecordReadError::InvalidJsonOffset { .. } => (
            "result_read offset is outside the selected JSON value",
            "offset",
            "offset within the selected JSON value",
        ),
        ToolResultRecordReadError::InvalidJsonLimit { .. } => (
            "result_read limit is outside the allowed range",
            "limit",
            "collection limit in the advertised range",
        ),
        ToolResultRecordReadError::JsonLimitRequiresCollection => (
            "result_read limit requires an object or array selection",
            "limit",
            "omit limit when selecting a string or scalar",
        ),
        ToolResultRecordReadError::InvalidJsonBudget { .. }
        | ToolResultRecordReadError::JsonViewBudgetTooSmall { .. } => (
            "result_read JSON page does not fit max_bytes",
            "max_bytes",
            "larger JSON page budget within the advertised range",
        ),
    };
    *invalid_input_failure(
        summary,
        CapabilityInputIssue {
            path: path.to_string(),
            code: DispatchInputIssueCode::InvalidValue,
            expected: Some(expected.to_string()),
            received: None,
            schema_path: Some(format!("properties/{path}")),
        },
    )
}

fn diagnostic_failure(error_kind: FailureKind, safe_summary: String) -> Resolution {
    resolution::failed(
        error_kind,
        safe_summary.clone(),
        CapabilityFailureDetail::Diagnostic { text: safe_summary },
    )
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
    json_pointer: Option<String>,
    limit: Option<u64>,
}

/// Builds the `InvalidInput` recoverable-failure `Resolution` every
/// `parse_result_read_input` error arm returns, carrying one structured
/// repair issue. Boxed because a `Resolution` in the `Err` position of the
/// parse result is large (`clippy::result_large_err`).
fn invalid_input_failure(safe_summary: &str, issue: CapabilityInputIssue) -> Box<Resolution> {
    Box::new(resolution::failed(
        FailureKind::InputEncode,
        safe_summary.to_string(),
        CapabilityFailureDetail::InvalidInput {
            issues: vec![issue],
        },
    ))
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

fn parse_result_read_input(value: &serde_json::Value) -> Result<ResultReadInput, Box<Resolution>> {
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
    if let Some(unexpected) = object.keys().find(|key| {
        *key != "result_ref"
            && *key != "offset"
            && *key != "max_bytes"
            && *key != "json_pointer"
            && *key != "limit"
    }) {
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
        .filter(|value| (RESULT_READ_MIN_BYTES..=result_read_max_bytes()).contains(value))
    {
        Some(value) => value,
        None => {
            return Err(invalid_input_failure(
                "result_read max_bytes is outside the allowed range",
                CapabilityInputIssue {
                    path: "max_bytes".to_string(),
                    code: DispatchInputIssueCode::InvalidValue,
                    expected: Some(format!(
                        "{RESULT_READ_MIN_BYTES}..={}",
                        result_read_max_bytes()
                    )),
                    received: Some(sanitized_issue_text(max_bytes_value.to_string())),
                    schema_path: Some("properties/max_bytes".to_string()),
                },
            ));
        }
    };
    let json_pointer = match object.get("json_pointer") {
        None => None,
        Some(serde_json::Value::String(pointer))
            if pointer.len() <= RESULT_READ_JSON_POINTER_MAX_BYTES =>
        {
            Some(pointer.clone())
        }
        Some(serde_json::Value::String(pointer)) => {
            return Err(invalid_input_failure(
                "result_read json_pointer is too long",
                CapabilityInputIssue {
                    path: "json_pointer".to_string(),
                    code: DispatchInputIssueCode::InvalidValue,
                    expected: Some(format!(
                        "at most {RESULT_READ_JSON_POINTER_MAX_BYTES} bytes"
                    )),
                    received: Some(format!("{} bytes", pointer.len())),
                    schema_path: Some("properties/json_pointer/maxLength".to_string()),
                },
            ));
        }
        Some(other) => {
            return Err(invalid_input_failure(
                "result_read json_pointer must be a string",
                CapabilityInputIssue {
                    path: "json_pointer".to_string(),
                    code: DispatchInputIssueCode::TypeMismatch,
                    expected: Some("string".to_string()),
                    received: Some(json_value_kind(other).to_string()),
                    schema_path: Some("properties/json_pointer".to_string()),
                },
            ));
        }
    };
    if json_pointer.is_some() && max_bytes > MODEL_RESULT_PREVIEW_MAX_BYTES as u64 {
        return Err(invalid_input_failure(
            "result_read JSON pages must fit the model preview budget",
            CapabilityInputIssue {
                path: "max_bytes".to_string(),
                code: DispatchInputIssueCode::InvalidValue,
                expected: Some(format!(
                    "{RESULT_READ_MIN_BYTES}..={MODEL_RESULT_PREVIEW_MAX_BYTES} for JSON reads"
                )),
                received: Some(max_bytes.to_string()),
                schema_path: Some("properties/max_bytes".to_string()),
            },
        ));
    }
    // Presence is the primary contract error for byte reads. Check it before
    // parsing the collection range so an invalid value cannot obscure the
    // reason `limit` is unsupported without a JSON selection.
    if let Some(value) = object.get("limit")
        && json_pointer.is_none()
    {
        return Err(invalid_input_failure(
            "result_read limit requires json_pointer",
            CapabilityInputIssue {
                path: "limit".to_string(),
                code: DispatchInputIssueCode::InvalidValue,
                expected: Some("omit limit for byte reads".to_string()),
                received: Some(if value.is_number() {
                    sanitized_issue_text(value.to_string())
                } else {
                    json_value_kind(value).to_string()
                }),
                schema_path: Some("properties/limit".to_string()),
            },
        ));
    }
    let limit = match object.get("limit") {
        None => None,
        Some(value) => match value
            .as_u64()
            .filter(|limit| (1..=TOOL_RESULT_JSON_MAX_LIMIT as u64).contains(limit))
        {
            Some(limit) => Some(limit),
            None => {
                return Err(invalid_input_failure(
                    "result_read limit is outside the allowed range",
                    CapabilityInputIssue {
                        path: "limit".to_string(),
                        code: if value.is_number() {
                            DispatchInputIssueCode::InvalidValue
                        } else {
                            DispatchInputIssueCode::TypeMismatch
                        },
                        expected: Some(format!("1..={TOOL_RESULT_JSON_MAX_LIMIT}")),
                        received: Some(if value.is_number() {
                            sanitized_issue_text(value.to_string())
                        } else {
                            json_value_kind(value).to_string()
                        }),
                        schema_path: Some("properties/limit".to_string()),
                    },
                ));
            }
        },
    };
    Ok(ResultReadInput {
        result_ref,
        offset,
        max_bytes,
        json_pointer,
        limit,
    })
}

fn result_read_input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["result_ref", "offset", "max_bytes"],
        "properties": {
            "result_ref": {"type": "string", "description": "Opaque result reference from a prior tool result."},
            "offset": {"type": "integer", "minimum": 0, "description": "Byte offset for raw reads; immediate child or UTF-8 byte offset for JSON reads."},
            "max_bytes": {"type": "integer", "minimum": RESULT_READ_MIN_BYTES, "maximum": result_read_max_bytes()},
            "json_pointer": {"type": "string", "maxLength": RESULT_READ_JSON_POINTER_MAX_BYTES, "description": "Optional RFC 6901 JSON Pointer. Empty string selects the root. Omit for exact legacy byte reads."},
            "limit": {"type": "integer", "minimum": 1, "maximum": TOOL_RESULT_JSON_MAX_LIMIT, "description": "Maximum immediate object keys or array items in a JSON page. Only valid with json_pointer."}
        },
        "allOf": [{
            "if": {"required": ["json_pointer"]},
            "then": {
                "properties": {
                    "max_bytes": {"maximum": MODEL_RESULT_PREVIEW_MAX_BYTES}
                }
            }
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The caller-facing gate must be the EFFECTIVE cap, not the compile-time constant.
    ///
    /// This is the whole of the inert-knob bug: `IRONCLAW_TOOL_RESULT_READ_MAX_BYTES` widened
    /// `validate_tool_result_record_read`, which sits downstream, while this gate and the advertised
    /// schema stayed pinned to `TOOL_RESULT_RECORD_READ_MAX_BYTES`. A larger read was rejected before
    /// it could reach the widened validator, so the knob changed nothing at all.
    ///
    /// Asserted as a wiring identity rather than by setting the env var: these tests run in-process
    /// and in parallel, so mutating the process environment races every other test reading it, and
    /// the identity is what actually regressed.
    #[test]
    fn the_caller_gate_and_schema_track_the_effective_read_cap() {
        assert_eq!(
            result_read_max_bytes(),
            effective_tool_result_read_max_bytes() as u64,
            "the gate must resolve the same cap the record validator enforces, or the env override \
             is inert"
        );

        let schema = result_read_input_schema();
        let advertised = schema["properties"]["max_bytes"]["maximum"]
            .as_u64()
            .expect("the schema must advertise a max_bytes ceiling");
        assert_eq!(
            advertised,
            result_read_max_bytes(),
            "the model is told what it may ask for; advertising the compile-time constant while the \
             gate allows more (or less) is how the knob went unnoticed"
        );
        assert_eq!(
            schema["allOf"][0]["then"]["properties"]["max_bytes"]["maximum"].as_u64(),
            Some(MODEL_RESULT_PREVIEW_MAX_BYTES as u64),
            "JSON selections must advertise their tighter model-preview ceiling"
        );
    }

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

    #[test]
    fn json_pointer_alone_selects_json_and_defaults_the_collection_limit() {
        let parsed = parse_result_read_input(&serde_json::json!({
            "result_ref": "result:json-selection",
            "offset": 0,
            "max_bytes": MODEL_RESULT_PREVIEW_MAX_BYTES,
            "json_pointer": "/payload/items/2"
        }))
        .expect("valid JSON selection parses");

        assert_eq!(parsed.json_pointer.as_deref(), Some("/payload/items/2"));
        assert_eq!(parsed.limit, None);
    }

    #[test]
    fn json_selection_rejects_pages_larger_than_the_model_preview() {
        let result = parse_result_read_input(&serde_json::json!({
            "result_ref": "result:json-selection",
            "offset": 0,
            "max_bytes": MODEL_RESULT_PREVIEW_MAX_BYTES + 1,
            "json_pointer": ""
        }));

        assert!(result.is_err());
    }

    #[test]
    fn collection_limit_without_json_pointer_is_rejected() {
        let result = parse_result_read_input(&serde_json::json!({
            "result_ref": "result:byte-selection",
            "offset": 0,
            "max_bytes": 32,
            "limit": 10
        }));

        assert!(result.is_err());
    }

    #[test]
    fn pointerless_limit_presence_is_reported_before_range_validation() {
        let Err(result) = parse_result_read_input(&serde_json::json!({
            "result_ref": "result:byte-selection",
            "offset": 0,
            "max_bytes": 32,
            "limit": 0
        })) else {
            panic!("limit must not be valid for byte reads");
        };
        let rendered = serde_json::to_string(result.as_ref()).expect("resolution serializes");

        assert!(rendered.contains("limit requires json_pointer"));
        assert!(!rendered.contains("limit is outside the allowed range"));
    }

    #[test]
    fn typed_domain_read_errors_keep_model_recovery_classes() {
        let malformed = tool_result_read_failure(ToolResultRecordReadError::MalformedStoredJson {
            reason: "test parser detail".to_string(),
        });
        let malformed_json = serde_json::to_string(&malformed).expect("resolution serializes");
        assert!(malformed_json.contains("output_decode"));
        assert!(!malformed_json.contains("test parser detail"));

        let invalid_offset =
            tool_result_read_failure(ToolResultRecordReadError::InvalidJsonOffset { offset: 99 });
        let invalid_offset_json =
            serde_json::to_string(&invalid_offset).expect("resolution serializes");
        assert!(invalid_offset_json.contains("invalid_input"));
        assert!(invalid_offset_json.contains("offset"));
    }
}
