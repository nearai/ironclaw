//! Lifecycle hooks for intercepting and transforming agent operations.
//!
//! The hook system provides 11 well-defined interception points:
//!
//! - **BeforeInbound** — Before processing an inbound user message
//! - **BeforeToolCall** — Before executing a tool call
//! - **BeforeOutbound** — Before sending an outbound response
//! - **OnSessionStart** — When a new session starts
//! - **OnSessionEnd** — When a session ends
//! - **TransformResponse** — Transform the final response before completing a turn
//! - **AfterParse** — After parsing the user submission
//! - **BeforeAgenticLoop** — Before entering the agentic loop
//! - **BeforeLlmCall** — Before each LLM call
//! - **AfterToolCall** — After a tool call completes
//! - **BeforeApproval** — Before presenting a tool-approval request
//!
//! Hooks are executed in priority order (lower number = higher priority).
//! Each hook can pass through, modify content, or reject the event.

pub mod bundled;
pub mod hook;
pub mod registry;
pub mod wasm_hook;
pub mod webhook;
pub mod workspace;

pub use hook::{Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint};
pub use registry::HookRegistry;
pub use wasm_hook::WasmHookWrapper;
pub use webhook::WebhookHook;
pub use workspace::register_workspace_hooks;
