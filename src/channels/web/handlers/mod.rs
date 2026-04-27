//! Handler modules for the web gateway API.
//!
//! Each module groups related endpoint handlers by domain.

pub mod auth;
pub mod engine;
pub mod llm;
pub mod memory;
pub mod secrets;
pub mod skills;
pub mod system_prompt;
pub mod tokens;
pub mod tool_policy;
pub mod users;

pub mod frontend;
pub mod webhooks;
