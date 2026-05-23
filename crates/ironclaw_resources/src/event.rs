//! Budget audit/event sink contracts.
//!
//! Emitting budget events is a downstream concern (SSE, audit log,
//! observability). To keep the governor crate boundary-clean, the
//! contract is a single trait that downstream crates implement. The
//! governor never *requires* a sink; callers wire one if they want UI
//! chips and audit trails.

use chrono::{DateTime, Utc};

use crate::{
    BudgetApprovalGate, BudgetGateId, BudgetGateOutcome, BudgetWarning, ResourceAccount,
    ResourceApprovalNeeded, ResourceDenial, ResourceReceipt, ResourceReservation,
};

/// One observable budget event.
///
/// All variants carry the account they apply to so downstream filters
/// can route per-user/per-project without re-deriving the cascade.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetEvent {
    Reserved {
        account: ResourceAccount,
        reservation: ResourceReservation,
        warnings: Vec<BudgetWarning>,
        at: DateTime<Utc>,
    },
    Reconciled {
        account: ResourceAccount,
        receipt: ResourceReceipt,
        at: DateTime<Utc>,
    },
    Released {
        account: ResourceAccount,
        receipt: ResourceReceipt,
        at: DateTime<Utc>,
    },
    Warned {
        warning: BudgetWarning,
        at: DateTime<Utc>,
    },
    Denied {
        denial: ResourceDenial,
        at: DateTime<Utc>,
    },
    /// The governor's cascade said approval is needed but no gate has been
    /// opened yet. Internal signal from the governor — the accountant
    /// (or any other handler that holds a [`BudgetGateStore`]) reacts to
    /// this by opening a gate and emitting [`BudgetEvent::GateOpened`]
    /// with the real `BudgetGateId`.
    ///
    /// SSE projection consumers should ignore this variant and listen
    /// for `GateOpened` instead (review feedback: invented-gate-id bug).
    ApprovalRequested {
        needed: ResourceApprovalNeeded,
        at: DateTime<Utc>,
    },
    /// A pending approval gate has been opened in the
    /// [`BudgetGateStore`]. Carries the real `BudgetGateId` so the
    /// SSE projection / audit log can route the user to a resolvable
    /// gate. Produced by `GovernorBackedAccountant` after a successful
    /// `BudgetGateStore::open`.
    GateOpened {
        gate_id: BudgetGateId,
        needed: ResourceApprovalNeeded,
        at: DateTime<Utc>,
    },
    ApprovalResolved {
        gate: BudgetApprovalGate,
        outcome: BudgetGateOutcome,
        at: DateTime<Utc>,
    },
    LimitChanged {
        account: ResourceAccount,
        at: DateTime<Utc>,
    },
}

/// Sink for budget events. Implementations must be cheap and non-blocking
/// — the governor calls them on its mutation path and cannot afford
/// synchronous I/O. Production sinks typically forward to a tokio
/// `mpsc` channel that drains into the audit/SSE bus.
pub trait BudgetEventSink: Send + Sync + std::fmt::Debug {
    fn emit(&self, event: BudgetEvent);
}

/// Default no-op sink. Allows callers to keep the same construction
/// shape whether or not budget observability is wired.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpBudgetEventSink;

impl BudgetEventSink for NoOpBudgetEventSink {
    fn emit(&self, _event: BudgetEvent) {}
}

/// In-memory sink used in tests and the local-dev gateway. Captures
/// every event so assertions can inspect ordering and counts.
#[derive(Debug, Default)]
pub struct InMemoryBudgetEventSink {
    events: std::sync::Mutex<Vec<BudgetEvent>>,
}

impl InMemoryBudgetEventSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn drain(&self) -> Vec<BudgetEvent> {
        let mut guard = self
            .events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *guard)
    }

    pub fn snapshot(&self) -> Vec<BudgetEvent> {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl BudgetEventSink for InMemoryBudgetEventSink {
    fn emit(&self, event: BudgetEvent) {
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

/// Sink that publishes every event onto a `tokio::sync::broadcast`
/// channel. Production composition wires this so downstream consumers
/// (SSE projection, audit log, observability) subscribe via
/// [`BroadcastBudgetEventSink::subscribe`] and project events without
/// the governor knowing about them (review feedback #3841 A2).
///
/// `emit` is best-effort and non-blocking: when no receiver is active
/// (or the broadcast buffer is full) the send returns an error which
/// we discard. The governor's mutation path stays synchronous.
#[derive(Debug, Clone)]
pub struct BroadcastBudgetEventSink {
    sender: tokio::sync::broadcast::Sender<BudgetEvent>,
}

impl BroadcastBudgetEventSink {
    /// Construct a sink with the given broadcast capacity. A value of
    /// 256 covers a comfortable burst of events without blocking; very
    /// slow subscribers may miss events under sustained load, which
    /// matches the audit/SSE "best-effort observability" shape.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(capacity);
        Self { sender }
    }

    /// Open a new subscriber. Each subscriber gets every event emitted
    /// after the moment of subscription.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<BudgetEvent> {
        self.sender.subscribe()
    }

    /// Number of active subscribers — handy for tests asserting that
    /// production composition wired through to the projection task.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for BroadcastBudgetEventSink {
    fn default() -> Self {
        Self::new(256)
    }
}

impl BudgetEventSink for BroadcastBudgetEventSink {
    fn emit(&self, event: BudgetEvent) {
        // `send` returns Err only when there are no active receivers —
        // a perfectly valid state (nobody is projecting yet). Drop
        // silently.
        let _ = self.sender.send(event);
    }
}

/// Composite sink that fans out to every wrapped sink in order. Lets
/// composition keep the `InMemoryBudgetEventSink` for tests while also
/// publishing to a `BroadcastBudgetEventSink` for SSE projection.
#[derive(Debug)]
pub struct CompositeBudgetEventSink {
    sinks: Vec<std::sync::Arc<dyn BudgetEventSink>>,
}

impl CompositeBudgetEventSink {
    pub fn new(sinks: Vec<std::sync::Arc<dyn BudgetEventSink>>) -> Self {
        Self { sinks }
    }
}

impl BudgetEventSink for CompositeBudgetEventSink {
    fn emit(&self, event: BudgetEvent) {
        for sink in &self.sinks {
            sink.emit(event.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResourceDimension, ResourceValue};
    use ironclaw_host_api::TenantId;
    use rust_decimal::Decimal;

    fn sample_warning() -> BudgetWarning {
        BudgetWarning {
            account: ResourceAccount::tenant(TenantId::new("t").unwrap()),
            dimension: ResourceDimension::Usd,
            utilization: 0.80,
            limit: ResourceValue::Decimal(Decimal::from(10)),
            period_end: None,
        }
    }

    #[test]
    fn in_memory_sink_captures_events_in_order() {
        let sink = InMemoryBudgetEventSink::new();
        sink.emit(BudgetEvent::Warned {
            warning: sample_warning(),
            at: Utc::now(),
        });
        assert_eq!(sink.snapshot().len(), 1);
        assert_eq!(sink.drain().len(), 1);
        assert!(sink.snapshot().is_empty());
    }

    #[test]
    fn noop_sink_drops_events() {
        let sink = NoOpBudgetEventSink;
        sink.emit(BudgetEvent::Warned {
            warning: sample_warning(),
            at: Utc::now(),
        });
    }
}
