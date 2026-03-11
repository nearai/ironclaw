//! Automatic fact extraction from conversation transcripts.
//!
//! Uses a 2-phase pipeline:
//! 1. **Extract** — Send transcript to a fast LLM to pull out structured facts
//! 2. **Reconcile** — Deduplicate against existing facts, decide ADD/UPDATE/NOOP

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::{ExtractionLogEntry, FactStore};
use crate::error::WorkspaceError;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};
use crate::workspace::EmbeddingProvider;

/// A candidate fact extracted from a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateFact {
    pub content: String,
    pub category: String,
    pub confidence: f32,
}

/// Result of reconciling a single candidate against existing facts.
#[derive(Debug, Clone)]
pub enum ReconcileAction {
    /// Insert as a new fact.
    Add { id: Uuid },
    /// Update an existing fact.
    Update { existing_id: Uuid },
    /// Skip — already exists with same meaning.
    Noop,
}

/// Summary of an extraction run.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub added: i32,
    pub updated: i32,
    pub skipped: i32,
    pub actions: Vec<(CandidateFact, ReconcileAction)>,
}

/// Configuration for fact extraction.
#[derive(Debug, Clone)]
pub struct FactExtractionConfig {
    /// Maximum facts to extract per session.
    pub max_facts_per_session: usize,
    /// Minimum messages to trigger extraction.
    pub min_messages: usize,
    /// Cosine similarity threshold for dedup.
    pub dedup_threshold: f32,
    /// Model override for extraction (e.g., "claude-3-5-haiku-20241022").
    pub extraction_model: Option<String>,
}

impl Default for FactExtractionConfig {
    fn default() -> Self {
        Self {
            max_facts_per_session: 15,
            min_messages: 5,
            dedup_threshold: 0.85,
            extraction_model: None,
        }
    }
}

const EXTRACTION_PROMPT: &str = r#"Extract important facts from this conversation that should be remembered for future sessions. For each fact, provide a JSON object with:
- "content": the fact itself (concise, standalone, one sentence)
- "category": one of "preference", "learned", "procedural", "context"
- "confidence": 0.0-1.0 (how certain this is a lasting fact)

Categories:
- preference: user likes, dislikes, preferred workflows, timezone, formatting preferences
- learned: technical facts, account details, system configurations, project context
- procedural: how-to knowledge, step sequences, workarounds discovered
- context: relationships, project status, deadlines, goals

Rules:
- Focus on facts that will be useful in FUTURE conversations
- Skip: transient debugging, one-off commands, greetings, errors that were fixed
- Each fact must be self-contained (understandable without the conversation)
- Maximum 15 facts
- Return ONLY a JSON array, no other text

Example output:
[
  {"content": "User prefers CET timezone for all scheduling", "category": "preference", "confidence": 0.95},
  {"content": "NEAR wallet address is 281a79...", "category": "learned", "confidence": 1.0}
]"#;

/// Fact extractor using a 2-phase pipeline.
pub struct FactExtractor {
    llm: Arc<dyn LlmProvider>,
    db: Arc<dyn FactStore>,
    embeddings: Option<Arc<dyn EmbeddingProvider>>,
    config: FactExtractionConfig,
}

