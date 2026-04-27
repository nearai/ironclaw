//! Runtime event and audit-history sinks for IronClaw Reborn.
//!
//! `ironclaw_events` defines the small event vocabulary used by the first live
//! Reborn slice. Events carry typed scope and capability metadata, never raw
//! host paths or raw secrets. The in-memory sink supports tests/live progress;
//! the JSONL sink demonstrates durable history through the Reborn filesystem
//! contract.

use std::sync::{Arc, Mutex};

use tokio::sync::Mutex as AsyncMutex;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    AuditEnvelope, CapabilityId, ExtensionId, ProcessId, ResourceScope, RuntimeKind, Timestamp,
    VirtualPath,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;
use uuid::Uuid;

/// Runtime event identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeEventId(Uuid);

impl RuntimeEventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for RuntimeEventId {
    fn default() -> Self {
        Self::new()
    }
}

/// Event kinds emitted by the composition/runtime path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEventKind {
    DispatchRequested,
    RuntimeSelected,
    DispatchSucceeded,
    DispatchFailed,
    ProcessStarted,
    ProcessCompleted,
    ProcessFailed,
    ProcessKilled,
}

/// Redacted runtime event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub event_id: RuntimeEventId,
    pub timestamp: Timestamp,
    pub kind: RuntimeEventKind,
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
}

impl RuntimeEvent {
    pub fn dispatch_requested(scope: ResourceScope, capability_id: CapabilityId) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::DispatchRequested,
            scope,
            capability_id,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn runtime_selected(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::RuntimeSelected,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: None,
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn dispatch_succeeded(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        output_bytes: u64,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::DispatchSucceeded,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: None,
            output_bytes: Some(output_bytes),
            error_kind: None,
        })
    }

    pub fn dispatch_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: Option<ExtensionId>,
        runtime: Option<RuntimeKind>,
        error_kind: impl Into<String>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::DispatchFailed,
            scope,
            capability_id,
            provider,
            runtime,
            process_id: None,
            output_bytes: None,
            error_kind: Some(sanitize_error_kind(error_kind)),
        })
    }

    pub fn process_started(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessStarted,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn process_completed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessCompleted,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: None,
        })
    }

    pub fn process_failed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
        error_kind: impl Into<String>,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessFailed,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: Some(sanitize_error_kind(error_kind)),
        })
    }

    pub fn process_killed(
        scope: ResourceScope,
        capability_id: CapabilityId,
        provider: ExtensionId,
        runtime: RuntimeKind,
        process_id: ProcessId,
    ) -> Self {
        Self::new(RuntimeEventPayload {
            kind: RuntimeEventKind::ProcessKilled,
            scope,
            capability_id,
            provider: Some(provider),
            runtime: Some(runtime),
            process_id: Some(process_id),
            output_bytes: None,
            error_kind: None,
        })
    }

    fn new(payload: RuntimeEventPayload) -> Self {
        Self {
            event_id: RuntimeEventId::new(),
            timestamp: Utc::now(),
            kind: payload.kind,
            scope: payload.scope,
            capability_id: payload.capability_id,
            provider: payload.provider,
            runtime: payload.runtime,
            process_id: payload.process_id,
            output_bytes: payload.output_bytes,
            error_kind: payload.error_kind,
        }
    }
}

fn sanitize_error_kind(error_kind: impl Into<String>) -> String {
    let error_kind = error_kind.into();
    let is_safe = !error_kind.is_empty()
        && error_kind.len() <= 128
        && error_kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'));
    if is_safe {
        error_kind
    } else {
        "Unclassified".to_string()
    }
}

struct RuntimeEventPayload {
    kind: RuntimeEventKind,
    scope: ResourceScope,
    capability_id: CapabilityId,
    provider: Option<ExtensionId>,
    runtime: Option<RuntimeKind>,
    process_id: Option<ProcessId>,
    output_bytes: Option<u64>,
    error_kind: Option<String>,
}

