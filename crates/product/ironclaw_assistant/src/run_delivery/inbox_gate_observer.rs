//! Bounded Inbox-only observation after external gate delivery settles.
//!
//! A background run may remain parked after its channel delivery outcome is
//! final. This lane keeps the durable Inbox record aligned without retaining a
//! delivery semaphore permit or retrying external fan-out. Its total lifetime
//! is capped by the caller's `max_wait`, so abandoned gates cannot accumulate
//! permanent polling tasks.

use ironclaw_host_api::ids::UserId;
use ironclaw_turns::{TurnRunId, TurnScope};

use super::{
    BlockedActionableMarker, RunDeliveryError, RunDeliveryServices, RunDeliverySettings,
    blocked_actionable_marker, blocked_status_notification_kind, wait_for_actionable_state,
};

const TRACE_TARGET: &str = "ironclaw::reborn::run_delivery";

pub(super) fn spawn_inbox_gate_observer(
    services: &RunDeliveryServices,
    settings: RunDeliverySettings,
    scope: TurnScope,
    creator_user_id: UserId,
    run_id: TurnRunId,
    marker: BlockedActionableMarker,
) {
    if services.notification_inbox.is_none() {
        return;
    }
    let services = services.clone();
    tokio::spawn(async move {
        observe_inbox_gate_lifecycle(
            &services,
            &settings,
            &scope,
            &creator_user_id,
            run_id,
            marker,
        )
        .await;
    });
}

async fn observe_inbox_gate_lifecycle(
    services: &RunDeliveryServices,
    settings: &RunDeliverySettings,
    scope: &TurnScope,
    creator_user_id: &UserId,
    run_id: TurnRunId,
    mut marker: BlockedActionableMarker,
) {
    let started_at = tokio::time::Instant::now();
    loop {
        let elapsed = started_at.elapsed();
        if elapsed >= settings.max_wait {
            tracing::debug!(
                target: TRACE_TARGET,
                %run_id,
                "durable Inbox gate observation reached its total deadline"
            );
            return;
        }
        let bounded_settings = RunDeliverySettings {
            max_wait: settings.max_wait - elapsed,
            ..*settings
        };
        let state = match wait_for_actionable_state(
            services.turn_coordinator.as_ref(),
            scope,
            run_id,
            &bounded_settings,
            Some(&marker),
        )
        .await
        {
            Ok(state) => state,
            Err(RunDeliveryError::RunWaitTimedOut { .. }) => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    %run_id,
                    "durable Inbox gate observation reached its total deadline"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    target: TRACE_TARGET,
                    %run_id,
                    %error,
                    "durable Inbox gate observation failed"
                );
                return;
            }
        };

        if let Some(kind) = blocked_status_notification_kind(marker.status) {
            services
                .resolve_inbox_notification(
                    creator_user_id,
                    scope,
                    run_id,
                    kind,
                    marker.gate_ref.as_deref(),
                )
                .await;
        }

        let Some(next_marker) = blocked_actionable_marker(&state) else {
            return;
        };
        if let Some(kind) = blocked_status_notification_kind(next_marker.status) {
            services
                .publish_inbox_notification(
                    creator_user_id,
                    scope,
                    run_id,
                    kind,
                    next_marker.gate_ref.as_deref(),
                )
                .await;
        }
        marker = next_marker;
    }
}
