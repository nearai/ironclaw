//! Context compaction for preserving and summarizing conversation history.
//!
//! When the context window approaches its limit, compaction:
//! 1. Summarizes old turns
//! 2. Writes the summary to the workspace daily log
//! 3. Extracts durable memories from the archived turns into MEMORY.md
//! 4. Trims the context to keep only recent turns

use std::{collections::HashSet, sync::Arc};

use chrono::Utc;
use serde::Deserialize;

use crate::agent::context_monitor::{CompactionStrategy, ContextBreakdown};
use crate::agent::session::Thread;
use crate::error::Error;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider, Reasoning};
use crate::workspace::Workspace;

/// Result of a compaction operation.
#[derive(Debug)]
pub struct CompactionResult {
    /// Number of turns removed.
    pub turns_removed: usize,
    /// Tokens before compaction.
    pub tokens_before: usize,
    /// Tokens after compaction.
    pub tokens_after: usize,
    /// Whether a summary was written to workspace.
    pub summary_written: bool,
    /// Number of structured memories written to workspace.
    pub memories_written: usize,
    /// The generated summary (if any).
    pub summary: Option<String>,
}

/// Compacts conversation context to stay within limits.
pub struct ContextCompactor {
    llm: Arc<dyn LlmProvider>,
}

impl ContextCompactor {
    /// Create a new context compactor.
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self {
        Self { llm }
    }

    /// Compact a thread's context using the given strategy.
    pub async fn compact(
        &self,
        thread: &mut Thread,
        strategy: CompactionStrategy,
        workspace: Option<&Workspace>,
    ) -> Result<CompactionResult, Error> {
        let messages = thread.messages();
        let tokens_before = ContextBreakdown::analyze(&messages).total_tokens;

        let result = match strategy {
            CompactionStrategy::Summarize { keep_recent } => {
                self.compact_with_summary(thread, keep_recent, workspace)
                    .await?
            }
            CompactionStrategy::Truncate { keep_recent } => {
                self.compact_truncate(thread, keep_recent, workspace).await
            }
            CompactionStrategy::MoveToWorkspace => {
                self.compact_to_workspace(thread, workspace).await?
            }
        };

        let messages_after = thread.messages();
        let tokens_after = ContextBreakdown::analyze(&messages_after).total_tokens;

        Ok(CompactionResult {
            turns_removed: result.turns_removed,
            tokens_before,
            tokens_after,
            summary_written: result.summary_written,
            memories_written: result.memories_written,
            summary: result.summary,
        })
    }

    /// Compact by summarizing old turns.
    async fn compact_with_summary(
        &self,
        thread: &mut Thread,
        keep_recent: usize,
        workspace: Option<&Workspace>,
    ) -> Result<CompactionPartial, Error> {
        if thread.turns.len() <= keep_recent {
            return Ok(CompactionPartial::empty());
        }

        // Get turns to summarize
        let turns_to_remove = thread.turns.len() - keep_recent;
        let old_turns = &thread.turns[..turns_to_remove];

        // Build messages for summarization
        let mut to_summarize = Vec::new();
        for turn in old_turns {
            to_summarize.push(ChatMessage::user(&turn.user_input));
            if let Some(ref response) = turn.response {
                to_summarize.push(ChatMessage::assistant(response));
            }
        }

        // Generate summary
        let summary = self.generate_summary(&to_summarize).await?;

        // Write to workspace if available.
        // If archival fails, preserve turns to avoid context loss.
        let (summary_written, memories_written, turns_removed) = if let Some(ws) = workspace {
            match self.write_summary_to_workspace(ws, &summary).await {
                Ok(()) => {
                    let memories_written = self.extract_structured_memories(ws, old_turns).await;
                    thread.truncate_turns(keep_recent);
                    (true, memories_written, turns_to_remove)
                }
                Err(e) => {
                    tracing::warn!("Compaction summary write failed (turns preserved): {}", e);
                    (false, 0, 0)
                }
            }
        } else {
            thread.truncate_turns(keep_recent);
            (false, 0, turns_to_remove)
        };

        Ok(CompactionPartial {
            turns_removed,
            summary_written,
            memories_written,
            summary: Some(summary),
        })
    }

