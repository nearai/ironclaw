//! Filesystem-backed [`OutboundStateStore`] implementation.
//!
//! Persists outbound metadata under a fixed virtual-path tree
//! (`/engine/outbound/...`) using the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) surface. Adding
//! this alongside the SQL backends gives every operator the option of
//! mounting outbound state on the universal filesystem fabric (libSQL,
//! Postgres, in-memory, or HSM-decorated) without reaching back into a
//! per-crate driver.
//!
//! Per-record paths:
//! - `/engine/outbound/policies/<thread-scope-key>.json` — thread
//!   notification policy keyed by `(tenant, agent?, project?, thread)`.
//! - `/engine/outbound/subscriptions/<subscription-key>.json` — projection
//!   subscription cursor keyed by `(subscription_id, actor, scope, thread)`.
//!   The key is a deterministic hash so the path doesn't leak the actor on
//!   list operations.
//! - `/engine/outbound/deliveries/<delivery_id>.json` — delivery attempt
//!   keyed by `delivery_id`.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_filesystem::{CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem};
use ironclaw_host_api::{ThreadId, VirtualPath};
use ironclaw_turns::{TurnActor, TurnScope};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::validation::{
    validate_advance_request, validate_delivery_attempt, validate_delivery_identity,
    validate_delivery_status_request, validate_policy, validate_subscription_identity,
    validate_subscription_record, validate_subscription_request,
};
use crate::{
    AdvanceSubscriptionCursorRequest, LoadSubscriptionCursorRequest, OutboundDeliveryAttempt,
    OutboundDeliveryId, OutboundError, OutboundStateStore, ProjectionSubscriptionId,
    ProjectionSubscriptionRecord, ThreadNotificationPolicy, UpdateDeliveryStatusRequest,
};

/// Filesystem-backed outbound store. Construct with any
/// [`RootFilesystem`] implementation (libSQL, Postgres, in-memory, …) — the
/// store doesn't care which.
pub struct FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

impl<F> FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    async fn put_json<T: Serialize>(
        &self,
        path: &VirtualPath,
        value: &T,
    ) -> Result<(), OutboundError> {
        let body = serde_json::to_vec(value).map_err(|_| OutboundError::Serialization)?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(map_fs_error)
    }

    async fn get_json<T: for<'de> serde::Deserialize<'de>>(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<T>, OutboundError> {
        let Some(versioned) = self.filesystem.get(path).await.map_err(map_fs_error)? else {
            return Ok(None);
        };
        let parsed = serde_json::from_slice(&versioned.entry.body)
            .map_err(|_| OutboundError::Serialization)?;
        Ok(Some(parsed))
    }
}

