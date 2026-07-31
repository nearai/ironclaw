//! The extension-tier contract: what an installable extension *declares* and
//! *exposes*, expressed so lanes, hosts, packages, product, and the extension
//! manager can share one vocabulary without any of them importing a registry
//! or an owner.
//!
//! This crate is vocabulary and ports only — value types, manifest-surface
//! descriptors, recipe schemas, lifecycle state machines, and the one
//! vendor-implemented codec port. It parses no manifests, stores no
//! installations, routes no ingress, and executes no lifecycle. Those live
//! above it, in `ironclaw_extension_registry` (records) and
//! `ironclaw_extension_host` (execution). See
//! `docs/reborn/target-architecture/PROPOSAL.md` §6.1.2 and
//! `families/contracts.md`.
//!
//! Admission test for a type here (the contracts-family four-part test):
//! it names a concept crossing the host↔extension membrane, it is neutral
//! across vendor/runtime/storage/deployment, at least two consumers need it
//! without importing an owner, and it carries no execution, persistence,
//! policy engine, or workflow.
//!
//! Two rules this crate is enforced against, both in
//! `crates/ironclaw_architecture/tests/`:
//!
//! - **Contracts purity** (§11.2.3, `reborn_dependency_boundaries.rs`): the
//!   only internal dependency is `ironclaw_host_api`; no framework, driver, or
//!   runtime client may appear.
//! - **Port location** (§11.2.4, `reborn_extension_contract_location_scan.rs`):
//!   every type here is defined here exactly once and no other crate
//!   re-exports it. There is deliberately no second import path for anything
//!   in this crate — the dual-path re-export is the defect the scan exists to
//!   prevent.
#![warn(unreachable_pub)]

pub mod auth_prompt;
pub mod channel;
pub mod channel_adapter;
pub mod channel_identity;
pub mod egress;
pub mod extension;
pub mod external;
pub mod memory;
pub mod preference_target;
pub mod recipe;
pub mod state;
pub mod surface;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod tool_adapter;

// There is deliberately no flat prelude and no cross-module re-export here.
// Every contract is reached through the module that owns it —
// `ironclaw_extension_contracts::state::InstallationState`, not
// `ironclaw_extension_contracts::InstallationState` — so each consumer's real
// dependency stays compiler-visible, exactly as `ironclaw_host_api` does after
// its de-wildcarding. The port-location scan pins that no other crate re-exports
// these names either.
