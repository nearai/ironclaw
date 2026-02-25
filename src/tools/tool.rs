//! Tool trait and types.

use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::context::JobContext;

/// How much approval a specific tool invocation requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalRequirement {
    /// No approval needed.
    Never,
    /// Needs approval, but session auto-approve can bypass.
    UnlessAutoApproved,
    /// Always needs explicit approval (even if auto-approved).
    Always,
}

impl ApprovalRequirement {
    /// Whether this invocation requires approval in contexts where
    /// auto-approve is irrelevant (e.g. autonomous worker/scheduler).
    pub fn is_required(&self) -> bool {
        !matches!(self, Self::Never)
    }
}

/// Per-tool rate limit configuration for built-in tool invocations.
///
/// Controls how many times a tool can be invoked per user, per time window.
/// Read-only tools (echo, time, json, file_read, etc.) should NOT be rate limited.
/// Write/external tools (shell, http, file_write, memory_write, create_job) should be.
#[derive(Debug, Clone)]
pub struct ToolRateLimitConfig {
    /// Maximum invocations per minute.
    pub requests_per_minute: u32,
    /// Maximum invocations per hour.
    pub requests_per_hour: u32,
}

impl ToolRateLimitConfig {
    /// Create a config with explicit limits.
    pub fn new(requests_per_minute: u32, requests_per_hour: u32) -> Self {
        Self {
            requests_per_minute,
            requests_per_hour,
        }
    }
}

impl Default for ToolRateLimitConfig {
    /// Default: 60 requests/minute, 1000 requests/hour (generous for WASM HTTP).
    fn default() -> Self {
        Self {
            requests_per_minute: 60,
            requests_per_hour: 1000,
        }
    }
}

/// Risk level of a tool invocation.
///
/// Used by the shell tool to classify commands and by the worker to drive
/// approval decisions and observability logging. Implements `Ord` so callers
/// can compare levels (e.g. `risk >= RiskLevel::High`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Read-only, safe, reversible (e.g. `ls`, `cat`, `grep`).
    Low,
    /// Creates or modifies state, but generally reversible
    /// (e.g. `mkdir`, `git commit`, `cargo build`).
    Medium,
    /// Destructive, irreversible, or security-sensitive
    /// (e.g. `rm -rf`, `git push --force`, `kill -9`).
    High,
}

/// Where a tool should execute: orchestrator process or inside a container.
///
/// Orchestrator tools run in the main agent process (memory access, job mgmt, etc).
/// Container tools run inside Docker containers (shell, file ops, code mods).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolDomain {
    /// Safe to run in the orchestrator (pure functions, memory, job management).
    Orchestrator,
    /// Must run inside a sandboxed container (filesystem, shell, code).
    Container,
}

/// Error type for tool execution.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Invalid parameters: {0}")]
    InvalidParameters(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Timeout after {0:?}")]
    Timeout(Duration),

    #[error("Not authorized: {0}")]
    NotAuthorized(String),

    #[error("Rate limited, retry after {0:?}")]
    RateLimited(Option<Duration>),

    #[error("External service error: {0}")]
    ExternalService(String),

    #[error("Sandbox error: {0}")]
    Sandbox(String),
}

/// Output from a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// The result data.
    pub result: serde_json::Value,
    /// Cost incurred (if any).
    pub cost: Option<Decimal>,
    /// Time taken.
    pub duration: Duration,
    /// Raw output before sanitization (for debugging).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl ToolOutput {
    /// Create a successful output with a JSON result.
    pub fn success(result: serde_json::Value, duration: Duration) -> Self {
        Self {
            result,
            cost: None,
            duration,
            raw: None,
        }
    }

    /// Create a text output.
    pub fn text(text: impl Into<String>, duration: Duration) -> Self {
        Self {
            result: serde_json::Value::String(text.into()),
            cost: None,
            duration,
            raw: None,
        }
    }

    /// Set the cost.
    pub fn with_cost(mut self, cost: Decimal) -> Self {
        self.cost = Some(cost);
        self
    }

    /// Set the raw output.
    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }
}

