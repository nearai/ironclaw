//! Session summary hook -- writes a conversation summary on session end.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Semaphore;

use ironclaw_safety::Sanitizer;

use crate::db::ConversationStore;
use crate::hooks::hook::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint,
};
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};
use crate::tools::builtin::memory::WorkspaceResolver;

/// Maximum number of concurrent LLM summarization calls.
/// Prevents thundering herd when many sessions expire at once (e.g. restart after idle).
const MAX_CONCURRENT_SUMMARIES: usize = 3;

/// Writes a conversation summary to workspace when a session ends.
///
/// Uses the LLM to generate a brief summary of the most recent
/// conversation, then appends it to `daily/{date}-session-summary.md`.
pub struct SessionSummaryHook {
    store: Arc<dyn ConversationStore>,
    workspace_resolver: Arc<dyn WorkspaceResolver>,
    llm: Arc<dyn LlmProvider>,
    semaphore: Arc<Semaphore>,
}

impl SessionSummaryHook {
    pub fn new(
        store: Arc<dyn ConversationStore>,
        workspace_resolver: Arc<dyn WorkspaceResolver>,
        llm: Arc<dyn LlmProvider>,
    ) -> Self {
        Self {
            store,
            workspace_resolver,
            llm,
            semaphore: Arc::new(Semaphore::new(MAX_CONCURRENT_SUMMARIES)),
        }
    }
}

#[async_trait]
impl Hook for SessionSummaryHook {
    fn name(&self) -> &str {
        "session_summary"
    }

    fn hook_points(&self) -> &[HookPoint] {
        &[HookPoint::OnSessionEnd]
    }

    fn failure_mode(&self) -> HookFailureMode {
        HookFailureMode::FailOpen
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let (user_id, thread_ids) = match event {
            HookEvent::SessionEnd {
                user_id,
                thread_ids,
                ..
            } => (user_id, thread_ids),
            _ => return Ok(HookOutcome::ok()),
        };

        // Use thread_ids from the session when available. Each thread_id
        // doubles as a conversation_id in the conversations table.
        // Pick the first thread with enough messages.
        // Fallback: most-recent conversation (legacy path for events
        // fired without thread_ids populated).
        let messages = if !thread_ids.is_empty() {
            let mut best = Vec::new();
            for tid in thread_ids {
                match self.store.list_conversation_messages(*tid).await {
                    Ok(msgs) if msgs.len() >= 3 && msgs.len() > best.len() => {
                        best = msgs;
                    }
                    _ => {}
                }
            }
            best
        } else {
            let conversations = self
                .store
                .list_conversations_all_channels(user_id, 1)
                .await
                .map_err(|e| HookError::ExecutionFailed {
                    reason: format!("Failed to list conversations: {e}"),
                })?;
            match conversations.first() {
                Some(c) => self
                    .store
                    .list_conversation_messages(c.id)
                    .await
                    .map_err(|e| HookError::ExecutionFailed {
                        reason: format!("Failed to load messages: {e}"),
                    })?,
                None => return Ok(HookOutcome::ok()),
            }
        };

        if messages.len() < 3 {
            tracing::debug!(
                user_id = %user_id,
                message_count = messages.len(),
                "Skipping session summary: too few messages"
            );
            return Ok(HookOutcome::ok());
        }

        let transcript: String = messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        // Truncate to avoid sending too much to the LLM.
        let truncated = if transcript.len() > 8000 {
            &transcript[..transcript.floor_char_boundary(8000)]
        } else {
            &transcript
        };

        let llm_messages = vec![
            ChatMessage::system(include_str!(
                "../../crates/ironclaw_engine/prompts/session_summary.md"
            )),
            ChatMessage::user(truncated.to_string()),
        ];

        let request = CompletionRequest::new(llm_messages).with_max_tokens(300);

        // Acquire a permit to cap concurrent LLM calls across hook instances.
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| HookError::ExecutionFailed {
                reason: "Semaphore closed".to_string(),
            })?;

        let response =
            self.llm
                .complete(request)
                .await
                .map_err(|e| HookError::ExecutionFailed {
                    reason: format!("LLM summarization failed: {e}"),
                })?;

        let summary = response.content.trim();
        if summary.is_empty() {
            return Ok(HookOutcome::ok());
        }

        // Sanitize the LLM-generated summary before persisting to workspace.
        // Conversation content may contain attacker-controlled text that flows
        // through the summary and could be retrieved into future LLM contexts.
        let sanitizer = Sanitizer::new();
        let sanitized = sanitizer.sanitize(summary);
        if sanitized.was_modified {
            tracing::debug!(
                user_id = %user_id,
                warnings = sanitized.warnings.len(),
                "Session summary contained suspicious patterns; content was sanitized"
            );
        }
        let summary = &sanitized.content;

