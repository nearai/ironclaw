//! Integration tests for the A2A bridge tool.
//!
//! Construction tests run always. Live tests require a running A2A-compatible
//! agent and are marked `#[ignore]`.
//! Run with: `cargo test --test a2a_bridge_integration -- --ignored`

use std::sync::Arc;
use std::time::Duration;

use ironclaw::config::A2aConfig;
use ironclaw::secrets::{InMemorySecretsStore, SecretsCrypto, SecretsStore};
use ironclaw::tools::builtin::A2aBridgeTool;
use ironclaw::tools::{ApprovalRequirement, Tool, ToolOutput};
use secrecy::SecretString;
use tokio::sync::mpsc;

fn test_secrets_store() -> Arc<dyn SecretsStore + Send + Sync> {
    let key = SecretString::from("test-key-32-bytes-long-enough!!!".to_string());
    let crypto = Arc::new(SecretsCrypto::new(key).expect("test crypto"));
    Arc::new(InMemorySecretsStore::new(crypto))
}

fn test_config() -> A2aConfig {
    A2aConfig {
        enabled: true,
        agent_url: std::env::var("A2A_AGENT_URL")
            .unwrap_or_else(|_| "https://a2a-test.example.com".to_string()),
        assistant_id: std::env::var("A2A_ASSISTANT_ID")
            .unwrap_or_else(|_| "test-assistant".to_string()),
        tool_name: "a2a_test".to_string(),
        tool_description: "Test A2A bridge".to_string(),
        message_prefix: "[test]".to_string(),
        request_timeout: Duration::from_secs(30),
        task_timeout: Duration::from_secs(120),
        api_key_secret: "a2a_test_key".to_string(),
    }
}

async fn create_tool(config: A2aConfig) -> Result<A2aBridgeTool, ironclaw::tools::ToolError> {
    let (tx, _rx) = mpsc::channel(10);
    A2aBridgeTool::new(config, test_secrets_store(), tx).await
}

// ── Construction tests (run always) ────────────────────────────────

#[tokio::test]
async fn construction_rejects_localhost() {
    let mut config = test_config();
    config.agent_url = "http://localhost:5085".to_string();
    assert!(create_tool(config).await.is_err());
}

#[tokio::test]
async fn construction_rejects_private_ip() {
    let mut config = test_config();
    config.agent_url = "http://192.168.1.100:5085".to_string();
    assert!(create_tool(config).await.is_err());
}

#[tokio::test]
async fn construction_rejects_link_local() {
    let mut config = test_config();
    config.agent_url = "http://169.254.169.254/latest".to_string();
    assert!(create_tool(config).await.is_err());
}

#[tokio::test]
async fn construction_accepts_public_url() {
    let config = test_config();
    assert!(create_tool(config).await.is_ok());
}

#[tokio::test]
async fn tool_uses_configured_name() {
    let mut config = test_config();
    config.tool_name = "custom_a2a".to_string();
    let tool = create_tool(config).await.unwrap();
    assert_eq!(tool.name(), "custom_a2a");
}

#[tokio::test]
async fn tool_requires_always_approval() {
    let config = test_config();
    let tool = create_tool(config).await.unwrap();
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Always,
    );
}

// ── Live agent tests (require A2A_AGENT_URL) ───────────────────────

#[tokio::test]
#[ignore = "requires running A2A agent (set A2A_AGENT_URL)"]
async fn live_query_returns_result() {
    let config = test_config();
    let (tx, mut rx) = mpsc::channel(10);
    let tool = A2aBridgeTool::new(config, test_secrets_store(), tx)
        .await
        .unwrap();

    let ctx = ironclaw::context::JobContext::default();
    let params = serde_json::json!({ "query": "What is 2+2?" });

    let result = tool.execute(params, &ctx).await;
    assert!(result.is_ok(), "execute failed: {:?}", result.err());

    let output: ToolOutput = result.unwrap();
    let status = output.result["status"].as_str().unwrap();
    assert!(
        status == "completed" || status == "submitted",
        "unexpected status: {}",
        status
    );

    // If submitted, wait for the background consumer to push a result
    if status == "submitted" {
        let msg = tokio::time::timeout(Duration::from_secs(120), rx.recv())
            .await
            .expect("timed out waiting for background result")
            .expect("channel closed");
        assert!(
            msg.content.contains("[test]"),
            "expected message_prefix in: {}",
            msg.content
        );
    }
}
