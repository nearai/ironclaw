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
//! - [`messaging`]: [`StandardMessagingOp`], the closed standard messaging
//!   operation vocabulary, canonical input/output schemas, description cores,
//!   and the `messaging.*` error-code taxonomy.
#![warn(unreachable_pub)]

pub mod action;
pub mod approval;
pub mod attachment;
pub mod audit;
pub mod authorized;
pub mod capability;
pub mod capability_profile;
pub mod capability_surface;
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
pub mod messaging;
pub mod mount;
pub mod outbound;
pub mod path;
pub mod resolution;
pub mod resource;
pub mod result_meta;
pub mod runtime;
pub mod runtime_policy;
pub mod safe_summary;
pub mod scope;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod trust;
pub mod turn;
pub mod user_identity;

mod credential_redaction;
pub mod model_result_preview;
pub mod process;
pub mod product_adapter;
pub mod product_adapter_error;

// There is deliberately no flat re-export prelude here. Every contract is
// reached through the module that owns it — `ironclaw_host_api::scope::
// ExecutionContext`, not `ironclaw_host_api::ExecutionContext` — so each
// consumer's real dependency is compiler-visible. That visibility is what
// lets a vocabulary family be carved out of this crate without having to
// guess which consumers used it. Do not re-add per-module glob re-exports here
// (see this crate's CLAUDE.md); the `Timestamp` alias below is the only item
// this crate exposes at its root.

/// Canonical timestamp type for host API wire contracts.
pub type Timestamp = chrono::DateTime<chrono::Utc>;