    /// Compact by simple truncation (no summary).
    async fn compact_truncate(
        &self,
        thread: &mut Thread,
        keep_recent: usize,
        workspace: Option<&Workspace>,
    ) -> CompactionPartial {
        let turns_before = thread.turns.len();
        let turns_to_remove = turns_before.saturating_sub(keep_recent);
        let memories_written = if let Some(ws) = workspace {
            let old_turns = &thread.turns[..turns_to_remove];
            self.extract_structured_memories(ws, old_turns).await
        } else {
            0
        };

        thread.truncate_turns(keep_recent);
        let turns_removed = turns_before - thread.turns.len();

        CompactionPartial {
            turns_removed,
            summary_written: false,
            memories_written,
            summary: None,
        }
    }

    /// Move context to workspace without summarization.
    async fn compact_to_workspace(
        &self,
        thread: &mut Thread,
        workspace: Option<&Workspace>,
    ) -> Result<CompactionPartial, Error> {
        let Some(ws) = workspace else {
            // Fall back to truncation if no workspace
            return Ok(self.compact_truncate(thread, 5, None).await);
        };

        // Keep more turns when moving to workspace (we have a backup)
        let keep_recent = 10;
        if thread.turns.len() <= keep_recent {
            return Ok(CompactionPartial::empty());
        }

        let turns_to_remove = thread.turns.len() - keep_recent;
        let old_turns = &thread.turns[..turns_to_remove];

        // Format turns for storage
        let content = format_turns_for_storage(old_turns);

        // Write to workspace. If archival fails, preserve turns.
        let (written, memories_written, turns_removed) =
            match self.write_context_to_workspace(ws, &content).await {
                Ok(()) => {
                    let memories_written = self.extract_structured_memories(ws, old_turns).await;
                    thread.truncate_turns(keep_recent);
                    (true, memories_written, turns_to_remove)
                }
                Err(e) => {
                    tracing::warn!("Compaction context write failed (turns preserved): {}", e);
                    (false, 0, 0)
                }
            };

        Ok(CompactionPartial {
            turns_removed,
            summary_written: written,
            memories_written,
            summary: None,
        })
    }

