//! Composio integration — connects to 250+ apps via Composio's REST API.
//!
//! Enabled when `COMPOSIO_API_KEY` env var is set. Provides a single multiplexed
//! tool with actions: list, execute, connect, connected_accounts.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str};

const API_BASE: &str = "https://backend.composio.dev/api/v3";

/// Composio tool — single multiplexed interface to Composio's REST API.
pub struct ComposioTool {
    client: Client,
    api_key: String,
    entity_id: String,
    /// Cache: "entity_id:app" -> connected_account_id
    account_cache: Mutex<HashMap<String, String>>,
}

impl ComposioTool {
    pub fn new(api_key: String, entity_id: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to create composio HTTP client");
        Self {
            client,
            api_key,
            entity_id,
            account_cache: Mutex::new(HashMap::new()),
        }
    }

    /// GET with api key header.
    async fn get(&self, path: &str) -> Result<Value, ToolError> {
        let resp = self
            .client
            .get(format!("{API_BASE}{path}"))
            .header("x-api-key", &self.api_key)
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(e.to_string()))?;
        Self::parse_response(resp).await
    }

    /// POST with api key header and JSON body.
    async fn post(&self, path: &str, body: &Value) -> Result<Value, ToolError> {
        let resp = self
            .client
            .post(format!("{API_BASE}{path}"))
            .header("x-api-key", &self.api_key)
            .json(body)
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(e.to_string()))?;
        Self::parse_response(resp).await
    }

    async fn parse_response(resp: reqwest::Response) -> Result<Value, ToolError> {
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| ToolError::ExternalService(e.to_string()))?;
        if !status.is_success() {
            return Err(ToolError::ExternalService(format!(
                "Composio API {status}: {body}"
            )));
        }
        serde_json::from_str(&body)
            .map_err(|e| ToolError::ExternalService(format!("invalid JSON: {e}")))
    }

    /// List available tools, optionally filtered by app.
    async fn list_tools(&self, app: Option<&str>) -> Result<Value, ToolError> {
        let path = match app {
            Some(a) => format!("/tools?toolkit_slug={a}"),
            None => "/tools".to_string(),
        };
        self.get(&path).await
    }

    /// Execute a tool action.
    async fn execute_action(
        &self,
        tool_slug: &str,
        params: &Value,
        entity_id: &str,
        connected_account_id: Option<&str>,
    ) -> Result<Value, ToolError> {
        // Auto-resolve connected account if not provided
        let account_id = match connected_account_id {
            Some(id) => id.to_string(),
            None => self.resolve_account(tool_slug, entity_id).await?,
        };

        let body = json!({
            "connected_account_id": account_id,
            "entity_id": entity_id,
            "input": params,
        });
        self.post(&format!("/tools/execute/{tool_slug}"), &body)
            .await
    }

    /// Initiate OAuth connection for an app.
    async fn connect_app(&self, app: &str, entity_id: &str) -> Result<Value, ToolError> {
        // Resolve auth config for this app
        let configs = self
            .get(&format!("/auth_configs?toolkit_slug={app}"))
            .await?;
        let auth_config_id = configs
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("id"))
            .and_then(|id| id.as_str())
            .ok_or_else(|| {
                ToolError::ExternalService(format!(
                    "no auth config found for {app} — configure it at app.composio.dev"
                ))
            })?;

        let body = json!({
            "auth_config_id": auth_config_id,
            "user_id": entity_id,
        });
        self.post("/connected_accounts/link", &body).await
    }

    /// List connected accounts, optionally filtered by app.
    async fn list_accounts(&self, app: Option<&str>, entity_id: &str) -> Result<Value, ToolError> {
        let mut path = format!("/connected_accounts?user_id={entity_id}");
        if let Some(a) = app {
            path.push_str(&format!("&toolkit_slug={a}"));
        }
        self.get(&path).await
    }

    /// Auto-resolve connected account for a tool slug.
    async fn resolve_account(
        &self,
        tool_slug: &str,
        entity_id: &str,
    ) -> Result<String, ToolError> {
        // Extract app from tool slug (e.g., "GMAIL_SEND_EMAIL" -> "gmail")
        let app = tool_slug
            .split('_')
            .next()
            .unwrap_or(tool_slug)
            .to_ascii_lowercase();
        let cache_key = format!("{entity_id}:{app}");

        // Check cache
        {
            let cache = self.account_cache.lock().await;
            if let Some(id) = cache.get(&cache_key) {
                return Ok(id.clone());
            }
        }

        // Fetch from API
        let accounts = self.list_accounts(Some(&app), entity_id).await?;
        let account_id = accounts
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .filter(|a| a.get("status").and_then(|s| s.as_str()) == Some("ACTIVE"))
                    .max_by_key(|a| a.get("updatedAt").and_then(|u| u.as_str()).unwrap_or(""))
            })
            .and_then(|a| a.get("id"))
            .and_then(|id| id.as_str())
            .ok_or_else(|| {
                ToolError::ExternalService(format!(
                    "no connected account for {app} — use composio with action=\"connect\" first"
                ))
            })?
            .to_string();

        // Cache it
        {
            let mut cache = self.account_cache.lock().await;
            cache.insert(cache_key, account_id.clone());
        }

        Ok(account_id)
    }
}

#[async_trait]
impl Tool for ComposioTool {
    fn name(&self) -> &str {
        "composio"
    }

