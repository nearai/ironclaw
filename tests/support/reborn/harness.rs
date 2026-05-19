//! Reborn binary-E2E harness skeleton.
//!
//! The strict harness must wire real Reborn workflow/runtime state and only
//! mock external boundaries. The reusable external-boundary shims live in the
//! sibling modules; the full runtime composition is intentionally implemented
//! separately so it cannot silently fall back to internal fakes.
//!
//! The product-workflow support module provides filesystem-backed
//! conversation-binding and idempotency services for strict harness composition.
//! Full runtime and approval block/resume wiring still belongs with the binary
//! harness itself; tests must not fall back to product-workflow fakes when
//! claiming #3702 parity.

#![allow(dead_code)] // Test-only harness skeleton consumed by future binary-E2E tests.

use super::config::WaitConfig;

pub type HarnessWaitConfig = WaitConfig;
