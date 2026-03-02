//! Handler modules for the web gateway API.
//!
//! Each module groups related endpoint handlers by domain.
//!
//! # Migration status
//!
//! `skills` is the canonical implementation used by `server.rs`.
//! The remaining modules are in-progress migrations from inline server.rs
//! handlers; their functions are not yet wired up, hence the `dead_code` allow.

pub mod jobs;
pub mod skills;

// Modules not yet wired into server.rs router -- suppress dead_code until
// they replace their inline counterparts.
#[allow(dead_code)]
pub mod chat;
#[allow(dead_code)]
pub mod extensions;
#[allow(dead_code)]
pub mod memory;
#[allow(dead_code)]
pub mod routines;
#[allow(dead_code)]
pub mod settings;
#[allow(dead_code)]
pub mod static_files;
