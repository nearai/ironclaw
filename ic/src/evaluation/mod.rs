//! Success evaluation for completed jobs.
//!
//! Evaluates whether jobs were completed successfully based on:
//! - Output quality
//! - Requirements matching
//! - Error rates
//! - User feedback

mod metrics;
mod success;

pub use metrics::{MetricsCollector, QualityMetrics};
pub use success::{EvaluationResult, SuccessEvaluator};
