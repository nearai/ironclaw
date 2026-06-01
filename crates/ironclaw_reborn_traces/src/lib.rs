//! Trace Commons / TraceDAO client extracted from the IronClaw monolith.
//!
//! This crate holds the trace contribution pipeline (`contribution`), the
//! host-facing trace client (`client`), the redaction helpers used to scrub
//! sensitive JSON before submission (`redaction`), and the shared
//! `ConversationMessage` type that the legacy monolith's `history` module now
//! re-exports for backward compatibility.

pub mod client;
pub mod contribution;
pub mod conversation_message;
pub mod redaction;

pub use conversation_message::ConversationMessage;

/// Recorded-trace deserialization surface for callers that load JSON traces
/// off disk (e.g. `ironclaw-reborn traces preview`). Re-exports from
/// `ironclaw_llm::recording` so reborn-cli does not need a direct
/// `ironclaw_llm` dependency, preserving the architectural boundary.
pub mod recording {
    pub use ironclaw_llm::recording::*;
}

/// Filesystem path resolution for trace-contribution storage. Re-exports
/// from `ironclaw_common::paths` so reborn-cli does not need a direct
/// `ironclaw_common` dependency.
pub mod paths {
    pub use ironclaw_common::paths::*;
}
