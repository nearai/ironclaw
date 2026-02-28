//! Context compaction for preserving and summarizing conversation history.
//!
//! When the context window approaches its limit, compaction:
//! 1. Summarizes old turns
//! 2. Writes the summary to the workspace daily log
//! 3. Trims the context to keep only recent turns

use std::sync::Arc;

use chrono::Utc;

use crate::agent::context_monitor::{CompactionStrategy, ContextBreakdown};
use crate::agent::session::Thread;
use crate::error::Error;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider, Reasoning};
use crate::safety::SafetyLayer;
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
    /// The generated summary (if any).
    pub summary: Option<String>,
}

/// Compacts conversation context to stay within limits.
pub struct ContextCompactor {
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
}

impl ContextCompactor {
    /// Create a new context compactor.
    pub fn new(llm: Arc<dyn LlmProvider>, safety: Arc<SafetyLayer>) -> Self {
        Self { llm, safety }
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
                self.compact_truncate(thread, keep_recent)
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

        // Write to workspace if available
        let summary_written = if let Some(ws) = workspace {
            match self.write_summary_to_workspace(ws, &summary).await {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!(
                        "Compaction summary write failed (turns will still be truncated): {}",
                        e
                    );
                    false
                }
            }
        } else {
            false
        };

        // Truncate thread
        thread.truncate_turns(keep_recent);

