//! Test-support vocabulary for auth contracts — deterministic conformance
//! suites shared across this crate's own tier and downstream integration
//! tiers. Gated behind `#[cfg(any(test, feature = "test-support"))]` so the
//! panic-on-violation assertion harnesses ship zero bytes in production
//! binaries; downstream crates enable the `test-support` feature from their
//! `[dev-dependencies]`.

pub mod conformance;
