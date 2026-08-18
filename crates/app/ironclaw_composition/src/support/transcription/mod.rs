//! Host-side speech-to-text assembly.
//!
//! Two halves, both thin:
//!
//! - [`build_transcription_provider`] resolves whether this deployment *has* a
//!   transcription backend, from the LLM config it already resolved. No new
//!   credential, no new egress host.
//! - [`LlmTranscriptionService`] adapts `ironclaw_llm`'s
//!   [`TranscriptionProvider`] to the product-tier
//!   [`TranscriptionService`] port, which is the one boundary where a provider
//!   failure becomes a stable, redacted product error.
//!
//! This lives in composition rather than in product because the port exists
//! precisely so product need not name the model tier — see
//! `ironclaw_product_contracts::transcription`.

pub(crate) mod service;

pub(crate) use service::{LlmTranscriptionService, build_transcription_provider};
