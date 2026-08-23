//! Authenticated user notification inbox wire contracts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::descriptors::{ProductSurfaceCommandDescriptor, ProductView};

pub const NOTIFICATIONS_VIEW: ProductView<
    ProductListNotificationsRequest,
    ProductListNotificationsResponse,
> = ProductView::paginated("notifications");

pub const NOTIFICATIONS_MARK_READ_COMMAND_ID: &str = "notifications.mark_read";
pub const NOTIFICATIONS_MARK_READ_COMMAND: ProductSurfaceCommandDescriptor<
    ProductNotificationMutationRequest,
    ProductNotificationMutationResponse,
> = ProductSurfaceCommandDescriptor::new(NOTIFICATIONS_MARK_READ_COMMAND_ID);

pub const NOTIFICATIONS_MARK_ALL_READ_COMMAND_ID: &str = "notifications.mark_all_read";
pub const NOTIFICATIONS_MARK_ALL_READ_COMMAND: ProductSurfaceCommandDescriptor<
    ProductMarkAllNotificationsReadRequest,
    ProductNotificationMutationResponse,
> = ProductSurfaceCommandDescriptor::new(NOTIFICATIONS_MARK_ALL_READ_COMMAND_ID);

pub const NOTIFICATIONS_ARCHIVE_COMMAND_ID: &str = "notifications.archive";
pub const NOTIFICATIONS_ARCHIVE_COMMAND: ProductSurfaceCommandDescriptor<
    ProductNotificationMutationRequest,
    ProductNotificationMutationResponse,
> = ProductSurfaceCommandDescriptor::new(NOTIFICATIONS_ARCHIVE_COMMAND_ID);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductNotificationKind {
    ApprovalRequired,
    AuthenticationRequired,
    RunBlocked,
    RunFailed,
    RunCompleted,
    DeliveryFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductNotificationSeverity {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProductNotificationAction {
    OpenThread { thread_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductNotification {
    pub id: String,
    pub kind: ProductNotificationKind,
    pub severity: ProductNotificationSeverity,
    pub action: ProductNotificationAction,
    pub thread_id: String,
    pub turn_run_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub read_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductListNotificationsRequest {
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductListNotificationsResponse {
    pub notifications: Vec<ProductNotification>,
    pub next_cursor: Option<String>,
    pub unread_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductNotificationMutationRequest {
    pub notification_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductMarkAllNotificationsReadRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductNotificationMutationResponse {
    pub updated: bool,
}
