//! Policy-free transport adapter contracts for IronClaw Reborn.
//!
//! Transport adapters own protocol translation at the edge: channel/web/gateway
//! ingress becomes [`TransportIngress`], and runtime output becomes
//! [`TransportEgress`] for a named adapter. They do not own authorization,
//! approval resolution, prompt assembly, durable transcript state, or projection
//! source-of-truth semantics.

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use ironclaw_host_api::{ApprovalRequestId, CorrelationId, ExtensionId, ResourceScope, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Transport-owned metadata that must never override typed fields.
pub type TransportMetadata = BTreeMap<String, Value>;

type RegisteredAdapter = (TransportAdapterId, Arc<dyn TransportAdapter>);

fn validate_public_id(kind: &'static str, value: &str) -> Result<(), TransportError> {
    if value.is_empty() {
        return Err(TransportError::new(
            TransportErrorKind::InvalidRequest,
            format!("{kind} id must not be empty"),
        ));
    }
    if value.len() > 128 {
        return Err(TransportError::new(
            TransportErrorKind::InvalidRequest,
            format!("{kind} id must be at most 128 bytes"),
        ));
    }
    let first = value.as_bytes()[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(TransportError::new(
            TransportErrorKind::InvalidRequest,
            format!("{kind} id must start with lowercase ASCII or digit"),
        ));
    }
    if value == "." || value == ".." || value.contains("..") {
        return Err(TransportError::new(
            TransportErrorKind::InvalidRequest,
            format!("{kind} id must not contain dot-dot segments"),
        ));
    }
    if value.bytes().any(|byte| {
        !(byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || byte == b'_'
            || byte == b'-'
            || byte == b'.')
    }) {
        return Err(TransportError::new(
            TransportErrorKind::InvalidRequest,
            format!("{kind} id may only contain lowercase ASCII, digits, '_', '-', and '.'"),
        ));
    }
    if value.split('.').any(str::is_empty) {
        return Err(TransportError::new(
            TransportErrorKind::InvalidRequest,
            format!("{kind} id must not contain empty dot segments"),
        ));
    }
    Ok(())
}

fn validate_opaque_id(kind: &'static str, value: &str) -> Result<(), TransportError> {
    if value.is_empty() {
        return Err(TransportError::new(
            TransportErrorKind::InvalidRequest,
            format!("{kind} id must not be empty"),
        ));
    }
    if value.len() > 512 {
        return Err(TransportError::new(
            TransportErrorKind::InvalidRequest,
            format!("{kind} id must be at most 512 bytes"),
        ));
    }
    if value.chars().any(|ch| ch == '\0' || ch.is_control()) {
        return Err(TransportError::new(
            TransportErrorKind::InvalidRequest,
            format!("{kind} id must not contain NUL/control characters"),
        ));
    }
    Ok(())
}

/// Stable identifier for an adapter surface such as `gateway`, `slack`, or `tui`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransportAdapterId(String);

impl TransportAdapterId {
    pub fn new(value: impl Into<String>) -> Result<Self, TransportError> {
        let value = value.into();
        validate_public_id("transport adapter", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for TransportAdapterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Opaque protocol message id from the source transport.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransportMessageId(String);

impl TransportMessageId {
    pub fn new(value: impl Into<String>) -> Result<Self, TransportError> {
        let value = value.into();
        validate_opaque_id("transport message", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for TransportMessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Opaque transport-supplied thread identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransportThreadId(String);

impl TransportThreadId {
    pub fn new(value: impl Into<String>) -> Result<Self, TransportError> {
        let value = value.into();
        validate_opaque_id("transport thread", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for TransportThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Routing information that identifies the transport edge and host resource scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportRoute {
    pub adapter_id: TransportAdapterId,
    pub scope: ResourceScope,
    pub recipient: Option<String>,
    pub conversation_id: Option<String>,
    pub thread_id: Option<TransportThreadId>,
    #[serde(default, skip_serializing_if = "TransportMetadata::is_empty")]
    pub metadata: TransportMetadata,
}

/// Normalized inbound message for the Reborn kernel boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransportIngress {
    pub message_id: TransportMessageId,
    pub route: TransportRoute,
    pub message: TransportMessage,
    pub sender_display_name: Option<String>,
    pub timezone: Option<String>,
    pub received_at: Timestamp,
    #[serde(default, skip_serializing_if = "TransportMetadata::is_empty")]
    pub metadata: TransportMetadata,
}

/// User-authored transport message content plus attachment descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportMessage {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<TransportAttachment>,
}

/// Attachment category understood across transports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Audio,
    Document,
    Image,
    Video,
    Other,
}

/// Descriptor for attachment content, with optional inline bytes for legacy paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportAttachment {
    pub id: String,
    pub kind: AttachmentKind,
    pub mime_type: Option<String>,
    pub filename: Option<String>,
    pub size_bytes: Option<u64>,
    /// Inline payload bytes already accepted and size-limited by the adapter.
    ///
    /// Empty when the payload is staged out of band and referenced by
    /// `storage_ref` or `source_url`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<u8>,
    pub storage_ref: Option<String>,
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "TransportMetadata::is_empty")]
    pub metadata: TransportMetadata,
}

/// Kernel acknowledgement for accepted ingress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportSubmission {
    pub accepted_at: Timestamp,
    pub correlation_id: Option<CorrelationId>,
}

/// Normalized outbound work for a transport adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum TransportEgress {
    Reply(TransportReply),
    Status(TransportStatusUpdate),
    ApprovalPrompt(TransportApprovalPrompt),
    AuthPrompt(TransportAuthPrompt),
    ProjectionUpdate(TransportProjectionUpdate),
    Heartbeat(TransportHeartbeat),
}

impl TransportEgress {
    pub fn route(&self) -> Option<&TransportRoute> {
        match self {
            Self::Reply(reply) => Some(&reply.route),
            Self::Status(status) => Some(&status.route),
            Self::ApprovalPrompt(prompt) => Some(&prompt.route),
            Self::AuthPrompt(prompt) => Some(&prompt.route),
            Self::ProjectionUpdate(update) => Some(&update.route),
            Self::Heartbeat(heartbeat) => heartbeat.route.as_ref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportReply {
    pub route: TransportRoute,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<TransportAttachment>,
    #[serde(default, skip_serializing_if = "TransportMetadata::is_empty")]
    pub metadata: TransportMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportStatusUpdate {
    pub route: TransportRoute,
    pub status: TransportStatus,
    #[serde(default, skip_serializing_if = "TransportMetadata::is_empty")]
    pub metadata: TransportMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TransportStatus {
    Thinking {
        message: Option<String>,
    },
    StreamChunk {
        content: String,
    },
    ToolStarted {
        tool_name: String,
        call_id: Option<String>,
    },
    ToolCompleted {
        tool_name: String,
        call_id: Option<String>,
        success: bool,
        error_kind: Option<String>,
    },
    Generic {
        label: String,
        message: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportApprovalPrompt {
    pub route: TransportRoute,
    pub request_id: ApprovalRequestId,
    pub title: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "TransportMetadata::is_empty")]
    pub metadata: TransportMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportAuthPrompt {
    pub route: TransportRoute,
    pub extension_id: Option<ExtensionId>,
    pub credential_name: Option<String>,
    pub title: String,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "TransportMetadata::is_empty")]
    pub metadata: TransportMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransportProjectionUpdate {
    pub route: TransportRoute,
    pub cursor: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportHeartbeat {
    pub route: Option<TransportRoute>,
    pub cursor: Option<String>,
}

/// Adapter-level acknowledgement after transport delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportDeliveryAck {
    pub adapter_id: TransportAdapterId,
    pub delivered_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportHealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Health result for a single adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportHealth {
    pub status: TransportHealthStatus,
    pub checked_at: Timestamp,
    pub detail: Option<String>,
}

impl TransportHealth {
    pub fn healthy() -> Self {
        Self {
            status: TransportHealthStatus::Healthy,
            checked_at: chrono::Utc::now(),
            detail: None,
        }
    }
}

/// Sink implemented by the kernel side of the adapter boundary.
#[async_trait]
pub trait TransportIngressSink: Send + Sync {
    async fn submit_ingress(
        &self,
        ingress: TransportIngress,
    ) -> Result<TransportSubmission, TransportError>;
}

/// Protocol adapter for one inbound/outbound transport.
#[async_trait]
pub trait TransportAdapter: Send + Sync {
    fn adapter_id(&self) -> &TransportAdapterId;

    async fn start(&self, sink: Arc<dyn TransportIngressSink>) -> Result<(), TransportError>;

    async fn deliver(
        &self,
        egress: TransportEgress,
    ) -> Result<TransportDeliveryAck, TransportError>;

    async fn health_check(&self) -> Result<TransportHealth, TransportError>;

    async fn shutdown(&self) -> Result<(), TransportError> {
        Ok(())
    }
}

/// Registry for named transport adapters.
#[derive(Default)]
pub struct TransportRegistry {
    adapters: RwLock<HashMap<TransportAdapterId, Arc<dyn TransportAdapter>>>,
}

impl TransportRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, adapter: Arc<dyn TransportAdapter>) -> Result<(), TransportError> {
        let adapter_id = adapter.adapter_id().clone();
        let mut adapters = self.adapters.write().map_err(|_| {
            TransportError::new(
                TransportErrorKind::Internal,
                "transport registry lock poisoned",
            )
        })?;
        if adapters.contains_key(&adapter_id) {
            return Err(TransportError::new(
                TransportErrorKind::AdapterAlreadyExists,
                format!("transport adapter '{adapter_id}' is already registered"),
            ));
        }
        adapters.insert(adapter_id, adapter);
        Ok(())
    }

    pub fn replace(
        &self,
        adapter: Arc<dyn TransportAdapter>,
    ) -> Result<Option<Arc<dyn TransportAdapter>>, TransportError> {
        let adapter_id = adapter.adapter_id().clone();
        let mut adapters = self.adapters.write().map_err(|_| {
            TransportError::new(
                TransportErrorKind::Internal,
                "transport registry lock poisoned",
            )
        })?;
        Ok(adapters.insert(adapter_id, adapter))
    }

    pub fn unregister(
        &self,
        adapter_id: &TransportAdapterId,
    ) -> Result<Option<Arc<dyn TransportAdapter>>, TransportError> {
        let mut adapters = self.adapters.write().map_err(|_| {
            TransportError::new(
                TransportErrorKind::Internal,
                "transport registry lock poisoned",
            )
        })?;
        Ok(adapters.remove(adapter_id))
    }

    pub async fn start_all(
        &self,
        sink: Arc<dyn TransportIngressSink>,
    ) -> Result<(), TransportError> {
        let entries = self.snapshot_entries()?;
        if entries.is_empty() {
            return Err(TransportError::new(
                TransportErrorKind::StartupFailed,
                "no transport adapters registered",
            ));
        }

        let mut started = 0usize;
        let mut first_error = None;
        for (adapter_id, adapter) in entries {
            match adapter.start(sink.clone()).await {
                Ok(()) => started += 1,
                Err(error) => {
                    first_error.get_or_insert_with(|| {
                        TransportError::new(
                            error.kind(),
                            format!(
                                "transport adapter '{adapter_id}' failed to start: {}",
                                error.safe_reason()
                            ),
                        )
                    });
                }
            }
        }

        if started == 0 {
            return Err(first_error.unwrap_or_else(|| {
                TransportError::new(
                    TransportErrorKind::StartupFailed,
                    "no transport adapters started successfully",
                )
            }));
        }
        Ok(())
    }

    pub async fn deliver(
        &self,
        adapter_id: &TransportAdapterId,
        egress: TransportEgress,
    ) -> Result<TransportDeliveryAck, TransportError> {
        if let Some(route) = egress.route()
            && &route.adapter_id != adapter_id
        {
            return Err(TransportError::new(
                TransportErrorKind::InvalidRequest,
                "egress route adapter does not match requested adapter",
            ));
        }
        let adapter = self.get(adapter_id)?;
        adapter.deliver(egress).await
    }

    pub async fn health_check_all(
        &self,
    ) -> Result<BTreeMap<TransportAdapterId, TransportHealth>, TransportError> {
        let entries = self.snapshot_entries()?;
        let mut results = BTreeMap::new();
        for (adapter_id, adapter) in entries {
            results.insert(adapter_id, adapter.health_check().await?);
        }
        Ok(results)
    }

    pub async fn shutdown_all(&self) -> Result<(), TransportError> {
        let adapters = self.snapshot_adapters()?;
        for adapter in adapters {
            adapter.shutdown().await?;
        }
        Ok(())
    }

    pub fn adapter_ids(&self) -> Result<Vec<TransportAdapterId>, TransportError> {
        let adapters = self.adapters.read().map_err(|_| {
            TransportError::new(
                TransportErrorKind::Internal,
                "transport registry lock poisoned",
            )
        })?;
        Ok(adapters.keys().cloned().collect())
    }

    fn get(
        &self,
        adapter_id: &TransportAdapterId,
    ) -> Result<Arc<dyn TransportAdapter>, TransportError> {
        let adapters = self.adapters.read().map_err(|_| {
            TransportError::new(
                TransportErrorKind::Internal,
                "transport registry lock poisoned",
            )
        })?;
        adapters.get(adapter_id).cloned().ok_or_else(|| {
            TransportError::new(
                TransportErrorKind::AdapterNotFound,
                format!("transport adapter '{adapter_id}' is not registered"),
            )
        })
    }

    fn snapshot_adapters(&self) -> Result<Vec<Arc<dyn TransportAdapter>>, TransportError> {
        let adapters = self.adapters.read().map_err(|_| {
            TransportError::new(
                TransportErrorKind::Internal,
                "transport registry lock poisoned",
            )
        })?;
        Ok(adapters.values().cloned().collect())
    }

    fn snapshot_entries(&self) -> Result<Vec<RegisteredAdapter>, TransportError> {
        let adapters = self.adapters.read().map_err(|_| {
            TransportError::new(
                TransportErrorKind::Internal,
                "transport registry lock poisoned",
            )
        })?;
        Ok(adapters
            .iter()
            .map(|(adapter_id, adapter)| (adapter_id.clone(), adapter.clone()))
            .collect())
    }
}

/// Stable transport error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportErrorKind {
    AdapterAlreadyExists,
    AdapterNotFound,
    DeliveryFailed,
    Internal,
    InvalidRequest,
    StartupFailed,
    Timeout,
    Unauthorized,
    Unavailable,
    Unsupported,
}

impl TransportErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AdapterAlreadyExists => "adapter_already_exists",
            Self::AdapterNotFound => "adapter_not_found",
            Self::DeliveryFailed => "delivery_failed",
            Self::Internal => "internal",
            Self::InvalidRequest => "invalid_request",
            Self::StartupFailed => "startup_failed",
            Self::Timeout => "timeout",
            Self::Unauthorized => "unauthorized",
            Self::Unavailable => "unavailable",
            Self::Unsupported => "unsupported",
        }
    }
}

impl fmt::Display for TransportErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{kind}: {reason}")]
pub struct TransportError {
    kind: TransportErrorKind,
    reason: String,
}

impl TransportError {
    pub fn new(kind: TransportErrorKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: sanitize_reason(reason.into()),
        }
    }

    pub fn kind(&self) -> TransportErrorKind {
        self.kind
    }

    pub fn safe_reason(&self) -> &str {
        &self.reason
    }
}

fn sanitize_reason(reason: String) -> String {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        return "unspecified".to_string();
    }

    let lower = trimmed.to_ascii_lowercase();
    let sensitive_markers = [
        "authorization",
        "bearer ",
        "password",
        "secret",
        "token",
        "api_key",
        "apikey",
        "sk-",
        "/users/",
        "\\users\\",
        ".ironclaw",
    ];
    if sensitive_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return "redacted".to_string();
    }

    let mut safe = trimmed.replace(['\n', '\r', '\t'], " ");
    if safe.len() > 256 {
        safe = safe.chars().take(256).collect();
    }
    safe
}
