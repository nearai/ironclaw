//! OpenAI-compatible HTTP API (`/v1/chat/completions`, `/v1/models`).
//!
//! This module provides a direct LLM proxy through the web gateway so any
//! standard OpenAI client library can use IronClaw as a backend by simply
//! changing the `base_url`.

pub mod handlers;
pub mod stream;
pub mod translate;
pub mod types;

pub use handlers::{chat_completions_handler, models_handler};