        let date = chrono::Utc::now().format("%Y-%m-%d");
        let path = format!("daily/{date}-session-summary.md");
        let timestamp = chrono::Utc::now().format("%H:%M UTC");
        let entry = format!("\n## Session Summary ({timestamp})\n\n{summary}\n");

        let workspace = self.workspace_resolver.resolve(user_id).await;
        workspace
            .append(&path, &entry)
            .await
            .map_err(|e| HookError::ExecutionFailed {
                reason: format!("Failed to write session summary: {e}"),
            })?;

        tracing::debug!(
            user_id = %user_id,
            path = %path,
            "Session summary written to workspace"
        );

        Ok(HookOutcome::ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ConversationStore;
    use crate::history::{ConversationMessage, ConversationSummary};
    use crate::llm::{
        CompletionResponse, FinishReason, LlmError, ToolCompletionRequest, ToolCompletionResponse,
    };
    use crate::workspace::Workspace;
    use chrono::Utc;
    use rust_decimal::Decimal;
    use uuid::Uuid;

    // ── Mock ConversationStore ──────────────────────────────────────

    struct MockConversationStore {
        conversations: Vec<ConversationSummary>,
        messages: Vec<ConversationMessage>,
    }

    #[async_trait]
    impl ConversationStore for MockConversationStore {
        async fn create_conversation(
            &self,
            _channel: &str,
            _user_id: &str,
            _thread_id: Option<&str>,
        ) -> Result<Uuid, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn touch_conversation(&self, _id: Uuid) -> Result<(), crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn add_conversation_message(
            &self,
            _conversation_id: Uuid,
            _role: &str,
            _content: &str,
        ) -> Result<Uuid, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn add_conversation_message_if_empty(
            &self,
            _conversation_id: Uuid,
            _role: &str,
            _content: &str,
        ) -> Result<bool, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn ensure_conversation(
            &self,
            _id: Uuid,
            _channel: &str,
            _user_id: &str,
            _thread_id: Option<&str>,
            _source_channel: Option<&str>,
        ) -> Result<bool, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn list_conversations_with_preview(
            &self,
            _user_id: &str,
            _channel: &str,
            _limit: i64,
        ) -> Result<Vec<ConversationSummary>, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn list_conversations_all_channels(
            &self,
            _user_id: &str,
            _limit: i64,
        ) -> Result<Vec<ConversationSummary>, crate::error::DatabaseError> {
            Ok(self.conversations.clone())
        }

        async fn get_or_create_routine_conversation(
            &self,
            _routine_id: Uuid,
            _routine_name: &str,
            _user_id: &str,
        ) -> Result<Uuid, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn find_routine_conversation(
            &self,
            _routine_id: Uuid,
            _user_id: &str,
        ) -> Result<Option<Uuid>, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn get_or_create_heartbeat_conversation(
            &self,
            _user_id: &str,
        ) -> Result<Uuid, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn get_or_create_assistant_conversation(
            &self,
            _user_id: &str,
            _channel: &str,
        ) -> Result<Uuid, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn create_conversation_with_metadata(
            &self,
            _channel: &str,
            _user_id: &str,
            _metadata: &serde_json::Value,
        ) -> Result<Uuid, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn list_conversation_messages_paginated(
            &self,
            _conversation_id: Uuid,
            _before: Option<chrono::DateTime<Utc>>,
            _limit: i64,
        ) -> Result<(Vec<ConversationMessage>, bool), crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn update_conversation_metadata_field(
            &self,
            _id: Uuid,
            _key: &str,
            _value: &serde_json::Value,
        ) -> Result<(), crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn get_conversation_metadata(
            &self,
            _id: Uuid,
        ) -> Result<Option<serde_json::Value>, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn list_conversation_messages(
            &self,
            _conversation_id: Uuid,
        ) -> Result<Vec<ConversationMessage>, crate::error::DatabaseError> {
            Ok(self.messages.clone())
        }

        async fn conversation_belongs_to_user(
            &self,
            _conversation_id: Uuid,
            _user_id: &str,
        ) -> Result<bool, crate::error::DatabaseError> {
            unimplemented!()
        }

        async fn get_conversation_source_channel(
            &self,
            _conversation_id: Uuid,
        ) -> Result<Option<String>, crate::error::DatabaseError> {
            unimplemented!()
        }
    }

    // ── Mock LlmProvider ────────────────────────────────────────────

    struct MockLlm {
        response: String,
    }

    #[async_trait]
    impl LlmProvider for MockLlm {
        fn model_name(&self) -> &str {
            "mock"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: self.response.clone(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            unimplemented!()
        }
    }

    /// Mock LLM that always fails.
    struct FailingMockLlm;

    #[async_trait]
    impl LlmProvider for FailingMockLlm {
        fn model_name(&self) -> &str {
            "failing-mock"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: "mock".into(),
                reason: "simulated LLM failure".into(),
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            unimplemented!()
        }
    }

    // ── Test helpers ─────────────────────────────────────────────────

    #[cfg(feature = "libsql")]
    async fn make_test_db() -> Arc<dyn crate::db::Database> {
        use crate::db::Database as _;

        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("test.db");
        let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
            .await
            .expect("local libsql");
        backend.run_migrations().await.expect("migrations");
        // Leak the tempdir so it outlives the test (cleaned up on process exit).
        std::mem::forget(tmp);
        Arc::new(backend)
    }

    #[cfg(feature = "libsql")]
    async fn make_dummy_workspace() -> Arc<Workspace> {
        let db = make_test_db().await;
        Arc::new(Workspace::new_with_db("test_dummy", db))
    }

    #[cfg(all(feature = "postgres", not(feature = "libsql")))]
    async fn make_dummy_workspace() -> Arc<Workspace> {
        Arc::new(Workspace::new(
            "test_dummy",
            deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
                tokio_postgres::Config::new(),
                tokio_postgres::NoTls,
            ))
            .build()
            .unwrap(),
        ))
    }

