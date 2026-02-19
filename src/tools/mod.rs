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
pub mod idempotency;
pub mod mcp;
pub mod wasm;

mod registry;
mod tool;

pub use builder::{
    BuildPhase, BuildRequirement, BuildResult, BuildSoftwareTool, BuilderConfig, Language,
    LlmSoftwareBuilder, SoftwareBuilder, SoftwareType, Template, TemplateEngine, TemplateType,
    TestCase, TestHarness, TestResult, TestSuite, ValidationError, ValidationResult, WasmValidator,
};
pub use idempotency::{IdempotencyCacheConfig, ToolIdempotencyCache};
pub use registry::ToolRegistry;
pub use tool::{Tool, ToolDomain, ToolError, ToolOutput};
