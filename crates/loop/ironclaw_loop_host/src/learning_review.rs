//! Shared post-run learning review for the Reborn runtime.
//!
//! Every successful owned run can produce bounded memory candidates and one
//! skill-routing decision. This phase stores candidate records only. It does
//! not write provider memory, install skills, change the agent prompt, or send
//! user notifications.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use ironclaw_memory::{
    LearningAction, LearningCandidateStore, LearningReview, LearningReviewRecord, LearningScope,
    MAX_LEARNING_UNRESOLVED_PROPOSALS,
};
use ironclaw_product_contracts::operator_llm::{LearningRuntimeController, LearningSettings};
use ironclaw_safety::LeakDetector;
use ironclaw_threads::{
    CompletedRunMessages, CompletedRunMessagesRequest, MessageKind, SessionThreadService,
    ThreadMessageRecord, ThreadScope,
};
use ironclaw_turns::{TurnError, TurnEventKind, TurnEventSink, TurnLifecycleEvent, TurnRunId};
use tokio::{sync::Semaphore, task::JoinHandle};

const TRANSCRIPT_READ_LIMIT: usize = 64;
const TRANSCRIPT_MAX_BYTES: usize = 16 * 1024;
const LEARNING_REVIEW_OUTPUT_MAX_BYTES: usize = 32 * 1024;
pub(crate) const LEARNING_REVIEW_MAX_TOKENS: u32 = 4_096;
const MAX_CONCURRENT_REVIEWS: usize = 4;
const LEARNING_REVIEW_SYSTEM_PROMPT: &str = include_str!("../prompts/learning_review.md");
/// Fan out the single turn-event sink slot to independent best-effort sinks.
pub struct CompositeTurnEventSink {
    sinks: Vec<Arc<dyn TurnEventSink>>,
}

impl CompositeTurnEventSink {
    pub fn new(sinks: Vec<Arc<dyn TurnEventSink>>) -> Self {
        Self { sinks }
    }
}

#[async_trait]
impl TurnEventSink for CompositeTurnEventSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        for sink in &self.sinks {
            if let Err(error) = sink.publish(event.clone()).await {
                tracing::debug!(%error, "turn-event sink failed");
            }
        }
        Ok(())
    }
}

/// Live deployment-wide gate shared by settings and the turn-event sink.
pub struct LearningRuntimeControllerImpl {
    settings: RwLock<LearningSettings>,
}

impl LearningRuntimeControllerImpl {
    pub fn new(settings: LearningSettings) -> Self {
        Self {
            settings: RwLock::new(settings),
        }
    }

    pub fn enabled(&self) -> bool {
        match self.settings.read() {
            Ok(settings) => settings.enabled,
            Err(poisoned) => poisoned.into_inner().enabled,
        }
    }

    pub fn current_model(&self) -> Option<String> {
        match self.settings.read() {
            Ok(settings) => settings.model.clone(),
            Err(poisoned) => poisoned.into_inner().model.clone(),
        }
    }
}

impl Default for LearningRuntimeControllerImpl {
    fn default() -> Self {
        Self::new(LearningSettings::default())
    }
}

impl LearningRuntimeController for LearningRuntimeControllerImpl {
    fn apply(&self, settings: LearningSettings) {
        match self.settings.write() {
            Ok(mut current) => *current = settings,
            Err(poisoned) => *poisoned.into_inner() = settings,
        }
    }
}

#[derive(Debug)]
pub struct LearningInferenceError(String);

impl std::fmt::Display for LearningInferenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl LearningInferenceError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

#[async_trait]
pub trait LearningInferencePort: Send + Sync {
    async fn infer(&self, system: &str, user: &str) -> Result<String, LearningInferenceError>;
}

pub use crate::learning_candidate_store::FilesystemLearningCandidateStore;

/// Runtime-owned post-run tasks. Shutdown aborts all remaining model and store
/// work before their dependencies are dropped.
pub struct LearningReviewTasks {
    handles: Mutex<Vec<JoinHandle<()>>>,
    in_flight: Arc<Mutex<BTreeSet<TurnRunId>>>,
    permits: Arc<Semaphore>,
}