#[async_trait]
impl<F> OutboundStateStore for FilesystemOutboundStateStore<F>
where
    F: RootFilesystem,
{
    async fn put_thread_notification_policy(
        &self,
        policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        validate_policy(&policy)?;
        let path = policy_path(&policy.scope)?;
        self.put_json(&path, &policy).await
    }

    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        let path = policy_path(&scope)?;
        match self.get_json::<ThreadNotificationPolicy>(&path).await? {
            Some(policy) => Ok(policy),
            None => Ok(ThreadNotificationPolicy::default_for_scope(scope)),
        }
    }

    async fn upsert_subscription(
        &self,
        record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        validate_subscription_record(&record)?;
        let path = subscription_path(
            &record.subscription_id,
            &record.actor,
            &record.scope,
            &record.thread_id,
        )?;
        if let Some(existing) = self.get_json::<ProjectionSubscriptionRecord>(&path).await? {
            validate_subscription_identity(&existing, &record)?;
        }
        self.put_json(&path, &record).await
    }

    async fn load_subscription_cursor(
        &self,
        request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError> {
        let path = subscription_path(
            &request.subscription_id,
            &request.actor,
            &request.scope,
            &request.thread_id,
        )?;
        let Some(record) = self.get_json::<ProjectionSubscriptionRecord>(&path).await? else {
            return Ok(None);
        };
        validate_subscription_request(&record, &request)?;
        Ok(record.cursor)
    }

    async fn advance_subscription_cursor(
        &self,
        request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        let path = subscription_path(
            &request.subscription_id,
            &request.actor,
            &request.cursor.scope,
            &request.thread_id,
        )?;
        let Some(mut record) = self.get_json::<ProjectionSubscriptionRecord>(&path).await? else {
            return Err(OutboundError::SubscriptionScopeMismatch);
        };
        validate_advance_request(&record, &request)?;
        record.cursor = Some(request.cursor);
        self.put_json(&path, &record).await
    }

    async fn record_delivery_attempt(
        &self,
        attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        validate_delivery_attempt(&attempt)?;
        let path = delivery_path(&attempt.delivery_id)?;
        if let Some(existing) = self.get_json::<OutboundDeliveryAttempt>(&path).await? {
            validate_delivery_identity(&existing, &attempt)?;
            return Ok(());
        }
        self.put_json(&path, &attempt).await
    }

    async fn update_delivery_status(
        &self,
        request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        validate_delivery_status_request(&request)?;
        let path = delivery_path(&request.delivery_id)?;
        let Some(mut attempt) = self.get_json::<OutboundDeliveryAttempt>(&path).await? else {
            return Err(OutboundError::DeliveryNotFound);
        };
        if attempt.scope != request.scope {
            return Err(OutboundError::SubscriptionScopeMismatch);
        }
        attempt.status = request.status;
        attempt.failure_kind = request.failure_kind;
        self.put_json(&path, &attempt).await
    }

    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        // Without native query/indexes on the unified filesystem yet (deferred
        // to a follow-up port that translates IndexKind::Exact on indexed
        // projections), we scan the deliveries directory and filter by scope
        // in memory. The on-disk layout is flat per `delivery_id`, so this
        // bounded scan is acceptable until the indexer port lands here.
        let root =
            VirtualPath::new("/engine/outbound/deliveries").map_err(|_| OutboundError::Backend)?;
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(Vec::new()),
            Err(error) => return Err(map_fs_error(error)),
        };
        let mut deliveries = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let Some(attempt) = self
                .get_json::<OutboundDeliveryAttempt>(&entry.path)
                .await?
            else {
                continue;
            };
            if scope_matches(&attempt.scope, &scope) {
                deliveries.push(attempt);
            }
        }
        deliveries.sort_by_key(|attempt| (attempt.attempted_at, attempt.delivery_id.to_string()));
        Ok(deliveries)
    }
}

fn policy_path(scope: &TurnScope) -> Result<VirtualPath, OutboundError> {
    let key = thread_scope_key(scope);
    VirtualPath::new(format!("/engine/outbound/policies/{key}.json"))
        .map_err(|_| OutboundError::Backend)
}

fn subscription_path(
    subscription_id: &ProjectionSubscriptionId,
    actor: &TurnActor,
    scope: &ProjectionScope,
    thread_id: &ThreadId,
) -> Result<VirtualPath, OutboundError> {
    #[derive(Serialize)]
    struct SubscriptionIdentity<'a> {
        subscription_id: &'a ProjectionSubscriptionId,
        actor: &'a TurnActor,
        scope: &'a ProjectionScope,
        thread_id: &'a ThreadId,
    }
    let identity = SubscriptionIdentity {
        subscription_id,
        actor,
        scope,
        thread_id,
    };
    let serialized = serde_json::to_string(&identity).map_err(|_| OutboundError::Serialization)?;
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    let digest = hex::encode(hasher.finalize());
    VirtualPath::new(format!("/engine/outbound/subscriptions/{digest}.json"))
        .map_err(|_| OutboundError::Backend)
}

fn delivery_path(delivery_id: &OutboundDeliveryId) -> Result<VirtualPath, OutboundError> {
    VirtualPath::new(format!("/engine/outbound/deliveries/{delivery_id}.json"))
        .map_err(|_| OutboundError::Backend)
}

fn thread_scope_key(scope: &TurnScope) -> String {
    let agent = scope
        .agent_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "_".to_string());
    let project = scope
        .project_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "_".to_string());
    let serialized = format!(
        "{}|{}|{}|{}",
        scope.tenant_id, agent, project, scope.thread_id
    );
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    hex::encode(hasher.finalize())
}

fn scope_matches(left: &TurnScope, right: &TurnScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.thread_id == right.thread_id
}

fn map_fs_error(_error: FilesystemError) -> OutboundError {
    // The outbound CLAUDE.md guardrails forbid leaking backend error detail
    // strings. The FilesystemError variants already keep host paths internal,
    // but we collapse to OutboundError::Backend to honour the crate's no-leak
    // contract.
    OutboundError::Backend
}
