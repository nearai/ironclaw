//! Sealed decision and patch types returned by hooks.
//!
//! Each module here defines a public outer type whose internals are
//! `pub(crate)`. Hooks cannot construct decisions directly — they go through
//! the sink trait surface in [`crate::sink`], which is the only path that can
//! reach the `pub(crate)` constructors. This is the same witness pattern
//! `LoopExitValidationPolicy` adopted in PR #3460: the trust property is
//! enforced by the type system, not by convention.

pub mod gate;
pub mod mutator;
pub mod observer;

pub use gate::BeforeCapabilityHookDecision;
pub use mutator::{HookPatch, PatchOrdinalHint};
pub use observer::ObserverFact;
