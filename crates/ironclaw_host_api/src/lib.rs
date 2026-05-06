//! Shared host API contracts for IronClaw Reborn.
//!
//! `ironclaw_host_api` is the vocabulary every Reborn system-service crate uses
//! to describe authority: who is acting, which extension/runtime is acting, what
//! filesystem mounts are visible, which capabilities were granted, what resources
//! may be spent, what action is requested, and what decision/obligations the host
//! produced.
//!
//! This crate intentionally contains authority-bearing types, validation, and
//! serialization contracts only. Runtime behavior belongs in system-service
//! crates such as filesystem, resources, extensions, WASM, MCP, auth, network,
//! and kernel.
//!
//! The main contract groups are:
//!
//! - [`ids`]: validated identity, scope, extension, capability, and audit IDs.
//! - [`path`] and [`mount`]: host-internal paths, virtual durable paths, scoped
//!   runtime paths, and mount permissions.
//! - [`scope`]: [`ExecutionContext`], the authority envelope for one invocation.
//! - [`capability`]: capability descriptors and grants; declarations do not grant
//!   authority by themselves.
//! - [`action`], [`decision`], and [`approval`]: normalized requested effects,
//!   host decisions, obligations, and approval scopes.
//! - [`resource`]: budget/resource scopes, estimates, usage, and quota contracts.
//! - [`audit`]: redacted durable audit envelope shapes.
//! - [`trust`]: requested-trust vocabulary and `PackageIdentity` consumed by
//!   the host trust policy engine in `ironclaw_trust`.

pub mod action;
pub mod approval;
pub mod audit;
pub mod capability;
pub mod decision;
pub mod dispatch;
pub mod error;
pub mod http;
pub mod ids;
pub mod mount;
pub mod path;
pub mod resource;
pub mod runtime;
pub mod scope;
pub mod trust;

// Flat re-exports are intentional: downstream Reborn service crates consume
// `ironclaw_host_api` as a contract prelude, while module docs remain the
// authoritative grouping for each vocabulary family.
pub use action::*;
pub use approval::*;
pub use audit::*;
pub use capability::*;
pub use decision::*;
pub use dispatch::*;
pub use error::*;
pub use http::*;
pub use ids::*;
pub use mount::*;
pub use path::*;
pub use resource::*;
pub use runtime::*;
pub use scope::*;
pub use trust::*;

/// Canonical timestamp type for host API wire contracts.
pub type Timestamp = chrono::DateTime<chrono::Utc>;
