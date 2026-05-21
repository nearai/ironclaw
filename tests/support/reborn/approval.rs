//! Approval helpers for Reborn parity harnesses.
//!
//! This module intentionally does not replace run state, gate persistence, or
//! authorization stores. Full approval helpers are added with the runtime
//! harness that drives the real blocked/resume path.

#![allow(dead_code)] // External-boundary shims consumed by future binary-E2E tests.

use super::config::WaitConfig;

pub type ApprovalWaitConfig = WaitConfig;