impl Default for LearningReviewTasks {
    fn default() -> Self {
        Self {
            handles: Mutex::new(Vec::new()),
            in_flight: Arc::new(Mutex::new(BTreeSet::new())),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT_REVIEWS)),
        }
    }
}

impl LearningReviewTasks {
    pub fn new() -> Self {
        Self::default()
    }

    fn spawn(&self, job: LearningReviewJob) {
        let run_id = job.run_id;
        let permit = match Arc::clone(&self.permits).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                tracing::debug!(
                    ?run_id,
                    limit = MAX_CONCURRENT_REVIEWS,
                    "learning review dropped because concurrency is saturated"
                );
                return;
            }
        };
        {
            let mut in_flight = match self.in_flight.lock() {
                Ok(in_flight) => in_flight,
                Err(poisoned) => poisoned.into_inner(),
            };
            if !in_flight.insert(run_id) {
                tracing::debug!(?run_id, "duplicate learning review dropped");
                return;
            }
        }
        let in_flight = Arc::clone(&self.in_flight);
        let handle = tokio::spawn(async move {
            let _permit = permit;
            job.run().await;
            match in_flight.lock() {
                Ok(mut in_flight) => {
                    in_flight.remove(&run_id);
                }
                Err(poisoned) => {
                    poisoned.into_inner().remove(&run_id);
                }
            }
        });
        let mut handles = match self.handles.lock() {
            Ok(handles) => handles,
            Err(poisoned) => poisoned.into_inner(),
        };
        handles.retain(|handle| !handle.is_finished());
        handles.push(handle);
    }

    pub async fn shutdown(&self) {
        let handles = {
            let mut handles = match self.handles.lock() {
                Ok(handles) => handles,
                Err(poisoned) => poisoned.into_inner(),
            };
            std::mem::take(&mut *handles)
        };
        for handle in &handles {
            handle.abort();
        }
        for handle in handles {
            if let Err(error) = handle.await {
                if error.is_panic() {
                    tracing::error!(%error, "learning review task panicked during shutdown");
                } else {
                    tracing::debug!(%error, "learning review task cancelled during shutdown");
                }
            }
        }
        match self.in_flight.lock() {
            Ok(mut in_flight) => in_flight.clear(),
            Err(poisoned) => poisoned.into_inner().clear(),
        }
    }

    /// Await all currently owned review jobs without cancelling them.
    pub async fn wait(&self) {
        let handles = {
            let mut handles = match self.handles.lock() {
                Ok(handles) => handles,
                Err(poisoned) => poisoned.into_inner(),
            };
            std::mem::take(&mut *handles)
        };
        for handle in handles {
            if let Err(error) = handle.await
                && error.is_panic()
            {
                tracing::error!(%error, "learning review task panicked while waiting");
            }
        }
    }
}

/// Successful-run subscriber for the shared learning router.
pub struct LearningReviewTurnEventSink {
    thread_service: Arc<dyn SessionThreadService>,
    inference: Arc<dyn LearningInferencePort>,
    candidate_store: Arc<dyn LearningCandidateStore>,
    tasks: Arc<LearningReviewTasks>,
    controller: Arc<LearningRuntimeControllerImpl>,
}

impl LearningReviewTurnEventSink {
    pub fn new(
        thread_service: Arc<dyn SessionThreadService>,
        inference: Arc<dyn LearningInferencePort>,
        candidate_store: Arc<dyn LearningCandidateStore>,
        tasks: Arc<LearningReviewTasks>,
        controller: Arc<LearningRuntimeControllerImpl>,
    ) -> Self {
        Self {
            thread_service,
            inference,
            candidate_store,
            tasks,
            controller,
        }
    }
}

