use crate::{RunSummary, sweep};

#[derive(Debug)]
pub(crate) enum CapturedRun {
    Single(Box<RunSummary>),
    Parent {
        aggregate: serde_json::Value,
        summaries: Vec<RunSummary>,
    },
}

impl CapturedRun {
    pub(crate) fn summary_value(&self) -> serde_json::Value {
        match self {
            Self::Single(summary) => serde_json::to_value(summary).unwrap_or_else(|error| {
                serde_json::json!({
                    "serialization_error": error.to_string(),
                })
            }),
            Self::Parent { aggregate, .. } => aggregate.clone(),
        }
    }

    pub(crate) fn metrics(&self) -> sweep::RunMetrics {
        match self {
            Self::Single(summary) => sweep::RunMetrics {
                attempted: summary.attempted,
                failed: summary.failed,
                throughput_ops_sec: summary.throughput_ops_sec,
                p95_us: summary.latency.p95_us,
                p99_us: summary.latency.p99_us,
                max_us: summary.latency.max_us,
            },
            Self::Parent { summaries, .. } => {
                let attempted = summaries.iter().map(|summary| summary.attempted).sum();
                let failed = summaries.iter().map(|summary| summary.failed).sum();
                let max_duration_ms = summaries
                    .iter()
                    .map(|summary| summary.duration_ms)
                    .max()
                    .unwrap_or(0);
                let throughput_ops_sec = if max_duration_ms == 0 {
                    0.0
                } else {
                    attempted as f64 / (max_duration_ms as f64 / 1000.0)
                };
                sweep::RunMetrics {
                    attempted,
                    failed,
                    throughput_ops_sec,
                    p95_us: summaries
                        .iter()
                        .map(|summary| summary.latency.p95_us)
                        .max()
                        .unwrap_or(0),
                    p99_us: summaries
                        .iter()
                        .map(|summary| summary.latency.p99_us)
                        .max()
                        .unwrap_or(0),
                    max_us: summaries
                        .iter()
                        .map(|summary| summary.latency.max_us)
                        .max()
                        .unwrap_or(0),
                }
            }
        }
    }
}