    fn description(&self) -> &str {
        "Connect to 250+ apps (Gmail, GitHub, Slack, Notion, etc.) via Composio. \
         Actions: \"list\" (browse tools), \"execute\" (run a tool), \
         \"connect\" (OAuth-link an app), \"connected_accounts\" (list linked accounts)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "execute", "connect", "connected_accounts"],
                    "description": "Action to perform"
                },
                "app": {
                    "type": "string",
                    "description": "App/toolkit slug (e.g., \"gmail\", \"github\", \"notion\")"
                },
                "tool_slug": {
                    "type": "string",
                    "description": "Tool action slug for execute (e.g., \"GMAIL_SEND_EMAIL\")"
                },
                "params": {
                    "description": "Parameters for the tool action (JSON object)"
                },
                "entity_id": {
                    "type": "string",
                    "description": "User/entity ID (defaults to config value)"
                },
                "connected_account_id": {
                    "type": "string",
                    "description": "Specific connected account ID (auto-resolved if omitted)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let action = require_str(&params, "action")?;
        let entity_id = params
            .get("entity_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&self.entity_id);

        let result = match action {
            "list" => {
                let app = params.get("app").and_then(|v| v.as_str());
                self.list_tools(app).await?
            }
            "execute" => {
                let tool_slug = require_str(&params, "tool_slug")?;
                let action_params = params.get("params").cloned().unwrap_or(json!({}));
                let account_id = params
                    .get("connected_account_id")
                    .and_then(|v| v.as_str());
                self.execute_action(tool_slug, &action_params, entity_id, account_id)
                    .await?
            }
            "connect" => {
                let app = require_str(&params, "app")?;
                self.connect_app(app, entity_id).await?
            }
            "connected_accounts" => {
                let app = params.get("app").and_then(|v| v.as_str());
                self.list_accounts(app, entity_id).await?
            }
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown action \"{other}\", expected: list, execute, connect, connected_accounts"
                )));
            }
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(30, 500))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_valid() {
        let tool = ComposioTool::new("test-key".into(), "default".into());
        let schema = tool.parameters_schema();
        let errors = crate::tools::tool::validate_tool_schema(&schema, "composio");
        assert!(errors.is_empty(), "schema errors: {errors:?}");
    }

    #[test]
    fn test_name_and_description() {
        let tool = ComposioTool::new("test-key".into(), "default".into());
        assert_eq!(tool.name(), "composio");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn test_missing_action_param() {
        let tool = ComposioTool::new("test-key".into(), "default".into());
        let ctx = JobContext::default();
        let err = tool.execute(json!({}), &ctx).await.unwrap_err();
        assert!(err.to_string().contains("missing 'action'"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = ComposioTool::new("test-key".into(), "default".into());
        let ctx = JobContext::default();
        let err = tool
            .execute(json!({"action": "invalid"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown action"));
    }

    #[tokio::test]
    async fn test_execute_missing_tool_slug() {
        let tool = ComposioTool::new("test-key".into(), "default".into());
        let ctx = JobContext::default();
        let err = tool
            .execute(json!({"action": "execute"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'tool_slug'"));
    }

    #[tokio::test]
    async fn test_connect_missing_app() {
        let tool = ComposioTool::new("test-key".into(), "default".into());
        let ctx = JobContext::default();
        let err = tool
            .execute(json!({"action": "connect"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing 'app'"));
    }

    #[test]
    fn test_default_entity_id() {
        let tool = ComposioTool::new("key".into(), "my-tenant".into());
        assert_eq!(tool.entity_id, "my-tenant");
    }

    #[test]
    fn test_execution_timeout() {
        let tool = ComposioTool::new("key".into(), "default".into());
        assert_eq!(tool.execution_timeout(), Duration::from_secs(30));
    }

    #[test]
    fn test_rate_limit_configured() {
        let tool = ComposioTool::new("key".into(), "default".into());
        let rl = tool.rate_limit_config().expect("should have rate limit");
        assert_eq!(rl.requests_per_minute, 30);
        assert_eq!(rl.requests_per_hour, 500);
    }

    #[test]
    fn test_requires_sanitization() {
        let tool = ComposioTool::new("key".into(), "default".into());
        // External service — should sanitize output
        assert!(tool.requires_sanitization());
    }

    #[test]
    fn test_tool_schema_complete() {
        let tool = ComposioTool::new("key".into(), "default".into());
        let schema = tool.schema();
        assert_eq!(schema.name, "composio");
        assert!(!schema.description.is_empty());
        // Verify all expected properties exist
        let props = schema.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("action"));
        assert!(props.contains_key("app"));
        assert!(props.contains_key("tool_slug"));
        assert!(props.contains_key("params"));
        assert!(props.contains_key("entity_id"));
        assert!(props.contains_key("connected_account_id"));
        // Only action is required
        let required = schema.parameters["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "action");
    }

    #[tokio::test]
    async fn test_entity_id_override_in_params() {
        // Verify custom entity_id in params doesn't crash (will fail on network, but
        // validates param extraction path)
        let tool = ComposioTool::new("fake-key".into(), "default".into());
        let ctx = JobContext::default();
        let result = tool
            .execute(
                json!({"action": "list", "entity_id": "custom-entity"}),
                &ctx,
            )
            .await;
        // Will fail with network error (fake key), but should NOT fail with param error
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("missing") && !err.contains("unknown action"),
            "should be a network error, not a param error: {err}"
        );
    }
}
