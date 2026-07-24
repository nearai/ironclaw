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
//! - [`runtime_policy`]: deployment mode, runtime profile, and effective
//!   runtime policy vocabulary consumed by the resolver in
//!   `ironclaw_runtime_policy` and the host runtime planner.
//! - [`ingress`]: host-owned HTTP ingress descriptors for product/API surfaces.
#![warn(unreachable_pub)]

pub mod action;
pub mod approval;
pub mod audit;
pub mod authorized;
pub mod capability;
pub mod capability_profile;
pub mod channel;
pub mod decision;
pub mod dispatch;
#[cfg(feature = "test-support")]
pub mod dispatch_test_support;
mod dotted_id;
pub mod error;
pub mod failure;
pub mod gate_record;
pub mod host_port;
pub mod host_remediation;
pub mod http;
pub mod ids;
pub mod ingress;
pub mod invocation;
pub mod lane;
pub mod mount;
pub mod outbound;
pub mod path;
pub mod recipe;
pub mod resolution;
pub mod resource;
pub mod result_meta;
pub mod runtime;
pub mod runtime_policy;
pub mod safe_summary;
pub mod scope;
pub mod state;
pub mod surface;
pub mod tool_adapter;
pub mod trust;
pub mod turn;

mod credential_redaction;
pub mod model_result_preview;
pub mod product_adapter;
pub mod product_adapter_error;
pub mod product_surface;

// Flat re-exports are intentional: downstream Reborn service crates consume
// `ironclaw_host_api` as a contract prelude, while module docs remain the
// authoritative grouping for each vocabulary family.
pub use action::*;
pub use approval::*;
pub use audit::*;
pub use authorized::*;
pub use capability::*;
pub use capability_profile::*;
pub use channel::*;
pub use decision::*;
pub use dispatch::*;
pub use error::*;
pub use failure::*;
pub use gate_record::*;
pub use host_port::*;
pub use host_remediation::*;
pub use http::*;
pub use ids::*;
pub use ingress::*;
pub use invocation::*;
pub use lane::*;
pub use model_result_preview::*;
pub use mount::*;
pub use outbound::*;
pub use path::*;
pub use product_adapter::*;
pub use product_adapter_error::*;
pub use product_surface::*;
pub use recipe::*;
pub use resolution::*;
pub use resource::*;
pub use result_meta::*;
pub use runtime::*;
pub use runtime_policy::*;
pub use safe_summary::*;
pub use scope::*;
pub use state::*;
pub use surface::*;
pub use tool_adapter::*;
pub use trust::*;
pub use turn::*;

/// Canonical timestamp type for host API wire contracts.
pub type Timestamp = chrono::DateTime<chrono::Utc>;
