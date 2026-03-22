use std::sync::Arc;

use crate::context::JobContext;
use crate::secrets::{CreateSecretParams, SecretsStore, set_global_store};
use crate::testing::credentials::{TEST_GITHUB_TOKEN, test_secrets_store};
use crate::tools::tool::Tool;

use super::ShellTool;

#[tokio::test]
async fn test_shell_injects_requested_credentials() -> Result<(), String> {
    let store = Arc::new(test_secrets_store());
    store
        .create(
            "user1",
            CreateSecretParams::new("github_token", TEST_GITHUB_TOKEN),
        )
        .await
        .map_err(|e| e.to_string())?;
    set_global_store(Some(store.clone()));

    let tool = ShellTool::new();
    let ctx = JobContext::with_user("user1", "Credential test", "Credential test");

    let result = tool
        .execute(
            serde_json::json!({
                "command": "printenv GITHUB_TOKEN",
                "credentials": {"github_token": "GITHUB_TOKEN"}
            }),
            &ctx,
        )
        .await
        .map_err(|e| e.to_string())?;

    let output = result
        .result
        .get("output")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing shell output".to_string())?;

    let result = if output.contains(TEST_GITHUB_TOKEN) {
        Ok(())
    } else {
        Err(format!(
            "shell output did not contain injected token: {output}"
        ))
    };

    set_global_store(None);
    result
}