/// Event sink failures.
#[derive(Debug, Error)]
pub enum EventError {
    #[error("event serialization failed: {reason}")]
    Serialize { reason: String },
    #[error("invalid event log path: {reason}")]
    InvalidPath { reason: String },
    #[error("filesystem event sink failed: {0}")]
    Filesystem(Box<FilesystemError>),
    #[error("event sink failed: {reason}")]
    Sink { reason: String },
}

impl From<FilesystemError> for EventError {
    fn from(error: FilesystemError) -> Self {
        Self::Filesystem(Box::new(error))
    }
}

pub fn scoped_runtime_event_log_path(
    scope: &ResourceScope,
    file_name: &str,
) -> Result<VirtualPath, EventError> {
    scoped_jsonl_path(scope, "runtime", file_name)
}

pub fn scoped_audit_log_path(
    scope: &ResourceScope,
    file_name: &str,
) -> Result<VirtualPath, EventError> {
    scoped_jsonl_path(scope, "audit", file_name)
}

fn scoped_jsonl_path(
    scope: &ResourceScope,
    category: &str,
    file_name: &str,
) -> Result<VirtualPath, EventError> {
    if !is_safe_jsonl_file_name(file_name) {
        return Err(EventError::InvalidPath {
            reason: "log file name must be a simple .jsonl file name".to_string(),
        });
    }
    VirtualPath::new(format!(
        "{}/events/{}/{}",
        scoped_owner_root(scope),
        category,
        file_name
    ))
    .map_err(|error| EventError::InvalidPath {
        reason: error.to_string(),
    })
}

fn scoped_owner_root(scope: &ResourceScope) -> String {
    let base = format!(
        "/engine/tenants/{}/users/{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    );
    match &scope.agent_id {
        Some(agent_id) => format!("{base}/agents/{}", agent_id.as_str()),
        None => base,
    }
}

fn is_safe_jsonl_file_name(file_name: &str) -> bool {
    file_name.ends_with(".jsonl")
        && !file_name.starts_with('.')
        && file_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// Async event sink used by runtime/composition services.
#[async_trait]
pub trait EventSink: Send + Sync {
    /// Persists or forwards a metadata-only runtime/process event without changing runtime outcome policy.
    async fn emit(&self, event: RuntimeEvent) -> Result<(), EventError>;
}

/// Async audit sink used by control-plane services.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Persists or forwards a metadata-only control-plane audit record without leaking raw approval/runtime data.
    async fn emit_audit(&self, record: AuditEnvelope) -> Result<(), EventError>;
}

/// In-memory event sink used by tests and live demos.
#[derive(Debug, Clone, Default)]
pub struct InMemoryEventSink {
    events: Arc<Mutex<Vec<RuntimeEvent>>>,
}

impl InMemoryEventSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<RuntimeEvent> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }
}

#[async_trait]
impl EventSink for InMemoryEventSink {
    async fn emit(&self, event: RuntimeEvent) -> Result<(), EventError> {
        self.events
            .lock()
            .map_err(|_| EventError::Sink {
                reason: "in-memory event sink lock poisoned".to_string(),
            })?
            .push(event);
        Ok(())
    }
}

/// In-memory audit sink used by tests and live demos.
#[derive(Debug, Clone, Default)]
pub struct InMemoryAuditSink {
    records: Arc<Mutex<Vec<AuditEnvelope>>>,
}

impl InMemoryAuditSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn records(&self) -> Vec<AuditEnvelope> {
        self.records
            .lock()
            .map(|records| records.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }
}

#[async_trait]
impl AuditSink for InMemoryAuditSink {
    async fn emit_audit(&self, record: AuditEnvelope) -> Result<(), EventError> {
        self.records
            .lock()
            .map_err(|_| EventError::Sink {
                reason: "in-memory audit sink lock poisoned".to_string(),
            })?
            .push(record);
        Ok(())
    }
}

/// Filesystem-backed JSONL event sink for durable runtime history.
#[derive(Debug, Clone)]
pub struct JsonlEventSink<F> {
    filesystem: Arc<F>,
    path: VirtualPath,
    lock: Arc<AsyncMutex<()>>,
}

