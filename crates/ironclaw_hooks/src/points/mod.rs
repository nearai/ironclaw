//! Hook point contexts — typed input the dispatcher hands a hook when it
//! fires. Each context is read-only (`&` access only); hooks express change
//! through the [`crate::kinds`] return types, never through mutating the
//! context.
//!
//! Contexts are intentionally minimal in this first slice. As the Reborn
//! middleware wiring lands, additional read-only fields can be added (e.g.,
//! `run_context: &LoopRunContext`, `iteration: u32`, capability surface
//! version) without breaking existing hook authors because everything is
//! `#[non_exhaustive]`.

pub mod capability;
pub mod event_triggered;
pub mod observer;
pub mod prompt;

pub use capability::{BeforeCapabilityHookContext, SanitizedArguments};
pub use event_triggered::EventTriggeredHookContext;
pub use observer::{ObservedKind, ObserverHookContext};
pub use prompt::BeforePromptHookContext;
