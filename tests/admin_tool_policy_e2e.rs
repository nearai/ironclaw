//! End-to-end regression test for admin tool policy enforcement.
//!
//! Verifies that a tool disabled by the admin policy is removed from the
//! actual tool list passed to the LLM in a multi-tenant chat turn.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use ironclaw::channels::IncomingMessage;
    use ironclaw::config::Config;
    use ironclaw::llm::{
        CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
        ToolCompletionResponse,
    };
    use ironclaw::tools::permissions::{ADMIN_SETTINGS_USER_ID, ADMIN_TOOL_POLICY_KEY};
    use rust_decimal::Decimal;

    use crate::support::test_rig::TestRigBuilder;

    #[derive(Default)]
    struct RecordingToolsProvider {
        seen_tools: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl LlmProvider for RecordingToolsProvider {
        fn model_name(&self) -> &str {
            "recording-tools-e2e"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, ironclaw::error::LlmError> {
            Ok(CompletionResponse {
                content: "ok".to_string(),
                input_tokens: 0,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, ironclaw::error::LlmError> {
            let names: Vec<String> = request.tools.iter().map(|tool| tool.name.clone()).collect();
            self.seen_tools
                .lock()
                .expect("recording tools mutex poisoned")
                .push(names);
            Ok(ToolCompletionResponse {
                content: Some("ok".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 0,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    #[tokio::test]
    async fn admin_disabled_tool_does_not_reach_llm() {
        let mut config = Config::for_testing(
            PathBuf::from("ignored.db"),
            PathBuf::from("ignored-skills"),
            PathBuf::from("ignored-installed-skills"),
        );
        config.agent.multi_tenant = true;

        let llm = Arc::new(RecordingToolsProvider::default());
        let llm_provider: Arc<dyn LlmProvider> = llm.clone();
        let rig = TestRigBuilder::new()
            .with_config(config)
            .with_llm(llm_provider)
            .build()
            .await;

        rig.database()
            .set_setting(
                ADMIN_SETTINGS_USER_ID,
                ADMIN_TOOL_POLICY_KEY,
                &serde_json::json!({
                    "disabled_tools": ["echo"]
                }),
            )
            .await
            .expect("failed to persist admin tool policy");

        rig.send_incoming(IncomingMessage::new("test", "member-user", "hello"))
            .await;
        let _responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        let calls = llm
            .seen_tools
            .lock()
            .expect("recording tools mutex poisoned")
            .clone();
        assert!(
            !calls.is_empty(),
            "the LLM should have received at least one tool-enabled request"
        );
        assert!(
            !calls[0].iter().any(|tool| tool == "echo"),
            "admin-disabled tool leaked into the LLM tool list: {:?}",
            calls[0]
        );
        assert!(
            calls[0].iter().any(|tool| tool == "time"),
            "expected at least one non-disabled built-in tool to remain available: {:?}",
            calls[0]
        );

        rig.shutdown();
    }
}
