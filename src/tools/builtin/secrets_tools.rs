//! Agent-callable tools for inspecting user secrets.
//!
//! These tools allow the LLM to query and manage secrets on behalf of the
//! user. The zero-exposure model is preserved throughout:
//!
//! - `secret_list` returns only names and metadata (no values).
//! - `secret_delete` removes a secret by name.
//!
//! Storing secrets is handled via the extensions setup flow — the user types
//! values directly into the secure UI, which submits them to
//! `/api/extensions/{name}/setup`. Values never appear in the LLM conversation,
//! logs, or ActionRecords.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

// ── secret_list ──────────────────────────────────────────────────────────────

pub struct SecretListTool {
    store: Arc<dyn SecretsStore + Send + Sync>,
}

impl SecretListTool {
    pub fn new(store: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for SecretListTool {
    fn name(&self) -> &str {
        "secret_list"
    }

    fn description(&self) -> &str {
        "List all stored secrets by name. Never returns values — only names and \
         optional provider metadata. Use this to check what credentials are available \
         before attempting a task that requires them."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let refs = self
            .store
            .list(&ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let secrets: Vec<serde_json::Value> = refs
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "provider": r.provider,
                })
            })
            .collect();

        let count = secrets.len();
        let output = serde_json::json!({
            "secrets": secrets,
            "count": count,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── secret_delete ─────────────────────────────────────────────────────────────

pub struct SecretDeleteTool {
    store: Arc<dyn SecretsStore + Send + Sync>,
}

impl SecretDeleteTool {
    pub fn new(store: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for SecretDeleteTool {
    fn name(&self) -> &str {
        "secret_delete"
    }

    fn description(&self) -> &str {
        "Permanently delete a stored secret by name. This cannot be undone."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the secret to delete."
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let deleted = self
            .store
            .delete(&ctx.user_id, name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = if deleted {
            serde_json::json!({
                "status": "deleted",
                "name": name,
            })
        } else {
            serde_json::json!({
                "status": "not_found",
                "name": name,
                "message": format!("No secret named '{}' found.", name),
            })
        };

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use secrecy::SecretString;

    use super::*;
    use crate::context::JobContext;
    use crate::secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto};

    fn test_store() -> Arc<InMemorySecretsStore> {
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        Arc::new(InMemorySecretsStore::new(crypto))
    }

    fn test_ctx() -> JobContext {
        JobContext::new("test", "test job")
    }

    #[tokio::test]
    async fn test_secret_list() {
        let store = test_store();
        let list = SecretListTool::new(Arc::clone(&store) as Arc<dyn SecretsStore + Send + Sync>);
        let ctx = test_ctx();

        store
            .create(
                &ctx.user_id,
                CreateSecretParams::new("openai_key", "sk-test"),
            )
            .await
            .unwrap();

        let list_result = list.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert_eq!(list_result.result["count"], 1);
        assert_eq!(list_result.result["secrets"][0]["name"], "openai_key");
        assert!(list_result.result["secrets"][0].get("value").is_none());
    }

    #[tokio::test]
    async fn test_secret_delete() {
        let store = test_store();
        let delete =
            SecretDeleteTool::new(Arc::clone(&store) as Arc<dyn SecretsStore + Send + Sync>);
        let ctx = test_ctx();

        store
            .create(&ctx.user_id, CreateSecretParams::new("to_delete", "secret"))
            .await
            .unwrap();

        let result = delete
            .execute(serde_json::json!({"name": "to_delete"}), &ctx)
            .await
            .unwrap();
        assert_eq!(result.result["status"], "deleted");

        // Deleting again returns not_found
        let result2 = delete
            .execute(serde_json::json!({"name": "to_delete"}), &ctx)
            .await
            .unwrap();
        assert_eq!(result2.result["status"], "not_found");
    }
}
