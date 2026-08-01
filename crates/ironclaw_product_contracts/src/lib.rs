//! The product-tier contract: the `ProductSurface` membrane every transport
//! calls through, the wire DTOs that cross it, and the product-side ports
//! whose implementations sit beside or below product — expressed so WebUI, the
//! OpenAI-compatible adapter, the operator surface, the extension host, and
//! channel packages can all compile against the product boundary without
//! compiling `ironclaw_product`.
//!
//! This crate is vocabulary and ports only. It implements no surface, admits
//! nothing, delivers nothing, and speaks no HTTP. `ironclaw_product` owns the
//! `ProductSurface` implementation and the frozen inventory of concrete
//! commands, views, and capabilities; this crate owns only their shapes. See
//! `docs/reborn/target-architecture/PROPOSAL.md` §6.1.3 and
//! `families/contracts.md`.
//!
//! Admission test for a type here (the contracts-family four-part test):
//! it names a concept crossing the product boundary, it is neutral across
//! vendor/runtime/storage/deployment, at least two consumers need it without
//! importing an owner, and it carries no execution, persistence, policy
//! engine, or workflow.
//!
//! Two rules this crate is enforced against, both in
//! `crates/ironclaw_architecture/tests/`:
//!
//! - **Contracts purity** (§11.2.3, `reborn_dependency_boundaries.rs`): the
//!   only internal dependencies are `ironclaw_host_api` and
//!   `ironclaw_extension_contracts` — the one-way street §6.1.3 grants, so
//!   channel-facing DTOs are reused rather than duplicated. No framework,
//!   driver, or runtime client may appear.
//! - **Port location** (§11.2.4, `reborn_product_contract_location_scan.rs`):
//!   every type here is defined here exactly once and no other crate
//!   re-exports it. There is deliberately no second import path for anything
//!   in this crate.
#![warn(unreachable_pub)]

pub mod inbound;
pub mod interaction_commands;
pub mod operator_llm;
pub mod outbound;
pub mod package_lifecycle;
pub mod projection;
pub mod surface;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

// There is deliberately no flat prelude and no cross-module re-export here.
// Every contract is reached through the module that owns it —
// `ironclaw_product_contracts::surface::ProductSurface`, not
// `ironclaw_product_contracts::ProductSurface` — so each consumer's real
// dependency stays compiler-visible, exactly as `ironclaw_host_api` does after
// its de-wildcarding, and exactly as `ironclaw_extension_contracts` does. The
// port-location scan pins that no other crate re-exports these names either.
