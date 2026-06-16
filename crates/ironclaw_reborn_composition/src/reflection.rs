use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::UserId;
use ironclaw_loop_support::{
    HostManagedModelGateway, HostManagedModelMessage, HostManagedModelMessageRole,
    HostManagedModelRequest,
};
use ironclaw_memory::{
    ChunkingMemoryDocumentIndexer, DocumentMetadata, FilesystemMemoryDocumentRepository,
    MemoryBackend, MemoryBackendCapabilities, MemoryBackendWriteOptions, MemoryContext,
    MemoryDocumentPath, MemoryDocumentScope, RepositoryMemoryBackend,
};
use ironclaw_threads::{
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadMessageRecord,
    ThreadScope,
};
use ironclaw_turns::{
    GetRunStateRequest, LoopMessageRef, TurnError, TurnEventKind, TurnEventSink,
    TurnLifecycleEvent, TurnRunId, TurnStateStore, TurnStatus,
    run_profile::{ModelProfileId, ParentLoopOutput},
};
use serde::Deserialize;
use thiserror::Error;
use tracing::debug;

const REFLECTION_PROMPT: &str = include_str!("assets/prompts/learning_reflection.md");
const REFLECTION_MODEL_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TRANSCRIPT_MESSAGES: usize = 32;
const MAX_TRANSCRIPT_CHARS: usize = 16 * 1024;

#[derive(Clone)]
pub(crate) struct LearningReflectionEventSink {
    service: Arc<LearningReflectionService>,
}

impl LearningReflectionEventSink {
    pub(crate) fn new(service: Arc<LearningReflectionService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl TurnEventSink for LearningReflectionEventSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        if !is_reflection_candidate_event(&event) {
            return Ok(());
        }

        let service = Arc::clone(&self.service);
        tokio::spawn(async move {
            if let Err(error) = service.reflect_committed_event(event.clone()).await {
                debug!(
                    error = %error,
                    run_id = %event.run_id,
                    "learning reflection skipped after committed turn"
                );
            }
        });
        Ok(())
    }
}

pub(crate) struct LearningReflectionService {
    thread_service: Arc<dyn SessionThreadService>,
    turn_state: Arc<dyn TurnStateStore>,
    thread_scope: ThreadScope,
    model_gateway: Arc<dyn HostManagedModelGateway>,
    model_profile_id: ModelProfileId,
    memory_writer: LearningMemoryWriter,
}

impl LearningReflectionService {
    pub(crate) fn new(
        thread_service: Arc<dyn SessionThreadService>,
        turn_state: Arc<dyn TurnStateStore>,
        thread_scope: ThreadScope,
        model_gateway: Arc<dyn HostManagedModelGateway>,
        model_profile_id: ModelProfileId,
        memory_filesystem: Arc<dyn RootFilesystem>,
    ) -> Self {
        Self {
            thread_service,
            turn_state,
            thread_scope,
            model_gateway,
            model_profile_id,
            memory_writer: LearningMemoryWriter::new(memory_filesystem),
        }
    }

    async fn reflect_committed_event(
        &self,
        event: TurnLifecycleEvent,
    ) -> Result<(), LearningReflectionError> {
        let transcript = self.load_transcript(&event).await?;
        let Some(signal) = reflection_signal_for_event(&event, &transcript.latest_user_message)
        else {
            return Ok(());
        };

        let state = self
            .turn_state
            .get_run_state(GetRunStateRequest {
                scope: event.scope.clone(),
                run_id: event.run_id,
            })
            .await
            .map_err(|error| LearningReflectionError::TurnState {
                reason: error.to_string(),
            })?;
        let request = self.model_request(&event, &state.turn_id, signal, &transcript.rendered)?;
        let response = tokio::time::timeout(
            REFLECTION_MODEL_TIMEOUT,
            self.model_gateway.stream_model(request),
        )
        .await
        .map_err(|_| LearningReflectionError::ModelTimeout)?
        .map_err(|error| LearningReflectionError::Model {
            kind: format!("{:?}", error.kind),
        })?;

        let output_text = match response.output {
            ParentLoopOutput::AssistantReply(reply) => reply.content,
            ParentLoopOutput::CapabilityCalls(_) => {
                return Err(LearningReflectionError::InvalidModelOutput {
                    reason: "reflection returned capability calls".to_string(),
                });
            }
        };
        let Some(learning) = parse_reflection_learning(&output_text)? else {
            return Ok(());
        };
        let owner_user_id = event
            .owner_user_id
            .as_ref()
            .ok_or(LearningReflectionError::MissingOwnerUser)?;
        self.memory_writer
            .write_learning(&event, owner_user_id, learning)
            .await
    }

