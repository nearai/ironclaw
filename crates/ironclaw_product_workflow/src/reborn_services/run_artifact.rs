//! Caller-owned, redacted evidence bundle for one completed or in-flight run.
//!
//! This is deliberately a neutral run artifact rather than a QA-specific
//! projection. QA fixture tooling is one consumer; the ownership, exact-run
//! selection, replay metadata reconstruction, and redaction rules live here so
//! other callers cannot grow parallel definitions of a trajectory.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use ironclaw_host_api::ThreadId;
use ironclaw_reborn_traces::contribution::DeterministicTraceRedactor;
use ironclaw_reborn_traces::redaction::redact_sensitive_json;
use ironclaw_threads::{
    ContextMessage, LoadContextMessagesRequest, MessageKind, MessageStatus, ThreadMessageId,
    ThreadMessageRecord, ThreadScope,
};
use serde::{Deserialize, Serialize};

use super::{
    OPERATOR_LOGS_MAX_LIMIT, RebornGetRunStateRequest, RebornGetRunStateResponse, RebornLogEntry,
    RebornLogQueryRequest, RebornServices, RebornServicesApi, RebornServicesError,
    RebornServicesErrorCode, RebornViewDescriptor, WebUiAuthenticatedCaller, bounded_log_query,
    map_thread_error, parse_run_id_field, parse_thread_id_field,
};

