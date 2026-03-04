//! Agent benchmark system.
//!
//! Measures agent effectiveness with real LLM calls, tracks improvement
//! via baseline comparison, and supports cross-model evaluation.

pub mod baseline;
pub mod bench_channel;
pub mod instrumented;
pub mod metrics;
pub mod report;
#[cfg(feature = "libsql")]
pub mod runner;
pub mod scenario;