impl FactExtractor {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        db: Arc<dyn FactStore>,
        embeddings: Option<Arc<dyn EmbeddingProvider>>,
        config: FactExtractionConfig,
    ) -> Self {
        Self {
            llm,
            db,
            embeddings,
            config,
        }
    }

    /// Phase 1: Extract candidate facts from a conversation transcript.
    pub async fn extract_candidates(
        &self,
        transcript: &str,
    ) -> Result<Vec<CandidateFact>, WorkspaceError> {
        let messages = vec![
            ChatMessage::system(EXTRACTION_PROMPT),
            ChatMessage::user(transcript),
        ];

        let mut request = CompletionRequest::new(messages)
            .with_max_tokens(2000)
            .with_temperature(0.1);

        // Use extraction model if configured
        if let Some(ref model) = self.config.extraction_model {
            request = request.with_model(model.clone());
        }

        let response = self
            .llm
            .complete(request)
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("LLM extraction failed: {}", e),
            })?;

        // Parse the JSON response
        let content = response.content.trim();

        // Try to find JSON array in the response
        let json_str = if let Some(start) = content.find('[') {
            if let Some(end) = content.rfind(']') {
                &content[start..=end]
            } else {
                content
            }
        } else {
            content
        };

        let candidates: Vec<CandidateFact> =
            serde_json::from_str(json_str).map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to parse extraction response: {} — raw: {}", e, json_str),
            })?;

        // Enforce max facts limit
        let mut filtered: Vec<CandidateFact> = candidates
            .into_iter()
            .filter(|c| {
                !c.content.trim().is_empty()
                    && ["preference", "learned", "procedural", "context"]
                        .contains(&c.category.as_str())
                    && c.confidence > 0.0
                    && c.confidence <= 1.0
            })
            .collect();
        filtered.truncate(self.config.max_facts_per_session);

        tracing::info!(
            count = filtered.len(),
            "Extracted candidate facts from transcript"
        );
        Ok(filtered)
    }

    /// Phase 2: Reconcile candidates against existing facts.
    ///
    /// For each candidate:
    /// - Embed it
    /// - Search existing facts for similar content
    /// - Decide: ADD (new fact), UPDATE (info changed), or NOOP (already known)
    pub async fn reconcile(
        &self,
        candidates: Vec<CandidateFact>,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<ExtractionResult, WorkspaceError> {
        let mut result = ExtractionResult {
            added: 0,
            updated: 0,
            skipped: 0,
            actions: Vec::new(),
        };

        for candidate in candidates {
            let action = self
                .reconcile_single(&candidate, user_id, agent_id)
                .await?;

            match &action {
                ReconcileAction::Add { .. } => result.added += 1,
                ReconcileAction::Update { .. } => result.updated += 1,
                ReconcileAction::Noop => result.skipped += 1,
            }

            result.actions.push((candidate, action));
        }

        Ok(result)
    }

    /// Reconcile a single candidate fact.
    async fn reconcile_single(
        &self,
        candidate: &CandidateFact,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<ReconcileAction, WorkspaceError> {
        // Try to find similar existing facts via embedding
        if let Some(ref embedder) = self.embeddings {
            match embedder.embed(&candidate.content).await {
                Ok(embedding) => {
                    let similar = self
                        .db
                        .find_similar_facts(
                            user_id,
                            agent_id,
                            &embedding,
                            self.config.dedup_threshold,
                            3,
                        )
                        .await?;

                    if let Some(best) = similar.first() {
                        if best.score > 0.92 {
                            // Very similar — check if content is meaningfully different
                            if content_is_update(&best.fact.content, &candidate.content) {
                                // Content changed — update
                                self.db
                                    .upsert_fact(
                                        best.fact.id,
                                        user_id,
                                        agent_id,
                                        &candidate.content,
                                        &candidate.category,
                                        candidate.confidence,
                                        None,
                                        Some(&embedding),
                                        None,
                                    )
                                    .await?;
                                return Ok(ReconcileAction::Update {
                                    existing_id: best.fact.id,
                                });
                            } else {
                                // Same content — skip
                                return Ok(ReconcileAction::Noop);
                            }
                        } else if best.score > self.config.dedup_threshold {
                            // Somewhat similar — treat as update
                            self.db
                                .upsert_fact(
                                    best.fact.id,
                                    user_id,
                                    agent_id,
                                    &candidate.content,
                                    &candidate.category,
                                    candidate.confidence,
                                    None,
                                    Some(&embedding),
                                    None,
                                )
                                .await?;
                            return Ok(ReconcileAction::Update {
                                existing_id: best.fact.id,
                            });
                        }
                    }

                    // No similar fact found — add new
                    let new_id = Uuid::new_v4();
                    self.db
                        .upsert_fact(
                            new_id,
                            user_id,
                            agent_id,
                            &candidate.content,
                            &candidate.category,
                            candidate.confidence,
                            None,
                            Some(&embedding),
                            None,
                        )
                        .await?;
                    return Ok(ReconcileAction::Add { id: new_id });
                }
                Err(e) => {
                    tracing::warn!("Embedding failed for fact, falling back to insert: {}", e);
                }
            }
        }

        // No embeddings available — just insert (FTS will handle dedup at search time)
        let new_id = Uuid::new_v4();
        self.db
            .upsert_fact(
                new_id,
                user_id,
                agent_id,
                &candidate.content,
                &candidate.category,
                candidate.confidence,
                None,
                None,
                None,
            )
            .await?;
        Ok(ReconcileAction::Add { id: new_id })
    }

    /// Full pipeline: extract candidates → reconcile → persist → log.
    pub async fn run(
        &self,
        session_id: &str,
        transcript: &str,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<ExtractionLogEntry, WorkspaceError> {
        let start = Instant::now();

        // Phase 1: Extract
        let candidates = self.extract_candidates(transcript).await?;

        if candidates.is_empty() {
            let entry = ExtractionLogEntry {
                id: Uuid::new_v4(),
                session_id: session_id.to_string(),
                user_id: user_id.to_string(),
                agent_id,
                extracted_at: Utc::now(),
                facts_added: 0,
                facts_updated: 0,
                facts_skipped: 0,
                duration_ms: Some(start.elapsed().as_millis() as i64),
                model_used: self.config.extraction_model.clone(),
            };
            self.db.log_extraction(&entry).await?;
            return Ok(entry);
        }

        // Phase 2: Reconcile and persist
        let result = self.reconcile(candidates, user_id, agent_id).await?;

        let duration_ms = start.elapsed().as_millis() as i64;

        let entry = ExtractionLogEntry {
            id: Uuid::new_v4(),
            session_id: session_id.to_string(),
            user_id: user_id.to_string(),
            agent_id,
            extracted_at: Utc::now(),
            facts_added: result.added,
            facts_updated: result.updated,
            facts_skipped: result.skipped,
            duration_ms: Some(duration_ms),
            model_used: self.config.extraction_model.clone(),
        };

        self.db.log_extraction(&entry).await?;

        tracing::info!(
            session_id = session_id,
            added = result.added,
            updated = result.updated,
            skipped = result.skipped,
            duration_ms = duration_ms,
            "Fact extraction complete"
        );

        Ok(entry)
    }
}

/// Check if candidate content represents an update vs the same information.
///
/// Simple heuristic: if the strings differ by more than 20% of characters,
/// it's likely an update. Otherwise it's the same fact rephrased.
fn content_is_update(existing: &str, candidate: &str) -> bool {
    let existing_lower = existing.to_lowercase();
    let candidate_lower = candidate.to_lowercase();

    if existing_lower == candidate_lower {
        return false;
    }

    // If lengths differ significantly, it's an update
    let len_diff = (existing.len() as f32 - candidate.len() as f32).abs();
    let max_len = existing.len().max(candidate.len()) as f32;
    if max_len > 0.0 && len_diff / max_len > 0.2 {
        return true;
    }

    // Check word-level overlap
    let existing_words: std::collections::HashSet<&str> =
        existing_lower.split_whitespace().collect();
    let candidate_words: std::collections::HashSet<&str> =
        candidate_lower.split_whitespace().collect();

    let intersection = existing_words.intersection(&candidate_words).count();
    let union = existing_words.union(&candidate_words).count();

    if union == 0 {
        return false;
    }

    let jaccard = intersection as f32 / union as f32;
    // If less than 70% word overlap, consider it an update
    jaccard < 0.7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_is_update_same() {
        assert!(!content_is_update(
            "User prefers CET timezone",
            "User prefers CET timezone"
        ));
    }

    #[test]
    fn test_content_is_update_case_insensitive() {
        assert!(!content_is_update(
            "User prefers CET timezone",
            "user prefers cet timezone"
        ));
    }

    #[test]
    fn test_content_is_update_different() {
        assert!(content_is_update(
            "NEAR wallet balance is $100K",
            "NEAR wallet balance is $154K with Burrow lending positions"
        ));
    }

    #[test]
    fn test_content_is_update_minor_rephrase() {
        // Minor rephrasing should NOT be considered an update
        assert!(!content_is_update(
            "User prefers dark mode",
            "The user prefers dark mode"
        ));
    }
}