        Ok(CompactionPartial {
            turns_removed: turns_to_remove,
            summary_written,
            summary: Some(summary),
        })
    }

    /// Compact by simple truncation (no summary).
    fn compact_truncate(&self, thread: &mut Thread, keep_recent: usize) -> CompactionPartial {
        let turns_before = thread.turns.len();
        thread.truncate_turns(keep_recent);
        let turns_removed = turns_before - thread.turns.len();

        CompactionPartial {
            turns_removed,
            summary_written: false,
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
            return Ok(self.compact_truncate(thread, 5));
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

        // Write to workspace
        let written = match self.write_context_to_workspace(ws, &content).await {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(
                    "Compaction context write failed (turns will still be truncated): {}",
                    e
                );
                false
            }
        };

        // Truncate
        thread.truncate_turns(keep_recent);

        Ok(CompactionPartial {
            turns_removed: turns_to_remove,
            summary_written: written,
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

        let reasoning = Reasoning::new(self.llm.clone(), self.safety.clone());
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
}

/// Partial result during compaction (internal).
struct CompactionPartial {
    turns_removed: usize,
    summary_written: bool,
    summary: Option<String>,
}

impl CompactionPartial {
    fn empty() -> Self {
        Self {
            turns_removed: 0,
            summary_written: false,
            summary: None,
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::Thread;
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
    use crate::config::SafetyConfig;
    use crate::safety::SafetyLayer;
    use crate::testing::StubLlm;

    /// Helper: build a `ContextCompactor` with the given `StubLlm`.
    fn make_compactor(llm: Arc<StubLlm>) -> ContextCompactor {
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        }));
        ContextCompactor::new(llm, safety)
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

    // ------------------------------------------------------------------
    // 1. compact_truncate keeps last N turns
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_truncate_keeps_last_n() {
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
            .await
            .expect("compact should succeed");

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
    }

    // ------------------------------------------------------------------
    // 2. compact_truncate with fewer turns than limit (no-op)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_truncate_with_fewer_turns_than_limit() {
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
            .await
            .expect("compact should succeed");

        // All turns preserved
        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].user_input, original_inputs[0]);
        assert_eq!(thread.turns[1].user_input, original_inputs[1]);

        // No turns removed
        assert_eq!(result.turns_removed, 0);
        assert!(!result.summary_written);
        assert!(result.summary.is_none());
    }

    // ------------------------------------------------------------------
    // 3. compact_truncate with empty turns list
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_truncate_empty_turns() {
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
            .await
            .expect("compact should succeed on empty turns");

        assert!(thread.turns.is_empty());
        assert_eq!(result.turns_removed, 0);
        assert_eq!(result.tokens_before, 0);
        assert_eq!(result.tokens_after, 0);
    }

    // ------------------------------------------------------------------
    // 4. compact_with_summary produces summary turn via StubLlm
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_with_summary_produces_summary_turn() {
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
            .await
            .expect("compact with summary should succeed");

        // Should keep only 2 recent turns
        assert_eq!(thread.turns.len(), 2);

        // The kept turns should be the last two (msg-3, msg-4)
        assert_eq!(thread.turns[0].user_input, "msg-3");
        assert_eq!(thread.turns[1].user_input, "msg-4");

        // Result should report the summary
        assert_eq!(result.turns_removed, 3);
        assert!(result.summary.is_some());
        let summary = result.summary.unwrap();
        assert!(summary.contains("User greeted the agent"));
        assert!(summary.contains("Five exchanges completed"));

        // summary_written should be false since no workspace was provided
        assert!(!result.summary_written);

        // StubLlm should have been called exactly once for the summary
        assert_eq!(llm.calls(), 1);
    }

    // ------------------------------------------------------------------
    // 5. compact_with_summary: LLM failure returns error (does not corrupt thread)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_with_summary_llm_failure() {
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
    }

    // ------------------------------------------------------------------
    // 6. compact_with_summary: fewer turns than keep_recent is a no-op
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_with_summary_fewer_turns_than_keep() {
        let llm = Arc::new(StubLlm::new("should not be called"));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(3);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Summarize { keep_recent: 5 },
                None,
            )
            .await
            .expect("compact should succeed");

        // No turns removed, LLM never called
        assert_eq!(thread.turns.len(), 3);
        assert_eq!(result.turns_removed, 0);
        assert!(result.summary.is_none());
        assert_eq!(llm.calls(), 0);
    }

    // ------------------------------------------------------------------
    // 7. compact_to_workspace without workspace falls back to truncation
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_to_workspace_without_workspace_falls_back() {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(20);

        let result = compactor
            .compact(&mut thread, CompactionStrategy::MoveToWorkspace, None)
            .await
            .expect("compact should succeed");

        // Without a workspace, compact_to_workspace falls back to truncation
        // keeping 5 turns (the hardcoded fallback in the code)
        assert_eq!(thread.turns.len(), 5);
        assert_eq!(result.turns_removed, 15);

        // The remaining turns should be the last 5
        assert_eq!(thread.turns[0].user_input, "msg-15");
        assert_eq!(thread.turns[4].user_input, "msg-19");
    }

    // ------------------------------------------------------------------
    // 8. compact_to_workspace: fewer turns than keep is a no-op
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_to_workspace_fewer_turns_noop() {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        // MoveToWorkspace keeps 10 turns when workspace is available.
        // Without workspace it falls back to truncate(5).
        // With fewer turns, test the no-workspace fallback path:
        let mut thread = make_thread(4);

        let result = compactor
            .compact(&mut thread, CompactionStrategy::MoveToWorkspace, None)
            .await
            .expect("compact should succeed");

        // 4 turns < 5 (fallback keep_recent), so no truncation
        assert_eq!(thread.turns.len(), 4);
        assert_eq!(result.turns_removed, 0);
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
            turn.record_tool_call(
                "search",
                serde_json::json!({"query": "X"}),
                "search for X".to_string(),
                None,
            );
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
    async fn test_tokens_decrease_after_compaction() {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(20);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 5 },
                None,
            )
            .await
            .expect("compact should succeed");

        assert!(
            result.tokens_after < result.tokens_before,
            "tokens_after ({}) should be less than tokens_before ({})",
            result.tokens_after,
            result.tokens_before
        );
    }

    // ------------------------------------------------------------------
    // 13. compact_with_summary: keep_recent=0 removes all turns
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_truncate_keep_zero() {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(5);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 0 },
                None,
            )
            .await
            .expect("compact should succeed");

        assert!(thread.turns.is_empty());
        assert_eq!(result.turns_removed, 5);
        assert_eq!(result.tokens_after, 0);
    }

    // ------------------------------------------------------------------
    // 14. Summarize with keep_recent=0 summarizes all and removes all
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_compact_with_summary_keep_zero() {
        let llm = Arc::new(StubLlm::new("Summary of all turns"));
        let compactor = make_compactor(llm.clone());
        let mut thread = make_thread(5);

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Summarize { keep_recent: 0 },
                None,
            )
            .await
            .expect("compact should succeed");

        assert!(thread.turns.is_empty());
        assert_eq!(result.turns_removed, 5);
        assert!(result.summary.is_some());
        assert_eq!(result.summary.unwrap(), "Summary of all turns");
        assert_eq!(llm.calls(), 1);
    }

    // ------------------------------------------------------------------
    // 15. Messages are correctly built from turns for thread.messages()
    //     after compaction
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_messages_coherent_after_compaction() {
        let llm = Arc::new(StubLlm::new("unused"));
        let compactor = make_compactor(llm);
        let mut thread = make_thread(10);

        compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 3 },
                None,
            )
            .await
            .expect("compact should succeed");

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
    }

    // ------------------------------------------------------------------
    // 16. Multiple sequential compactions work correctly
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_sequential_compactions() {
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
            .await
            .expect("first compact");
        assert_eq!(thread.turns.len(), 10);
        assert_eq!(r1.turns_removed, 10);

        // Second compaction: 10 -> 3
        let r2 = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Truncate { keep_recent: 3 },
                None,
            )
            .await
            .expect("second compact");
        assert_eq!(thread.turns.len(), 3);
        assert_eq!(r2.turns_removed, 7);

        // The remaining turns should be the very last 3 from the original 20
        assert_eq!(thread.turns[0].user_input, "msg-17");
        assert_eq!(thread.turns[1].user_input, "msg-18");
        assert_eq!(thread.turns[2].user_input, "msg-19");
    }
}
