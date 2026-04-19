//! Fan-out observer that dispatches to multiple backends.
//!
//! Useful for combining backends, e.g. log + OpenTelemetry simultaneously.

use crate::observability::traits::{Observer, ObserverEvent, ObserverMetric};

/// Dispatches events and metrics to all inner observers.
pub struct MultiObserver {
    observers: Vec<Box<dyn Observer>>,
}

impl MultiObserver {
    /// Create from a list of observers. If the list is empty the result
    /// behaves like a noop.
    pub fn new(observers: Vec<Box<dyn Observer>>) -> Self {
        Self { observers }
    }
}

impl Observer for MultiObserver {
    fn record_event(&self, event: &ObserverEvent) {
        for obs in &self.observers {
            obs.record_event(event);
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        for obs in &self.observers {
            obs.record_metric(metric);
        }
    }

    fn flush(&self) {
        for obs in &self.observers {
            obs.flush();
        }
    }

    fn name(&self) -> &str {
        "multi"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::observability::multi::MultiObserver;
    use crate::observability::traits::*;

    /// Test observer that counts calls via shared atomic counters.
    struct CountingObserver {
        events: Arc<AtomicUsize>,
        metrics: Arc<AtomicUsize>,
        flushes: Arc<AtomicUsize>,
    }

    impl CountingObserver {
        fn new() -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
            let events = Arc::new(AtomicUsize::new(0));
            let metrics = Arc::new(AtomicUsize::new(0));
            let flushes = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    events: Arc::clone(&events),
                    metrics: Arc::clone(&metrics),
                    flushes: Arc::clone(&flushes),
                },
                events,
                metrics,
                flushes,
            )
        }
    }

    impl Observer for CountingObserver {
        fn record_event(&self, _event: &ObserverEvent) {
            self.events.fetch_add(1, Ordering::Relaxed);
        }
        fn record_metric(&self, _metric: &ObserverMetric) {
            self.metrics.fetch_add(1, Ordering::Relaxed);
        }
        fn flush(&self) {
            self.flushes.fetch_add(1, Ordering::Relaxed);
        }
        fn name(&self) -> &str {
            "counting"
        }
    }

    #[test]
    fn name_is_multi() {
        let multi = MultiObserver::new(vec![]);
        assert_eq!(multi.name(), "multi");
    }

    #[test]
    fn empty_multi_does_not_panic() {
        let multi = MultiObserver::new(vec![]);
        multi.record_event(&ObserverEvent::TurnComplete);
        multi.record_metric(&ObserverMetric::TokensUsed(100));
        multi.flush();
    }

    #[test]
    fn dispatches_to_all_observers() {
        let (a, a_events, a_metrics, a_flushes) = CountingObserver::new();
        let (b, b_events, b_metrics, b_flushes) = CountingObserver::new();

        let multi = MultiObserver::new(vec![Box::new(a), Box::new(b)]);

        multi.record_event(&ObserverEvent::TurnComplete);
        multi.record_event(&ObserverEvent::HeartbeatTick);
        multi.record_metric(&ObserverMetric::TokensUsed(50));
        multi.flush();

        assert_eq!(a_events.load(Ordering::Relaxed), 2);
        assert_eq!(a_metrics.load(Ordering::Relaxed), 1);
        assert_eq!(a_flushes.load(Ordering::Relaxed), 1);
        assert_eq!(b_events.load(Ordering::Relaxed), 2);
        assert_eq!(b_metrics.load(Ordering::Relaxed), 1);
        assert_eq!(b_flushes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn single_observer_works() {
        let (obs, events, _, _) = CountingObserver::new();

        let multi = MultiObserver::new(vec![Box::new(obs)]);
        multi.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_secs(1),
            tokens_used: None,
        });

        assert_eq!(events.load(Ordering::Relaxed), 1);
    }
}