#[async_trait]
impl TurnEventSink for LearningReviewTurnEventSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        if !matches!(event.kind, TurnEventKind::Completed) || !self.controller.enabled() {
            return Ok(());
        }
        let Some(user_id) = event
            .owner_user_id
            .clone()
            .or_else(|| event.scope.explicit_owner_user_id().cloned())
        else {
            return Ok(());
        };
        let Some(agent_id) = event.scope.agent_id.clone() else {
            return Ok(());
        };
        let thread_scope = ThreadScope {
            tenant_id: event.scope.tenant_id.clone(),
            agent_id: agent_id.clone(),
            project_id: event.scope.project_id.clone(),
            owner_user_id: Some(user_id.clone()),
            mission_id: None,
        };
        let learning_scope = LearningScope::new(
            event.scope.tenant_id.clone(),
            user_id,
            agent_id,
            event.scope.project_id.clone(),
        );
        self.tasks.spawn(LearningReviewJob {
            thread_service: Arc::clone(&self.thread_service),
            inference: Arc::clone(&self.inference),
            candidate_store: Arc::clone(&self.candidate_store),
            controller: Arc::clone(&self.controller),
            thread_scope,
            thread_id: event.scope.thread_id.clone(),
            run_id: event.run_id,
            learning_scope,
        });
        Ok(())
    }
}

struct LearningReviewJob {
    thread_service: Arc<dyn SessionThreadService>,
    inference: Arc<dyn LearningInferencePort>,
    candidate_store: Arc<dyn LearningCandidateStore>,
    controller: Arc<LearningRuntimeControllerImpl>,
    thread_scope: ThreadScope,
    thread_id: ironclaw_host_api::ids::ThreadId,
    run_id: TurnRunId,
    learning_scope: LearningScope,
}

impl LearningReviewJob {
    async fn run(self) {
        if !self.controller.enabled() {
            return;
        }
        match self
            .candidate_store
            .get(&self.learning_scope, self.run_id)
            .await
        {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(?error, run_id = ?self.run_id, "learning idempotency check failed");
                return;
            }
        }
        let messages = match self
            .thread_service
            .list_completed_run_messages_bounded(CompletedRunMessagesRequest {
                scope: self.thread_scope,
                thread_id: self.thread_id,
                turn_run_id: self.run_id,
                max_messages: TRANSCRIPT_READ_LIMIT,
                max_bytes: TRANSCRIPT_MAX_BYTES,
            })
            .await
        {
            Ok(CompletedRunMessages::Complete(messages)) => messages,
            Ok(CompletedRunMessages::LimitExceeded) => {
                tracing::debug!(run_id = ?self.run_id, "learning review transcript exceeded bounds");
                return;
            }
            Err(error) => {
                tracing::debug!(%error, run_id = ?self.run_id, "learning review transcript read failed");
                return;
            }
        };
        let transcript = format_transcript(&messages);
        if transcript.content.is_empty() || !self.controller.enabled() {
            return;
        }
        let unresolved_proposals = match self
            .candidate_store
            .list_unresolved(&self.learning_scope)
            .await
        {
            Ok(records) => records
                .into_iter()
                .flat_map(|record| record.review.memory)
                .take(MAX_LEARNING_UNRESOLVED_PROPOSALS as usize)
                .collect::<Vec<_>>(),
            Err(error) => {
                tracing::debug!(?error, run_id = ?self.run_id, "learning unresolved candidate read failed");
                Vec::new()
            }
        };
        let user_prompt = match serde_json::to_string(&serde_json::json!({
            "transcript": transcript.content.as_str(),
            "related_memories": [],
            "unresolved_proposals": unresolved_proposals,
        })) {
            Ok(prompt) => prompt,
            Err(error) => {
                tracing::debug!(%error, run_id = ?self.run_id, "learning review input encoding failed");
                return;
            }
        };
        let output = match self
            .inference
            .infer(LEARNING_REVIEW_SYSTEM_PROMPT, &user_prompt)
            .await
        {
            Ok(output) if output.len() <= LEARNING_REVIEW_OUTPUT_MAX_BYTES => output,
            Ok(_) => {
                tracing::debug!(
                    run_id = ?self.run_id,
                    limit = LEARNING_REVIEW_OUTPUT_MAX_BYTES,
                    "learning review provider output exceeded byte limit"
                );
                return;
            }
            Err(error) => {
                tracing::debug!(%error, run_id = ?self.run_id, "learning review inference failed");
                return;
            }
        };
        let review = match parse_review(&output)
            .and_then(|review| seal_review_sources(review, &transcript))
            .and_then(reject_secret_bearing_candidates)
        {
            Ok(review) => review,
            Err(reason) => {
                tracing::debug!(%reason, run_id = ?self.run_id, "learning review output rejected");
                return;
            }
        };
        if !self.controller.enabled() {
            return;
        }
        let record = match LearningReviewRecord::new(self.run_id, self.learning_scope, review) {
            Ok(record) => record,
            Err(error) => {
                tracing::debug!(?error, run_id = ?self.run_id, "learning review record rejected");
                return;
            }
        };
        if let Err(error) = self.candidate_store.insert_if_absent(&record).await {
            tracing::debug!(?error, run_id = ?self.run_id, "learning candidate persistence failed");
        }
    }
}

