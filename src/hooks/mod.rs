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

pub mod hook;
pub mod registry;

pub use hook::{Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint};
pub use registry::HookRegistry;
