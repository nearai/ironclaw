//! The extension-tier contract: what an installable extension *declares* and
//! *exposes*, expressed so lanes, hosts, packages, product, and the extension
//! manager can share one vocabulary without any of them importing a registry
//! or an owner.
//!
//! This crate is vocabulary and ports only — value types, manifest-surface
//! descriptors, recipe schemas, lifecycle state machines, the one
//! vendor-implemented codec port, and the sealed verified-inbound evidence
//! constructors. It parses no manifests, stores no installations, routes no
//! ingress, and executes no lifecycle. Those live above it, in
//! `ironclaw_extension_registry` (records) and `ironclaw_extension_host`
//! (execution). See `docs/reborn/target-architecture/PROPOSAL.md` §6.1.2 and
//! `families/contracts.md`.
//!
//! **Security role (§6.1.2, §12.1a):** this crate is the host↔extension
//! membrane, and it owns inbound-verification evidence minting exclusively —
//! see [`verified_inbound`]. A channel package may misreport parsed content but
//! can never forge verification or scope, because the mint family is witness-
//! gated and the sole grant source lives in the generic ingress verifier. It
//! declares that seal; it never performs verification itself.
//!
//! Admission test for a type here (the contracts-family four-part test):
//! it names a concept crossing the host↔extension membrane, it is neutral
//! across vendor/runtime/storage/deployment, at least two consumers need it
//! without importing an owner, and it carries no execution, persistence,
//! policy engine, or workflow.
//!
//! Two rules this crate is enforced against, both in
//! `crates/app/ironclaw_architecture_tests/tests/`:
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
pub mod hosted_mcp;
pub mod lifecycle_id;
pub mod memory;
pub mod preference_target;
pub mod product_adapter_section;
pub mod recipe;
pub mod runtime;
pub mod state;
pub mod surface;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod tool_adapter;
pub mod verified_inbound;

// There is deliberately no flat prelude and no cross-module re-export here.
// Every contract is reached through the module that owns it —
// `ironclaw_extension_contracts::state::InstallationState`, not
// `ironclaw_extension_contracts::InstallationState` — so each consumer's real
// dependency stays compiler-visible, exactly as `ironclaw_host_api` does after
// its de-wildcarding. The port-location scan pins that no other crate re-exports
// these names either.
