use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, FilesystemError, IndexKey, IndexValue,
    RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    ids::{ThreadId, VendorId},
    path::ScopedPath,
    resource::ResourceScope,
    turn::TurnRunId,
};
use serde::{Deserialize, Serialize};

use crate::{
    LifecycleRef, ListNotificationsRequest, MarkAllNotificationsReadRequest,
    NOTIFICATION_INBOX_MAX_RECORDS, NOTIFICATION_PAGE_LIMIT_MAX, NotificationAction,
    NotificationId, NotificationInboxError, NotificationInboxStorePort, NotificationKind,
    NotificationMutationOutcome, NotificationMutationRequest, NotificationPage,
    NotificationRecipient, NotificationRecord, NotificationSeverity, NotificationSource,
    PublishNotificationRequest,
};

const NOTIFICATION_INBOX_PATH: &str = "/notifications/inbox.json";
const NOTIFICATION_INBOX_SCHEMA_VERSION: u8 = 1;
/// Required only in the legacy schema-v1 fields of a non-thread-backed record.
/// New readers recover the absent thread from the additive compatibility
/// metadata. Rollback readers see a valid, non-route placeholder plus an
/// archived lifecycle, so their default ProductSurface hides the unusable
/// action while retaining and mutating the snapshot. The decoder also
/// recognizes the placeholder after a rollback writer drops additive fields.
const LEGACY_NO_THREAD_COMPAT_ID: &str = "ironclaw-notification-no-thread-v1";
/// Schema-v1-visible companion identity for a non-actionable record that the
/// recipient really archived. Keeping the real archive bit in the legacy
/// source/action identity lets a current reader distinguish it after a
/// rollback writer strips additive lifecycle metadata.
const LEGACY_NO_THREAD_ARCHIVED_COMPAT_ID: &str = "ironclaw-notification-no-thread-archived-v1";
const TENANT_ID_INDEX_KEY: &str = "tenant_id";
const NOTIFICATION_CURSOR_MAX_BYTES: usize = (20 + 1 + crate::types::NOTIFICATION_ID_MAX_BYTES) * 2;

#[derive(Debug, Clone, PartialEq, Eq)]
struct NotificationInboxSnapshot {
    schema_version: u8,
    recipient: NotificationRecipient,
    notifications: Vec<NotificationRecord>,
}

/// On-disk schema-v1 representation. The top-level shape remains closed; each
/// record stays open to additive compatibility fields because the
/// pre-change `NotificationRecord` reader ignored unknown fields as well.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedNotificationInboxSnapshotV1 {
    schema_version: u8,
    recipient: NotificationRecipient,
    notifications: Vec<PersistedNotificationRecordV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedNotificationRecordV1 {
    id: NotificationId,
    recipient: NotificationRecipient,
    kind: NotificationKind,
    severity: NotificationSeverity,
    source: PersistedNotificationSourceV1,
    action: NotificationAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_v2: Option<PersistedNotificationSourceV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action_v2: Option<NotificationAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifecycle_v2: Option<PersistedNotificationLifecycleV2>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    read_at: Option<DateTime<Utc>>,
    resolved_at: Option<DateTime<Utc>>,
    archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedNotificationSourceV1 {
    thread_id: ThreadId,
    turn_run_id: Option<TurnRunId>,
    lifecycle_ref: Option<LifecycleRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    credential_providers: Vec<VendorId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedNotificationSourceV2 {
    thread_id: Option<ThreadId>,
}

/// Lifecycle fields whose legacy representation must deliberately differ from
/// the head representation. Non-actionable records are archived for rollback
/// readers so their placeholder `OpenThread` action never reaches the old
/// ProductSurface, while head readers restore the real archive state here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedNotificationLifecycleV2 {
    archived_at: Option<DateTime<Utc>>,
}

pub struct NotificationInboxStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    /// The per-recipient record bound this store enforces. It is the
    /// deduplication window as well as the size bound, so it is configuration
    /// the caller states rather than a constant compiled into the store.
    capacity: usize,
}

impl<F> NotificationInboxStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, capacity: usize) -> Self {
        Self {
            filesystem,
            capacity,
        }
    }

    async fn get_snapshot(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<NotificationInboxSnapshot>, NotificationInboxError> {
        let Some(versioned) = self
            .filesystem
            .get(scope, path)
            .await
            .map_err(map_filesystem_error)?
        else {
            return Ok(None);
        };
        decode_snapshot(&versioned.entry.body).map(Some)
    }
}

