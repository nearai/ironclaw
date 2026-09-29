//! Jev as the turn-start tool classifier.
//!
//! [`JevToolClassifier`] implements `ironclaw_loop_contracts`'
//! `ToolSelectionClassifier` port with Jev, TypeSafe's hosted classification
//! model, reached through a configured decisions endpoint: TypeSafe's by
//! default, or any provider serving the same decisions API. For every
//! candidate tool it asks one `noul` question, "how likely is it that this
//! tool will be used in the following conversation?", and keeps the tools
//! with the highest probabilities. See this package's `README.md` for the
//! request shape, slicing, selection and failure rules.
//!
//! # Confidentiality
//!
//! Every request sends the conversation context (at a conversation's first
//! turn, its first user message, cut to 16 KiB; at a re-selection, a window
//! of recent user messages) and every candidate tool's
//! name, description and parameter names to the configured Jev provider, a
//! third party. Nothing runs on the host. Neither the text sent, the answers
//! received nor the API key is ever logged; errors carry stable labels only.
//!
//! All HTTP goes through `ironclaw_network`'s policy egress, pinned to the
//! configured endpoint's host and port over HTTPS with private address
//! ranges denied.

mod classifier;
mod endpoint;
mod request;
mod response;

#[cfg(feature = "test-support")]
pub use classifier::with_stub_endpoint;
pub use classifier::{
    DEFAULT_JEV_MODEL, DEFAULT_MAX_SLICE_TOKENS, JEV_CLASSIFIER_NAME, JevApiKey, JevConfigError,
    JevToolClassifier,
};
pub use endpoint::{DEFAULT_JEV_ENDPOINT, JevEndpoint};
