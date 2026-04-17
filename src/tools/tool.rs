//! Tool trait and types.

use std::fmt;
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

/// Precomputed autonomous tool scope for background jobs and routines.
///
/// Interactive sessions don't use this type — they still rely on
/// `requires_approval()` and session-level approval state.
#[derive(Debug, Clone)]
pub enum ApprovalContext {
    /// Autonomous job with no interactive user. Only tools in `allowed_tools`
    /// may run; interactive approval requirements are ignored.
    Autonomous {
        /// Tool names that may run autonomously for this job/run.
        allowed_tools: std::collections::HashSet<String>,
    },
}

impl ApprovalContext {
    /// Create an autonomous context with no allowed tools.
    pub fn autonomous() -> Self {
        Self::Autonomous {
            allowed_tools: std::collections::HashSet::new(),
        }
    }

    /// Create an autonomous context with specific allowed tools.
    pub fn autonomous_with_tools(tools: impl IntoIterator<Item = String>) -> Self {
        Self::Autonomous {
            allowed_tools: tools.into_iter().collect(),
        }
    }

    /// Check whether a tool invocation is blocked in this context.
    ///
    /// - `Never` tools are always allowed (no approval needed).
    /// - `UnlessAutoApproved` tools are allowed in autonomous contexts
    ///   (autonomous execution implies auto-approve).
    /// - `Always` tools are only allowed if explicitly listed in `allowed_tools`.
    pub fn is_blocked(&self, tool_name: &str, requirement: ApprovalRequirement) -> bool {
        match self {
            Self::Autonomous { allowed_tools } => match requirement {
                ApprovalRequirement::Never => false,
                ApprovalRequirement::UnlessAutoApproved => false,
                ApprovalRequirement::Always => !allowed_tools.contains(tool_name),
            },
        }
    }

    /// Check whether a tool is blocked given an optional context.
    ///
    /// When `None`, falls back to legacy behavior: all non-`Never` tools are blocked.
    pub fn is_blocked_or_default(
        context: &Option<Self>,
        tool_name: &str,
        requirement: ApprovalRequirement,
    ) -> bool {
        match context {
            Some(ctx) => ctx.is_blocked(tool_name, requirement),
            None => requirement.is_required(),
        }
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

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Low => f.write_str("low"),
            Self::Medium => f.write_str("medium"),
            Self::High => f.write_str("high"),
        }
    }
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

/// Which engine versions a tool is available in.
///
/// Declared by each tool via `Tool::engine_compatibility()`. Tools default to
/// `Both`; override for version-specific tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineCompatibility {
    /// Available in both v1 (legacy agent loop) and v2 (engine threads).
    Both,
    /// Only available in v1 (legacy agent loop). Replaced by engine-native
    /// capabilities in v2 (e.g. `routine_create` → `mission_create`).
    V1Only,
    /// Only available in v2 (engine threads/capabilities).
    V2Only,
}

impl EngineCompatibility {
    /// Whether a tool with this compatibility is visible in the given engine version.
    pub fn is_visible_in(self, version: EngineVersion) -> bool {
        match self {
            Self::Both => true,
            Self::V1Only => version == EngineVersion::V1,
            Self::V2Only => version == EngineVersion::V2,
        }
    }
}

/// Engine version selector for filtering tools.
///
/// Used by `ToolRegistry::tool_definitions_for_engine()` as the filter
/// parameter. Separate from `EngineCompatibility` to avoid the footgun of
/// passing `Both` as a filter (which would confusingly exclude version-specific
/// tools).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineVersion {
    /// V1 legacy agent loop.
    V1,
    /// V2 engine threads/capabilities.
    V2,
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

