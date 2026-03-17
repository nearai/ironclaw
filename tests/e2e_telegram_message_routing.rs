//! E2E tests for Telegram message routing through the real agent + message tool.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use futures::StreamExt;
    use ironclaw::agent::{Agent, AgentDeps};
    use ironclaw::app::{AppBuilder, AppBuilderFlags};
    use ironclaw::channels::web::log_layer::LogBroadcaster;
    use ironclaw::channels::{
        Channel, ChannelManager, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    };
    use ironclaw::config::Config;
    use ironclaw::db::{Database, libsql::LibSqlBackend};
    use ironclaw::error::ChannelError;
    use ironclaw::llm::{LlmProvider, SessionConfig, SessionManager};
    use tokio::sync::{Mutex, mpsc};
    use tokio_stream::wrappers::ReceiverStream;

    use crate::support::test_channel::{TestChannel, TestChannelHandle};
    use crate::support::trace_llm::{LlmTrace, TraceLlm, TraceResponse, TraceStep, TraceToolCall};

    type TelegramCaptures = Arc<Mutex<Vec<(String, OutgoingResponse)>>>;

    struct RecordingTelegramChannel {
        captures: TelegramCaptures,
    }

    impl RecordingTelegramChannel {
        fn new() -> (Self, TelegramCaptures) {
            let captures = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    captures: Arc::clone(&captures),
                },
                captures,
            )
        }
    }

    #[async_trait]
    impl Channel for RecordingTelegramChannel {
        fn name(&self) -> &str {
            "telegram"
        }

        async fn start(&self) -> Result<MessageStream, ChannelError> {
            let (_tx, rx) = mpsc::channel::<IncomingMessage>(1);
            Ok(ReceiverStream::new(rx).boxed())
        }

        async fn respond(
            &self,
            _msg: &IncomingMessage,
            response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            self.captures
                .lock()
                .await
                .push(("respond".to_string(), response));
            Ok(())
        }

        async fn send_status(
            &self,
            _status: StatusUpdate,
            _metadata: &serde_json::Value,
        ) -> Result<(), ChannelError> {
            Ok(())
        }

        async fn broadcast(
            &self,
            user_id: &str,
            response: OutgoingResponse,
        ) -> Result<(), ChannelError> {
            self.captures
                .lock()
                .await
                .push((user_id.to_string(), response));
            Ok(())
        }

        async fn health_check(&self) -> Result<(), ChannelError> {
            Ok(())
        }
    }

    struct Harness {
        gateway: Arc<TestChannel>,
        telegram_captures: Arc<Mutex<Vec<(String, OutgoingResponse)>>>,
        db: Arc<dyn Database>,
        owner_id: String,
        _temp_dir: tempfile::TempDir,
        agent_handle: Option<tokio::task::JoinHandle<()>>,
    }

    impl Harness {
        async fn store_telegram_owner_binding(&self, owner_id: i64) {
            for scope in [&self.owner_id, "test-user"] {
                self.db
                    .set_setting(
                        scope,
                        "channels.wasm_channel_owner_ids.telegram",
                        &serde_json::json!(owner_id),
                    )
                    .await
                    .expect("failed to store telegram owner binding");
            }
        }

        async fn wait_for_telegram_broadcasts(
            &self,
            expected: usize,
            timeout: Duration,
        ) -> Vec<(String, OutgoingResponse)> {
            let deadline = tokio::time::Instant::now() + timeout;
            loop {
                let snapshot = self.telegram_captures.lock().await.clone();
                if snapshot.len() >= expected || tokio::time::Instant::now() >= deadline {
                    return snapshot;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.gateway.signal_shutdown();
            if let Some(handle) = self.agent_handle.take() {
                handle.abort();
            }
        }
    }

    async fn build_harness(trace: LlmTrace) -> Harness {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = temp_dir.path().join("telegram_message_routing.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("failed to create test LibSqlBackend");
        backend
            .run_migrations()
            .await
            .expect("failed to run migrations");
        let db: Arc<dyn Database> = Arc::new(backend);

        let skills_dir = temp_dir.path().join("skills");
        let installed_skills_dir = temp_dir.path().join("installed_skills");
        let _ = std::fs::create_dir_all(&skills_dir);
        let _ = std::fs::create_dir_all(&installed_skills_dir);
        let mut config = Config::for_testing(db_path, skills_dir, installed_skills_dir);
        config.agent.auto_approve_tools = true;

        let session = Arc::new(SessionManager::new(SessionConfig::default()));
        let log_broadcaster = Arc::new(LogBroadcaster::new());
        let llm: Arc<dyn LlmProvider> = Arc::new(TraceLlm::from_trace(trace));

        let mut builder = AppBuilder::new(
            config,
            AppBuilderFlags::default(),
            None,
            session,
            log_broadcaster,
        );
        builder.with_database(Arc::clone(&db));
        builder.with_llm(llm);

        let mut components = builder
            .build_all()
            .await
            .expect("AppBuilder::build_all() failed");
        components.config.agent.auto_approve_tools = true;
        components.config.agent.allow_local_tools = true;

        let deps = AgentDeps {
            owner_id: components.config.owner_id.clone(),
            store: components.db.clone(),
            llm: components.llm.clone(),
            cheap_llm: components.cheap_llm.clone(),
            safety: components.safety.clone(),
            tools: components.tools.clone(),
            workspace: components.workspace.clone(),
            extension_manager: components.extension_manager.clone(),
            skill_registry: components.skill_registry.clone(),
            skill_catalog: components.skill_catalog.clone(),
            skills_config: components.config.skills.clone(),
            hooks: components.hooks.clone(),
            cost_guard: components.cost_guard.clone(),
            sse_tx: None,
            http_interceptor: None,
            transcription: None,
            document_extraction: None,
        };

        let gateway = Arc::new(TestChannel::new());
        let gateway_handle = TestChannelHandle::new(Arc::clone(&gateway));
        let (telegram_channel, telegram_captures) = RecordingTelegramChannel::new();

        let channel_manager = ChannelManager::new();
        channel_manager.add(Box::new(gateway_handle)).await;
        channel_manager.add(Box::new(telegram_channel)).await;
        let channels = Arc::new(channel_manager);

        deps.tools
            .register_message_tools(Arc::clone(&channels), deps.extension_manager.clone())
            .await;

        let agent = Agent::new(
            components.config.agent.clone(),
            deps,
            channels,
            None,
            None,
            None,
            Some(Arc::clone(&components.context_manager)),
            None,
        );

        let agent_handle = tokio::spawn(async move {
            if let Err(err) = agent.run().await {
                eprintln!("[telegram routing e2e] Agent exited with error: {err}");
            }
        });

        if let Some(rx) = gateway.take_ready_rx().await {
            let _ = tokio::time::timeout(Duration::from_secs(5), rx).await;
        }

        Harness {
            gateway,
            telegram_captures,
            db,
            owner_id: components.config.owner_id.clone(),
            _temp_dir: temp_dir,
            agent_handle: Some(agent_handle),
        }
    }

    fn single_message_trace(arguments: serde_json::Value, final_text: &str) -> LlmTrace {
        LlmTrace::single_turn(
            "telegram-message-routing",
            "send a reminder",
            vec![
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::ToolCalls {
                        tool_calls: vec![TraceToolCall {
                            id: "call_message_1".to_string(),
                            name: "message".to_string(),
                            arguments,
                        }],
                        input_tokens: 32,
                        output_tokens: 12,
                    },
                    expected_tool_results: Vec::new(),
                },
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::Text {
                        content: final_text.to_string(),
                        input_tokens: 24,
                        output_tokens: 8,
                    },
                    expected_tool_results: Vec::new(),
                },
            ],
        )
    }

    #[tokio::test]
    async fn telegram_message_tool_uses_bound_owner_target_when_target_omitted() {
        let harness = build_harness(single_message_trace(
            serde_json::json!({
                "content": "Walk Conan",
                "channel": "telegram",
            }),
            "Sent on Telegram.",
        ))
        .await;

        harness.store_telegram_owner_binding(424242).await;

        harness
            .gateway
            .send_message("remind me to walk conan")
            .await;
        let responses = harness
            .gateway
            .wait_for_responses(1, Duration::from_secs(10))
            .await;
        assert!(
            responses
                .iter()
                .any(|response| response.content.contains("Sent on Telegram")),
            "expected assistant confirmation, got: {:?}",
            responses
                .iter()
                .map(|response| &response.content)
                .collect::<Vec<_>>()
        );

        let broadcasts = harness
            .wait_for_telegram_broadcasts(1, Duration::from_secs(10))
            .await;
        assert_eq!(
            broadcasts.len(),
            1,
            "expected exactly one telegram broadcast"
        );
        assert_eq!(broadcasts[0].0, "424242");
        assert_eq!(broadcasts[0].1.content, "Walk Conan");
    }

    #[tokio::test]
    async fn telegram_message_tool_prefers_explicit_target_over_bound_owner_target() {
        let harness = build_harness(single_message_trace(
            serde_json::json!({
                "content": "Walk Conan",
                "channel": "telegram",
                "target": "999999",
            }),
            "Sent on Telegram.",
        ))
        .await;

        harness.store_telegram_owner_binding(424242).await;

        harness.gateway.send_message("send the reminder").await;
        let _ = harness
            .gateway
            .wait_for_responses(1, Duration::from_secs(10))
            .await;

        let broadcasts = harness
            .wait_for_telegram_broadcasts(1, Duration::from_secs(10))
            .await;
        assert_eq!(
            broadcasts.len(),
            1,
            "expected exactly one telegram broadcast"
        );
        assert_eq!(broadcasts[0].0, "999999");
        assert_eq!(broadcasts[0].1.content, "Walk Conan");
    }
}
