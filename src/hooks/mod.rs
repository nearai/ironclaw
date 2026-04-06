//! Lifecycle hooks for intercepting and transforming agent operations.
//!
//! The hook system provides 6 well-defined interception points:
//!
//! - **BeforeInbound** — Before processing an inbound user message
//! - **BeforeToolCall** — Before executing a tool call
//! - **BeforeOutbound** — Before sending an outbound response
//! - **OnSessionStart** — When a new session starts
//! - **OnSessionEnd** — When a session ends
//! - **TransformResponse** — Transform the final response before completing a turn
//!
//! Hooks are executed in priority order (lower number = higher priority).
//! Each hook can pass through, modify content, or reject the event.

pub mod bootstrap;
pub mod bundled;
pub mod hook;
pub mod registry;

pub use bootstrap::{HookBootstrapSummary, bootstrap_hooks};
pub use bundled::{
    HookBundleConfig, HookRegistrationSummary, register_bundle, register_bundled_hooks,
};
// Re-export from ironclaw_hooks so the rest of the codebase can use
// ironclaw::hooks::{Hook, ...} without needing to know about ironclaw_hooks directly.
pub use ironclaw_hooks::{Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint};
pub use registry::HookRegistry;