/// Definition of a tool's parameters using JSON Schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSchema {
    /// Create a new tool schema.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    /// Set the parameters schema.
    pub fn with_parameters(mut self, parameters: serde_json::Value) -> Self {
        self.parameters = parameters;
        self
    }
}

/// Trait for tools that the agent can use.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool name.
    fn name(&self) -> &str;

    /// Get a description of what the tool does.
    fn description(&self) -> &str;

    /// Get the JSON Schema for the tool's parameters.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given parameters.
    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError>;

    /// Estimate the cost of running this tool with the given parameters.
    fn estimated_cost(&self, _params: &serde_json::Value) -> Option<Decimal> {
        None
    }

    /// Estimate how long this tool will take with the given parameters.
    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        None
    }

    /// Whether this tool's output needs sanitization.
    ///
    /// Returns true for tools that interact with external services,
    /// where the output might contain malicious content.
    fn requires_sanitization(&self) -> bool {
        true
    }

    /// Risk level for a specific invocation of this tool.
    ///
    /// Defaults to `Low` (read-only, safe). Override for tools whose risk
    /// depends on the parameters — the shell tool classifies commands into
    /// `Low` / `Medium` / `High` based on the command string.
    ///
    /// The worker logs this value with every tool call so operators can audit
    /// the risk level at which each execution was classified.
    fn risk_level_for(&self, _params: &serde_json::Value) -> RiskLevel {
        RiskLevel::Low
    }

    /// Whether this tool invocation requires user approval.
    ///
    /// Returns `Never` by default (most tools run in a sandboxed environment).
    /// Override to return `UnlessAutoApproved` for tools that need approval
    /// but can be session-auto-approved, or `Always` for invocations that
    /// must always prompt (e.g. destructive shell commands, HTTP with auth).
    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }

    /// Maximum time this tool is allowed to run before the caller kills it.
    /// Override for long-running tools like sandbox execution.
    /// Default: 60 seconds.
    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    /// Where this tool should execute.
    ///
    /// `Orchestrator` tools run in the main agent process (safe, no FS access).
    /// `Container` tools run inside Docker containers (shell, file ops).
    ///
    /// Default: `Orchestrator` (safe for the main process).
    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    /// Per-invocation rate limit for this tool.
    ///
    /// Return `Some(config)` to throttle how often this tool can be called per user.
    /// Read-only tools (echo, time, json, file_read, memory_search, etc.) should
    /// return `None`. Write/external tools (shell, http, file_write, memory_write,
    /// create_job) should return sensible limits to prevent runaway agents.
    ///
    /// Rate limits are per-user, per-tool, and in-memory (reset on restart).
    /// This is orthogonal to `requires_approval()` — a tool can be both
    /// approval-gated and rate limited. Rate limit is checked first (cheaper).
    ///
    /// Default: `None` (no rate limiting).
    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        None
    }

    /// Get the tool schema for LLM function calling.
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
        }
    }
}

/// Extract a required string parameter from a JSON object.
///
/// Returns `ToolError::InvalidParameters` if the key is missing or not a string.
pub fn require_str<'a>(params: &'a serde_json::Value, name: &str) -> Result<&'a str, ToolError> {
    params
        .get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing '{}' parameter", name)))
}

/// Extract a required parameter of any type from a JSON object.
///
/// Returns `ToolError::InvalidParameters` if the key is missing.
pub fn require_param<'a>(
    params: &'a serde_json::Value,
    name: &str,
) -> Result<&'a serde_json::Value, ToolError> {
    params
        .get(name)
        .ok_or_else(|| ToolError::InvalidParameters(format!("missing '{}' parameter", name)))
}

