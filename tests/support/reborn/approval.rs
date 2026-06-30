//! Approval helpers for Reborn parity harnesses.
//!
//! This module intentionally does not replace run state, gate persistence, or
//! authorization stores. Full approval helpers are added with the runtime
//! harness that drives the real blocked/resume path.

#![allow(dead_code)] // External-boundary shims consumed by future binary-E2E tests.

use super::config::WaitConfig;

pub type ApprovalWaitConfig = WaitConfig;

/// Re-export of the canonical gate reference so approval tests name a single
/// `GateRef` type. The gate-resolution *logic* lives on the harness (where the
/// approval-store fields live, `builder.rs` + `harness.rs`); this module is
/// types-only.
// Not every test binary that mounts the support tree consumes this re-export,
// so it reads as unused there under `-D warnings`; the module-level
// `#![allow(dead_code)]` does not cover `unused_imports` for a `pub use`.
#[allow(unused_imports)]
pub use ironclaw_turns::GateRef;
