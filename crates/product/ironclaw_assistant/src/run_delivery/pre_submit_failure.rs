//! Durable Inbox publication for trigger fires that fail before a run exists.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::ids::{TenantId, UserId};
use ironclaw_notifications::{
    LifecycleRef, NotificationAction, NotificationId, NotificationInboxStorePort,
    NotificationInitialState, NotificationKind, NotificationRecipient, NotificationSeverity,
    NotificationSource, PublishNotificationRequest,
};
use ironclaw_outbound::{
    ProjectionUpdateRef, TriggeredFireFailureDeliveryRequest, TriggeredRunDelivery,
    TriggeredRunDeliveryRequest,
};

/// Inbox persistence is best-effort on a detached settlement path. A stalled
/// storage backend must not hold external channel delivery indefinitely.
const PRE_SUBMIT_INBOX_PUBLISH_TIMEOUT: Duration = Duration::from_secs(2);
const TRACE_TARGET: &str = "ironclaw::reborn::run_delivery";

/// Publishes permanently settled pre-submit failures to the durable Inbox.
///
/// This notifier is intentionally independent from channel delivery: runtimes
/// without an egress coordinator still have an Inbox, and a stalled Inbox must
/// remain bounded when the settlement hook fans out concurrently.
pub struct PreSubmitFailureInboxNotifier {
    inbox: Arc<dyn NotificationInboxStorePort>,
    publish_timeout: Duration,
}

impl PreSubmitFailureInboxNotifier {
    pub fn new(inbox: Arc<dyn NotificationInboxStorePort>) -> Self {
        Self::with_publish_timeout(inbox, PRE_SUBMIT_INBOX_PUBLISH_TIMEOUT)
    }

    /// Override the persistence deadline for deployments with a known storage
    /// latency budget and for deterministic contract tests.
    pub fn with_publish_timeout(
        inbox: Arc<dyn NotificationInboxStorePort>,
        publish_timeout: Duration,
    ) -> Self {
        Self {
            inbox,
            publish_timeout,
        }
    }
}

#[async_trait]
impl TriggeredRunDelivery for PreSubmitFailureInboxNotifier {
    async fn on_trigger_submitted(&self, _request: TriggeredRunDeliveryRequest) {}

    async fn on_trigger_failed_before_submit(&self, request: TriggeredFireFailureDeliveryRequest) {
        if request.project_scoped {
            return;
        }
        publish_pre_submit_failure_inbox_notification(
            self.inbox.as_ref(),
            &request.creator_user_id,
            &request.scope.tenant_id,
            &request.failure_ref,
            self.publish_timeout,
        )
        .await;
    }
}

/// Persist one metadata-only failure record for a permanently settled fire.
///
/// A pre-submit failure has no run id, so the stable, opaque fire reference is
/// hashed into the notification identity. Inbox publication remains
/// best-effort and independent from configured external notification channels.
async fn publish_pre_submit_failure_inbox_notification(
    inbox: &dyn NotificationInboxStorePort,
    user_id: &UserId,
    tenant_id: &TenantId,
    failure_ref: &ProjectionUpdateRef,
    publish_timeout: Duration,
) {
    let fire_key = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, failure_ref.as_str().as_bytes());
    let notification_id = match NotificationId::new(format!(
        "trigger-fire:{fire_key}:{}",
        NotificationKind::RunFailed.stable_key()
    )) {
        Ok(id) => id,
        Err(error) => {
            tracing::debug!(target: TRACE_TARGET, %error, "invalid pre-submit failure Inbox id");
            return;
        }
    };
    let lifecycle_ref = match LifecycleRef::new(format!("trigger-fire:{fire_key}")) {
        Ok(reference) => reference,
        Err(error) => {
            tracing::debug!(target: TRACE_TARGET, %error, "invalid pre-submit failure lifecycle ref");
            return;
        }
    };
    let publication = inbox.publish(PublishNotificationRequest {
        id: notification_id,
        recipient: NotificationRecipient {
            tenant_id: tenant_id.clone(),
            user_id: user_id.clone(),
        },
        kind: NotificationKind::RunFailed,
        severity: NotificationSeverity::Error,
        source: NotificationSource {
            thread_id: None,
            turn_run_id: None,
            lifecycle_ref: Some(lifecycle_ref),
            credential_providers: Vec::new(),
        },
        action: NotificationAction::None,
        initial_state: NotificationInitialState::Resolved,
        occurred_at: Utc::now(),
    });
    match tokio::time::timeout(publish_timeout, publication).await {
        Ok(Ok(_record)) => {}
        Ok(Err(error)) => {
            tracing::debug!(
                target: TRACE_TARGET,
                %error,
                fire_key = %fire_key,
                "failed to publish pre-submit failure to durable Inbox"
            );
        }
        Err(_elapsed) => {
            tracing::debug!(
                target: TRACE_TARGET,
                fire_key = %fire_key,
                timeout_ms = publish_timeout.as_millis(),
                "timed out publishing pre-submit failure to durable Inbox"
            );
        }
    }
}
