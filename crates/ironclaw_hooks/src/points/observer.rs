//! Context for observer hook points (`after_model`, `after_capability`,
//! `after_checkpoint`).

use ironclaw_host_api::TenantId;

/// Read-only context handed to an observer hook. As with the other points,
/// `#[non_exhaustive]` so additional fields can land without breaking authors.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ObserverHookContext {
    pub tenant_id: TenantId,
    pub observed_kind: ObservedKind,
}

/// What kind of fact the observer is being notified about. The dispatcher
/// dispatches one hook list per kind, so a single hook implementation is
/// scoped to one observation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedKind {
    /// A model call returned. The observer sees only that an exchange
    /// happened, never the model's raw output.
    AfterModel,
    /// A capability invocation completed (successfully or otherwise).
    AfterCapability,
    /// A checkpoint was written.
    AfterCheckpoint,
}
