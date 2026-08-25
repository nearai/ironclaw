use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, FilesystemError, IndexKey, IndexValue,
    RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{path::ScopedPath, resource::ResourceScope};
use serde::{Deserialize, Serialize};

use crate::{
    ListNotificationsRequest, MarkAllNotificationsReadRequest, NOTIFICATION_INBOX_MAX_RECORDS,
    NOTIFICATION_PAGE_LIMIT_MAX, NotificationAction, NotificationId, NotificationInboxError,
    NotificationInboxStorePort, NotificationMutationOutcome, NotificationMutationRequest,
    NotificationPage, NotificationRecipient, NotificationRecord, NotificationSource,
    PublishNotificationRequest,
};

const NOTIFICATION_INBOX_PATH: &str = "/notifications/inbox.json";
const NOTIFICATION_INBOX_SCHEMA_VERSION: u8 = 1;
const TENANT_ID_INDEX_KEY: &str = "tenant_id";
const NOTIFICATION_CURSOR_MAX_BYTES: usize = (20 + 1 + crate::types::NOTIFICATION_ID_MAX_BYTES) * 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NotificationInboxSnapshot {
    schema_version: u8,
    recipient: NotificationRecipient,
    notifications: Vec<NotificationRecord>,
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
        validate_notification_action(&request.source, &request.action)?;
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
                        if existing.recipient != request.recipient
                            || existing.kind != request.kind
                            || existing.severity != request.severity
                            || existing.source != request.source
                            || existing.action != request.action
                        {
                            return Err(NotificationInboxError::InvalidRequest {
                                reason: "notification id conflicts with an existing event",
                            });
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
    match action {
        NotificationAction::OpenThread { thread_id } if thread_id != &source.thread_id => {
            Err(NotificationInboxError::InvalidRequest {
                reason: "notification action does not match its source thread",
            })
        }
        NotificationAction::OpenThread { .. } => Ok(()),
    }
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
    serde_json::from_slice(bytes).map_err(|error| NotificationInboxError::Serialization {
        reason: error.to_string(),
    })
}

fn encode_snapshot(snapshot: &NotificationInboxSnapshot) -> Result<Entry, NotificationInboxError> {
    validate_snapshot(snapshot, &snapshot.recipient)?;
    let body =
        serde_json::to_vec(snapshot).map_err(|error| NotificationInboxError::Serialization {
            reason: error.to_string(),
        })?;
    Ok(Entry::bytes(body)
        .with_content_type(ContentType::json())
        .with_indexed(
            tenant_id_index_key()?,
            tenant_id_index_value(&snapshot.recipient),
        ))
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
