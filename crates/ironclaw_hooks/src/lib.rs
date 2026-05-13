//! Reborn loop hook framework.
//!
//! See `CLAUDE.md` in this crate for the trust model, dependency direction, and
//! non-negotiable invariants. The short version:
//!
//! - Hooks have three trust classes (Builtin, Trusted, Installed) enforced at
//!   the type level via the [`sink`] traits.
//! - Decision and patch types in [`kinds`] are sealed: only this crate can mint
//!   them, so an extension cannot forge a trusted policy through `pub` fields.
//! - The framework owns the contract, not the runtime composition. Reborn wraps
//!   `LoopCapabilityPort` / `LoopPromptPort` / etc. with [`dispatch`] in a
//!   follow-up slice.

pub mod dispatch;
pub mod error;
pub mod failure_policy;
pub mod identity;
pub mod kinds;
pub mod manifest;
pub mod ordering;
pub mod points;
pub mod predicate;
pub mod registry;
pub mod sink;
pub mod trust;

pub use error::HookError;
pub use failure_policy::{FailureCategory, FailureDisposition};
pub use identity::{ExtensionId, HookId, HookLocalId, HookVersion};
pub use ordering::{HookPhase, HookPriority};
pub use registry::{HookBinding, HookRegistry};
pub use trust::HookTrustClass;
