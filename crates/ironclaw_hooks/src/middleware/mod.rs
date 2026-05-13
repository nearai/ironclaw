//! Port middleware wrappers that compose `HookDispatcher` with the existing
//! Reborn host ports. Each wrapper presents the same `Loop*Port` trait
//! signature as the inner port, so callers (`PlannedDriver`,
//! `TextOnlyModelReplyDriver`, etc.) are unaffected.
//!
//! The wrappers live in this crate (rather than in `ironclaw_reborn`) so the
//! dispatcher's invariants (panic isolation, fail-closed gate composition,
//! envelope-only Installed snippets, ordering, poisoning) stay co-located
//! with the dispatcher itself. Reborn's composition root just plumbs the
//! wrapped port through.

pub mod capability_port;
pub mod checkpoint_port;
pub mod gate_ref;
pub mod model_port;
pub mod prompt_port;
pub mod resolver;
pub mod transcript_port;

pub use capability_port::HookedLoopCapabilityPort;
pub use checkpoint_port::HookedLoopCheckpointPort;
pub use gate_ref::{HookGateRefFactory, UuidHookGateRefFactory};
pub use model_port::HookedLoopModelPort;
pub use prompt_port::HookedLoopPromptPort;
pub use resolver::{
    CapabilityInputResolver, CapabilityProviderResolver, NullCapabilityInputResolver,
    NullCapabilityProviderResolver,
};
pub use transcript_port::HookedLoopTranscriptPort;