    /// Generate a summary of messages using the LLM.
    async fn generate_summary(&self, messages: &[ChatMessage]) -> Result<String, Error> {
        let prompt = ChatMessage::system(
            r#"Summarize the following conversation concisely. Focus on:
- Key decisions made
- Important information exchanged
- Actions taken
- Outcomes achieved

Be brief but capture all important details. Use bullet points."#,
        );

        let mut request_messages = vec![prompt];

        // Add a user message with the conversation to summarize
        let formatted = messages
            .iter()
            .map(|m| {
                let role_str = match m.role {
                    crate::llm::Role::User => "User",
                    crate::llm::Role::Assistant => "Assistant",
                    crate::llm::Role::System => "System",
                    crate::llm::Role::Tool => {
                        return format!(
                            "Tool {}: {}",
                            m.name.as_deref().unwrap_or("unknown"),
                            m.content
                        );
                    }
                };
                format!("{}: {}", role_str, m.content)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        request_messages.push(ChatMessage::user(format!(
            "Please summarize this conversation:\n\n{}",
            formatted
        )));

        let request = CompletionRequest::new(request_messages)
            .with_max_tokens(1024)
            .with_temperature(0.3);

        let reasoning =
            Reasoning::new(self.llm.clone()).with_model_name(self.llm.active_model_name());
        let (text, _) = reasoning.complete(request).await?;
        Ok(text)
    }

    /// Write a summary to the workspace daily log.
    async fn write_summary_to_workspace(
        &self,
        workspace: &Workspace,
        summary: &str,
    ) -> Result<(), Error> {
        let date = Utc::now().format("%Y-%m-%d");
        let entry = format!(
            "\n## Context Summary ({})\n\n{}\n",
            Utc::now().format("%H:%M UTC"),
            summary
        );

        workspace
            .append(&format!("daily/{}.md", date), &entry)
            .await?;
        Ok(())
    }

    /// Write full context to workspace for archival.
    async fn write_context_to_workspace(
        &self,
        workspace: &Workspace,
        content: &str,
    ) -> Result<(), Error> {
        let date = Utc::now().format("%Y-%m-%d");
        let entry = format!(
            "\n## Archived Context ({})\n\n{}\n",
            Utc::now().format("%H:%M UTC"),
            content
        );

        workspace
            .append(&format!("daily/{}.md", date), &entry)
            .await?;
        Ok(())
    }

    /// Extract structured memories from archived turns and append them to MEMORY.md.
    ///
    /// This is best-effort: failures are logged and skipped so compaction
    /// itself can still complete after the archive write succeeds.
    async fn extract_structured_memories(
        &self,
        workspace: &Workspace,
        turns: &[crate::agent::session::Turn],
    ) -> usize {
        if turns.is_empty() {
            return 0;
        }

        let prompt = ChatMessage::system(
            r#"Extract durable memories from the conversation below.

Return ONLY JSON with this shape:
{
  "memories": [
    {
      "kind": "preference | identity | project | plan | fact",
      "content": "short durable memory in one sentence",
      "confidence": 0.0-1.0,
      "evidence": "optional short supporting quote"
    }
  ]
}

Rules:
- Return at most 5 memories.
- Only include high-confidence memories that are likely useful later.
- Skip greetings, transient chat, and one-off task steps.
- Prefer stable user preferences, long-term plans, active projects, and important facts.
- If there are no durable memories, return {"memories":[]}.
"#,
        );

        let conversation = format_turns_for_storage(turns);
        let request = CompletionRequest::new(vec![
            prompt,
            ChatMessage::user(format!("Conversation to analyze:\n\n{}", conversation)),
        ])
        .with_max_tokens(512)
        .with_temperature(0.0);

        let reasoning =
            Reasoning::new(self.llm.clone()).with_model_name(self.llm.active_model_name());
        let Ok((response, _)) = reasoning.complete(request).await else {
            tracing::warn!("Structured memory extraction failed: LLM call failed");
            return 0;
        };

        let Some(json) = extract_json_object(&response) else {
            tracing::warn!("Structured memory extraction failed: no JSON object in response");
            return 0;
        };

        let parsed = match serde_json::from_str::<MemoryExtraction>(json) {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::warn!(
                    "Structured memory extraction failed to parse JSON from LLM: {}",
                    err
                );
                return 0;
            }
        };

        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        for memory in parsed
            .memories
            .into_iter()
            .take(MEMORY_EXTRACTION_MAX_CANDIDATES)
        {
            if memory.confidence < MEMORY_EXTRACTION_MIN_CONFIDENCE {
                continue;
            }

            let normalized_content = normalize_memory_text(&memory.content);
            if normalized_content.is_empty() || !seen.insert(normalized_content) {
                continue;
            }

            if self
                .structured_memory_exists(workspace, &memory.content)
                .await
            {
                continue;
            }

            entries.push(format_structured_memory_entry(&memory));
        }

        if entries.is_empty() {
            return 0;
        }

        let entry = format!(
            "\n## Structured Memory Extraction ({})\n\n{}\n",
            Utc::now().format("%Y-%m-%d %H:%M UTC"),
            entries.join("\n\n")
        );

        match workspace.append_memory(&entry).await {
            Ok(()) => entries.len(),
            Err(e) => {
                tracing::warn!("Structured memory append failed: {}", e);
                0
            }
        }
    }

    /// Check whether a structured memory already appears in workspace search results.
    async fn structured_memory_exists(&self, workspace: &Workspace, content: &str) -> bool {
        let query = content.trim();
        if query.is_empty() {
            return true;
        }

        let normalized_query = normalize_memory_text(query);

        let Ok(memory_doc) = workspace.memory().await else {
            tracing::warn!("Structured memory dedupe read failed; treating as new");
            return false;
        };

        memory_doc.content.lines().any(|line| {
            let normalized_existing = line
                .trim()
                .starts_with("- **")
                .then(|| line.split_once("**: ").map(|(_, content)| content))
                .flatten()
                .map(normalize_memory_text);

            normalized_existing.is_some_and(|existing| {
                existing.contains(&normalized_query) || normalized_query.contains(&existing)
            })
        })
    }
}

/// Partial result during compaction (internal).
struct CompactionPartial {
    turns_removed: usize,
    summary_written: bool,
    memories_written: usize,
    summary: Option<String>,
}

impl CompactionPartial {
    fn empty() -> Self {
        Self {
            turns_removed: 0,
            summary_written: false,
            memories_written: 0,
            summary: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct MemoryExtraction {
    memories: Vec<StructuredMemory>,
}

#[derive(Debug, Clone, Deserialize)]
struct StructuredMemory {
    kind: String,
    content: String,
    confidence: f32,
    #[serde(default)]
    evidence: Option<String>,
}

const MEMORY_EXTRACTION_MAX_CANDIDATES: usize = 5;
const MEMORY_EXTRACTION_MIN_CONFIDENCE: f32 = 0.75;

/// Format turns for storage in workspace.
fn format_turns_for_storage(turns: &[crate::agent::session::Turn]) -> String {
    turns
        .iter()
        .map(|turn| {
            let mut s = format!("**Turn {}**\n", turn.turn_number + 1);
            s.push_str(&format!("User: {}\n", turn.user_input));
            if let Some(ref response) = turn.response {
                s.push_str(&format!("Agent: {}\n", response));
            }
            if !turn.tool_calls.is_empty() {
                s.push_str("Tools: ");
                let tools: Vec<_> = turn.tool_calls.iter().map(|t| t.name.as_str()).collect();
                s.push_str(&tools.join(", "));
                s.push('\n');
            }
            s
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_structured_memory_entry(memory: &StructuredMemory) -> String {
    let mut entry = format!(
        "- **{}**: {}\n  - Confidence: {:.2}",
        memory.kind.trim(),
        memory.content.trim(),
        memory.confidence
    );
    if let Some(evidence) = memory
        .evidence
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        entry.push_str(&format!("\n  - Evidence: {}", evidence));
    }
    entry
}

fn normalize_memory_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (start < end).then(|| &text[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::Thread;
    use crate::workspace::paths;
    use uuid::Uuid;

    #[test]
    fn test_format_turns() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("Hello");
        thread.complete_turn("Hi there");
        thread.start_turn("How are you?");
        thread.complete_turn("I'm good!");

        let formatted = format_turns_for_storage(&thread.turns);
        assert!(formatted.contains("Turn 1"));
        assert!(formatted.contains("Hello"));
        assert!(formatted.contains("Turn 2"));
    }

    #[test]
    fn test_compaction_partial_empty() {
        let partial = CompactionPartial::empty();
        assert_eq!(partial.turns_removed, 0);
        assert!(!partial.summary_written);
    }

    // === QA Plan - Compaction strategy tests ===

    use crate::agent::context_monitor::CompactionStrategy;
    use crate::testing::StubLlm;

    /// Helper: build a `ContextCompactor` with the given `StubLlm`.
    fn make_compactor(llm: Arc<StubLlm>) -> ContextCompactor {
        ContextCompactor::new(llm)
    }

    /// Helper: build a thread with `n` completed turns.
    /// Turn `i` has user_input "msg-{i}" and response "resp-{i}".
    fn make_thread(n: usize) -> Thread {
        let mut thread = Thread::new(Uuid::new_v4());
        for i in 0..n {
            thread.start_turn(format!("msg-{}", i));
            thread.complete_turn(format!("resp-{}", i));
        }
        thread
    }

    #[cfg(feature = "libsql")]
    async fn make_unmigrated_workspace() -> anyhow::Result<crate::workspace::Workspace> {
        use crate::db::Database;
        use crate::db::libsql::LibSqlBackend;

        // Intentionally skip migrations so workspace append operations fail.
        let backend = LibSqlBackend::new_memory().await?;
        let db: Arc<dyn Database> = Arc::new(backend);
        Ok(crate::workspace::Workspace::new_with_db(
            "compaction-test",
            db,
        ))
    }

    #[cfg(feature = "libsql")]
    async fn make_test_workspace()
    -> anyhow::Result<(crate::workspace::Workspace, tempfile::TempDir)> {
        use crate::db::Database;
        use crate::db::libsql::LibSqlBackend;

        let temp_dir = tempfile::tempdir()?;
        let db_path = temp_dir.path().join("compaction_memory_test.db");
        let backend = LibSqlBackend::new_local(&db_path).await?;
        backend.run_migrations().await?;
        let db: Arc<dyn Database> = Arc::new(backend);
        let ws = crate::workspace::Workspace::new_with_db("compaction-memory-test", db);
        Ok((ws, temp_dir))
    }

    // ------------------------------------------------------------------
    // 1. compact_truncate keeps last N turns
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_truncate_keeps_last_n() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(10);
        assert_eq!(thread.turns.len(), 10);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 3 },
                None,
            )
            .await?;

        // Only 3 turns remain
        assert_eq!(thread.turns.len(), 3);

        // They are the most recent ones (msg-7, msg-8, msg-9)
        assert_eq!(thread.turns[0].user_input, "msg-7");
        assert_eq!(thread.turns[1].user_input, "msg-8");
        assert_eq!(thread.turns[2].user_input, "msg-9");

        // Turn numbers are re-indexed to 0, 1, 2
        assert_eq!(thread.turns[0].turn_number, 0);
        assert_eq!(thread.turns[1].turn_number, 1);
        assert_eq!(thread.turns[2].turn_number, 2);

        // Result metadata
        assert_eq!(result.turns_removed, 7);
        assert!(!result.summary_written);
        assert!(result.summary.is_none());

        // Tokens should be reported (before > 0 since we had content)
        assert!(result.tokens_before > 0);
        assert!(result.tokens_after > 0);
        assert!(result.tokens_before > result.tokens_after);

        Ok(())
    }

    // ------------------------------------------------------------------
    // 2. compact_truncate with fewer turns than limit (no-op)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_truncate_with_fewer_turns_than_limit() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(2);

        let original_inputs: Vec<String> =
            thread.turns.iter().map(|t| t.user_input.clone()).collect();

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 5 },
                None,
            )
            .await?;

