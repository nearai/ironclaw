mod tests {
    use std::sync::Arc;

    use crate::agent::Agent;
    use crate::agent::AgentDeps;
    use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
    use crate::agent::thread_ops::sanitize_pending_tool_rationale;
    use crate::channels::ChannelManager;
    use crate::channels::IncomingMessage;
    use crate::config::{AgentConfig, SafetyConfig, SkillsConfig};
    use crate::context::ContextManager;
    use crate::hooks::HookRegistry;
    use crate::llm::LlmProvider;
    use crate::safety::SafetyLayer;
    use crate::tools::ToolRegistry;

    fn make_test_agent_with_store(store: Arc<dyn crate::db::Database>) -> Agent {
        let deps = AgentDeps {
            store: Some(store),
            llm: Arc::new(crate::testing::StubLlm::default()) as Arc<dyn LlmProvider>,
            cheap_llm: None,
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: true,
            })),
            tools: Arc::new(ToolRegistry::new()),
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks: Arc::new(HookRegistry::new()),
            cost_guard: Arc::new(CostGuard::new(CostGuardConfig::default())),
        };

        Agent::new(
            AgentConfig {
                name: "test-agent".to_string(),
                max_parallel_jobs: 1,
                job_timeout: std::time::Duration::from_secs(30),
                stuck_threshold: std::time::Duration::from_secs(60),
                repair_check_interval: std::time::Duration::from_secs(60),
                max_repair_attempts: 1,
                use_planning: true,
                session_idle_timeout: std::time::Duration::from_secs(3600),
                allow_local_tools: true,
                max_cost_per_day_cents: None,
                max_actions_per_hour: None,
                max_tool_iterations: 4,
                auto_approve_tools: false,
            },
            deps,
            Arc::new(ChannelManager::new()),
            None,
            None,
            None,
            Some(Arc::new(ContextManager::new(1))),
            None,
        )
    }

    #[test]
    fn sanitize_pending_tool_rationale_uses_fallback_when_blocked() {
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        }));

        let blocked = "my key is sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ABCDEFGHIJKLMNOPQRST";
        let rationale = sanitize_pending_tool_rationale(&safety, blocked);

        assert_eq!(rationale, crate::llm::DEFAULT_TOOL_RATIONALE);
    }

    #[test]
    fn sanitize_pending_tool_rationale_keeps_clean_text() {
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        }));

        let rationale = sanitize_pending_tool_rationale(&safety, " inspect prior context ");
        assert_eq!(rationale, "inspect prior context");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn maybe_hydrate_thread_refuses_unowned_thread() {
        let (db, _tmp) = crate::testing::test_db().await;
        let owner_user = "owner";
        let other_user = "other";

        let thread_id = db
            .create_conversation("gateway", owner_user, None)
            .await
            .expect("create conversation");

        db.add_conversation_message(thread_id, "user", "hello")
            .await
            .expect("add message");

        let agent = make_test_agent_with_store(db);
        let incoming =
            IncomingMessage::new("gateway", other_user, "ping").with_thread(thread_id.to_string());

        agent
            .maybe_hydrate_thread(&incoming, &thread_id.to_string())
            .await;

        let session = agent
            .session_manager
            .get_or_create_session(other_user)
            .await;
        let sess = session.lock().await;
        assert!(!sess.threads.contains_key(&thread_id));
    }
}
