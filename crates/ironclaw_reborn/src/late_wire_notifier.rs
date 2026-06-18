//! `LateWireWakeNotifier` — a deferred wake-notifier adapter.
//!
//! This adapter breaks the coordinator → wake_notifier ← scheduler_notifier ←
//! scheduler → executor → host_factory → capability_factory → coordinator
//! construction cycle. The coordinator is given this notifier during the build
//! phase; the real scheduler notifier is wired in via `set()` after the
//! scheduler starts. Any `notify_queued_run` calls before `set()` return
//! `Ok(())` harmlessly — the scheduler's poll tick picks up queued runs within
//! one poll interval.

use std::sync::Arc;

use ironclaw_turns::{TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError};

/// A deferred wake notifier that forwards to the real notifier once it is set.
///
/// See module-level documentation for the usage contract.
pub(crate) struct LateWireWakeNotifier {
    inner: std::sync::OnceLock<Arc<dyn TurnRunWakeNotifier>>,
}

impl LateWireWakeNotifier {
    pub(crate) fn new() -> Self {
        Self {
            inner: std::sync::OnceLock::new(),
        }
    }

    /// Wire the real notifier in.
    ///
    /// # Unset-window contract
    ///
    /// Any `notify_queued_run` calls between `LateWireWakeNotifier::new()` and
    /// the first call to `set()` return `Ok(())` without forwarding the wake.
    /// This is intentional: the scheduler has not started yet, so there is
    /// nobody to wake. The scheduler's initial poll tick will drain any runs
    /// that were queued in this window.
    ///
    /// Calling `set()` more than once is safe and idempotent — subsequent calls
    /// are silently ignored.
    pub(crate) fn set(&self, notifier: Arc<dyn TurnRunWakeNotifier>) {
        // Ignore if already set (should not happen in practice).
        let _ = self.inner.set(notifier);
    }
}

impl TurnRunWakeNotifier for LateWireWakeNotifier {
    fn notify_queued_run(&self, wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        match self.inner.get() {
            Some(notifier) => notifier.notify_queued_run(wake),
            // Scheduler not started yet; run will be picked up on first poll tick.
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use ironclaw_turns::{TurnRunWake, TurnRunWakeNotifier, TurnRunWakeNotifyError};

    use super::LateWireWakeNotifier;

    fn any_wake() -> TurnRunWake {
        use ironclaw_host_api::{TenantId, ThreadId};
        use ironclaw_turns::{EventCursor, TurnRunId, TurnScope, TurnStatus};
        let scope = TurnScope::new(
            TenantId::new("test-tenant").expect("valid"),
            None,
            None,
            ThreadId::new("test-thread").expect("valid"),
        );
        TurnRunWake {
            scope,
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            event_cursor: EventCursor(0),
        }
    }

    /// A notifier that records every wake it receives.
    #[derive(Default)]
    struct RecordingNotifier {
        wakes: Mutex<Vec<TurnRunWake>>,
    }

    impl RecordingNotifier {
        fn wake_count(&self) -> usize {
            self.wakes.lock().unwrap().len()
        }
    }

    impl TurnRunWakeNotifier for RecordingNotifier {
        fn notify_queued_run(&self, wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
            self.wakes.lock().unwrap().push(wake);
            Ok(())
        }
    }

    /// Before `set()` is called, `notify_queued_run` must return `Ok(())` and
    /// must not panic — the scheduler has not started yet.
    #[test]
    fn notify_before_set_returns_ok() {
        let notifier = LateWireWakeNotifier::new();
        let result = notifier.notify_queued_run(any_wake());
        assert!(
            result.is_ok(),
            "notify_queued_run before set() must return Ok(())"
        );
    }

    /// After `set()` is called, `notify_queued_run` must forward to the inner
    /// notifier.
    #[test]
    fn notify_after_set_forwards_to_inner() {
        let late = LateWireWakeNotifier::new();
        let inner = Arc::new(RecordingNotifier::default());
        late.set(inner.clone() as Arc<dyn TurnRunWakeNotifier>);

        late.notify_queued_run(any_wake())
            .expect("forward to inner should succeed");

        assert_eq!(
            inner.wake_count(),
            1,
            "inner notifier should have received the wake"
        );
    }

    /// Calling `set()` a second time must be silently ignored — the first wired
    /// notifier remains active and the second is dropped.
    #[test]
    fn double_set_is_harmless() {
        let late = LateWireWakeNotifier::new();

        let first = Arc::new(RecordingNotifier::default());
        let second = Arc::new(RecordingNotifier::default());

        late.set(first.clone() as Arc<dyn TurnRunWakeNotifier>);
        // Second set must be a no-op.
        late.set(second.clone() as Arc<dyn TurnRunWakeNotifier>);

        late.notify_queued_run(any_wake())
            .expect("forward succeeds");

        assert_eq!(
            first.wake_count(),
            1,
            "first wired notifier should receive the wake"
        );
        assert_eq!(second.wake_count(), 0, "second set() must be ignored");
    }

    /// A notifier that always returns `Err(TurnRunWakeNotifyError::DeliveryUnavailable)`.
    struct ErrorNotifier;

    impl TurnRunWakeNotifier for ErrorNotifier {
        fn notify_queued_run(&self, _wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
            Err(TurnRunWakeNotifyError::DeliveryUnavailable)
        }
    }

    /// After `set()`, if the inner notifier returns `Err`, the error must be
    /// forwarded by `notify_queued_run` rather than swallowed — regression guard
    /// for any future change that might silently suppress inner errors.
    #[test]
    fn notify_after_set_forwards_inner_err() {
        let late = LateWireWakeNotifier::new();
        late.set(Arc::new(ErrorNotifier) as Arc<dyn TurnRunWakeNotifier>);

        let result = late.notify_queued_run(any_wake());
        assert!(
            result.is_err(),
            "inner Err must be forwarded, not suppressed"
        );
        assert_eq!(
            result.unwrap_err(),
            TurnRunWakeNotifyError::DeliveryUnavailable,
            "forwarded error must match the inner error variant"
        );
    }
}