/// Curated discovery guidance surfaced by `tool_info(detail: "summary")`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolDiscoverySummary {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always_required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditional_requirements: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<serde_json::Value>,
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

    /// Which engine versions this tool is available in.
    ///
    /// Default: `Both`. Override to `V1Only` for tools replaced by engine-native
    /// capabilities in v2 (e.g. `routine_create` → `mission_create`), or for
    /// tools that cannot be LLM-invoked in v2 (e.g. `ApprovalRequirement::Always`
    /// tools with no interactive approval path).
    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::Both
    }

    /// Parameter names whose values must be redacted before logging, hooks, and approvals.
    ///
    /// The agent framework replaces these parameter values with `"[REDACTED]"` before:
    /// - Writing to debug logs
    /// - Storing in `ActionRecord` (in-memory job history)
    /// - Recording in `TurnToolCall` (session state)
    /// - Sending to `BeforeToolCall` hooks
    /// - Displaying in the approval UI
    ///
    /// **The `execute()` method still receives the original, unredacted parameters.**
    /// Redaction only applies to the observability and audit paths, not execution.
    ///
    /// Use this for tools that accept plaintext secrets as parameters (e.g. `secret_save`).
    fn sensitive_params(&self) -> &[&str] {
        &[]
    }

    /// UI-friendly display label for this tool in a DM context.
    ///
    /// Defaults to [`Self::name`]. Override with a human-readable label
    /// (e.g. `"HR Query"` instead of `"internal_hr_query"`) to improve the
    /// DingTalk anti-silence status line.
    fn display_name(&self) -> &str {
        self.name()
    }

    /// UI-friendly display label for this tool when rendered in a group
    /// chat — visible to non-requesters. Defaults to a generic "external
    /// service" fallback to avoid leaking internal capability surface to
    /// bystanders. Opt into sharper labeling by overriding both this and
    /// [`Self::safe_for_group_display`].
    fn group_display_name(&self) -> &str {
        "外部服务"
    }

    /// Whether this tool's real name + scrubbed parameter summary is safe
    /// to render in group chats. Defaults to `false`: unknown tools are
    /// opaque by default; tool authors opt in deliberately. Review:
    /// `docs/plans/2026-04-18-001-feat-dingtalk-anti-silence-ux-plan.md`
    /// (T1 threat mitigation).
    fn safe_for_group_display(&self) -> bool {
        false
    }

    /// Channel-facing one-line summary of a tool invocation (e.g.
    /// `"shell: ls -la"` or `"searching: \"ZStack best practices\""`).
    /// Defaults to the existing `channel::tool_call_detail` helper so
    /// unchanged tools keep their current rendering.
    ///
    /// Implementations SHOULD keep the summary under ~60 chars and MUST
    /// NOT include raw sensitive parameter values — the channel-side
    /// scrubber (see `channels::dingtalk::scrubber`) will run on the
    /// returned string before rendering, but defense-in-depth starts
    /// here.
    fn summary_for_ui(&self, params: &serde_json::Value) -> String {
        crate::channels::tool_call_detail(self.name(), params)
            .unwrap_or_else(|| self.name().to_string())
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

    /// Optional host-side webhook verification configuration for this tool.
    ///
    /// When present, `/webhook/tools/{tool}` validates shared secret/signatures
    /// before invoking the tool. Tools should then only handle payload normalization.
    fn webhook_capability(&self) -> Option<crate::tools::wasm::WebhookCapability> {
        None
    }

    /// Full parameter schema for discovery and coercion purposes.
    ///
    /// Unlike `parameters_schema()` (which may be permissive to keep the tools
    /// array compact), this returns the complete typed schema. Used by the
    /// `tool_info` built-in and by WASM parameter coercion.
    ///
    /// Default: delegates to `parameters_schema()`.
    fn discovery_schema(&self) -> serde_json::Value {
        self.parameters_schema()
    }

    /// Curated discovery guidance used by `tool_info(detail: "summary")`.
    ///
    /// Default: no custom summary; callers may derive a minimal fallback from
    /// `discovery_schema()`.
    fn discovery_summary(&self) -> Option<ToolDiscoverySummary> {
        None
    }

    /// Canonical provider extension that owns this action, when one exists.
    ///
    /// This lets the runtime resolve `action -> provider extension` without
    /// inferring ownership from the action name. MCP subtools should report the
    /// server extension name, and extension-backed WASM tools should report
    /// their extension id.
    fn provider_extension(&self) -> Option<&str> {
        None
    }

    /// Get the tool schema for LLM function calling.
    fn schema(&self) -> ToolSchema {
        let parameters = self.parameters_schema();
        let has_discovery_hint =
            self.discovery_summary().is_some() || self.discovery_schema() != parameters;
        let description = if has_discovery_hint {
            format!(
                "{} (call tool_info(name: \"{}\", detail: \"summary\") for rules/examples or detail: \"schema\" for the full discovery schema)",
                self.description(),
                self.name()
            )
        } else {
            self.description().to_string()
        };
        ToolSchema {
            name: self.name().to_string(),
            description,
            parameters,
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

/// Check if a tool invocation is allowed based on the job's approval context.
///
/// This helper function should be called by tools that execute sub-tools
/// (like the builder) to ensure proper approval checking is done even when
/// bypassing the worker's normal approval flow.
///
/// Returns `Ok(())` if the tool is allowed, `Err(ToolError::NotAuthorized)` if blocked.
///
/// # Security semantics
///
/// When `approval_context` is `None`, this function uses **legacy blocking behavior**:
/// - `Never` tools: allowed
/// - `UnlessAutoApproved` tools: blocked (require interactive approval)
/// - `Always` tools: blocked (require explicit approval)
///
/// This matches the worker-level `ApprovalContext::is_blocked_or_default()` semantics
/// to prevent privilege escalation.
///
/// # Example
///
/// ```rust,ignore
/// # use ironclaw::context::JobContext;
/// # use ironclaw::tools::{Tool, ToolError, ToolOutput, check_approval_in_context};
/// # use serde_json::Value;
/// async fn execute(&self, params: Value, ctx: &JobContext) -> Result<ToolOutput, ToolError> {
///     // If this tool executes sub-tools, check their approval first
///     check_approval_in_context(ctx, "sub_tool_name", self.requires_approval(&params))?;
///
///     // ... rest of implementation
///     # todo!()
/// }
/// ```
pub fn check_approval_in_context(
    ctx: &crate::context::JobContext,
    tool_name: &str,
    requirement: ApprovalRequirement,
) -> Result<(), ToolError> {
    // Match worker-level approval semantics exactly to prevent inconsistency
    if ApprovalContext::is_blocked_or_default(&ctx.approval_context, tool_name, requirement) {
        return Err(ToolError::NotAuthorized(format!(
            "Tool '{}' requires approval in this context",
            tool_name
        )));
    }
    Ok(())
}

/// Replace sensitive parameter values with `"[REDACTED]"`.
///
/// Returns a new JSON value with the specified keys replaced. Non-object params
/// and unknown keys are passed through unchanged. The original value is cloned
/// only if there are sensitive params to redact; otherwise it is cloned once
/// (cheap — callers own the result).
///
/// Used by the agent framework before logging, hook dispatch, approval display,
/// and `ActionRecord` storage so plaintext secrets never reach those paths.
pub fn redact_params(params: &serde_json::Value, sensitive: &[&str]) -> serde_json::Value {
    if sensitive.is_empty() {
        return params.clone();
    }
    let mut redacted = params.clone();
    if let Some(obj) = redacted.as_object_mut() {
        for key in sensitive {
            if obj.contains_key(*key) {
                obj.insert(
                    (*key).to_string(),
                    serde_json::Value::String("[REDACTED]".into()),
                );
            }
        }
    }
    redacted
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
/// Maximum nesting depth for tool schema validation to prevent stack overflow
/// on maliciously crafted schemas.
const MAX_SCHEMA_DEPTH: usize = 16;

/// Returns true if the schema uses `oneOf`, `anyOf`, or `allOf` combinators
/// where at least one variant is an object type (has `type: "object"` or `properties`).
fn has_object_combinator_variants(schema: &serde_json::Value) -> bool {
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(variants) = schema.get(key).and_then(|v| v.as_array())
            && variants.iter().any(|v| {
                v.get("type").and_then(|t| t.as_str()) == Some("object")
                    || v.get("properties").is_some()
            })
        {
            return true;
        }
    }
    false
}

pub fn validate_tool_schema(schema: &serde_json::Value, path: &str) -> Vec<String> {
    validate_tool_schema_inner(schema, path, 0)
}

fn validate_tool_schema_inner(schema: &serde_json::Value, path: &str, depth: usize) -> Vec<String> {
    let mut errors = Vec::new();

    if depth > MAX_SCHEMA_DEPTH {
        errors.push(format!(
            "{path}: schema nesting exceeds maximum depth of {MAX_SCHEMA_DEPTH}"
        ));
        return errors;
    }

    // Report non-array combinator values as errors.
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(val) = schema.get(key)
            && !val.is_array()
        {
            errors.push(format!("{path}: \"{key}\" must be an array"));
        }
    }

    let has_combinators = has_object_combinator_variants(schema);

    // Rule 1: must have "type": "object" at this level (unless combinators define the structure)
    match schema.get("type").and_then(|t| t.as_str()) {
        Some("object") => {}
        Some(other) => {
            errors.push(format!("{path}: expected type \"object\", got \"{other}\""));
            return errors; // Can't check further
        }
        None => {
            if !has_combinators {
                errors.push(format!("{path}: missing \"type\": \"object\""));
                return errors;
            }
        }
    }

    // Validate combinator variants recursively
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(variants) = schema.get(key).and_then(|v| v.as_array()) {
            for (i, variant) in variants.iter().enumerate() {
                if variant.get("type").and_then(|t| t.as_str()) == Some("object")
                    || variant.get("properties").is_some()
                {
                    let variant_path = format!("{path}.{key}[{i}]");
                    errors.extend(validate_tool_schema_inner(
                        variant,
                        &variant_path,
                        depth + 1,
                    ));
                }
            }
        }
    }

    // Rule 2: must have "properties" as an object (unless combinators define them)
    let properties = match schema.get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => {
            if !has_combinators {
                errors.push(format!("{path}: missing or non-object \"properties\""));
                return errors;
            }
            // Combinators define the structure — validate top-level `required` keys
            // against merged properties from all combinator variants.
            if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
                let mut merged_keys = std::collections::HashSet::new();
                if let Some(all_of) = schema.get("allOf").and_then(|a| a.as_array()) {
                    for variant in all_of {
                        if let Some(props) = variant.get("properties").and_then(|p| p.as_object()) {
                            merged_keys.extend(props.keys().cloned());
                        }
                    }
                }
                for key in ["oneOf", "anyOf"] {
                    if let Some(variants) = schema.get(key).and_then(|v| v.as_array()) {
                        for variant in variants {
                            if let Some(props) =
                                variant.get("properties").and_then(|p| p.as_object())
                            {
                                merged_keys.extend(props.keys().cloned());
                            }
                        }
                    }
                }
                for req in required {
                    if let Some(key) = req.as_str()
                        && !merged_keys.contains(key)
                    {
                        errors.push(format!(
                            "{path}: required key \"{key}\" not found in any combinator variant properties"
                        ));
                    }
                }
            }
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
                    errors.extend(validate_tool_schema_inner(prop, &prop_path, depth + 1));
                }
                "array" => {
                    if let Some(items) = prop.get("items") {
                        // If items is an object type, recurse
                        if items.get("type").and_then(|t| t.as_str()) == Some("object") {
                            errors.extend(validate_tool_schema_inner(
                                items,
                                &format!("{prop_path}.items"),
                                depth + 1,
                            ));
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
    use crate::testing::credentials::TEST_REDACT_SECRET;

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
    fn test_redact_params_replaces_sensitive_key() {
        let params = serde_json::json!({"name": "openai_key", "value": TEST_REDACT_SECRET});
        let redacted = redact_params(&params, &["value"]);
        assert_eq!(redacted["name"], "openai_key");
        assert_eq!(redacted["value"], "[REDACTED]");
        // Original unchanged
        assert_eq!(params["value"], TEST_REDACT_SECRET);
    }

    #[test]
    fn test_redact_params_empty_sensitive_is_noop() {
        let params = serde_json::json!({"name": "key", "value": "secret"});
        let redacted = redact_params(&params, &[]);
        assert_eq!(redacted, params);
    }

    #[test]
    fn test_redact_params_missing_key_is_noop() {
        let params = serde_json::json!({"name": "key"});
        let redacted = redact_params(&params, &["value"]);
        assert_eq!(redacted, params);
    }

    #[test]
    fn test_redact_params_non_object_is_passthrough() {
        let params = serde_json::json!("just a string");
        let redacted = redact_params(&params, &["value"]);
        assert_eq!(redacted, params);
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

    /// Regression test for issue #975: deeply nested schemas must not cause
    /// stack overflow. The validator should stop at MAX_SCHEMA_DEPTH and
    /// report an error instead of recursing infinitely.
    #[test]
    fn test_validate_schema_depth_limit() {
        // Build a schema nested 20 levels deep (exceeds MAX_SCHEMA_DEPTH=16)
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "leaf": { "type": "string" }
            }
        });
        for _ in 0..20 {
            schema = serde_json::json!({
                "type": "object",
                "properties": {
                    "nested": schema
                }
            });
        }
        let errors = validate_tool_schema(&schema, "test");
        assert!(
            errors.iter().any(|e| e.contains("maximum depth")),
            "expected depth limit error, got: {errors:?}"
        );
    }

    #[test]
    fn test_approval_context_autonomous_blocks_always_but_allows_soft() {
        let ctx = ApprovalContext::autonomous();
        // Never and UnlessAutoApproved are always allowed in autonomous context
        assert!(!ctx.is_blocked("shell", ApprovalRequirement::Never));
        assert!(!ctx.is_blocked("shell", ApprovalRequirement::UnlessAutoApproved));
        // Always tools are blocked unless explicitly listed
        assert!(ctx.is_blocked("shell", ApprovalRequirement::Always));
    }

    #[test]
    fn test_approval_context_autonomous_with_tools_allows_registered_name() {
        let ctx =
            ApprovalContext::autonomous_with_tools(["shell".to_string(), "message".to_string()]);
        assert!(!ctx.is_blocked("shell", ApprovalRequirement::Never));
        assert!(!ctx.is_blocked("shell", ApprovalRequirement::Always));
        assert!(!ctx.is_blocked("message", ApprovalRequirement::Always));
        assert!(ctx.is_blocked("http", ApprovalRequirement::Always));
    }

    #[test]
    fn test_approval_context_never_always_passes() {
        let ctx = ApprovalContext::autonomous();
        assert!(
            !ctx.is_blocked("any_tool", ApprovalRequirement::Never),
            "Never tools should always be allowed regardless of allowlist"
        );
    }

    #[test]
    fn test_is_blocked_or_default_with_none_uses_legacy() {
        // None context: all non-Never tools are blocked
        assert!(!ApprovalContext::is_blocked_or_default(
            &None,
            "any",
            ApprovalRequirement::Never
        ));
        assert!(ApprovalContext::is_blocked_or_default(
            &None,
            "any",
            ApprovalRequirement::UnlessAutoApproved
        ));
        assert!(ApprovalContext::is_blocked_or_default(
            &None,
            "any",
            ApprovalRequirement::Always
        ));
    }

    #[test]
    fn test_is_blocked_or_default_with_some_delegates() {
        let ctx = Some(ApprovalContext::autonomous_with_tools(
            ["shell".to_string()],
        ));
        assert!(!ApprovalContext::is_blocked_or_default(
            &ctx,
            "shell",
            ApprovalRequirement::Always
        ));
        assert!(ApprovalContext::is_blocked_or_default(
            &ctx,
            "other",
            ApprovalRequirement::Always
        ));
        // UnlessAutoApproved is allowed in autonomous context (auto-approved)
        assert!(!ApprovalContext::is_blocked_or_default(
            &ctx,
            "any",
            ApprovalRequirement::UnlessAutoApproved
        ));
    }
}
