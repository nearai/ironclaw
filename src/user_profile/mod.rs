//! Encrypted user profile engine for IronClaw.
//!
//! Builds an evolving model of each user based on their interactions.
//! Profile facts are encrypted at rest using the same AES-256-GCM
//! mechanism as credentials — the LLM never sees raw profile data
//! outside of the system prompt injection (which is already in-context).

pub mod distiller;
pub mod engine;
pub mod error;
pub mod types;
