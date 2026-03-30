use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::context::JobContext;
use crate::sandbox::{SandboxManager, SandboxPolicy};
use crate::secrets::{SecretError, SecretsStore, global_store, set_global_store};
use crate::tools::builtin::shell::ShellTool as InnerShellTool;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};

/// Environment variable names that must never receive secret injection.
const DANGEROUS_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "PWD",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LD_PRELOAD",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_FORCE_FLAT_NAMESPACE",
];

fn validate_env_var_name(name: &str) -> Result<(), ToolError> {
    if name.is_empty() {
        return Err(ToolError::InvalidParameters(
            "env var name cannot be empty".into(),
        ));
    }

    let valid = name
        .bytes()
        .enumerate()
        .all(|(i, b)| matches!(b, b'A'..=b'Z' | b'_') || (i > 0 && b.is_ascii_digit()));

    if !valid {
        return Err(ToolError::InvalidParameters(format!(
            "env var '{}' must match [A-Z_][A-Z0-9_]* (uppercase, underscores, digits)",
            name
        )));
    }

    if DANGEROUS_ENV_VARS.contains(&name) {
        return Err(ToolError::InvalidParameters(format!(
            "env var '{}' is on the denylist (could hijack process behavior)",
            name
        )));
    }

    Ok(())
}

async fn parse_credentials(
    params: &Value,
    user_id: &str,
) -> Result<HashMap<String, String>, ToolError> {
    let creds_obj = match params.get("credentials").and_then(|v| v.as_object()) {
        Some(obj) if !obj.is_empty() => obj,
        _ => return Ok(HashMap::new()),
    };

    const MAX_CREDENTIAL_GRANTS: usize = 20;
    if creds_obj.len() > MAX_CREDENTIAL_GRANTS {
        return Err(ToolError::InvalidParameters(format!(
            "too many credential grants ({}, max {})",
            creds_obj.len(),
            MAX_CREDENTIAL_GRANTS
        )));
    }

    let secrets = global_store().ok_or_else(|| {
        ToolError::ExecutionFailed(
            "credentials requested but no secrets store is configured. Set SECRETS_MASTER_KEY to enable credential management."
                .to_string(),
        )
    })?;

    let mut extra_env = HashMap::with_capacity(creds_obj.len());
    for (secret_name, env_var_value) in creds_obj {
        let env_var = env_var_value.as_str().ok_or_else(|| {
            ToolError::InvalidParameters(format!(
                "credential env var for '{}' must be a string",
                secret_name
            ))
        })?;

        validate_env_var_name(env_var)?;

        let secret = match secrets.get_decrypted(user_id, secret_name).await {
            Ok(secret) => secret,
            Err(SecretError::NotFound(_)) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "secret '{}' not found. Store it first via 'ironclaw tool auth' or the web UI.",
                    secret_name
                )));
            }
            Err(e) => {
                return Err(ToolError::ExecutionFailed(format!(
                    "failed to retrieve secret '{}': {}",
                    secret_name, e
                )));
            }
        };

        extra_env.insert(env_var.to_string(), secret.expose().to_string());
    }

    Ok(extra_env)
}

fn strip_credentials(mut params: Value) -> Value {
    match params {
        Value::Object(ref mut map) => {
            map.remove("credentials");
            params
        }
        _ => params,
    }
}

/// Shell command execution tool that supports optional credential injection.
#[derive(Debug, Default)]
pub struct ShellTool {
    inner: InnerShellTool,
}

impl ShellTool {
    /// Create a new shell tool with default settings.
    pub fn new() -> Self {
        Self {
            inner: InnerShellTool::new(),
        }
    }

    /// Set the working directory.
    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.inner = self.inner.with_working_dir(dir);
        self
    }

    /// Set the command timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.with_timeout(timeout);
        self
    }

    /// Enable sandbox execution with the given manager.
    pub fn with_sandbox(mut self, sandbox: Arc<SandboxManager>) -> Self {
        self.inner = self.inner.with_sandbox(sandbox);
        self
    }

    /// Set the sandbox policy.
    pub fn with_sandbox_policy(mut self, policy: SandboxPolicy) -> Self {
        self.inner = self.inner.with_sandbox_policy(policy);
        self
    }

    /// Set the process-wide secrets store used by credential injection.
    pub fn with_secrets_store(self, secrets_store: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        set_global_store(Some(secrets_store));
        self
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        let mut schema = self.inner.parameters_schema();
        if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            props.insert(
                "credentials".to_string(),
                serde_json::json!({
                    "type": "object",
                    "description": "Optional map of secret names to env var names. Each secret must exist in the secrets store."
                }),
            );
        }
        schema
    }

    async fn execute(&self, params: Value, ctx: &JobContext) -> Result<ToolOutput, ToolError> {
        let credential_env = parse_credentials(&params, &ctx.user_id).await?;
        let mut extra_env = (*ctx.extra_env).clone();
        extra_env.extend(credential_env);

        let mut next_ctx = ctx.clone();
        next_ctx.extra_env = Arc::new(extra_env);

        self.inner
            .execute(strip_credentials(params), &next_ctx)
            .await
    }

    fn estimated_cost(&self, params: &Value) -> Option<rust_decimal::Decimal> {
        self.inner.estimated_cost(params)
    }

    fn estimated_duration(&self, params: &Value) -> Option<Duration> {
        self.inner.estimated_duration(params)
    }

    fn requires_sanitization(&self) -> bool {
        self.inner.requires_sanitization()
    }

    fn requires_approval(&self, params: &Value) -> ApprovalRequirement {
        self.inner.requires_approval(params)
    }

    fn execution_timeout(&self) -> Duration {
        self.inner.execution_timeout()
    }

    fn domain(&self) -> crate::tools::tool::ToolDomain {
        self.inner.domain()
    }

    fn sensitive_params(&self) -> &[&str] {
        self.inner.sensitive_params()
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        self.inner.rate_limit_config()
    }

    fn webhook_capability(&self) -> Option<crate::tools::wasm::WebhookCapability> {
        self.inner.webhook_capability()
    }

    fn discovery_schema(&self) -> Value {
        self.parameters_schema()
    }
}

#[cfg(test)]
mod tests;
