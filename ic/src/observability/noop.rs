//! Zero-overhead no-op observer.
//!
//! Default backend when observability is disabled. All methods compile to
//! nothing, so there is zero runtime cost.

use crate::observability::traits::{Observer, ObserverEvent, ObserverMetric};

/// Observer that discards all events and metrics.
pub struct NoopObserver;

impl Observer for NoopObserver {
    #[inline(always)]
    fn record_event(&self, _event: &ObserverEvent) {}

    #[inline(always)]
    fn record_metric(&self, _metric: &ObserverMetric) {}

    fn name(&self) -> &str {
        "noop"
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::observability::traits::*;

    use crate::observability::noop::NoopObserver;

    #[test]
    fn name_is_noop() {
        assert_eq!(NoopObserver.name(), "noop");
    }

    #[test]
    fn record_event_does_not_panic() {
        let obs = NoopObserver;
        obs.record_event(&ObserverEvent::TurnComplete);
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "x".into(),
            model: "y".into(),
        });
    }

    #[test]
    fn record_metric_does_not_panic() {
        let obs = NoopObserver;
        obs.record_metric(&ObserverMetric::TokensUsed(100));
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_millis(50)));
        obs.record_metric(&ObserverMetric::ActiveJobs(2));
        obs.record_metric(&ObserverMetric::QueueDepth(0));
    }

    #[test]
    fn flush_does_not_panic() {
        NoopObserver.flush();
    }
}
