//! Project [`BudgetEvent`](ironclaw_resources::BudgetEvent) records onto
//! the SSE `AppEvent` stream.
//!
//! Producer: [`spawn_budget_event_projection`] runs in a tokio task that
//! reads from a `tokio::sync::broadcast::Receiver<BudgetEvent>` and
//! emits [`AppEvent::Budget`] via the [`SseManager`]. This is the only
//! producer of `AppEvent::Budget` per `.claude/rules/gateway-events.md`
//! — `sse.broadcast_for_user` calls from anywhere else would split the
//! producer surface.
//!
//! Implements #3841 follow-up A2 (audit/SSE projection).

use std::sync::Arc;

use ironclaw_common::{AppBudgetEvent, AppEvent};
use ironclaw_host_api::UserId;
use ironclaw_resources::{BudgetEvent, ResourceAccount};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::channels::web::platform::sse::SseManager;

/// Spawn a tokio task that drains `receiver` and projects every
/// `BudgetEvent` onto `sse` for the resolved user.
///
/// The task exits when `cancel` is triggered or the broadcast sender
/// is dropped. Lagging subscribers (more events queued than the channel
/// capacity) are logged at `warn!` and resync silently — budget events
/// are best-effort observability, not durable state.
pub fn spawn_budget_event_projection(
    sse: Arc<SseManager>,
    mut receiver: broadcast::Receiver<BudgetEvent>,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::debug!("budget event projection cancelled — exiting");
                    return;
                }
                received = receiver.recv() => {
                    match received {
                        Ok(event) => project_budget_event(&sse, event),
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
    })
}

/// Map a single `BudgetEvent` to its `AppEvent` shape and broadcast it
/// to the per-user SSE stream. Visible in this module so tests can
/// drive a single event without standing up the projection task.
pub(crate) fn project_budget_event(sse: &SseManager, event: BudgetEvent) {
    let Some(user_id) = sse_user_id_for(&event) else {
        // System-scoped events (no associated user) are diagnostic
        // only — log and skip the SSE broadcast.
        tracing::debug!(?event, "skipping system-scoped BudgetEvent — no user id");
        return;
    };
    let Some(payload) = to_app_budget_event(&event) else {
        // Accounting bookkeeping (Reserved / Reconciled / Released /
        // ApprovalResolved) is captured by the in-memory audit sink
        // but too noisy for the client SSE stream.
        return;
    };
    // projection-exempt: bridge dispatcher, budget event sink
    sse.broadcast_for_user(user_id.as_str(), AppEvent::Budget(payload));
}

/// Pure mapping from `BudgetEvent` to its wire-projected
/// [`AppBudgetEvent`] (or `None` for accounting-only variants). Exposed
/// `pub(crate)` so tests can drive the mapping without standing up an
/// `SseManager`.
pub(crate) fn to_app_budget_event(event: &BudgetEvent) -> Option<AppBudgetEvent> {
    match event {
        BudgetEvent::Warned { warning, .. } => Some(AppBudgetEvent::Warn {
            account: warning.account.to_string(),
            dimension: warning.dimension.to_string(),
            utilization: warning.utilization,
            period_end_iso: warning.period_end.map(|t| t.to_rfc3339()),
        }),
        BudgetEvent::GateOpened {
            gate_id, needed, ..
        } => Some(AppBudgetEvent::Pause {
            gate_id: gate_id.to_string(),
            account: needed.account.to_string(),
            dimension: needed.dimension.to_string(),
            utilization: needed.utilization,
            period_end_iso: needed.period_end.map(|t| t.to_rfc3339()),
        }),
        BudgetEvent::Denied { denial, .. } => Some(AppBudgetEvent::Denied {
            account: denial.account.to_string(),
            dimension: denial.dimension.to_string(),
        }),
        BudgetEvent::LimitChanged { account, .. } => Some(AppBudgetEvent::LimitChanged {
            account: account.to_string(),
        }),
        // The governor's `ApprovalRequested` is an internal signal that
        // precedes the gate-open; the accountant's `GateOpened` is the
        // user-facing event with the real gate id. Skip the governor's
        // raw signal so consumers don't see a phantom Pause without a
        // resolvable gate.
        BudgetEvent::ApprovalRequested { .. } => None,
        // Accounting bookkeeping — useful for audit log but too noisy
        // to broadcast to clients. Skip.
        BudgetEvent::Reserved { .. }
        | BudgetEvent::Reconciled { .. }
        | BudgetEvent::Released { .. }
        | BudgetEvent::ApprovalResolved { .. } => None,
    }
}

