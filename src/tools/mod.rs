//! Extensible tool system.
//!
//! Tools are the agent's interface to the outside world. They can:
//! - Call external APIs
//! - Interact with the marketplace
//! - Execute sandboxed code (via WASM sandbox)
//! - Delegate tasks to other services
//! - Build new software and tools

mod autonomy;
pub mod builder;
pub mod builtin;
mod coercion;
pub mod execute;
pub mod mcp;
pub mod rate_limiter;
pub mod redaction;
pub mod schema_validator;
pub mod wasm;

mod registry;
mod tool;

pub use autonomy::{
    AUTONOMOUS_TOOL_DENYLIST, autonomous_allowed_tool_names, autonomous_unavailable_error,
    autonomous_unavailable_message, is_autonomous_tool_denylisted,
};
pub use builder::{
    BuildPhase, BuildRequirement, BuildResult, BuildSoftwareTool, BuilderConfig, Language,
    LlmSoftwareBuilder, SoftwareBuilder, SoftwareType, Template, TemplateEngine, TemplateType,
    TestCase, TestHarness, TestResult, TestSuite, ValidationError, ValidationResult, WasmValidator,
};
pub(crate) use coercion::prepare_tool_params;
pub use rate_limiter::RateLimiter;
pub use registry::ToolRegistry;
pub use tool::{
    ApprovalContext, ApprovalRequirement, RiskLevel, Tool, ToolDomain, ToolError, ToolOutput,
    ToolRateLimitConfig, redact_params, validate_tool_schema,
};
