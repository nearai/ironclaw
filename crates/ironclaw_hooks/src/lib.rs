//! Reborn loop hook framework.
//!
//! See `CLAUDE.md` in this crate for the trust model, dependency direction, and
//! non-negotiable invariants. The short version:
//!
//! - Hooks have four trust classes (Builtin, Trusted, Installed, SelfAuthored)
//!   enforced at the type level via the [`sink`] traits.
//! - Decision and patch types in [`kinds`] are sealed: only this crate can mint
//!   them, so an extension cannot forge a trusted policy through `pub` fields.
//! - The framework owns the contract, but Reborn host composition is now wired:
//!   `HookedLoopCapabilityPort`, `HookedLoopPromptPort`, and the other
//!   middleware in [`middleware`] wrap the corresponding Reborn loop ports and
//!   are installed by `ironclaw_reborn`'s loop driver host.

pub mod dispatch;
pub mod error;
pub mod evaluator;
pub mod failure_policy;
pub mod identity;
pub mod installed_hook;
pub mod kinds;
pub mod manifest;
pub mod middleware;
pub mod ordering;
pub mod points;
pub mod predicate;
pub mod predicate_state;
pub mod registrar;
pub mod registry;
pub mod self_authored;
pub mod sink;
pub mod telemetry;
pub mod trust;

pub use error::HookError;
pub use failure_policy::{FailureCategory, FailureDisposition};
pub use identity::{ExtensionId, HookId, HookLocalId, HookVersion};
pub use ordering::{HookPhase, HookPriority};
pub use registrar::HookRegistrar;
pub use registry::{HookBinding, HookBindingScope, HookRegistry};
pub use self_authored::{
    GenerationTraceRef, SelfAuthoredBeforeCapabilityHook, SelfAuthoredEvaluator,
    SelfAuthoredHookSink, SelfAuthoredHookSpec, SelfAuthoredReason, SelfAuthorshipProvenance,
    UserRatificationProof,
};
pub use trust::HookTrustClass;