pub const RUN_ARTIFACT_SCHEMA: &str = "ironclaw.run_artifact.v1";
pub const RUN_ARTIFACT_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "run_artifact",
    paginated: false,
};
pub(super) const ARTIFACT_REDACTION_PIPELINE: &str = "deterministic-trace-redactor-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornRunArtifactRequest {
    pub thread_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebornRunArtifact {
    pub schema: String,
    pub generated_at: DateTime<Utc>,
    pub thread_id: String,
    pub run: RebornGetRunStateResponse,
    pub messages: Vec<RunArtifactMessage>,
    pub logs: RunArtifactLogs,
    pub redaction: RunArtifactRedaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactMessage {
    pub message_id: String,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub kind: MessageKind,
    pub status: MessageStatus,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<RunArtifactToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactToolCall {
    pub provider_id: String,
    pub provider_model_id: String,
    pub provider_turn_id: String,
    pub provider_call_id: String,
    pub capability_id: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactLogs {
    pub source: String,
    pub available: bool,
    /// Always false for today's bounded, process-local buffer. A restart or
    /// eviction can remove earlier entries without leaving a durable marker.
    pub complete: bool,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    pub entries: Vec<RebornLogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactRedaction {
    pub pipeline: String,
    pub applied: bool,
}

impl RebornServices {
    pub(super) async fn build_run_artifact(
        &self,
        caller: WebUiAuthenticatedCaller,
        request: RebornRunArtifactRequest,
    ) -> Result<RebornRunArtifact, RebornServicesError> {
        let thread_id = parse_thread_id_field("thread_id", request.thread_id)?;
        let run_id = parse_run_id_field("run_id", request.run_id)?;
        let run_id_text = run_id.to_string();

        // get_run_state performs the canonical caller-scope ownership probe,
        // including the automation-trigger fallback. The history read repeats
        // that authorization because trigger visibility is intentionally never
        // cached between accesses.
        let run = self
            .get_run_state(
                caller.clone(),
                RebornGetRunStateRequest {
                    thread_id: thread_id.to_string(),
                    run_id: run_id_text.clone(),
                },
            )
            .await?;
        let scope = caller.turn_scope(thread_id.clone());
        let (thread_scope, history) = self
            .resolve_thread_history_for_caller(caller.clone(), &scope)
            .await?;

        let run_records: Vec<ThreadMessageRecord> = history
            .messages
            .into_iter()
            .filter(|message| message.turn_run_id.as_deref() == Some(run_id_text.as_str()))
            .collect();
        if run_records.is_empty() {
            return Err(RebornServicesError::from_status(
                RebornServicesErrorCode::NotFound,
                404,
                false,
            ));
        }

        let redactor = DeterministicTraceRedactor::new(Vec::new());
        let (messages, message_redaction_applied) = self
            .artifact_messages_for_records(thread_scope, &thread_id, run_records, &redactor)
            .await?;
        let (logs, log_redaction_applied) = self
            .artifact_logs(caller, thread_id.to_string(), Some(run_id_text), &redactor)
            .await;

        Ok(RebornRunArtifact {
            schema: RUN_ARTIFACT_SCHEMA.to_string(),
            generated_at: Utc::now(),
            thread_id: thread_id.to_string(),
            run,
            messages,
            logs,
            redaction: RunArtifactRedaction {
                pipeline: ARTIFACT_REDACTION_PIPELINE.to_string(),
                applied: message_redaction_applied || log_redaction_applied,
            },
        })
    }

    pub(super) async fn artifact_logs(
        &self,
        caller: WebUiAuthenticatedCaller,
        thread_id: String,
        run_id: Option<String>,
        redactor: &DeterministicTraceRedactor,
    ) -> (RunArtifactLogs, bool) {
        let query = bounded_log_query(RebornLogQueryRequest {
            limit: Some(OPERATOR_LOGS_MAX_LIMIT),
            thread_id: Some(thread_id),
            run_id,
            ..RebornLogQueryRequest::default()
        });
        match self.operator_logs.query_logs(caller, query).await {
            Ok(response) => {
                let truncated = response.next_cursor.is_some();
                let mut redaction_applied = false;
                let entries = response
                    .entries
                    .into_iter()
                    .map(|mut entry| {
                        let (message, changed) = redact_text(redactor, &entry.message);
                        entry.message = message;
                        redaction_applied |= changed;
                        entry
                    })
                    .collect();
                (
                    RunArtifactLogs {
                        source: response.source,
                        available: true,
                        complete: false,
                        truncated,
                        unavailable_reason: None,
                        entries,
                    },
                    redaction_applied,
                )
            }
            Err(error) => {
                tracing::debug!(
                    error_code = ?error.code,
                    "run artifact exported without optional process-local logs"
                );
                (
                    RunArtifactLogs {
                        source: "operator_buffer".to_string(),
                        available: false,
                        complete: false,
                        truncated: false,
                        unavailable_reason: Some("operator_log_buffer_unavailable".to_string()),
                        entries: Vec::new(),
                    },
                    false,
                )
            }
        }
    }

    pub(super) async fn artifact_messages_for_records(
        &self,
        thread_scope: ThreadScope,
        thread_id: &ThreadId,
        records: Vec<ThreadMessageRecord>,
        redactor: &DeterministicTraceRedactor,
    ) -> Result<(Vec<RunArtifactMessage>, bool), RebornServicesError> {
        // Product history intentionally strips provider replay metadata. Load
        // exactly the selected ids through the model-context projection, then
        // merge by stable message id so tool calls remain paired with results.
        let context = self
            .thread_service
            .load_context_messages(LoadContextMessagesRequest {
                scope: thread_scope,
                thread_id: thread_id.clone(),
                message_ids: records.iter().map(|message| message.message_id).collect(),
            })
            .await
            .map_err(map_thread_error)?;
        let context_by_id: HashMap<ThreadMessageId, ContextMessage> = context
            .messages
            .into_iter()
            .filter_map(|message| message.message_id.map(|id| (id, message)))
            .collect();
        Ok(artifact_messages(records, &context_by_id, redactor))
    }
}

pub(super) fn artifact_messages(
    records: Vec<ThreadMessageRecord>,
    context_by_id: &HashMap<ThreadMessageId, ContextMessage>,
    redactor: &DeterministicTraceRedactor,
) -> (Vec<RunArtifactMessage>, bool) {
    let mut redaction_applied = false;
    let messages = records
        .into_iter()
        .filter_map(|record| {
            if !matches!(
                record.kind,
                MessageKind::User | MessageKind::Assistant | MessageKind::ToolResultReference
            ) {
                return None;
            }
            if !matches!(
                record.status,
                MessageStatus::Accepted
                    | MessageStatus::Submitted
                    | MessageStatus::Finalized
                    | MessageStatus::Interrupted
            ) {
                return None;
            }
            let context = context_by_id.get(&record.message_id);
            let raw_content = context
                .map(|message| message.content.as_str())
                .or(record.content.as_deref())?;
            let (content, content_changed) = redact_text(redactor, raw_content);
            redaction_applied |= content_changed;

            let provider_call = context
                .and_then(|message| message.tool_result_provider_call.as_ref())
                .or(record.tool_result_provider_call.as_ref());
            let tool_call = provider_call.map(|call| {
                let (arguments, arguments_changed) = redact_json(redactor, &call.arguments);
                redaction_applied |= arguments_changed;
                RunArtifactToolCall {
                    provider_id: call.provider_id.clone(),
                    provider_model_id: call.provider_model_id.clone(),
                    provider_turn_id: call.provider_turn_id.clone(),
                    provider_call_id: call.provider_call_id.clone(),
                    capability_id: call.capability_id.as_str().to_string(),
                    arguments,
                }
            });

            Some(RunArtifactMessage {
                message_id: record.message_id.to_string(),
                sequence: record.sequence,
                run_id: record.turn_run_id.clone(),
                kind: record.kind,
                status: record.status,
                content,
                tool_call,
            })
        })
        .collect();
    (messages, redaction_applied)
}

fn redact_text(redactor: &DeterministicTraceRedactor, input: &str) -> (String, bool) {
    let (redacted, _) = redactor.redact_text(input);
    let changed = redacted != input;
    (redacted, changed)
}

fn redact_json(
    redactor: &DeterministicTraceRedactor,
    input: &serde_json::Value,
) -> (serde_json::Value, bool) {
    let redacted = redact_json_strings(redactor, redact_sensitive_json(input));
    let changed = redacted != *input;
    (redacted, changed)
}

fn redact_json_strings(
    redactor: &DeterministicTraceRedactor,
    input: serde_json::Value,
) -> serde_json::Value {
    match input {
        serde_json::Value::String(value) => {
            serde_json::Value::String(redactor.redact_text(&value).0)
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(|value| redact_json_strings(redactor, value))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, redact_json_strings(redactor, value)))
                .collect(),
        ),
        value => value,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{CapabilityId, ProviderToolName, ThreadId};
    use ironclaw_threads::ProviderToolCallReferenceEnvelope;
    use serde_json::json;

    use super::*;

    #[test]
    fn assembler_keeps_only_replayable_run_records_and_redacts_payloads() {
        let thread_id = ThreadId::new("thread-a").expect("thread id");
        let tool_message_id = ThreadMessageId::new();
        let records = vec![
            record(
                &thread_id,
                ThreadMessageId::new(),
                1,
                MessageKind::User,
                "email me at person@example.com",
            ),
            record(
                &thread_id,
                tool_message_id,
                2,
                MessageKind::ToolResultReference,
                "used /Users/alice/private.txt",
            ),
            record(
                &thread_id,
                ThreadMessageId::new(),
                3,
                MessageKind::CapabilityDisplayPreview,
                "display-only",
            ),
        ];
        let call = ProviderToolCallReferenceEnvelope {
            provider_id: "anthropic".to_string(),
            provider_model_id: "model".to_string(),
            provider_turn_id: "turn-1".to_string(),
            provider_call_id: "call-1".to_string(),
            provider_tool_name: ProviderToolName::new("builtin__read_file").expect("tool name"),
            capability_id: CapabilityId::new("builtin.read_file").expect("capability"),
            arguments: json!({"path": "/Users/alice/private.txt", "api_key": "secret-value"}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        };
        let context_by_id = HashMap::from([(
            tool_message_id,
            ContextMessage {
                message_id: Some(tool_message_id),
                summary_id: None,
                sequence: 2,
                kind: MessageKind::ToolResultReference,
                tool_result_provider_call: Some(call),
                content: "used /Users/alice/private.txt".to_string(),
                image_attachments: Vec::new(),
            },
        )]);

        let (messages, applied) = artifact_messages(
            records,
            &context_by_id,
            &DeterministicTraceRedactor::new(Vec::new()),
        );

        assert!(applied);
        assert_eq!(messages.len(), 2);
        let serialized = serde_json::to_string(&messages).expect("serialize messages");
        assert!(!serialized.contains("person@example.com"));
        assert!(!serialized.contains("/Users/alice"));
        assert!(!serialized.contains("secret-value"));
        assert!(serialized.contains("[REDACTED]"));
    }

    fn record(
        thread_id: &ThreadId,
        message_id: ThreadMessageId,
        sequence: u64,
        kind: MessageKind,
        content: &str,
    ) -> ThreadMessageRecord {
        ThreadMessageRecord {
            message_id,
            thread_id: thread_id.clone(),
            sequence,
            kind,
            status: MessageStatus::Finalized,
            created_at: Some(Utc::now()),
            updated_at: Some(Utc::now()),
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: Some("turn-a".to_string()),
            turn_run_id: Some("run-a".to_string()),
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(content.to_string()),
            attachments: Vec::new(),
            redaction_ref: None,
        }
    }
}