    fn make_mock_hook(
        store: Arc<dyn ConversationStore>,
        resolver: Arc<dyn WorkspaceResolver>,
    ) -> SessionSummaryHook {
        let llm: Arc<dyn LlmProvider> = Arc::new(MockLlm {
            response: String::new(),
        });
        SessionSummaryHook::new(store, resolver, llm)
    }

    // ── Unit tests for hook metadata ────────────────────────────────

    #[tokio::test]
    async fn hook_metadata_is_correct() {
        let ws = make_dummy_workspace().await;
        let store: Arc<dyn ConversationStore> = Arc::new(MockConversationStore {
            conversations: vec![],
            messages: vec![],
        });
        let resolver: Arc<dyn WorkspaceResolver> = Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(ws),
        );
        let hook = make_mock_hook(store, resolver);

        assert_eq!(hook.name(), "session_summary");
        assert_eq!(hook.hook_points(), &[HookPoint::OnSessionEnd]);
        assert_eq!(hook.failure_mode(), HookFailureMode::FailOpen);
        assert_eq!(hook.timeout(), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn skips_non_session_end_events() {
        let ws = make_dummy_workspace().await;
        let store: Arc<dyn ConversationStore> = Arc::new(MockConversationStore {
            conversations: vec![],
            messages: vec![],
        });
        let resolver: Arc<dyn WorkspaceResolver> = Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(ws),
        );
        let hook = make_mock_hook(store, resolver);

        let event = HookEvent::SessionStart {
            user_id: "user1".into(),
            session_id: "sess1".into(),
        };
        let ctx = HookContext::default();
        let outcome = hook.execute(&event, &ctx).await.unwrap();
        assert!(matches!(outcome, HookOutcome::Continue { modified: None }));
    }

    #[tokio::test]
    async fn skips_when_no_conversations() {
        let ws = make_dummy_workspace().await;
        let store: Arc<dyn ConversationStore> = Arc::new(MockConversationStore {
            conversations: vec![],
            messages: vec![],
        });
        let resolver: Arc<dyn WorkspaceResolver> = Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(ws),
        );
        let hook = make_mock_hook(store, resolver);

