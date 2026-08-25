//! Durable, metadata-only user notification Inbox records and storage.

mod error;
mod store;
mod types;

pub use error::NotificationInboxError;
pub use store::NotificationInboxStore;
pub use types::{
    LifecycleRef, ListNotificationsRequest, MarkAllNotificationsReadRequest,
    NOTIFICATION_INBOX_MAX_RECORDS, NOTIFICATION_PAGE_LIMIT_MAX, NoopNotificationInboxStore,
    NotificationAction, NotificationId, NotificationInboxStorePort, NotificationInitialState,
    NotificationKind, NotificationMutationOutcome, NotificationMutationRequest, NotificationPage,
    NotificationRecipient, NotificationRecord, NotificationSeverity, NotificationSource,
    PublishNotificationRequest,
};
