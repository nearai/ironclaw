//! Extensible tool system.
//!
//! Tools are the agent's interface to the outside world. They can:
//! - Call external APIs
//! - Interact with the marketplace
//! - Execute sandboxed code (via WASM sandbox)
//! - Delegate tasks to other services
//! - Build new software and tools

pub mod builder;
pub mod builtin;
pub mod mcp;
pub mod rate_limiter;
pub mod retry;
pub mod wasm;

mod registry;
mod tool;

pub use builder::{
    BuildPhase, BuildRequirement, BuildResult, BuildSoftwareTool, BuilderConfig, Language,
    LlmSoftwareBuilder, SoftwareBuilder, SoftwareType, Template, TemplateEngine, TemplateType,
    TestCase, TestHarness, TestResult, TestSuite, ValidationError, ValidationResult, WasmValidator,
};
pub use rate_limiter::RateLimiter;
pub use registry::ToolRegistry;
pub use retry::{ToolRetryOutcome, effective_retry_config, retry_tool_execute};
pub use tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolErrorKind, ToolOutput,
    ToolRateLimitConfig, ToolRetryConfig,
};
