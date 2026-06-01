//! Budget audit/event sink contracts.
//!
//! Emitting budget events is a downstream concern (SSE, audit log,
//! observability). To keep the governor crate boundary-clean, the
//! contract is a single trait that downstream crates implement. The
//! governor never *requires* a sink; callers wire one if they want UI
//! chips and audit trails.

use chrono::{DateTime, Utc};

use crate::{
    BudgetApprovalGate, BudgetGateOutcome, BudgetWarning, ResourceAccount, ResourceApprovalNeeded,
    ResourceDenial, ResourceReceipt, ResourceReservation,
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
    ApprovalRequested {
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