/// Lenient runtime validation of a tool's `parameters_schema()`.
///
/// Use this function at tool-registration time to catch structural mistakes
/// (missing `"type": "object"`, orphan `"required"` keys, arrays without
/// `"items"`) without rejecting intentional freeform properties.
///
/// For the stricter variant that also enforces `additionalProperties: false`,
/// enum-type consistency, and per-property `"type"` fields, see
/// [`validate_strict_schema`](crate::tools::schema_validator::validate_strict_schema)
/// in `schema_validator.rs` (used in CI tests).
///
/// Returns a list of validation errors. An empty list means the schema is valid.
///
/// # Rules enforced
///
/// 1. Top-level must have `"type": "object"`
/// 2. Top-level must have `"properties"` as an object
/// 3. Every key in `"required"` must exist in `"properties"`
/// 4. Nested objects follow the same rules recursively
/// 5. Array properties should have `"items"` defined
///
/// Properties without a `"type"` field are allowed (freeform/any-type).
/// This is an intentional pattern used by tools like `json` and `http` for
/// OpenAI compatibility, since union types with arrays require `items`.
pub fn validate_tool_schema(schema: &serde_json::Value, path: &str) -> Vec<String> {
    let mut errors = Vec::new();

    // Rule 1: must have "type": "object" at this level
    match schema.get("type").and_then(|t| t.as_str()) {
        Some("object") => {}
        Some(other) => {
            errors.push(format!("{path}: expected type \"object\", got \"{other}\""));
            return errors; // Can't check further
        }
        None => {
            errors.push(format!("{path}: missing \"type\": \"object\""));
            return errors;
        }
    }

    // Rule 2: must have "properties" as an object
    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => {
            errors.push(format!("{path}: missing or non-object \"properties\""));
            return errors;
        }
    };

    // Rule 3: every key in "required" must exist in "properties"
    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        for req in required {
            if let Some(key) = req.as_str()
                && !properties.contains_key(key)
            {
                errors.push(format!(
                    "{path}: required key \"{key}\" not found in properties"
                ));
            }
        }
    }

    // Rule 4 & 5: recurse into nested objects and check arrays
    for (key, prop) in properties {
        let prop_path = format!("{path}.{key}");
        if let Some(prop_type) = prop.get("type").and_then(|t| t.as_str()) {
            match prop_type {
                "object" => {
                    errors.extend(validate_tool_schema(prop, &prop_path));
                }
                "array" => {
                    if let Some(items) = prop.get("items") {
                        // If items is an object type, recurse
                        if items.get("type").and_then(|t| t.as_str()) == Some("object") {
                            errors
                                .extend(validate_tool_schema(items, &format!("{prop_path}.items")));
                        }
                    } else {
                        errors.push(format!("{prop_path}: array property missing \"items\""));
                    }
                }
                _ => {}
            }
        }
        // No "type" field is intentionally allowed (freeform properties)
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple no-op tool for testing.
    #[derive(Debug)]
    pub struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echoes back the input message. Useful for testing."
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The message to echo back"
                    }
                },
                "required": ["message"]
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            let message = require_str(&params, "message")?;

            Ok(ToolOutput::text(message, Duration::from_millis(1)))
        }

        fn requires_sanitization(&self) -> bool {
            false // Echo is a trusted internal tool
        }
    }

    #[tokio::test]
    async fn test_echo_tool() {
        let tool = EchoTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"message": "hello"}), &ctx)
            .await
            .unwrap();

        assert_eq!(result.result, serde_json::json!("hello"));
    }

    #[test]
    fn test_tool_schema() {
        let tool = EchoTool;
        let schema = tool.schema();

        assert_eq!(schema.name, "echo");
        assert!(!schema.description.is_empty());
    }

    #[test]
    fn test_execution_timeout_default() {
        let tool = EchoTool;
        assert_eq!(tool.execution_timeout(), Duration::from_secs(60));
    }

    #[test]
    fn test_require_str_present() {
        let params = serde_json::json!({"name": "alice"});
        assert_eq!(require_str(&params, "name").unwrap(), "alice");
    }

    #[test]
    fn test_require_str_missing() {
        let params = serde_json::json!({});
        let err = require_str(&params, "name").unwrap_err();
        assert!(err.to_string().contains("missing 'name'"));
    }

    #[test]
    fn test_require_str_wrong_type() {
        let params = serde_json::json!({"name": 42});
        let err = require_str(&params, "name").unwrap_err();
        assert!(err.to_string().contains("missing 'name'"));
    }

    #[test]
    fn test_require_param_present() {
        let params = serde_json::json!({"data": [1, 2, 3]});
        assert_eq!(
            require_param(&params, "data").unwrap(),
            &serde_json::json!([1, 2, 3])
        );
    }

    #[test]
    fn test_require_param_missing() {
        let params = serde_json::json!({});
        let err = require_param(&params, "data").unwrap_err();
        assert!(err.to_string().contains("missing 'data'"));
    }

    #[test]
    fn test_requires_approval_default() {
        let tool = EchoTool;
        // Default requires_approval() returns Never.
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"message": "hi"})),
            ApprovalRequirement::Never
        );
        assert!(!ApprovalRequirement::Never.is_required());
        assert!(ApprovalRequirement::UnlessAutoApproved.is_required());
        assert!(ApprovalRequirement::Always.is_required());
    }

    #[test]
    fn test_validate_schema_valid() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "A name" }
            },
            "required": ["name"]
        });
        let errors = validate_tool_schema(&schema, "test");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn test_validate_schema_missing_type() {
        let schema = serde_json::json!({
            "properties": {
                "name": { "type": "string" }
            }
        });
        let errors = validate_tool_schema(&schema, "test");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("missing \"type\": \"object\""));
    }

    #[test]
    fn test_validate_schema_wrong_type() {
        let schema = serde_json::json!({
            "type": "string"
        });
        let errors = validate_tool_schema(&schema, "test");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("expected type \"object\""));
    }

    #[test]
    fn test_validate_schema_required_not_in_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" }
            },
            "required": ["name", "age"]
        });
        let errors = validate_tool_schema(&schema, "test");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("\"age\" not found in properties"));
    }

    #[test]
    fn test_validate_schema_nested_object() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "config": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" }
                    },
                    "required": ["key", "missing"]
                }
            }
        });
        let errors = validate_tool_schema(&schema, "test");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("test.config"));
        assert!(errors[0].contains("\"missing\" not found"));
    }

    #[test]
    fn test_validate_schema_array_missing_items() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "tags": { "type": "array", "description": "Tags" }
            }
        });
        let errors = validate_tool_schema(&schema, "test");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("array property missing \"items\""));
    }

    #[test]
    fn test_validate_schema_array_with_items_ok() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "tags": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        });
        let errors = validate_tool_schema(&schema, "test");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn test_validate_schema_freeform_property_allowed() {
        // Properties without "type" are intentionally allowed (json/http tools)
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "data": { "description": "Any JSON value" }
            },
            "required": ["data"]
        });
        let errors = validate_tool_schema(&schema, "test");
        assert!(
            errors.is_empty(),
            "freeform property should be allowed: {errors:?}"
        );
    }

    #[test]
    fn test_validate_schema_nested_array_items_object() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "headers": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "value": { "type": "string" }
                        },
                        "required": ["name", "value"]
                    }
                }
            }
        });
        let errors = validate_tool_schema(&schema, "test");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn test_validate_schema_nested_array_items_object_bad() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "headers": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" }
                        },
                        "required": ["name", "missing_field"]
                    }
                }
            }
        });
        let errors = validate_tool_schema(&schema, "test");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("headers.items"));
        assert!(errors[0].contains("\"missing_field\""));
    }
}
