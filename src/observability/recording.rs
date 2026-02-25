//! Test-only observer that captures all events into a shared vector.
//!
//! Used by Tier 1 tests to verify event emission at agent boundaries
//! without any OpenTelemetry dependency.

use std::sync::{Arc, Mutex};

use crate::observability::traits::{Observer, ObserverEvent, ObserverMetric};

/// Observer that records all events for test assertions.
pub struct RecordingObserver {
    events: Arc<Mutex<Vec<ObserverEvent>>>,
    metrics: Arc<Mutex<Vec<ObserverMetric>>>,
    flush_count: Arc<std::sync::atomic::AtomicU32>,
}

impl RecordingObserver {
    /// Create a new recording observer and return handles to the captured data.
    #[allow(clippy::type_complexity)]
    pub fn new() -> (
        Self,
        Arc<Mutex<Vec<ObserverEvent>>>,
        Arc<Mutex<Vec<ObserverMetric>>>,
    ) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let metrics = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                events: Arc::clone(&events),
                metrics: Arc::clone(&metrics),
                flush_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            },
            events,
            metrics,
        )
    }

    /// Create a new recording observer with a shared flush counter.
    #[allow(clippy::type_complexity)]
    pub fn with_flush_counter(
    ) -> (
        Self,
        Arc<Mutex<Vec<ObserverEvent>>>,
        Arc<Mutex<Vec<ObserverMetric>>>,
        Arc<std::sync::atomic::AtomicU32>,
    ) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let metrics = Arc::new(Mutex::new(Vec::new()));
        let flush_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        (
            Self {
                events: Arc::clone(&events),
                metrics: Arc::clone(&metrics),
                flush_count: Arc::clone(&flush_count),
            },
            events,
            metrics,
            flush_count,
        )
    }
}

impl Observer for RecordingObserver {
    fn record_event(&self, event: &ObserverEvent) {
        self.events.lock().unwrap().push(event.clone());
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        self.metrics.lock().unwrap().push(metric.clone());
    }

    fn flush(&self) {
        self.flush_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn name(&self) -> &str {
        "recording"
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn captures_events() {
        let (obs, events, _) = RecordingObserver::new();

        obs.record_event(&ObserverEvent::AgentStart {
            provider: "test".into(),
            model: "m".into(),
        });
        obs.record_event(&ObserverEvent::TurnComplete {
            thread_id: None,
            iteration: 1,
            tool_calls_in_turn: 0,
        });

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert!(matches!(captured[0], ObserverEvent::AgentStart { .. }));
        assert!(matches!(captured[1], ObserverEvent::TurnComplete { .. }));
    }

    #[test]
    fn captures_metrics() {
        let (obs, _, metrics) = RecordingObserver::new();

        obs.record_metric(&ObserverMetric::TokensUsed(500));
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_millis(100)));

        let captured = metrics.lock().unwrap();
        assert_eq!(captured.len(), 2);
    }

    #[test]
    fn name_is_recording() {
        let (obs, _, _) = RecordingObserver::new();
        assert_eq!(obs.name(), "recording");
    }

    #[test]
    fn tracks_flush_calls() {
        let (obs, _, _, flush_count) = RecordingObserver::with_flush_counter();
        assert_eq!(
            flush_count.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        obs.flush();
        assert_eq!(
            flush_count.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        obs.flush();
        assert_eq!(
            flush_count.load(std::sync::atomic::Ordering::Relaxed),
            2
        );
    }
}