impl<F> JsonlEventSink<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>, path: VirtualPath) -> Self {
        Self {
            filesystem,
            path,
            lock: Arc::new(AsyncMutex::new(())),
        }
    }

    pub fn filesystem(&self) -> Arc<F> {
        Arc::clone(&self.filesystem)
    }

    pub fn path(&self) -> &VirtualPath {
        &self.path
    }

    pub async fn read_events(&self) -> Result<Vec<RuntimeEvent>, EventError> {
        let bytes = match self.filesystem.read_file(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(EventError::from(error)),
        };
        parse_jsonl(&bytes)
    }
}

#[async_trait]
impl<F> EventSink for JsonlEventSink<F>
where
    F: RootFilesystem,
{
    async fn emit(&self, event: RuntimeEvent) -> Result<(), EventError> {
        let line = serde_json::to_vec(&event).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?;

        let _guard = self.lock.lock().await;

        let bytes = match self.filesystem.read_file(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => Vec::new(),
            Err(error) => return Err(EventError::from(error)),
        };
        validate_jsonl::<RuntimeEvent>(&bytes)?;
        let mut entry = line;
        entry.push(b'\n');
        self.filesystem.append_file(&self.path, &entry).await?;
        Ok(())
    }
}

/// Filesystem-backed JSONL audit sink for durable control-plane audit history.
#[derive(Debug, Clone)]
pub struct JsonlAuditSink<F> {
    filesystem: Arc<F>,
    path: VirtualPath,
    lock: Arc<AsyncMutex<()>>,
}

impl<F> JsonlAuditSink<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>, path: VirtualPath) -> Self {
        Self {
            filesystem,
            path,
            lock: Arc::new(AsyncMutex::new(())),
        }
    }

    pub fn filesystem(&self) -> Arc<F> {
        Arc::clone(&self.filesystem)
    }

    pub fn path(&self) -> &VirtualPath {
        &self.path
    }

    pub async fn read_records(&self) -> Result<Vec<AuditEnvelope>, EventError> {
        let bytes = match self.filesystem.read_file(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(EventError::from(error)),
        };
        parse_jsonl(&bytes)
    }
}

#[async_trait]
impl<F> AuditSink for JsonlAuditSink<F>
where
    F: RootFilesystem,
{
    async fn emit_audit(&self, record: AuditEnvelope) -> Result<(), EventError> {
        let line = serde_json::to_vec(&record).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?;

        let _guard = self.lock.lock().await;

        let bytes = match self.filesystem.read_file(&self.path).await {
            Ok(bytes) => bytes,
            Err(error) if is_not_found(&error) => Vec::new(),
            Err(error) => return Err(EventError::from(error)),
        };
        validate_jsonl::<AuditEnvelope>(&bytes)?;
        let mut entry = line;
        entry.push(b'\n');
        self.filesystem.append_file(&self.path, &entry).await?;
        Ok(())
    }
}

fn validate_jsonl<T>(bytes: &[u8]) -> Result<(), EventError>
where
    T: DeserializeOwned,
{
    parse_jsonl::<T>(bytes).map(|_| ())
}

fn parse_jsonl<T>(bytes: &[u8]) -> Result<Vec<T>, EventError>
where
    T: DeserializeOwned,
{
    let text = String::from_utf8(bytes.to_vec()).map_err(|error| EventError::Serialize {
        reason: error.to_string(),
    })?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<T>(line).map_err(|error| EventError::Serialize {
                reason: error.to_string(),
            })
        })
        .collect()
}

fn is_not_found(error: &FilesystemError) -> bool {
    match error {
        FilesystemError::Backend { reason, .. } => {
            let reason = reason.to_ascii_lowercase();
            reason.contains("no such file")
                || reason.contains("not found")
                || reason.contains("entity not found")
                || reason.contains("os error 2")
        }
        _ => false,
    }
}