        let event = HookEvent::SessionEnd {
            user_id: "user1".into(),
            session_id: "sess1".into(),
            thread_ids: vec![],
        };
        let ctx = HookContext::default();
        let outcome = hook.execute(&event, &ctx).await.unwrap();
        assert!(matches!(outcome, HookOutcome::Continue { modified: None }));
    }

    #[tokio::test]
    async fn skips_when_too_few_messages() {
        let ws = make_dummy_workspace().await;
        let conv_id = Uuid::new_v4();
        let store: Arc<dyn ConversationStore> = Arc::new(MockConversationStore {
            conversations: vec![ConversationSummary {
                id: conv_id,
                title: Some("Test".into()),
                message_count: 2,
                started_at: Utc::now(),
                last_activity: Utc::now(),
                thread_type: None,
                channel: "test".into(),
            }],
            messages: vec![
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "user".into(),
                    content: "Hello".into(),
                    created_at: Utc::now(),
                },
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "assistant".into(),
                    content: "Hi".into(),
                    created_at: Utc::now(),
                },
            ],
        });
        let resolver: Arc<dyn WorkspaceResolver> = Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(ws),
        );
        let hook = make_mock_hook(store, resolver);

        let event = HookEvent::SessionEnd {
            user_id: "user1".into(),
            session_id: "sess1".into(),
            thread_ids: vec![conv_id],
        };
        let ctx = HookContext::default();
        let outcome = hook.execute(&event, &ctx).await.unwrap();
        assert!(matches!(outcome, HookOutcome::Continue { modified: None }));
    }

    /// Integration test: full hook execution with libsql backend.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn writes_summary_to_workspace() {
        let db = make_test_db().await;

        let conv_id = Uuid::new_v4();
        let store: Arc<dyn ConversationStore> = Arc::new(MockConversationStore {
            conversations: vec![ConversationSummary {
                id: conv_id,
                title: Some("Test conversation".into()),
                message_count: 4,
                started_at: Utc::now(),
                last_activity: Utc::now(),
                thread_type: None,
                channel: "test".into(),
            }],
            messages: vec![
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "user".into(),
                    content: "Can you help me plan the project?".into(),
                    created_at: Utc::now(),
                },
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "assistant".into(),
                    content: "Sure! Let me outline the key milestones.".into(),
                    created_at: Utc::now(),
                },
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "user".into(),
                    content: "Focus on the backend first.".into(),
                    created_at: Utc::now(),
                },
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "assistant".into(),
                    content: "Got it. Backend priorities: API design, database schema, auth."
                        .into(),
                    created_at: Utc::now(),
                },
            ],
        });

        let llm: Arc<dyn LlmProvider> = Arc::new(MockLlm {
            response: "- Decided to focus on backend first\n- Key priorities: API design, database schema, auth\n- Project planning initiated".into(),
        });

        let ws = Arc::new(Workspace::new_with_db("test_user", Arc::clone(&db)));
        let resolver: Arc<dyn WorkspaceResolver> = Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(ws.clone()),
        );

        let hook = SessionSummaryHook::new(store, resolver, llm);

        let event = HookEvent::SessionEnd {
            user_id: "test_user".into(),
            session_id: "sess1".into(),
            thread_ids: vec![conv_id],
        };
        let ctx = HookContext::default();
        let outcome = hook.execute(&event, &ctx).await.unwrap();
        assert!(matches!(outcome, HookOutcome::Continue { modified: None }));

        // Verify the summary was written to workspace.
        let date = chrono::Utc::now().format("%Y-%m-%d");
        let path = format!("daily/{date}-session-summary.md");
        let doc = ws.read(&path).await.unwrap();
        assert!(
            doc.content.contains("Session Summary"),
            "Expected session summary header in workspace doc, got: {}",
            doc.content
        );
        assert!(
            doc.content.contains("backend"),
            "Expected summary content in workspace doc, got: {}",
            doc.content
        );
    }

    /// LLM failure should propagate as HookError (fail-open mode means the
    /// hook runner won't abort the session, but the hook itself returns Err).
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn llm_failure_returns_error() {
        let db = make_test_db().await;
        let conv_id = Uuid::new_v4();

        let store: Arc<dyn ConversationStore> = Arc::new(MockConversationStore {
            conversations: vec![ConversationSummary {
                id: conv_id,
                title: Some("Test".into()),
                message_count: 4,
                started_at: Utc::now(),
                last_activity: Utc::now(),
                thread_type: None,
                channel: "test".into(),
            }],
            messages: vec![
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "user".into(),
                    content: "First message".into(),
                    created_at: Utc::now(),
                },
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "assistant".into(),
                    content: "Reply one".into(),
                    created_at: Utc::now(),
                },
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "user".into(),
                    content: "Second message".into(),
                    created_at: Utc::now(),
                },
                ConversationMessage {
                    id: Uuid::new_v4(),
                    role: "assistant".into(),
                    content: "Reply two".into(),
                    created_at: Utc::now(),
                },
            ],
        });

        let llm: Arc<dyn LlmProvider> = Arc::new(FailingMockLlm);
        let ws = Arc::new(Workspace::new_with_db("test_user", Arc::clone(&db)));
        let resolver: Arc<dyn WorkspaceResolver> = Arc::new(
            crate::tools::builtin::memory::FixedWorkspaceResolver::new(ws),
        );

        let hook = SessionSummaryHook::new(store, resolver, llm);

        let event = HookEvent::SessionEnd {
            user_id: "test_user".into(),
            session_id: "sess_fail".into(),
            thread_ids: vec![conv_id],
        };
        let ctx = HookContext::default();

        // The hook should return an error (LLM summarization failed).
        // Because failure_mode is FailOpen, the hook runner will swallow
        // this error and continue — but the hook itself propagates it.
        let result = hook.execute(&event, &ctx).await;
        assert!(result.is_err(), "Expected error from failing LLM, got Ok");
        let err = result.unwrap_err();
        assert!(
            matches!(err, HookError::ExecutionFailed { .. }),
            "Expected ExecutionFailed, got: {err:?}"
        );
    }
}