    async fn load_transcript(
        &self,
        event: &TurnLifecycleEvent,
    ) -> Result<ReflectionTranscript, LearningReflectionError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.thread_scope.clone(),
                thread_id: event.scope.thread_id.clone(),
            })
            .await
            .map_err(|error| LearningReflectionError::ThreadHistory {
                reason: error.to_string(),
            })?;
        let latest_user_message = history
            .messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == MessageKind::User
                    && is_reflection_transcript_status(message.status)
                    && message
                        .content
                        .as_ref()
                        .is_some_and(|content| !content.trim().is_empty())
            })
            .and_then(|message| message.content.clone())
            .ok_or(LearningReflectionError::MissingUserMessage)?;
        let rendered = render_transcript(&history.messages);
        Ok(ReflectionTranscript {
            latest_user_message,
            rendered,
        })
    }

    fn model_request(
        &self,
        event: &TurnLifecycleEvent,
        turn_id: &ironclaw_turns::TurnId,
        signal: ReflectionSignal,
        transcript: &str,
    ) -> Result<HostManagedModelRequest, LearningReflectionError> {
        let system_ref = reflection_message_ref(event.run_id, "system")?;
        let input_ref = reflection_message_ref(event.run_id, "input")?;
        let input = format!(
            "Signal: {}\nRun status: {:?}\nSanitized reason: {}\n\nTranscript:\n{}",
            signal.as_str(),
            event.status,
            event.sanitized_reason.as_deref().unwrap_or("none"),
            transcript
        );
        Ok(HostManagedModelRequest {
            model_profile_id: self.model_profile_id.clone(),
            messages: vec![
                HostManagedModelMessage {
                    role: HostManagedModelMessageRole::System,
                    content: REFLECTION_PROMPT.to_string(),
                    content_ref: system_ref,
                    tool_result_provider_call: None,
                    tool_result_content: None,
                },
                HostManagedModelMessage {
                    role: HostManagedModelMessageRole::User,
                    content: input,
                    content_ref: input_ref,
                    tool_result_provider_call: None,
                    tool_result_content: None,
                },
            ],
            surface_version: None,
            resolved_model_route: None,
            run_id: event.run_id,
            turn_id: *turn_id,
        })
    }
}

#[derive(Clone)]
struct LearningMemoryWriter {
    backend: Arc<dyn MemoryBackend>,
}