        // All turns preserved
        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].user_input, original_inputs[0]);
        assert_eq!(thread.turns[1].user_input, original_inputs[1]);

        // No turns removed
        assert_eq!(result.turns_removed, 0);
        assert!(!result.summary_written);
        assert!(result.summary.is_none());

        Ok(())
    }

    // ------------------------------------------------------------------
    // 3. compact_truncate with empty turns list
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_truncate_empty_turns() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = Thread::new(Uuid::new_v4());
        assert!(thread.turns.is_empty());

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 3 },
                None,
            )
            .await?;

        assert!(thread.turns.is_empty());
        assert_eq!(result.turns_removed, 0);
        assert_eq!(result.tokens_before, 0);
        assert_eq!(result.tokens_after, 0);

        Ok(())
    }

    // ------------------------------------------------------------------
    // 4. compact_with_summary produces summary turn via StubLlm
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_with_summary_produces_summary_turn() -> anyhow::Result<()> {
        let canned_summary =
            "- User greeted the agent\n- Agent responded warmly\n- Five exchanges completed";
        let llm = Arc::new(StubLlm::new(canned_summary));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(5);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Summarize { keep_recent: 2 },
                None,
            )
            .await?;

        // Should keep only 2 recent turns
        assert_eq!(thread.turns.len(), 2);

        // The kept turns should be the last two (msg-3, msg-4)
        assert_eq!(thread.turns[0].user_input, "msg-3");
        assert_eq!(thread.turns[1].user_input, "msg-4");

        // Result should report the summary
        assert_eq!(result.turns_removed, 3);
        assert!(result.summary.is_some());
        let summary = result
            .summary
            .ok_or_else(|| anyhow::anyhow!("missing summary"))?;
        assert!(summary.contains("User greeted the agent"));
        assert!(summary.contains("Five exchanges completed"));

        // summary_written should be false since no workspace was provided
        assert!(!result.summary_written);

        // StubLlm should have been called exactly once for the summary
        assert_eq!(llm.calls(), 1);

        Ok(())
    }

    // ------------------------------------------------------------------
    // 5. compact_with_summary: LLM failure returns error (does not corrupt thread)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_with_summary_llm_failure() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::failing("broken-llm"));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(8);
        let original_len = thread.turns.len();

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Summarize { keep_recent: 3 },
                None,
            )
            .await;

        // The LLM failure should propagate as an error
        assert!(result.is_err());

        // The thread should NOT have been modified (turns not truncated
        // on failure, since the error occurs before truncation)
        assert_eq!(thread.turns.len(), original_len);

        Ok(())
    }

    // ------------------------------------------------------------------
    // 6. compact_with_summary: fewer turns than keep_recent is a no-op
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_with_summary_fewer_turns_than_keep() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("should not be called"));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(3);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Summarize { keep_recent: 5 },
                None,
            )
            .await?;

        // No turns removed, LLM never called
        assert_eq!(thread.turns.len(), 3);
        assert_eq!(result.turns_removed, 0);
        assert!(result.summary.is_none());
        assert_eq!(llm.calls(), 0);

        Ok(())
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_compact_with_summary_preserves_turns_when_workspace_write_fails()
    -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("summary"));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(8);
        let original_inputs: Vec<String> =
            thread.turns.iter().map(|t| t.user_input.clone()).collect();
        let workspace = make_unmigrated_workspace().await?;

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Summarize { keep_recent: 3 },
                Some(&workspace),
            )
            .await?;

        // On archival failure, no turns should be removed.
        assert_eq!(thread.turns.len(), 8);
        assert_eq!(
            thread
                .turns
                .iter()
                .map(|t| t.user_input.as_str())
                .collect::<Vec<_>>(),
            original_inputs
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.turns_removed, 0);
        assert!(!result.summary_written);
        assert_eq!(llm.calls(), 1);

        Ok(())
    }

    // ------------------------------------------------------------------
    // 7. compact_to_workspace without workspace falls back to truncation
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_to_workspace_without_workspace_falls_back() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(20);

        let result = compactor
            .compact(&mut thread, CompactionStrategy::MoveToWorkspace, None)
            .await?;

        // Without a workspace, compact_to_workspace falls back to truncation
        // keeping 5 turns (the hardcoded fallback in the code)
        assert_eq!(thread.turns.len(), 5);
        assert_eq!(result.turns_removed, 15);

        // The remaining turns should be the last 5
        assert_eq!(thread.turns[0].user_input, "msg-15");
        assert_eq!(thread.turns[4].user_input, "msg-19");

        Ok(())
    }

    // ------------------------------------------------------------------
    // 8. compact_to_workspace: fewer turns than keep is a no-op
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_to_workspace_fewer_turns_noop() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        // MoveToWorkspace keeps 10 turns when workspace is available.
        // Without workspace it falls back to truncate(5).
        // With fewer turns, test the no-workspace fallback path:
        let mut thread = make_thread(4);

        let result = compactor
            .compact(&mut thread, CompactionStrategy::MoveToWorkspace, None)
            .await?;

        // 4 turns < 5 (fallback keep_recent), so no truncation
        assert_eq!(thread.turns.len(), 4);
        assert_eq!(result.turns_removed, 0);

        Ok(())
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_compact_to_workspace_preserves_turns_when_workspace_write_fails()
    -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(20);
        let original_inputs: Vec<String> =
            thread.turns.iter().map(|t| t.user_input.clone()).collect();
        let workspace = make_unmigrated_workspace().await?;

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::MoveToWorkspace,
                Some(&workspace),
            )
            .await?;

        // On archival failure, no turns should be removed.
        assert_eq!(thread.turns.len(), 20);
        assert_eq!(
            thread
                .turns
                .iter()
                .map(|t| t.user_input.as_str())
                .collect::<Vec<_>>(),
            original_inputs
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.turns_removed, 0);
        assert!(!result.summary_written);
        assert_eq!(llm.calls(), 0);

        Ok(())
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_move_to_workspace_extracts_structured_memories() -> anyhow::Result<()> {
        let memory_json = r#"{
            "memories": [
                {
                    "kind": "preference",
                    "content": "User prefers concise answers.",
                    "confidence": 0.96,
                    "evidence": "Please keep responses brief."
                }
            ]
        }"#;
        let llm = Arc::new(StubLlm::new(memory_json));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(12);
        let (workspace, _tmp) = make_test_workspace().await?;

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::MoveToWorkspace,
                Some(&workspace),
            )
            .await?;

        assert_eq!(result.turns_removed, 2);
        assert_eq!(result.memories_written, 1);
        assert_eq!(llm.calls(), 1);
        assert_eq!(thread.turns.len(), 10);

        let memory_doc = workspace.read(paths::MEMORY).await?;
        assert!(memory_doc.content.contains("Structured Memory Extraction"));
        assert!(memory_doc.content.contains("User prefers concise answers."));
        assert!(memory_doc.content.contains("preference"));

        Ok(())
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_move_to_workspace_skips_duplicate_structured_memories() -> anyhow::Result<()> {
        let memory_json = r#"{
            "memories": [
                {
                    "kind": "preference",
                    "content": "User prefers concise answers.",
                    "confidence": 0.96,
                    "evidence": "Please keep responses brief."
                }
            ]
        }"#;
        let llm = Arc::new(StubLlm::new(memory_json));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(12);
        let (workspace, _tmp) = make_test_workspace().await?;

        workspace
            .append_memory(
                "## Structured Memory Extraction (2026-03-23 00:00 UTC)\n\n- **preference**: User prefers concise answers.\n  - Confidence: 0.96",
            )
            .await
            ?;

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::MoveToWorkspace,
                Some(&workspace),
            )
            .await?;

        assert_eq!(result.turns_removed, 2);
        assert_eq!(result.memories_written, 0);
        assert_eq!(llm.calls(), 1);
        assert_eq!(thread.turns.len(), 10);

        let memory_doc = workspace.read(paths::MEMORY).await?;
        assert_eq!(
            memory_doc
                .content
                .matches("User prefers concise answers.")
                .count(),
            1
        );

        Ok(())
    }

    // ------------------------------------------------------------------
    // 9. format_turns_for_storage includes tool calls
    // ------------------------------------------------------------------

    #[test]
    fn test_format_turns_for_storage_with_tool_calls() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("Search for X");
        // Record a tool call on the current turn
        if let Some(turn) = thread.turns.last_mut() {
            turn.record_tool_call("search", serde_json::json!({"query": "X"}));
        }
        thread.complete_turn("Found X");

        let formatted = format_turns_for_storage(&thread.turns);
        assert!(formatted.contains("Turn 1"));
        assert!(formatted.contains("Search for X"));
        assert!(formatted.contains("Found X"));
        assert!(formatted.contains("Tools: search"));
    }

    // ------------------------------------------------------------------
    // 10. format_turns_for_storage with no response (incomplete turn)
    // ------------------------------------------------------------------

    #[test]
    fn test_format_turns_for_storage_incomplete_turn() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("In progress message");
        // Don't complete the turn

        let formatted = format_turns_for_storage(&thread.turns);
        assert!(formatted.contains("Turn 1"));
        assert!(formatted.contains("In progress message"));
        // No "Agent:" line since response is None
        assert!(!formatted.contains("Agent:"));
    }

    // ------------------------------------------------------------------
    // 11. format_turns_for_storage empty list
    // ------------------------------------------------------------------

    #[test]
    fn test_format_turns_for_storage_empty() {
        let formatted = format_turns_for_storage(&[]);
        assert!(formatted.is_empty());
    }

    // ------------------------------------------------------------------
    // 12. Token counts decrease after truncation
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_tokens_decrease_after_compaction() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(20);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 5 },
                None,
            )
            .await?;

        assert!(
            result.tokens_after < result.tokens_before,
            "tokens_after ({}) should be less than tokens_before ({})",
            result.tokens_after,
            result.tokens_before
        );

        Ok(())
    }

    // ------------------------------------------------------------------
    // 13. compact_with_summary: keep_recent=0 removes all turns
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_truncate_keep_zero() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(5);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 0 },
                None,
            )
            .await?;

        assert!(thread.turns.is_empty());
        assert_eq!(result.turns_removed, 5);
        assert_eq!(result.tokens_after, 0);

        Ok(())
    }

    // ------------------------------------------------------------------
    // 14. Summarize with keep_recent=0 summarizes all and removes all
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_with_summary_keep_zero() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("Summary of all turns"));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(5);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Summarize { keep_recent: 0 },
                None,
            )
            .await?;

        assert!(thread.turns.is_empty());
        assert_eq!(result.turns_removed, 5);
        assert!(result.summary.is_some());
        assert_eq!(
            result
                .summary
                .ok_or_else(|| anyhow::anyhow!("missing summary"))?,
            "Summary of all turns"
        );
        assert_eq!(llm.calls(), 1);

        Ok(())
    }

    // ------------------------------------------------------------------
    // 15. Messages are correctly built from turns for thread.messages()
    //     after compaction
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_messages_coherent_after_compaction() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(10);

        compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 3 },
                None,
            )
            .await?;

        let messages = thread.messages();
        // 3 turns * 2 messages each (user + assistant) = 6
        assert_eq!(messages.len(), 6);

        // Verify alternating user/assistant pattern
        for (i, msg) in messages.iter().enumerate() {
            if i % 2 == 0 {
                assert_eq!(msg.role, crate::llm::Role::User);
            } else {
                assert_eq!(msg.role, crate::llm::Role::Assistant);
            }
        }

        // Verify content matches the last 3 original turns
        assert_eq!(messages[0].content, "msg-7");
        assert_eq!(messages[1].content, "resp-7");
        assert_eq!(messages[4].content, "msg-9");
        assert_eq!(messages[5].content, "resp-9");

        Ok(())
    }

    // ------------------------------------------------------------------
    // 16. Multiple sequential compactions work correctly
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_sequential_compactions() -> anyhow::Result<()> {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(20);

        // First compaction: 20 -> 10
        let r1 = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 10 },
                None,
            )
            .await?;
        assert_eq!(thread.turns.len(), 10);
        assert_eq!(r1.turns_removed, 10);

        // Second compaction: 10 -> 3
        let r2 = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 3 },
                None,
            )
            .await?;
        assert_eq!(thread.turns.len(), 3);
        assert_eq!(r2.turns_removed, 7);

        // The remaining turns should be the very last 3 from the original 20
        assert_eq!(thread.turns[0].user_input, "msg-17");
        assert_eq!(thread.turns[1].user_input, "msg-18");
        assert_eq!(thread.turns[2].user_input, "msg-19");

        Ok(())
    }
}
