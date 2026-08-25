use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    ids::{TenantId, ThreadId, UserId},
    turn::TurnRunId,
};
use serde::{Deserialize, Serialize};

use crate::NotificationInboxError;

pub const NOTIFICATION_PAGE_LIMIT_MAX: usize = 100;
pub const NOTIFICATION_INBOX_MAX_RECORDS: usize = 1_000;
pub(crate) const NOTIFICATION_ID_MAX_BYTES: usize = 256;
const LIFECYCLE_REF_MAX_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct NotificationId(String);

impl NotificationId {
    fn validate(value: &str) -> Result<(), NotificationInboxError> {
        if value.is_empty()
            || value.len() > NOTIFICATION_ID_MAX_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(NotificationInboxError::InvalidRequest {
                reason: "notification id is invalid",
            });
        }
        Ok(())
    }

    pub fn new(value: impl Into<String>) -> Result<Self, NotificationInboxError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for NotificationId {
    type Error = NotificationInboxError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}

impl AsRef<str> for NotificationId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LifecycleRef(String);

impl LifecycleRef {
    fn validate(value: &str) -> Result<(), NotificationInboxError> {
        if value.is_empty()
            || value.len() > LIFECYCLE_REF_MAX_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(NotificationInboxError::InvalidRequest {
                reason: "notification lifecycle reference is invalid",
            });
        }
        Ok(())
    }

    pub fn new(value: impl Into<String>) -> Result<Self, NotificationInboxError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for LifecycleRef {
    type Error = NotificationInboxError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<LifecycleRef> for String {
    fn from(value: LifecycleRef) -> Self {
        value.into_inner()
    }
}

impl AsRef<str> for LifecycleRef {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationRecipient {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    ApprovalRequired,
    AuthenticationRequired,
    RunBlocked,
    RunFailed,
    RunCompleted,
    DeliveryFailed,
}

impl NotificationKind {
    /// Stable producer identity segment used by durable notification ids.
    ///
    /// These values are persistence vocabulary, not display copy. Changing a
    /// key would bypass idempotent replay and requires an explicit migration.
    pub fn stable_key(self) -> &'static str {
        match self {
            Self::ApprovalRequired => "approval",
            Self::AuthenticationRequired => "authentication",
            Self::RunBlocked => "blocked",
            Self::RunFailed => "failed",
            Self::RunCompleted => "completed",
            Self::DeliveryFailed => "delivery-failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity {
    Info,
    Success,
    Warning,
    Error,
}

/// Producer-selected lifecycle state at the moment a notification is created.
///
/// Terminal facts are born resolved; actionable notifications remain open
/// until their originating workflow settles them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationInitialState {
    Open,
    Resolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationAction {
    OpenThread { thread_id: ThreadId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSource {
    pub thread_id: ThreadId,
    pub turn_run_id: Option<TurnRunId>,
    pub lifecycle_ref: Option<LifecycleRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationRecord {
    pub id: NotificationId,
    pub recipient: NotificationRecipient,
    pub kind: NotificationKind,
    pub severity: NotificationSeverity,
    pub source: NotificationSource,
    pub action: NotificationAction,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishNotificationRequest {
    pub id: NotificationId,
    pub recipient: NotificationRecipient,
    pub kind: NotificationKind,
    pub severity: NotificationSeverity,
    pub source: NotificationSource,
    pub action: NotificationAction,
    pub initial_state: NotificationInitialState,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListNotificationsRequest {
    pub recipient: NotificationRecipient,
    pub limit: usize,
    pub cursor: Option<String>,
    pub include_archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationPage {
    pub notifications: Vec<NotificationRecord>,
    pub next_cursor: Option<String>,
    pub unread_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationMutationRequest {
    pub recipient: NotificationRecipient,
    pub notification_id: NotificationId,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkAllNotificationsReadRequest {
    pub recipient: NotificationRecipient,
    pub occurred_at: DateTime<Utc>,
}

/// Whether a mutation actually changed durable state. A repeated mark-read,
/// resolve, or archive succeeds without changing anything, and a caller that
/// reports success as a change would be inventing evidence the store never
/// produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationMutationOutcome {
    Applied,
    AlreadySettled,
}

impl NotificationMutationOutcome {
    pub fn applied(self) -> bool {
        matches!(self, Self::Applied)
    }
}

#[async_trait]
pub trait NotificationInboxStorePort: Send + Sync {
    async fn publish(
        &self,
        request: PublishNotificationRequest,
    ) -> Result<NotificationRecord, NotificationInboxError>;

    async fn list(
        &self,
        request: ListNotificationsRequest,
    ) -> Result<NotificationPage, NotificationInboxError>;

    async fn mark_read(
        &self,
        request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError>;

    async fn mark_all_read(
        &self,
        request: MarkAllNotificationsReadRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError>;

    async fn resolve(
        &self,
        request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError>;

    async fn archive(
        &self,
        request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError>;
}

#[derive(Debug, Default)]
pub struct NoopNotificationInboxStore;

#[async_trait]
impl NotificationInboxStorePort for NoopNotificationInboxStore {
    async fn publish(
        &self,
        _request: PublishNotificationRequest,
    ) -> Result<NotificationRecord, NotificationInboxError> {
        Err(notification_store_unavailable())
    }

    async fn list(
        &self,
        _request: ListNotificationsRequest,
    ) -> Result<NotificationPage, NotificationInboxError> {
        Err(notification_store_unavailable())
    }

    async fn mark_read(
        &self,
        _request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
        Err(notification_store_unavailable())
    }

    async fn mark_all_read(
        &self,
        _request: MarkAllNotificationsReadRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
        Err(notification_store_unavailable())
    }

    async fn resolve(
        &self,
        _request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
        Err(notification_store_unavailable())
    }

    async fn archive(
        &self,
        _request: NotificationMutationRequest,
    ) -> Result<NotificationMutationOutcome, NotificationInboxError> {
        Err(notification_store_unavailable())
    }
}

fn notification_store_unavailable() -> NotificationInboxError {
    NotificationInboxError::Backend {
        reason: "notification inbox store is not configured".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_id_uses_the_validated_newtype_contract() {
        let id = NotificationId::try_from("notification-1".to_string()).expect("valid id");
        assert_eq!(id.as_str(), "notification-1");
        assert_eq!(id.as_ref(), "notification-1");
        assert_eq!(id.into_inner(), "notification-1");
        assert!(NotificationId::try_from(String::new()).is_err());
        assert!(serde_json::from_str::<NotificationId>("\"bad\\nid\"").is_err());
    }

    #[test]
    fn lifecycle_ref_uses_the_validated_newtype_contract() {
        let lifecycle_ref = LifecycleRef::new("gate-1").expect("valid lifecycle ref");
        assert_eq!(lifecycle_ref.as_str(), "gate-1");
        assert_eq!(lifecycle_ref.as_ref(), "gate-1");
        assert_eq!(lifecycle_ref.into_inner(), "gate-1");
        assert!(LifecycleRef::try_from(String::new()).is_err());
        assert!(serde_json::from_str::<LifecycleRef>("\"bad\\nref\"").is_err());
        assert!(LifecycleRef::new("x".repeat(LIFECYCLE_REF_MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn notification_kind_stable_keys_are_unique_and_rollout_safe() {
        let keys = [
            (NotificationKind::ApprovalRequired, "approval"),
            (NotificationKind::AuthenticationRequired, "authentication"),
            (NotificationKind::RunBlocked, "blocked"),
            (NotificationKind::RunFailed, "failed"),
            (NotificationKind::RunCompleted, "completed"),
            (NotificationKind::DeliveryFailed, "delivery-failed"),
        ];
        let unique = keys
            .iter()
            .map(|(kind, expected)| {
                assert_eq!(kind.stable_key(), *expected);
                kind.stable_key()
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), keys.len());
    }
}