impl LearningMemoryWriter {
    fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        let repository = Arc::new(FilesystemMemoryDocumentRepository::new(filesystem));
        let indexer = Arc::new(ChunkingMemoryDocumentIndexer::new(Arc::clone(&repository)));
        let backend = RepositoryMemoryBackend::new(repository)
            .with_indexer(indexer)
            .with_capabilities(MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                versioning: true,
                prompt_write_safety: true,
                full_text_search: true,
                delete: true,
                transactions: true,
                ..MemoryBackendCapabilities::default()
            });
        Self {
            backend: Arc::new(backend),
        }
    }

    async fn write_learning(
        &self,
        event: &TurnLifecycleEvent,
        owner_user_id: &UserId,
        learning: ReflectionLearning,
    ) -> Result<(), LearningReflectionError> {
        let scope = MemoryDocumentScope::new_with_agent(
            event.scope.tenant_id.as_str(),
            owner_user_id.as_str(),
            event
                .scope
                .agent_id
                .as_ref()
                .map(|agent_id| agent_id.as_str()),
            event
                .scope
                .project_id
                .as_ref()
                .map(|project_id| project_id.as_str()),
        )
        .map_err(|error| LearningReflectionError::MemoryScope {
            reason: error.to_string(),
        })?;
        let path = MemoryDocumentPath::new_with_agent(
            event.scope.tenant_id.as_str(),
            owner_user_id.as_str(),
            event
                .scope
                .agent_id
                .as_ref()
                .map(|agent_id| agent_id.as_str()),
            event
                .scope
                .project_id
                .as_ref()
                .map(|project_id| project_id.as_str()),
            stable_learning_path(learning.category.as_str(), &learning.key)?,
        )
        .map_err(|error| LearningReflectionError::MemoryPath {
            reason: error.to_string(),
        })?;
        let metadata_overlay = DocumentMetadata {
            confidence: Some(learning.confidence),
            created_at: Some(Utc::now().to_rfc3339()),
            category: Some(learning.category.as_str().to_string()),
            key: Some(learning.key.clone()),
            source: Some("reflection".to_string()),
            ..DocumentMetadata::default()
        };
        self.backend
            .write_document_with_backend_options(
                &MemoryContext::new(scope),
                &path,
                learning.content.as_bytes(),
                &MemoryBackendWriteOptions {
                    metadata_overlay: Some(metadata_overlay),
                },
            )
            .await
            .map_err(|error| LearningReflectionError::MemoryWrite {
                reason: error.to_string(),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReflectionTranscript {
    latest_user_message: String,
    rendered: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReflectionSignal {
    Failure,
    CorrectionCue,
}

impl ReflectionSignal {
    fn as_str(self) -> &'static str {
        match self {
            Self::Failure => "failure",
            Self::CorrectionCue => "correction_cue",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ReflectionLearningCategory {
    Fact,
    Preference,
    Correction,
    Fp,
    Workflow,
}

impl ReflectionLearningCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Correction => "correction",
            Self::Fp => "fp",
            Self::Workflow => "workflow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReflectionLearning {
    key: String,
    category: ReflectionLearningCategory,
    content: String,
    confidence: u8,
}

#[derive(Debug, Deserialize)]
struct ReflectionDecision {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    category: Option<ReflectionLearningCategory>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    confidence: Option<u8>,
}

impl ReflectionDecision {
    fn into_learning(self) -> Result<Option<ReflectionLearning>, LearningReflectionError> {
        if self.key.is_none()
            && self.category.is_none()
            && self.content.is_none()
            && self.confidence.is_none()
        {
            return Ok(None);
        }
        let key = required_trimmed_field(self.key, "key")?;
        let category = self
            .category
            .ok_or_else(|| invalid_decision("category is required"))?;
        let content = required_trimmed_field(self.content, "content")?;
        let confidence = self
            .confidence
            .ok_or_else(|| invalid_decision("confidence is required"))?;
        if !(1..=10).contains(&confidence) {
            return Err(invalid_decision("confidence must be between 1 and 10"));
        }
        Ok(Some(ReflectionLearning {
            key,
            category,
            content,
            confidence,
        }))
    }
}

#[derive(Debug, Error)]
enum LearningReflectionError {
    #[error("thread history unavailable: {reason}")]
    ThreadHistory { reason: String },
    #[error("turn state unavailable: {reason}")]
    TurnState { reason: String },
    #[error("latest user message unavailable")]
    MissingUserMessage,
    #[error("owner user unavailable")]
    MissingOwnerUser,
    #[error("reflection message ref invalid: {reason}")]
    MessageRef { reason: String },
    #[error("reflection model timed out")]
    ModelTimeout,
    #[error("reflection model failed: {kind}")]
    Model { kind: String },
    #[error("reflection model output invalid: {reason}")]
    InvalidModelOutput { reason: String },
    #[error("memory scope invalid: {reason}")]
    MemoryScope { reason: String },
    #[error("memory path invalid: {reason}")]
    MemoryPath { reason: String },
    #[error("memory write failed: {reason}")]
    MemoryWrite { reason: String },
}

fn is_reflection_candidate_event(event: &TurnLifecycleEvent) -> bool {
    matches!(
        event.kind,
        TurnEventKind::Completed | TurnEventKind::Failed | TurnEventKind::RecoveryRequired
    ) || matches!(
        event.status,
        TurnStatus::Completed | TurnStatus::Failed | TurnStatus::RecoveryRequired
    )
}

fn reflection_signal_for_event(
    event: &TurnLifecycleEvent,
    latest_user_message: &str,
) -> Option<ReflectionSignal> {
    if event_has_failure_or_incident(event) {
        return Some(ReflectionSignal::Failure);
    }
    latest_user_message_has_correction_cue(latest_user_message)
        .then_some(ReflectionSignal::CorrectionCue)
}

fn event_has_failure_or_incident(event: &TurnLifecycleEvent) -> bool {
    matches!(
        event.kind,
        TurnEventKind::Failed | TurnEventKind::RecoveryRequired
    ) || matches!(
        event.status,
        TurnStatus::Failed | TurnStatus::RecoveryRequired
    ) || event.sanitized_reason.is_some()
}

fn latest_user_message_has_correction_cue(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let trimmed = lower.trim();
    let prefix_cues = [
        "actually",
        "correction:",
        "correction -",
        "i meant",
        "i said",
        "from now on",
        "remember that",
        "please remember",
        "for future reference",
        "next time",
        "i prefer",
        "prefer ",
        "do not ",
        "don't ",
        "never ",
        "always ",
    ];
    prefix_cues.iter().any(|cue| trimmed.starts_with(cue))
        || lower.contains("you were wrong")
        || lower.contains("you're wrong")
        || lower.contains("that was wrong")
        || lower.contains("that's wrong")
        || lower.contains("instead use")
        || lower.contains("should have")
}

fn render_transcript(messages: &[ThreadMessageRecord]) -> String {
    let selected = messages
        .iter()
        .filter(|message| is_reflection_transcript_status(message.status))
        .filter_map(renderable_message)
        .rev()
        .take(MAX_TRANSCRIPT_MESSAGES)
        .collect::<Vec<_>>();
    let mut rendered = String::new();
    for line in selected.into_iter().rev() {
        if rendered.len().saturating_add(line.len()).saturating_add(1) > MAX_TRANSCRIPT_CHARS {
            break;
        }
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&line);
    }
    rendered
}

fn renderable_message(message: &ThreadMessageRecord) -> Option<String> {
    let role = match message.kind {
        MessageKind::User => "user",
        MessageKind::Assistant => "assistant",
        MessageKind::System => "system",
        MessageKind::Summary => "summary",
        MessageKind::ToolResultReference => "tool_result",
        _ => return None,
    };
    let content = message.content.as_deref()?.trim();
    if content.is_empty() {
        return None;
    }
    Some(format!("{role}: {content}"))
}

fn is_reflection_transcript_status(status: MessageStatus) -> bool {
    matches!(
        status,
        MessageStatus::Accepted | MessageStatus::Submitted | MessageStatus::Finalized
    )
}

fn parse_reflection_learning(
    output_text: &str,
) -> Result<Option<ReflectionLearning>, LearningReflectionError> {
    let decision = parse_decision_json(output_text)?;
    decision.into_learning()
}

fn parse_decision_json(output_text: &str) -> Result<ReflectionDecision, LearningReflectionError> {
    match serde_json::from_str::<ReflectionDecision>(output_text.trim()) {
        Ok(decision) => Ok(decision),
        Err(first_error) => {
            let Some(json) = extract_json_object(output_text) else {
                return Err(LearningReflectionError::InvalidModelOutput {
                    reason: first_error.to_string(),
                });
            };
            serde_json::from_str::<ReflectionDecision>(json).map_err(|error| {
                LearningReflectionError::InvalidModelOutput {
                    reason: error.to_string(),
                }
            })
        }
    }
}

fn extract_json_object(output_text: &str) -> Option<&str> {
    let start = output_text.find('{')?;
    let end = output_text.rfind('}')?;
    (end >= start).then_some(&output_text[start..=end])
}

fn required_trimmed_field(
    value: Option<String>,
    field: &'static str,
) -> Result<String, LearningReflectionError> {
    let value = value.ok_or_else(|| invalid_decision(format!("{field} is required")))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(invalid_decision(format!("{field} must not be empty")));
    }
    Ok(trimmed.to_string())
}

fn invalid_decision(reason: impl Into<String>) -> LearningReflectionError {
    LearningReflectionError::InvalidModelOutput {
        reason: reason.into(),
    }
}

fn reflection_message_ref(
    run_id: TurnRunId,
    label: &'static str,
) -> Result<LoopMessageRef, LearningReflectionError> {
    LoopMessageRef::new(format!("msg:learning_reflection.{label}.{run_id}"))
        .map_err(|reason| LearningReflectionError::MessageRef { reason })
}

fn stable_learning_path(category: &str, key: &str) -> Result<String, LearningReflectionError> {
    Ok(format!(
        "keyed/{}/{}.md",
        encode_learning_path_segment(category)?,
        encode_learning_path_segment(key)?
    ))
}

fn encode_learning_path_segment(raw: &str) -> Result<String, LearningReflectionError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(LearningReflectionError::MemoryPath {
            reason: "learning path segment must not be empty".to_string(),
        });
    }
    let mut encoded = String::new();
    for byte in trimmed.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('_');
            encoded.push_str(&format!("{byte:02x}"));
        }
    }
    if encoded == "." || encoded == ".." {
        return Err(LearningReflectionError::MemoryPath {
            reason: "learning path segment must not be a dot segment".to_string(),
        });
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_cue_matches_direct_user_corrections() {
        assert!(latest_user_message_has_correction_cue(
            "Actually, I prefer pnpm for this repo."
        ));
        assert!(latest_user_message_has_correction_cue(
            "For future reference use compact summaries."
        ));
        assert!(latest_user_message_has_correction_cue(
            "You were wrong, the staging tenant is alpha."
        ));
    }

    #[test]
    fn correction_cue_ignores_ordinary_success_request() {
        assert!(!latest_user_message_has_correction_cue(
            "Can you list the available extensions?"
        ));
    }

    #[test]
    fn reflection_decision_allows_empty_noop() {
        let parsed = parse_reflection_learning("{}").expect("parse noop");
        assert!(parsed.is_none());
    }

    #[test]
    fn reflection_decision_parses_learning() {
        let parsed = parse_reflection_learning(
            r#"{"key":"editor_preference","category":"preference","content":"Use helix.","confidence":9}"#,
        )
        .expect("parse learning")
        .expect("learning");
        assert_eq!(parsed.key, "editor_preference");
        assert_eq!(parsed.category, ReflectionLearningCategory::Preference);
        assert_eq!(parsed.content, "Use helix.");
        assert_eq!(parsed.confidence, 9);
    }
}
