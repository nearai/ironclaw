//! Capability management.
//!
//! - [`CapabilityRegistry`] — stores known capabilities and their actions
//! - [`LeaseManager`] — grants, validates, and expires capability leases
//! - [`PolicyEngine`] — deterministic effect-level allow/deny/approve

pub mod lease;
pub mod policy;
pub mod registry;

pub use lease::LeaseManager;
pub use policy::{PolicyDecision, PolicyEngine};
pub use registry::CapabilityRegistry;