/// SSE is per-user; events that don't carry a user identity (tenant-
/// level limit change, system-scoped) skip the broadcast.
fn sse_user_id_for(event: &BudgetEvent) -> Option<UserId> {
    let account = match event {
        BudgetEvent::Reserved { account, .. }
        | BudgetEvent::Reconciled { account, .. }
        | BudgetEvent::Released { account, .. }
        | BudgetEvent::LimitChanged { account, .. } => account,
        BudgetEvent::Warned { warning, .. } => &warning.account,
        BudgetEvent::Denied { denial, .. } => &denial.account,
        BudgetEvent::ApprovalRequested { needed, .. } | BudgetEvent::GateOpened { needed, .. } => {
            &needed.account
        }
        BudgetEvent::ApprovalResolved { gate, .. } => &gate.needed.account,
    };
    match account {
        ResourceAccount::User { user_id, .. }
        | ResourceAccount::Project { user_id, .. }
        | ResourceAccount::Agent { user_id, .. }
        | ResourceAccount::Mission { user_id, .. }
        | ResourceAccount::Thread { user_id, .. } => Some(user_id.clone()),
        ResourceAccount::Tenant { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_host_api::TenantId;
    use ironclaw_resources::{
        BudgetEvent, BudgetGateId, BudgetWarning, ResourceAccount, ResourceApprovalNeeded,
        ResourceDenial, ResourceDimension, ResourceValue,
    };
    use rust_decimal::Decimal;

    fn user_warning() -> BudgetWarning {
        BudgetWarning {
            account: ResourceAccount::user(TenantId::new("t").unwrap(), UserId::new("u").unwrap()),
            dimension: ResourceDimension::Usd,
            utilization: 0.85,
            limit: ResourceValue::Decimal(Decimal::from(10)),
            period_end: None,
        }
    }

    fn user_denial() -> ResourceDenial {
        ResourceDenial {
            account: ResourceAccount::user(TenantId::new("t").unwrap(), UserId::new("u").unwrap()),
            dimension: ResourceDimension::Usd,
            limit: ResourceValue::Decimal(Decimal::from(10)),
            current_usage: ResourceValue::Decimal(Decimal::from(8)),
            active_reserved: ResourceValue::Decimal(Decimal::from(1)),
            requested: ResourceValue::Decimal(Decimal::from(5)),
        }
    }

    fn user_approval_needed() -> ResourceApprovalNeeded {
        ResourceApprovalNeeded {
            account: ResourceAccount::user(TenantId::new("t").unwrap(), UserId::new("u").unwrap()),
            dimension: ResourceDimension::Usd,
            limit: ResourceValue::Decimal(Decimal::from(10)),
            current_usage: ResourceValue::Decimal(Decimal::from(0)),
            active_reserved: ResourceValue::Decimal(Decimal::from(0)),
            requested: ResourceValue::Decimal(Decimal::from(9)),
            utilization: 0.9,
            period_end: None,
        }
    }

    #[test]
    fn warned_maps_to_warn_payload_with_canonical_account_label() {
        let event = BudgetEvent::Warned {
            warning: user_warning(),
            at: Utc::now(),
        };
        let payload = to_app_budget_event(&event).expect("warn maps");
        let AppBudgetEvent::Warn {
            account,
            dimension,
            utilization,
            period_end_iso,
        } = payload
        else {
            panic!("expected Warn");
        };
        assert_eq!(account, "tenant/t/user/u");
        assert_eq!(dimension, "usd");
        assert!((utilization - 0.85).abs() < 1e-9);
        assert!(period_end_iso.is_none());
    }

    /// Regression for the invented-gate-id bug: the projected
    /// `Pause` event carries the real `BudgetGateId` from
    /// `BudgetEvent::GateOpened`, not a freshly-minted phantom.
    #[test]
    fn gate_opened_carries_real_gate_id_into_pause_payload() {
        let real_gate_id = BudgetGateId::new();
        let event = BudgetEvent::GateOpened {
            gate_id: real_gate_id,
            needed: user_approval_needed(),
            at: Utc::now(),
        };
        let payload = to_app_budget_event(&event).expect("gate-opened maps");
        let AppBudgetEvent::Pause {
            gate_id, account, ..
        } = payload
        else {
            panic!("expected Pause");
        };
        assert_eq!(
            gate_id,
            real_gate_id.to_string(),
            "Pause must carry the real gate id from the accountant, \
             not a freshly invented UUID"
        );
        assert_eq!(account, "tenant/t/user/u");
    }

    /// Regression: the governor's raw `ApprovalRequested` (no gate id)
    /// is suppressed from the wire so consumers never see a phantom
    /// Pause without a resolvable gate.
    #[test]
    fn approval_requested_alone_does_not_project() {
        let event = BudgetEvent::ApprovalRequested {
            needed: user_approval_needed(),
            at: Utc::now(),
        };
        assert!(
            to_app_budget_event(&event).is_none(),
            "the governor's gateless ApprovalRequested must not project; \
             the accountant's GateOpened is the user-facing event"
        );
    }

    #[test]
    fn denied_maps_to_denied_payload() {
        let event = BudgetEvent::Denied {
            denial: user_denial(),
            at: Utc::now(),
        };
        let payload = to_app_budget_event(&event).expect("denied maps");
        let AppBudgetEvent::Denied { account, dimension } = payload else {
            panic!("expected Denied");
        };
        assert_eq!(account, "tenant/t/user/u");
        assert_eq!(dimension, "usd");
    }

    #[test]
    fn sse_user_id_resolves_for_user_scoped_events() {
        let warned = BudgetEvent::Warned {
            warning: user_warning(),
            at: Utc::now(),
        };
        let id = sse_user_id_for(&warned).expect("user-scoped warned has user_id");
        assert_eq!(id.as_str(), "u");

        let denied = BudgetEvent::Denied {
            denial: user_denial(),
            at: Utc::now(),
        };
        assert_eq!(sse_user_id_for(&denied).unwrap().as_str(), "u");

        let gate_opened = BudgetEvent::GateOpened {
            gate_id: BudgetGateId::new(),
            needed: user_approval_needed(),
            at: Utc::now(),
        };
        assert_eq!(sse_user_id_for(&gate_opened).unwrap().as_str(), "u");
    }

    #[test]
    fn sse_user_id_is_none_for_tenant_scoped_events() {
        let tenant_warning = BudgetWarning {
            account: ResourceAccount::tenant(TenantId::new("t").unwrap()),
            dimension: ResourceDimension::Usd,
            utilization: 0.5,
            limit: ResourceValue::Decimal(Decimal::from(10)),
            period_end: None,
        };
        let warned = BudgetEvent::Warned {
            warning: tenant_warning,
            at: Utc::now(),
        };
        assert!(sse_user_id_for(&warned).is_none());
    }
}
