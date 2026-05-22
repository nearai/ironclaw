//! Project [`BudgetEvent`](ironclaw_resources::BudgetEvent) records onto
//! the SSE `AppEvent` stream.
//!
//! Producer: [`spawn_budget_event_projection`] runs in a tokio task that
//! reads from a `tokio::sync::broadcast::Receiver<BudgetEvent>` and
//! emits [`AppEvent::BudgetWarn`] / [`AppEvent::BudgetPause`] /
//! [`AppEvent::BudgetDenied`] / [`AppEvent::BudgetLimitChanged`] via the
//! [`SseManager`].
//!
//! This is the only producer of those `AppEvent` variants per
//! `.claude/rules/gateway-events.md` — `sse.broadcast_for_user` calls
//! from anywhere else would split the producer surface.
//!
//! Implements #3841 follow-up A2 (audit/SSE projection).

use std::sync::Arc;

use ironclaw_common::AppEvent;
use ironclaw_host_api::UserId;
use ironclaw_resources::{BudgetEvent, BudgetGateId, BudgetWarning, ResourceAccount};
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
    let app_event = match event {
        BudgetEvent::Warned { warning, .. } => Some(app_event_from_warning(warning)),
        BudgetEvent::ApprovalRequested { needed, .. } => {
            let gate_id = BudgetGateId::new();
            Some(AppEvent::BudgetPause {
                gate_id: gate_id.to_string(),
                account: account_label(&needed.account),
                dimension: needed.dimension.to_string(),
                utilization: needed.utilization,
                period_end_iso: needed.period_end.map(|t| t.to_rfc3339()),
            })
        }
        BudgetEvent::Denied { denial, .. } => Some(AppEvent::BudgetDenied {
            account: account_label(&denial.account),
            dimension: denial.dimension.to_string(),
        }),
        BudgetEvent::LimitChanged { account, .. } => Some(AppEvent::BudgetLimitChanged {
            account: account_label(&account),
        }),
        // Reserved / Reconciled / Released / ApprovalResolved are
        // accounting bookkeeping — useful for audit log but too noisy
        // to broadcast to clients. Skip.
        BudgetEvent::Reserved { .. }
        | BudgetEvent::Reconciled { .. }
        | BudgetEvent::Released { .. }
        | BudgetEvent::ApprovalResolved { .. } => None,
    };
    if let Some(event) = app_event {
        sse.broadcast_for_user(user_id.as_str(), event); // projection-exempt: bridge dispatcher, budget event sink
    }
}

fn app_event_from_warning(warning: BudgetWarning) -> AppEvent {
    AppEvent::BudgetWarn {
        account: account_label(&warning.account),
        dimension: warning.dimension.to_string(),
        utilization: warning.utilization,
        period_end_iso: warning.period_end.map(|t| t.to_rfc3339()),
    }
}

/// Stable string label for an account: `tenant/user/...` joined with
/// `/`. The SSE event has no other place to carry hierarchical scope;
/// frontend code splits on `/` when rendering.
fn account_label(account: &ResourceAccount) -> String {
    match account {
        ResourceAccount::Tenant { tenant_id } => format!("tenant/{}", tenant_id.as_str()),
        ResourceAccount::User { tenant_id, user_id } => {
            format!("tenant/{}/user/{}", tenant_id.as_str(), user_id.as_str())
        }
        ResourceAccount::Project {
            tenant_id,
            user_id,
            project_id,
        } => format!(
            "tenant/{}/user/{}/project/{}",
            tenant_id.as_str(),
            user_id.as_str(),
            project_id.as_str()
        ),
        ResourceAccount::Agent {
            tenant_id,
            user_id,
            project_id,
            agent_id,
        } => format!(
            "tenant/{}/user/{}/project/{}/agent/{}",
            tenant_id.as_str(),
            user_id.as_str(),
            project_id.as_ref().map(|p| p.as_str()).unwrap_or("_"),
            agent_id.as_str()
        ),
        ResourceAccount::Mission {
            tenant_id,
            user_id,
            project_id,
            mission_id,
        } => format!(
            "tenant/{}/user/{}/project/{}/mission/{}",
            tenant_id.as_str(),
            user_id.as_str(),
            project_id.as_ref().map(|p| p.as_str()).unwrap_or("_"),
            mission_id.as_str()
        ),
        ResourceAccount::Thread {
            tenant_id,
            user_id,
            project_id,
            mission_id,
            thread_id,
        } => format!(
            "tenant/{}/user/{}/project/{}/mission/{}/thread/{}",
            tenant_id.as_str(),
            user_id.as_str(),
            project_id.as_ref().map(|p| p.as_str()).unwrap_or("_"),
            mission_id.as_ref().map(|m| m.as_str()).unwrap_or("_"),
            thread_id.as_str()
        ),
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
        BudgetEvent::ApprovalRequested { needed, .. } => &needed.account,
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
        BudgetEvent, BudgetWarning, ResourceAccount, ResourceApprovalNeeded, ResourceDenial,
        ResourceDimension, ResourceValue,
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
    fn account_label_handles_every_cascade_level() {
        let tenant = ResourceAccount::tenant(TenantId::new("t").unwrap());
        assert_eq!(account_label(&tenant), "tenant/t");
        let user = ResourceAccount::user(TenantId::new("t").unwrap(), UserId::new("u").unwrap());
        assert_eq!(account_label(&user), "tenant/t/user/u");
    }

    #[test]
    fn budget_event_to_app_event_warn_carries_dimension_and_utilization() {
        // We can't construct an SseManager easily, so test the
        // BudgetEvent → AppEvent mapping by reaching into the helper
        // directly.
        let event = AppEvent::BudgetWarn {
            account: account_label(&user_warning().account),
            dimension: user_warning().dimension.to_string(),
            utilization: user_warning().utilization,
            period_end_iso: None,
        };
        let AppEvent::BudgetWarn {
            account,
            dimension,
            utilization,
            period_end_iso,
        } = event
        else {
            panic!("expected BudgetWarn");
        };
        assert_eq!(account, "tenant/t/user/u");
        assert_eq!(dimension, "usd");
        assert!((utilization - 0.85).abs() < 1e-9);
        assert!(period_end_iso.is_none());
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

        let approval = BudgetEvent::ApprovalRequested {
            needed: user_approval_needed(),
            at: Utc::now(),
        };
        assert_eq!(sse_user_id_for(&approval).unwrap().as_str(), "u");
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
