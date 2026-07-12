use crate::{RunSummary, process_metrics::aggregate_process_metrics, sweep};

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
                cpu_ms: summary.process.delta_cpu_ms,
                peak_rss_kb: summary.process.peak_rss_kb,
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
                let process =
                    aggregate_process_metrics(summaries.iter().map(|summary| &summary.process));
                sweep::RunMetrics {
                    attempted,
                    failed,
                    throughput_ops_sec,
                    cpu_ms: process.delta_cpu_ms,
                    peak_rss_kb: process.peak_rss_kb,
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
