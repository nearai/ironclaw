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
