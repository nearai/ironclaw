//! Memory extraction hook — automatically extracts facts from conversations.
//!
//! Registered on `HookPoint::OnSessionEnd`. When a session ends, collects the
//! recent transcript and runs the 2-phase fact extraction pipeline to persist
//! important facts to the `memory_facts` table.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::db::Database;
use crate::hooks::{Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint};
use crate::llm::LlmProvider;
use crate::workspace::{EmbeddingProvider, FactExtractionConfig, FactExtractor};

/// Hook that extracts facts from session transcripts on session end.
pub struct MemoryExtractorHook {
    db: Arc<dyn Database>,
    llm: Arc<dyn LlmProvider>,
    embeddings: Option<Arc<dyn EmbeddingProvider>>,
    config: FactExtractionConfig,
}

impl MemoryExtractorHook {
    /// Create a new memory extractor hook.
    pub fn new(
        db: Arc<dyn Database>,
        llm: Arc<dyn LlmProvider>,
        embeddings: Option<Arc<dyn EmbeddingProvider>>,
        config: FactExtractionConfig,
    ) -> Self {
        Self {
            db,
            llm,
            embeddings,
            config,
        }
    }
}

#[async_trait]
impl Hook for MemoryExtractorHook {
    fn name(&self) -> &str {
        "builtin.memory_extractor"
    }

    fn hook_points(&self) -> &[HookPoint] {
        &[HookPoint::OnSessionEnd]
    }

    fn failure_mode(&self) -> HookFailureMode {
        // Extraction is best-effort; never block session cleanup
        HookFailureMode::FailOpen
    }

    fn timeout(&self) -> Duration {
        // LLM calls can take a while — allow up to 30 seconds
        Duration::from_secs(30)
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let (user_id, session_id) = match event {
            HookEvent::SessionEnd {
                user_id,
                session_id,
            } => (user_id.clone(), session_id.clone()),
            _ => return Ok(HookOutcome::ok()),
        };

        tracing::debug!(
            user_id = %user_id,
            session_id = %session_id,
            "Memory extraction hook triggered"
        );

        // Get recent transcript for this user (max 50 messages)
        let max_messages = 50;
        let transcript_pairs = match self
            .db
            .get_recent_transcript_for_user(&user_id, max_messages)
            .await
        {
            Ok(pairs) => pairs,
            Err(e) => {
                tracing::warn!(
                    user_id = %user_id,
                    error = %e,
                    "Failed to get transcript for extraction"
                );
                return Ok(HookOutcome::ok());
            }
        };

        // Check minimum message threshold
        if transcript_pairs.len() < self.config.min_messages {
            tracing::debug!(
                user_id = %user_id,
                message_count = transcript_pairs.len(),
                min_required = self.config.min_messages,
                "Transcript too short for extraction, skipping"
            );
            return Ok(HookOutcome::ok());
        }

        // Build transcript string
        let transcript = transcript_pairs
            .iter()
            .map(|(role, content)| format!("{}: {}", role, content))
            .collect::<Vec<_>>()
            .join("\n\n");

        // Skip if transcript is mostly empty or system messages
        let non_system_count = transcript_pairs
            .iter()
            .filter(|(role, _)| role != "system")
            .count();
        if non_system_count < self.config.min_messages {
            tracing::debug!(
                user_id = %user_id,
                non_system_messages = non_system_count,
                "Not enough non-system messages for extraction"
            );
            return Ok(HookOutcome::ok());
        }

        // Run the extraction pipeline
        let extractor = FactExtractor::new(
            self.llm.clone(),
            self.db.clone(),
            self.embeddings.clone(),
            self.config.clone(),
        );

        match extractor
            .run(&session_id, &transcript, &user_id, None)
            .await
        {
            Ok(entry) => {
                tracing::info!(
                    user_id = %user_id,
                    session_id = %session_id,
                    added = entry.facts_added,
                    updated = entry.facts_updated,
                    skipped = entry.facts_skipped,
                    duration_ms = entry.duration_ms.unwrap_or(0),
                    "Memory extraction completed"
                );
            }
            Err(e) => {
                tracing::warn!(
                    user_id = %user_id,
                    session_id = %session_id,
                    error = %e,
                    "Memory extraction failed"
                );
            }
        }

        Ok(HookOutcome::ok())
    }
}