fn parse_review(output: &str) -> Result<LearningReview, &'static str> {
    if output.len() > LEARNING_REVIEW_OUTPUT_MAX_BYTES {
        return Err("output too large");
    }
    let review: LearningReview = serde_json::from_str(output).map_err(|_| "invalid JSON")?;
    review.validate().map_err(|_| "invalid learning review")?;
    Ok(review)
}

struct FormattedTranscript {
    content: String,
    source_indices: BTreeSet<u16>,
    tainted_indices: BTreeSet<u16>,
}

fn seal_review_sources(
    mut review: LearningReview,
    transcript: &FormattedTranscript,
) -> Result<LearningReview, &'static str> {
    let transcript_tainted = !transcript.tainted_indices.is_empty();
    for proposal in &mut review.memory {
        if !proposal
            .source_message_indices
            .iter()
            .all(|index| transcript.source_indices.contains(index))
        {
            return Err("unknown source message index");
        }
        proposal.tainted |= transcript_tainted
            || proposal
                .source_message_indices
                .iter()
                .any(|index| transcript.tainted_indices.contains(index));
    }
    if !review
        .skill
        .source_message_indices
        .iter()
        .all(|index| transcript.source_indices.contains(index))
    {
        return Err("unknown skill source message index");
    }
    if review.skill.action == LearningAction::Distill {
        review.skill.tainted |= transcript_tainted
            || review
                .skill
                .source_message_indices
                .iter()
                .any(|index| transcript.tainted_indices.contains(index));
    }
    Ok(review)
}

fn reject_secret_bearing_candidates(
    review: LearningReview,
) -> Result<LearningReview, &'static str> {
    let detector = LeakDetector::new();
    if review
        .memory
        .iter()
        .any(|proposal| !detector.scan(&proposal.content).is_clean())
        || review
            .skill
            .reason
            .as_deref()
            .is_some_and(|reason| !detector.scan(reason).is_clean())
    {
        return Err("secret detected in learning candidate");
    }
    Ok(review)
}

fn format_transcript(messages: &[ThreadMessageRecord]) -> FormattedTranscript {
    let mut output = String::new();
    let mut source_indices = BTreeSet::new();
    let mut tainted_indices = BTreeSet::new();
    for (index, message) in messages.iter().enumerate() {
        let role = match message.kind {
            MessageKind::User => "user",
            MessageKind::Assistant => "assistant",
            MessageKind::ToolResultReference => "tool_result",
            MessageKind::System => "system",
            _ => continue,
        };
        let Some(content) = message.content.as_deref() else {
            continue;
        };
        let mut line = format!("[{index}] {role}: ");
        if matches!(message.kind, MessageKind::ToolResultReference)
            && let Some(call) = message.tool_result_provider_call.as_ref()
        {
            line.push_str("capability=");
            line.push_str(call.capability_id.as_str());
            line.push(' ');
        }
        line.push_str(content);
        line.push('\n');
        if output.len().saturating_add(line.len()) > TRANSCRIPT_MAX_BYTES {
            break;
        }
        let Ok(index) = u16::try_from(index) else {
            break;
        };
        output.push_str(&line);
        source_indices.insert(index);
        if matches!(message.kind, MessageKind::ToolResultReference)
            || message.source_binding_id.is_some()
        {
            tainted_indices.insert(index);
        }
    }
    FormattedTranscript {
        content: output,
        source_indices,
        tainted_indices,
    }
}

#[cfg(test)]
#[path = "learning_review/tests.rs"]
mod tests;