#[async_trait]
impl<F> NotificationInboxStorePort for NotificationInboxStore<F>
where
    F: RootFilesystem,
{
    async fn publish(
        &self,
        request: PublishNotificationRequest,
    ) -> Result<NotificationRecord, NotificationInboxError> {
        validate_new_notification_action(&request.source, &request.action)?;
        validate_notification_source(request.kind, &request.source)?;
        let scope = notification_resource_scope(&request.recipient);
        let path = notification_inbox_path()?;
        let recipient = request.recipient.clone();
        let capacity = self.capacity;
        cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            decode_snapshot,
            encode_snapshot,
            move |current: Option<NotificationInboxSnapshot>| {
                let request = request.clone();
                let recipient = recipient.clone();
                async move {
                    let mut snapshot = current.unwrap_or_else(|| NotificationInboxSnapshot {
                        schema_version: NOTIFICATION_INBOX_SCHEMA_VERSION,
                        recipient: recipient.clone(),
                        notifications: Vec::new(),
                    });
                    validate_snapshot(&snapshot, &recipient)?;
                    if let Some(existing) = snapshot
                        .notifications
                        .iter_mut()
                        .find(|record| record.id == request.id)
                    {
                        let same_source_identity = existing.source.thread_id
                            == request.source.thread_id
                            && existing.source.turn_run_id == request.source.turn_run_id
                            && existing.source.lifecycle_ref == request.source.lifecycle_ref;
                        let providers_match = existing.source.credential_providers
                            == request.source.credential_providers;
                        let provider_metadata_is_compatible = providers_match
                            || (request.kind == crate::NotificationKind::AuthenticationRequired
                                && (existing.source.credential_providers.is_empty()
                                    || request.source.credential_providers.is_empty()));
                        if existing.recipient != request.recipient
                            || existing.kind != request.kind
                            || existing.severity != request.severity
                            || !same_source_identity
                            || !provider_metadata_is_compatible
                            || existing.action != request.action
                        {
                            return Err(NotificationInboxError::InvalidRequest {
                                reason: "notification id conflicts with an existing event",
                            });
                        }
                        if existing.source.credential_providers.is_empty()
                            && !request.source.credential_providers.is_empty()
                        {
                            // One-way compatibility enrichment for records
                            // written before auth sources carried providers.
                            // Mixed-version retries with an empty set remain a
                            // no-op and never erase trusted metadata.
                            existing.source.credential_providers =
                                request.source.credential_providers.clone();
                            let record = existing.clone();
                            return Ok(CasApply::new(snapshot, record));
                        }
                        if matches!(
                            request.kind,
                            crate::NotificationKind::RunFailed
                                | crate::NotificationKind::RunCompleted
                                | crate::NotificationKind::DeliveryFailed
                        ) && request.initial_state == crate::NotificationInitialState::Resolved
                            && existing.resolved_at.is_none()
                        {
                            // Compatibility reconciliation for terminal records
                            // written before producers supplied initial lifecycle
                            // state. Repair only the missing terminal fact: read,
                            // archive, and update timestamps are user/durable state
                            // and must survive retries and process restarts intact.
                            existing.resolved_at = Some(existing.created_at);
                            let record = existing.clone();
                            return Ok(CasApply::new(snapshot, record));
                        }
                        let record = existing.clone();
                        return Ok(CasApply::no_op(snapshot, record));
                    }
                    // Drain to the active bound rather than shedding one record
                    // per call: a bound that was lowered while records already
                    // existed has to actually take effect, and reads never reject
                    // an over-bound snapshot, so draining here is what converges
                    // it instead of locking the recipient out.
                    while snapshot.notifications.len() >= capacity
                        && evict_oldest_closed_record(&mut snapshot)
                    {}
                    if snapshot.notifications.len() >= capacity {
                        return Err(NotificationInboxError::InvalidRequest {
                            reason: "notification inbox is at capacity",
                        });
                    }
                    let record = NotificationRecord {
                        id: request.id,
                        recipient: request.recipient,
                        kind: request.kind,
                        severity: request.severity,
                        source: request.source,
                        action: request.action,
                        created_at: request.occurred_at,
                        updated_at: request.occurred_at,
                        read_at: None,
                        resolved_at: (request.initial_state
                            == crate::NotificationInitialState::Resolved)
                            .then_some(request.occurred_at),
                        archived_at: None,
                    };
                    snapshot.notifications.push(record.clone());
                    Ok(CasApply::new(snapshot, record))
                }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn list(
        &self,
        request: ListNotificationsRequest,
    ) -> Result<NotificationPage, NotificationInboxError> {
        if request.limit == 0 || request.limit > NOTIFICATION_PAGE_LIMIT_MAX {
            return Err(NotificationInboxError::InvalidRequest {
                reason: "notification page limit is invalid",
            });
        }
        let scope = notification_resource_scope(&request.recipient);
        let path = notification_inbox_path()?;
        let Some(snapshot) = self.get_snapshot(&scope, &path).await? else {
            return Ok(NotificationPage {
                notifications: Vec::new(),
                next_cursor: None,
                unread_count: 0,
            });
        };
        validate_snapshot(&snapshot, &request.recipient)?;
        let unread_count = snapshot
            .notifications
            .iter()
            .filter(|record| record.archived_at.is_none() && record.read_at.is_none())
            .count();
        let mut notifications = snapshot
            .notifications
            .into_iter()
            .filter(|record| request.include_archived || record.archived_at.is_none())
            .collect::<Vec<_>>();
        notifications.sort_by(|left, right| {
            right
                .created_at
                .timestamp_micros()
                .cmp(&left.created_at.timestamp_micros())
                .then_with(|| right.id.as_str().cmp(left.id.as_str()))
        });
        let start = request
            .cursor
            .as_deref()
            .map(decode_cursor)
            .transpose()?
            .map(|cursor| {
                notifications
                    .iter()
                    .position(|record| notification_is_after_cursor(record, &cursor))
                    .unwrap_or(notifications.len())
            })
            .unwrap_or(0);
        let end = start.saturating_add(request.limit).min(notifications.len());
        let page = notifications[start..end].to_vec();
        let next_cursor = (end < notifications.len())
            .then(|| page.last().map(notification_cursor))
            .flatten();
        Ok(NotificationPage {
            notifications: page,
            next_cursor,
            unread_count,
        })
    }

    async fn mark_read(
        &self,
        request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
        mutate_notification(self, request, |record, occurred_at| {
            if record.read_at.is_some() {
                return false;
            }
            record.read_at = Some(occurred_at.max(record.created_at));
            true
        })
        .await
    }

    async fn mark_all_read(
        &self,
        request: MarkAllNotificationsReadRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
        let scope = notification_resource_scope(&request.recipient);
        let path = notification_inbox_path()?;
        let recipient = request.recipient.clone();
        cas_update(
            self.filesystem.as_ref(),
            &scope,
            &path,
            decode_snapshot,
            encode_snapshot,
            move |current: Option<NotificationInboxSnapshot>| {
                let recipient = recipient.clone();
                async move {
                    let Some(mut snapshot) = current else {
                        return Ok(CasApply::no_op(
                            NotificationInboxSnapshot {
                                schema_version: NOTIFICATION_INBOX_SCHEMA_VERSION,
                                recipient,
                                notifications: Vec::new(),
                            },
                            NotificationMutationOutcome::AlreadySettled,
                        ));
                    };
                    validate_snapshot(&snapshot, &recipient)?;
                    let mut changed = false;
                    for notification in &mut snapshot.notifications {
                        if notification.archived_at.is_none() && notification.read_at.is_none() {
                            notification.read_at =
                                Some(request.occurred_at.max(notification.created_at));
                            notification.updated_at =
                                notification.updated_at.max(request.occurred_at);
                            changed = true;
                        }
                    }
                    if changed {
                        Ok(CasApply::new(
                            snapshot,
                            NotificationMutationOutcome::Applied,
                        ))
                    } else {
                        Ok(CasApply::no_op(
                            snapshot,
                            NotificationMutationOutcome::AlreadySettled,
                        ))
                    }
                }
            },
        )
        .await
        .map_err(map_cas_error)
    }

    async fn resolve(
        &self,
        request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
        mutate_notification(self, request, |record, occurred_at| {
            let occurred_at = occurred_at.max(record.created_at);
            if record.resolved_at.is_some() {
                return false;
            }
            record.resolved_at = Some(occurred_at);
            true
        })
        .await
    }

    async fn reopen(
        &self,
        request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
        mutate_notification(self, request, |record, _occurred_at| {
            if !matches!(
                record.kind,
                crate::NotificationKind::ApprovalRequired
                    | crate::NotificationKind::AuthenticationRequired
                    | crate::NotificationKind::RunBlocked
            ) || record.resolved_at.is_none()
            {
                return false;
            }
            record.resolved_at = None;
            true
        })
        .await
    }

    async fn archive(
        &self,
        request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
        mutate_notification(self, request, |record, occurred_at| {
            let occurred_at = occurred_at.max(record.created_at);
            if record.archived_at.is_some() {
                return false;
            }
            record.archived_at = Some(occurred_at);
            true
        })
        .await
    }
}

async fn mutate_notification<F, M>(
    store: &NotificationInboxStore<F>,
    request: NotificationMutationRequest,
    mutation: M,
) -> Result<NotificationMutationOutcome, NotificationInboxError>
where
    F: RootFilesystem,
    M: Fn(&mut NotificationRecord, DateTime<Utc>) -> bool + Send + Sync + Clone + 'static,
{
    let scope = notification_resource_scope(&request.recipient);
    let path = notification_inbox_path()?;
    let recipient = request.recipient.clone();
    cas_update(
        store.filesystem.as_ref(),
        &scope,
        &path,
        decode_snapshot,
        encode_snapshot,
        move |current: Option<NotificationInboxSnapshot>| {
            let recipient = recipient.clone();
            let request = request.clone();
            let mutation = mutation.clone();
            async move {
                let Some(mut snapshot) = current else {
                    return Err(NotificationInboxError::NotificationNotFound);
                };
                validate_snapshot(&snapshot, &recipient)?;
                let record = snapshot
                    .notifications
                    .iter_mut()
                    .find(|record| record.id == request.notification_id)
                    .ok_or(NotificationInboxError::NotificationNotFound)?;
                if !mutation(record, request.occurred_at) {
                    return Ok(CasApply::no_op(
                        snapshot,
                        NotificationMutationOutcome::AlreadySettled,
                    ));
                }
                record.updated_at = record.updated_at.max(request.occurred_at);
                Ok(CasApply::new(
                    snapshot,
                    NotificationMutationOutcome::Applied,
                ))
            }
        },
    )
    .await
    .map_err(map_cas_error)
}

fn notification_inbox_path() -> Result<ScopedPath, NotificationInboxError> {
    ScopedPath::new(NOTIFICATION_INBOX_PATH).map_err(|error| NotificationInboxError::Backend {
        reason: error.to_string(),
    })
}

fn notification_resource_scope(recipient: &NotificationRecipient) -> ResourceScope {
    let mut scope = ResourceScope::system();
    scope.tenant_id = recipient.tenant_id.clone();
    scope.user_id = recipient.user_id.clone();
    scope
}

fn validate_notification_action(
    source: &NotificationSource,
    action: &NotificationAction,
) -> Result<(), NotificationInboxError> {
    if source.thread_id.is_none() && source.turn_run_id.is_some() {
        return Err(NotificationInboxError::InvalidRequest {
            reason: "notification run source has no canonical thread",
        });
    }
    match action {
        NotificationAction::None if source.thread_id.is_none() => Ok(()),
        NotificationAction::None => Err(NotificationInboxError::InvalidRequest {
            reason: "non-actionable notification unexpectedly has a source thread",
        }),
        NotificationAction::OpenThread { thread_id }
            if source.thread_id.as_ref() != Some(thread_id) =>
        {
            Err(NotificationInboxError::InvalidRequest {
                reason: "notification action does not match its source thread",
            })
        }
        NotificationAction::OpenThread { .. } => Ok(()),
    }
}

fn validate_new_notification_action(
    source: &NotificationSource,
    action: &NotificationAction,
) -> Result<(), NotificationInboxError> {
    if source
        .thread_id
        .as_ref()
        .is_some_and(is_reserved_compatibility_thread_id)
    {
        return Err(NotificationInboxError::InvalidRequest {
            reason: "notification source uses a reserved compatibility identity",
        });
    }
    validate_notification_action(source, action)
}

fn validate_notification_source(
    kind: crate::NotificationKind,
    source: &NotificationSource,
) -> Result<(), NotificationInboxError> {
    if kind != crate::NotificationKind::AuthenticationRequired
        && !source.credential_providers.is_empty()
    {
        return Err(NotificationInboxError::InvalidRequest {
            reason: "credential providers are only valid for authentication notifications",
        });
    }
    Ok(())
}

fn validate_snapshot(
    snapshot: &NotificationInboxSnapshot,
    recipient: &NotificationRecipient,
) -> Result<(), NotificationInboxError> {
    if snapshot.schema_version != NOTIFICATION_INBOX_SCHEMA_VERSION {
        return Err(NotificationInboxError::Serialization {
            reason: "unsupported notification inbox schema version".to_string(),
        });
    }
    if &snapshot.recipient != recipient {
        return Err(NotificationInboxError::AccessDenied);
    }
    if snapshot.notifications.len() > NOTIFICATION_INBOX_MAX_RECORDS {
        return Err(NotificationInboxError::Serialization {
            reason: "notification inbox record bound exceeded".to_string(),
        });
    }
    let mut ids = HashSet::with_capacity(snapshot.notifications.len());
    for record in &snapshot.notifications {
        let invalid_lifecycle_timestamp = [record.read_at, record.resolved_at, record.archived_at]
            .into_iter()
            .flatten()
            .any(|timestamp| timestamp < record.created_at || timestamp > record.updated_at);
        if &record.recipient != recipient
            || validate_notification_action(&record.source, &record.action).is_err()
            || record.updated_at < record.created_at
            || invalid_lifecycle_timestamp
            || !ids.insert(record.id.clone())
        {
            return Err(NotificationInboxError::Serialization {
                reason: "notification inbox record invariant failed".to_string(),
            });
        }
    }
    Ok(())
}

fn decode_snapshot(bytes: &[u8]) -> Result<NotificationInboxSnapshot, NotificationInboxError> {
    let persisted: PersistedNotificationInboxSnapshotV1 =
        serde_json::from_slice(bytes).map_err(|error| NotificationInboxError::Serialization {
            reason: error.to_string(),
        })?;
    Ok(NotificationInboxSnapshot {
        schema_version: persisted.schema_version,
        recipient: persisted.recipient,
        notifications: persisted
            .notifications
            .into_iter()
            .map(notification_record_from_persisted_v1)
            .collect(),
    })
}

fn encode_snapshot(snapshot: &NotificationInboxSnapshot) -> Result<Entry, NotificationInboxError> {
    validate_snapshot(snapshot, &snapshot.recipient)?;
    let persisted = PersistedNotificationInboxSnapshotV1 {
        schema_version: snapshot.schema_version,
        recipient: snapshot.recipient.clone(),
        notifications: snapshot
            .notifications
            .iter()
            .map(notification_record_to_persisted_v1)
            .collect::<Result<Vec<_>, _>>()?,
    };
    let body =
        serde_json::to_vec(&persisted).map_err(|error| NotificationInboxError::Serialization {
            reason: error.to_string(),
        })?;
    Ok(Entry::bytes(body)
        .with_content_type(ContentType::json())
        .with_indexed(
            tenant_id_index_key()?,
            tenant_id_index_value(&snapshot.recipient),
        ))
}

fn notification_record_to_persisted_v1(
    record: &NotificationRecord,
) -> Result<PersistedNotificationRecordV1, NotificationInboxError> {
    let legacy_thread_id = match record.source.thread_id.as_ref() {
        Some(thread_id) => thread_id.clone(),
        None => ThreadId::new(if record.archived_at.is_some() {
            LEGACY_NO_THREAD_ARCHIVED_COMPAT_ID
        } else {
            LEGACY_NO_THREAD_COMPAT_ID
        })
        .map_err(|error| NotificationInboxError::Serialization {
            reason: format!("notification compatibility identity is invalid: {error}"),
        })?,
    };
    let source_v2 = record
        .source
        .thread_id
        .is_none()
        .then_some(PersistedNotificationSourceV2 { thread_id: None });
    let non_actionable = matches!(&record.action, NotificationAction::None);
    let (action, action_v2) = match &record.action {
        NotificationAction::None => (
            NotificationAction::OpenThread {
                thread_id: legacy_thread_id.clone(),
            },
            Some(NotificationAction::None),
        ),
        NotificationAction::OpenThread { thread_id } => (
            NotificationAction::OpenThread {
                thread_id: thread_id.clone(),
            },
            None,
        ),
    };
    Ok(PersistedNotificationRecordV1 {
        id: record.id.clone(),
        recipient: record.recipient.clone(),
        kind: record.kind,
        severity: record.severity,
        source: PersistedNotificationSourceV1 {
            thread_id: legacy_thread_id,
            turn_run_id: record.source.turn_run_id,
            lifecycle_ref: record.source.lifecycle_ref.clone(),
            credential_providers: record.source.credential_providers.clone(),
        },
        action,
        source_v2,
        action_v2,
        lifecycle_v2: non_actionable.then_some(PersistedNotificationLifecycleV2 {
            archived_at: record.archived_at,
        }),
        created_at: record.created_at,
        updated_at: record.updated_at,
        read_at: record.read_at,
        resolved_at: record.resolved_at,
        archived_at: if non_actionable {
            record.archived_at.or(Some(record.updated_at))
        } else {
            record.archived_at
        },
    })
}

fn notification_record_from_persisted_v1(
    record: PersistedNotificationRecordV1,
) -> NotificationRecord {
    let compatibility_placeholder = is_legacy_no_thread_compatibility_record(&record);
    let source_thread_id = record.source_v2.map_or_else(
        || {
            if compatibility_placeholder || matches!(&record.action, NotificationAction::None) {
                None
            } else {
                Some(record.source.thread_id.clone())
            }
        },
        |source| source.thread_id,
    );
    let action = match record.action_v2 {
        Some(action) => action,
        None if compatibility_placeholder => NotificationAction::None,
        None => record.action,
    };
    let archived_at = match record.lifecycle_v2 {
        Some(lifecycle) => lifecycle.archived_at,
        None if compatibility_placeholder
            && record.source.thread_id.as_str() == LEGACY_NO_THREAD_COMPAT_ID =>
        {
            None
        }
        None => record.archived_at,
    };
    NotificationRecord {
        id: record.id,
        recipient: record.recipient,
        kind: record.kind,
        severity: record.severity,
        source: NotificationSource {
            thread_id: source_thread_id,
            turn_run_id: record.source.turn_run_id,
            lifecycle_ref: record.source.lifecycle_ref,
            credential_providers: record.source.credential_providers,
        },
        action,
        created_at: record.created_at,
        updated_at: record.updated_at,
        read_at: record.read_at,
        resolved_at: record.resolved_at,
        archived_at,
    }
}

fn is_legacy_no_thread_compatibility_record(record: &PersistedNotificationRecordV1) -> bool {
    if record.kind != NotificationKind::RunFailed
        || !is_reserved_compatibility_thread_id(&record.source.thread_id)
        || record.source.turn_run_id.is_some()
    {
        return false;
    }
    let Some(lifecycle_ref) = record.source.lifecycle_ref.as_ref() else {
        return false;
    };
    let action_uses_placeholder = matches!(
        &record.action,
        NotificationAction::OpenThread { thread_id }
            if is_reserved_compatibility_thread_id(thread_id)
    );
    action_uses_placeholder
        && record.id.as_str()
            == format!(
                "{}:{}",
                lifecycle_ref.as_str(),
                NotificationKind::RunFailed.stable_key()
            )
}

fn is_reserved_compatibility_thread_id(thread_id: &ThreadId) -> bool {
    matches!(
        thread_id.as_str(),
        LEGACY_NO_THREAD_COMPAT_ID | LEGACY_NO_THREAD_ARCHIVED_COMPAT_ID
    )
}

/// Reclaim one slot from a snapshot that is at its bound, so a new event is
/// never lost to the ceiling. Callers loop to drain, which is what converges a
/// snapshot that already exceeded a bound lowered under it. Only a record the
/// producer resolved *and* the recipient archived is
/// eligible; an open record is never evicted, because dropping one would lose
/// an actionable gate, which is the very thing this inbox exists to deliver.
/// Returns false when nothing is closed, leaving the write to fail rather than
/// sacrificing live state.
///
/// Reclaiming ends deduplication for that id: the snapshot is the only record
/// of it, so a later retry lands as a new record. The charter states the bound
/// as the idempotency window for exactly this reason — widening it means
/// persisting tombstones, which is a schema change with its own rollback
/// review.
fn evict_oldest_closed_record(snapshot: &mut NotificationInboxSnapshot) -> bool {
    let mut oldest: Option<(usize, DateTime<Utc>)> = None;
    for (index, record) in snapshot.notifications.iter().enumerate() {
        if record.archived_at.is_none() || record.resolved_at.is_none() {
            continue;
        }
        let is_older = match oldest {
            Some((_, created_at)) => record.created_at < created_at,
            None => true,
        };
        if is_older {
            oldest = Some((index, record.created_at));
        }
    }
    match oldest {
        Some((index, _)) => {
            snapshot.notifications.remove(index);
            true
        }
        None => false,
    }
}

fn tenant_id_index_key() -> Result<IndexKey, NotificationInboxError> {
    IndexKey::new(TENANT_ID_INDEX_KEY).map_err(|error| NotificationInboxError::Serialization {
        reason: format!("notification inbox tenant index key is invalid: {error}"),
    })
}

fn tenant_id_index_value(recipient: &NotificationRecipient) -> IndexValue {
    IndexValue::Text(recipient.tenant_id.as_str().to_string())
}

fn notification_cursor(record: &NotificationRecord) -> String {
    hex::encode(format!(
        "{}:{}",
        record.created_at.timestamp_micros(),
        record.id.as_str()
    ))
}

struct NotificationCursor {
    created_at_micros: i64,
    id: NotificationId,
}

fn decode_cursor(cursor: &str) -> Result<NotificationCursor, NotificationInboxError> {
    if cursor.is_empty() || cursor.len() > NOTIFICATION_CURSOR_MAX_BYTES {
        return Err(invalid_cursor());
    }
    let bytes = hex::decode(cursor).map_err(|error| {
        tracing::debug!(%error, "rejected malformed notification cursor encoding");
        invalid_cursor()
    })?;
    let raw = String::from_utf8(bytes).map_err(|error| {
        tracing::debug!(%error, "rejected non-UTF-8 notification cursor");
        invalid_cursor()
    })?;
    let (created_at_micros, id) = raw.split_once(':').ok_or_else(invalid_cursor)?;
    let created_at_micros = created_at_micros.parse::<i64>().map_err(|error| {
        tracing::debug!(%error, "rejected notification cursor timestamp");
        invalid_cursor()
    })?;
    let id = NotificationId::try_from(id.to_string()).map_err(|error| {
        tracing::debug!(%error, "rejected malformed notification cursor id");
        invalid_cursor()
    })?;
    Ok(NotificationCursor {
        created_at_micros,
        id,
    })
}

fn invalid_cursor() -> NotificationInboxError {
    NotificationInboxError::InvalidRequest {
        reason: "notification cursor is invalid",
    }
}

fn notification_is_after_cursor(record: &NotificationRecord, cursor: &NotificationCursor) -> bool {
    let created_at_micros = record.created_at.timestamp_micros();
    created_at_micros < cursor.created_at_micros
        || (created_at_micros == cursor.created_at_micros
            && record.id.as_str() < cursor.id.as_str())
}

fn map_cas_error(error: CasUpdateError<NotificationInboxError>) -> NotificationInboxError {
    match error {
        CasUpdateError::Apply(error) => error,
        CasUpdateError::Timeout => NotificationInboxError::Backend {
            reason: "notification inbox CAS update timed out".to_string(),
        },
        CasUpdateError::RetriesExhausted => NotificationInboxError::Backend {
            reason: "notification inbox CAS retries exhausted".to_string(),
        },
        CasUpdateError::CasUnsupported => NotificationInboxError::Backend {
            reason: "notification inbox backend does not support CAS".to_string(),
        },
        CasUpdateError::Backend(error) => NotificationInboxError::Backend {
            reason: error.to_string(),
        },
    }
}

fn map_filesystem_error(error: FilesystemError) -> NotificationInboxError {
    NotificationInboxError::Backend {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
    use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
    use ironclaw_host_api::path::{MountAlias, VirtualPath};
    use ironclaw_host_api::turn::TurnRunId;
    use serde::{Deserialize, Serialize};

    use super::*;

    /// Frozen copy of the pre-non-actionable schema-v1 reader. Record fields
    /// intentionally remain open to additive metadata, matching the shipped
    /// `NotificationRecord` serde contract.
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LegacyNotificationInboxSnapshotV1 {
        schema_version: u8,
        recipient: NotificationRecipient,
        notifications: Vec<LegacyNotificationRecordV1>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct LegacyNotificationRecordV1 {
        id: NotificationId,
        recipient: NotificationRecipient,
        kind: NotificationKind,
        severity: NotificationSeverity,
        source: LegacyNotificationSourceV1,
        action: LegacyNotificationActionV1,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        read_at: Option<DateTime<Utc>>,
        resolved_at: Option<DateTime<Utc>>,
        archived_at: Option<DateTime<Utc>>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct LegacyNotificationSourceV1 {
        thread_id: ThreadId,
        turn_run_id: Option<TurnRunId>,
        lifecycle_ref: Option<LifecycleRef>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum LegacyNotificationActionV1 {
        OpenThread { thread_id: ThreadId },
    }

    #[test]
    fn schema_v1_rollback_hides_non_actionable_head_write_instead_of_presenting_dead_link() {
        let recipient = NotificationRecipient {
            tenant_id: TenantId::new("rollback-tenant").expect("tenant"),
            user_id: UserId::new("rollback-user").expect("user"),
        };
        let occurred_at = Utc
            .timestamp_opt(1_700_000_001, 0)
            .single()
            .expect("occurred at");
        let snapshot = NotificationInboxSnapshot {
            schema_version: NOTIFICATION_INBOX_SCHEMA_VERSION,
            recipient: recipient.clone(),
            notifications: vec![NotificationRecord {
                id: NotificationId::new("trigger-fire:rollback:failed").expect("notification id"),
                recipient,
                kind: NotificationKind::RunFailed,
                severity: NotificationSeverity::Error,
                source: NotificationSource {
                    thread_id: None,
                    turn_run_id: None,
                    lifecycle_ref: Some(
                        LifecycleRef::new("trigger-fire:rollback").expect("lifecycle ref"),
                    ),
                    credential_providers: Vec::new(),
                },
                action: NotificationAction::None,
                created_at: occurred_at,
                updated_at: occurred_at,
                read_at: None,
                resolved_at: Some(occurred_at),
                archived_at: None,
            }],
        };

        let head_entry = encode_snapshot(&snapshot).expect("head writer encodes snapshot");
        let head_visible =
            decode_snapshot(&head_entry.body).expect("head reader decodes head write");
        assert!(head_visible.notifications[0].archived_at.is_none());
        assert_eq!(
            head_visible.notifications[0].action,
            NotificationAction::None,
        );
        let mut legacy: LegacyNotificationInboxSnapshotV1 =
            serde_json::from_slice(&head_entry.body).expect("base reader decodes head write");
        assert!(
            legacy.notifications[0].archived_at.is_some(),
            "the base reader must hide a non-actionable record from its default ProductSurface list",
        );
        let base_product_surface_notifications = legacy
            .notifications
            .iter()
            .filter(|record| record.archived_at.is_none())
            .collect::<Vec<_>>();
        assert!(
            base_product_surface_notifications.is_empty(),
            "a rollback WebUI must not receive an OpenThread action for the compatibility identity",
        );
        assert_eq!(
            legacy.notifications[0].source.thread_id.as_str(),
            LEGACY_NO_THREAD_COMPAT_ID,
            "the base reader receives a bounded compatibility identity, not a route key",
        );
        match &legacy.notifications[0].action {
            LegacyNotificationActionV1::OpenThread { thread_id } => assert_eq!(
                thread_id.as_str(),
                LEGACY_NO_THREAD_COMPAT_ID,
                "the frozen source/action invariant remains valid for rollback mutation",
            ),
        }
        let read_at = Utc
            .timestamp_opt(1_700_000_010, 0)
            .single()
            .expect("read at");
        legacy.notifications[0].read_at = Some(read_at);
        legacy.notifications[0].updated_at = read_at;

        let legacy_mutation = serde_json::to_vec(&legacy).expect("base writer persists mutation");
        let reopened =
            decode_snapshot(&legacy_mutation).expect("head reader reopens base mutation");
        assert_eq!(reopened.notifications[0].read_at, Some(read_at));
        assert_eq!(reopened.notifications[0].action, NotificationAction::None);
        assert!(reopened.notifications[0].source.thread_id.is_none());
        assert!(
            reopened.notifications[0].archived_at.is_none(),
            "a rollback rewrite must not promote the compatibility archive into durable user state",
        );
        let mut capacity_candidate = reopened.clone();
        assert!(
            !evict_oldest_closed_record(&mut capacity_candidate),
            "the synthetic rollback archive must never make an unarchived record eligible for eviction",
        );

        let archived_at = Utc
            .timestamp_opt(1_700_000_020, 0)
            .single()
            .expect("archived at");
        let mut archived_snapshot = snapshot;
        archived_snapshot.notifications[0].archived_at = Some(archived_at);
        archived_snapshot.notifications[0].updated_at = archived_at;
        let archived_entry =
            encode_snapshot(&archived_snapshot).expect("head writer encodes real archive");
        let archived_legacy: LegacyNotificationInboxSnapshotV1 =
            serde_json::from_slice(&archived_entry.body).expect("base reader decodes real archive");
        assert_eq!(
            archived_legacy.notifications[0].source.thread_id.as_str(),
            LEGACY_NO_THREAD_ARCHIVED_COMPAT_ID,
            "the base shape must retain a durable distinction for a real user archive",
        );
        let archived_legacy_mutation =
            serde_json::to_vec(&archived_legacy).expect("base writer persists real archive");
        let archived_reopened = decode_snapshot(&archived_legacy_mutation)
            .expect("head reader reopens base real archive mutation");
        assert_eq!(
            archived_reopened.notifications[0].archived_at,
            Some(archived_at)
        );
    }

    #[test]
    fn schema_v1_reader_preserves_a_historical_thread_that_matches_the_compatibility_id() {
        let recipient = NotificationRecipient {
            tenant_id: TenantId::new("historical-tenant").expect("tenant"),
            user_id: UserId::new("historical-user").expect("user"),
        };
        let thread_id = ThreadId::new(LEGACY_NO_THREAD_COMPAT_ID).expect("thread id");
        let occurred_at = Utc
            .timestamp_opt(1_700_000_001, 0)
            .single()
            .expect("occurred at");
        let legacy = LegacyNotificationInboxSnapshotV1 {
            schema_version: NOTIFICATION_INBOX_SCHEMA_VERSION,
            recipient: recipient.clone(),
            notifications: vec![LegacyNotificationRecordV1 {
                id: NotificationId::new("historical-sentinel-thread").expect("notification id"),
                recipient,
                kind: NotificationKind::ApprovalRequired,
                severity: NotificationSeverity::Warning,
                source: LegacyNotificationSourceV1 {
                    thread_id: thread_id.clone(),
                    turn_run_id: None,
                    lifecycle_ref: None,
                },
                action: LegacyNotificationActionV1::OpenThread {
                    thread_id: thread_id.clone(),
                },
                created_at: occurred_at,
                updated_at: occurred_at,
                read_at: None,
                resolved_at: None,
                archived_at: None,
            }],
        };

        let bytes = serde_json::to_vec(&legacy).expect("encode legacy snapshot");
        let reopened = decode_snapshot(&bytes).expect("decode historical snapshot");
        validate_snapshot(&reopened, &reopened.recipient)
            .expect("historical sentinel thread remains a valid readable snapshot");
        let record = &reopened.notifications[0];
        assert_eq!(record.source.thread_id.as_ref(), Some(&thread_id));
        assert_eq!(record.action, NotificationAction::OpenThread { thread_id });
    }

    #[tokio::test]
    async fn publish_rejects_reserved_compatibility_threads_without_persisting() {
        let recipient = NotificationRecipient {
            tenant_id: TenantId::new("reserved-tenant").expect("tenant"),
            user_id: UserId::new("reserved-user").expect("user"),
        };
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/notifications").expect("alias"),
            VirtualPath::new("/engine/tenants/reserved/users/reserved/notifications")
                .expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        ));
        let store = NotificationInboxStore::new(filesystem, NOTIFICATION_INBOX_MAX_RECORDS);

        for (index, compatibility_id) in [
            LEGACY_NO_THREAD_COMPAT_ID,
            LEGACY_NO_THREAD_ARCHIVED_COMPAT_ID,
        ]
        .into_iter()
        .enumerate()
        {
            let thread_id = ThreadId::new(compatibility_id).expect("thread id");
            let request = PublishNotificationRequest {
                id: NotificationId::new(format!("reserved-compatibility-{index}"))
                    .expect("notification id"),
                recipient: recipient.clone(),
                kind: NotificationKind::RunFailed,
                severity: NotificationSeverity::Error,
                source: NotificationSource {
                    thread_id: Some(thread_id.clone()),
                    turn_run_id: None,
                    lifecycle_ref: None,
                    credential_providers: Vec::new(),
                },
                action: NotificationAction::OpenThread { thread_id },
                initial_state: crate::NotificationInitialState::Resolved,
                occurred_at: Utc
                    .timestamp_opt(1_700_000_100 + index as i64, 0)
                    .single()
                    .expect("occurred at"),
            };

            let error = store
                .publish(request)
                .await
                .expect_err("publish must reject a reserved compatibility identity");
            assert!(matches!(
                error,
                NotificationInboxError::InvalidRequest { .. }
            ));
        }

        let page = store
            .list(ListNotificationsRequest {
                recipient,
                limit: 10,
                cursor: None,
                include_archived: true,
            })
            .await
            .expect("list after rejected publishes");
        assert!(page.notifications.is_empty());
    }
}
