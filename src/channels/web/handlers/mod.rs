//! Handler modules for the web gateway API.
//!
//! Each module groups related endpoint handlers by domain.

pub mod auth;
pub mod engine;
pub mod jobs;
pub mod llm;
pub mod memory;
pub mod routines;
pub mod secrets;
pub mod skills;
pub mod system_prompt;
pub mod tokens;
pub mod tool_policy;
pub mod users;

// Modules not yet wired into server.rs router -- suppress dead_code until
// they replace their inline counterparts.
#[allow(dead_code)]
pub mod chat;
#[allow(dead_code)]
pub mod extensions;
pub mod frontend;
#[allow(dead_code)]
pub mod settings;
#[allow(dead_code)]
pub mod static_files;
pub mod webhooks;
