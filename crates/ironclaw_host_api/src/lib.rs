//! Shared host API contracts for IronClaw Reborn.
//!
//! This crate intentionally contains authority-bearing types, validation, and
//! serialization contracts only. Runtime behavior belongs in system-service
//! crates such as filesystem, resources, extensions, WASM, MCP, auth, network,
//! and kernel.

pub mod action;
pub mod approval;
pub mod audit;
pub mod capability;
pub mod decision;
pub mod error;
pub mod ids;
pub mod mount;
pub mod path;
pub mod resource;
pub mod runtime;
pub mod scope;

pub use action::*;
pub use approval::*;
pub use audit::*;
pub use capability::*;
pub use decision::*;
pub use error::*;
pub use ids::*;
pub use mount::*;
pub use path::*;
pub use resource::*;
pub use runtime::*;
pub use scope::*;

/// Canonical timestamp type for host API wire contracts.
pub type Timestamp = chrono::DateTime<chrono::Utc>;
