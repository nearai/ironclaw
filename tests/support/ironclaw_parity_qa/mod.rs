//! IronClaw binary-E2E harness family + trace-replay model gateway.
//!
//! Extracted from the `tests/integration/support/` tree (which now hosts only
//! the roadmap `IronClawIntegrationHarness` family) — this family is the older
//! flat-bin/`ironclaw_trace_*`/QA-scenario harness, consumed by `tests/ironclaw_*.rs`
//! parity and QA bins.

pub mod binary_e2e;
pub mod delivery;
pub mod model_replay;
pub mod network;
#[allow(dead_code)]
pub mod qa_scenarios;
pub mod qa_trace;
