//! Session summary hook -- writes a conversation summary on session end.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::db::ConversationStore;
use crate::hooks::hook::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint,
};
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider, Role};
use crate::tools::builtin::memory::WorkspaceResolver;

/// Writes a conversation summary to workspace when a session ends.
///
/// Uses the LLM to generate a brief summary of the most recent
/// conversation, then appends it to `daily/{date}-session-summary.md`.
pub struct SessionSummaryHook {
    store: Arc<dyn ConversationStore>,
    workspace_resolver: Arc<dyn WorkspaceResolver>,
    llm: Arc<dyn LlmProvider>,
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
        let (user_id, _session_id) = match event {
            HookEvent::SessionEnd {
                user_id,
                session_id,
            } => (user_id, session_id),
            _ => return Ok(HookOutcome::ok()),
        };

        let conversations = self
            .store
            .list_conversations_all_channels(user_id, 1)
            .await
            .map_err(|e| HookError::ExecutionFailed {
                reason: format!("Failed to list conversations: {e}"),
            })?;

        let conversation = match conversations.first() {
            Some(c) => c,
            None => return Ok(HookOutcome::ok()),
        };

        let messages = self
            .store
            .list_conversation_messages(conversation.id)
            .await
            .map_err(|e| HookError::ExecutionFailed {
                reason: format!("Failed to load messages: {e}"),
            })?;

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
            ChatMessage {
                role: Role::System,
                content: "You are a concise summarizer. Summarize the key decisions, action items, and context from this conversation in 3-5 bullet points. Be brief.".into(),
                content_parts: Vec::new(),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
            ChatMessage {
                role: Role::User,
                content: truncated.to_string(),
                content_parts: Vec::new(),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            },
        ];

        let request = CompletionRequest::new(llm_messages).with_max_tokens(300);

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
}
