//! Extensible tool system.
//!
//! Tools are the agent's interface to the outside world. They can:
//! - Call external APIs
//! - Interact with the marketplace
//! - Execute sandboxed code
//! - Delegate tasks to other services

pub mod builtin;
pub mod mcp;

mod builder;
mod registry;
mod sandbox;
mod tool;

pub use builder::{DynamicTool, SandboxConfig, ToolBuilder, ToolRequirement};
pub use registry::ToolRegistry;
pub use sandbox::ToolSandbox;
pub use tool::{Tool, ToolError, ToolOutput};
