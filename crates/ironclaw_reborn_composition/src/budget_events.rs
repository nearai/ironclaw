//! Project [`BudgetEvent`](ironclaw_resources::BudgetEvent) records.
//!
//! The runtime owns a `tokio::sync::broadcast::Sender<BudgetEvent>` via
//! [`BroadcastBudgetEventSink`]. Production composition spawns a
//! [`BudgetEventProjection`] task at runtime construction; the task
//! receives every emitted event and hands it to a
//! [`BudgetEventObserver`] for delivery.
//!
//! The default observer logs at `debug!`. Production owners that need to
//! fan budget events out to SSE / WS / external telemetry pass their own
//! observer through
//! [`RebornRuntimeInput::with_budget_event_observer`](crate::RebornRuntimeInput).

use std::sync::Arc;

use ironclaw_resources::{BroadcastBudgetEventSink, BudgetEvent};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Observer that receives every projected `BudgetEvent` in arrival order.
///
/// Implementors must not panic. The projection task does not isolate
/// the observer in a separate task — a panic propagates to the
/// projection task and shuts it down. Production observers should
/// catch their own errors and convert them to log lines or telemetry.
pub trait BudgetEventObserver: Send + Sync + std::fmt::Debug + 'static {
    fn observe(&self, event: BudgetEvent);
}

/// Default observer that logs every event at `debug!`. Used as the
/// fallback when no production owner installs a richer projection
/// (e.g. tracing-only deploys, local-dev binaries that just want the
/// observability without an SSE bridge).
#[derive(Debug, Default, Clone, Copy)]
pub struct TracingBudgetEventObserver;

impl BudgetEventObserver for TracingBudgetEventObserver {
    fn observe(&self, event: BudgetEvent) {
        tracing::debug!(?event, "reborn budget event observed");
    }
}

/// Background task that drains a `BudgetEvent` broadcast receiver and
/// hands every event to the configured observer.
///
/// Cancelled by the parent runtime's shutdown via the internal
/// [`CancellationToken`]. The handle is held by `RebornRuntime` and
/// awaited inside `RebornRuntime::shutdown` so the task always exits
/// before the runtime drops.
#[derive(Debug)]
pub(crate) struct BudgetEventProjection {
    handle: JoinHandle<()>,
    cancel: CancellationToken,
}

impl BudgetEventProjection {
    /// Subscribe to `sink` and spawn the projection task.
    pub(crate) fn spawn(
        sink: &BroadcastBudgetEventSink,
        observer: Arc<dyn BudgetEventObserver>,
    ) -> Self {
        let cancel = CancellationToken::new();
        let receiver = sink.subscribe();
        let cancel_for_task = cancel.clone();
        let handle = tokio::spawn(run_projection(receiver, observer, cancel_for_task));
        Self { handle, cancel }
    }

    /// Trigger cancellation and await the projection task. Idempotent
    /// — calling shutdown after the task has already exited is
    /// harmless (the `CancellationToken` only fires once and the
    /// `JoinHandle::await` immediately returns the cached result).
    pub(crate) async fn shutdown(self) {
        self.cancel.cancel();
        if let Err(error) = self.handle.await {
            if error.is_panic() {
                tracing::error!(%error, "budget event projection task panicked during shutdown");
            } else {
                tracing::debug!(%error, "budget event projection task cancelled during shutdown");
            }
        }
    }
}

async fn run_projection(
    mut receiver: broadcast::Receiver<BudgetEvent>,
    observer: Arc<dyn BudgetEventObserver>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::debug!("budget event projection cancelled — exiting");
                return;
            }
            received = receiver.recv() => {
                match received {
                    Ok(event) => observer.observe(event),
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            skipped,
                            "budget event projection fell behind the broadcast buffer; \
                             dropping {skipped} events and resuming"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::debug!("budget event broadcast closed — projection exiting");
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_resources::{
        BudgetEventSink, BudgetWarning, ResourceAccount, ResourceDimension, ResourceValue,
    };
    use rust_decimal::Decimal;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Debug, Default)]
    struct CapturingObserver {
        events: Mutex<Vec<BudgetEvent>>,
    }

    impl BudgetEventObserver for CapturingObserver {
        fn observe(&self, event: BudgetEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn user_warning() -> BudgetWarning {
        BudgetWarning {
            account: ResourceAccount::user(TenantId::new("t").unwrap(), UserId::new("u").unwrap()),
            dimension: ResourceDimension::Usd,
            utilization: 0.85,
            limit: ResourceValue::Decimal(Decimal::from(10)),
            period_end: None,
        }
    }

    /// Drives the projection task end-to-end: emit a `Warned` event
    /// through the broadcast sink, wait briefly for the task to drain
    /// it, and assert the observer received it. Exercises the
    /// production wiring shape — broadcast sink construction →
    /// `spawn` → task picks up event → observer invocation.
    #[tokio::test]
    async fn projection_forwards_every_event_to_observer() {
        let sink = BroadcastBudgetEventSink::default();
        let observer = Arc::new(CapturingObserver::default());
        let observer_for_projection: Arc<dyn BudgetEventObserver> = Arc::clone(&observer) as Arc<_>;
        let projection = BudgetEventProjection::spawn(&sink, observer_for_projection);

        sink.emit(BudgetEvent::Warned {
            warning: user_warning(),
            at: Utc::now(),
        });

        // Allow the spawned task to drain the broadcast queue. The
        // test stays small (~20 ms) so a slow runner does not turn
        // it flaky; on a healthy machine the receive is sub-ms.
        let observed = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if observer.events.lock().unwrap().len() == 1 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        assert!(observed.is_ok(), "projection did not deliver event in time");

        projection.shutdown().await;

        let events = observer.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], BudgetEvent::Warned { .. }));
    }

    /// Regression: shutdown is non-blocking even when no events have
    /// been emitted. Catches a subtle bug shape where the projection
    /// task waits on `recv()` and only exits when a message arrives.
    #[tokio::test]
    async fn shutdown_cancels_the_task_even_with_no_events() {
        let sink = BroadcastBudgetEventSink::default();
        let observer: Arc<dyn BudgetEventObserver> = Arc::new(TracingBudgetEventObserver);
        let projection = BudgetEventProjection::spawn(&sink, observer);

        let shutdown = tokio::time::timeout(Duration::from_secs(2), projection.shutdown()).await;
        assert!(
            shutdown.is_ok(),
            "shutdown blocked waiting for an event; cancellation token did not fire"
        );
    }

    /// Regression: after `shutdown()` returns, subsequent emits do
    /// not crash and are not observed. Locks the contract that
    /// shutdown cleanly drops the broadcast Receiver.
    #[tokio::test]
    async fn events_emitted_after_shutdown_are_not_observed() {
        let sink = BroadcastBudgetEventSink::default();
        let observer = Arc::new(CapturingObserver::default());
        let observer_for_projection: Arc<dyn BudgetEventObserver> = Arc::clone(&observer) as Arc<_>;
        let projection = BudgetEventProjection::spawn(&sink, observer_for_projection);
        projection.shutdown().await;

        sink.emit(BudgetEvent::Warned {
            warning: user_warning(),
            at: Utc::now(),
        });
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(observer.events.lock().unwrap().is_empty());
    }
}
