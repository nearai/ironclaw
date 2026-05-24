//! Product-facing projections over Reborn durable runtime and audit logs.
//!
//! This crate is a read-model boundary. Upper Reborn layers should consume
//! these DTOs instead of parsing durable event/audit rows directly. The first
//! implementation is replay-derived over [`ironclaw_events::DurableEventLog`]
//! so it stays independent of concrete JSONL/PostgreSQL/libSQL adapters.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_events::{
    AuditSink, DurableAuditLog, DurableEventLog, EventCursor, EventError, EventLogEntry,
    EventStreamKey, ReadScope, RuntimeEvent, RuntimeEventKind, UNCLASSIFIED_ERROR_KIND,
    sanitize_error_kind,
};
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AgentId, ApprovalRequestId, AuditEnvelope, AuditEventId,
    AuditStage, CapabilityId, CorrelationId, DecisionSummary, EffectKind, ExtensionId,
    InvocationId, OBLIGATION_EVALUATION_ORDER, ObligationKind, ProcessId, ProjectId, ResourceScope,
    RuntimeKind, TenantId, ThreadId, Timestamp, UserId,
};
use ironclaw_memory::{
    MemoryAuditContext, MemoryDocumentScope, MemoryEventSinkError, MemorySignificantEvent,
    MemorySignificantEventKind, MemorySignificantEventSink, PromptSafetyReasonCode,
    PromptWriteOperation, PromptWriteSafetyEvent, PromptWriteSafetyEventKind,
    PromptWriteSafetyEventSink,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod runtime_projection;
use runtime_projection::{
    RuntimeProjectionState, sort_capability_activities_for_projection, sort_runs_for_projection,
};

const STATE_REPLAY_PAGE_LIMIT: usize = 256;

/// Hard ceiling on how many runtime-prefix events `updates()` will fold while
/// reconstructing run state for touched invocations on a single call.
///
/// `updates()` does not collect the prefix into a `Vec`; it folds each page
/// incrementally so memory stays `O(touched_runs)` regardless of stream
/// length. This cap is a defense-in-depth against pathological streams (e.g.
/// a long-lived thread with millions of runtime events) where even paging
/// through the prefix would burn unbounded CPU on every poll. When the cap
/// is hit, the call surfaces [`ProjectionError::RebaseRequired`] so the
/// caller knows it must re-snapshot rather than silently see a partial
/// run-state view.
const STATE_REPLAY_MAX_EVENTS: usize = 100_000;

const MEMORY_EVENTS_EXTENSION_ID: &str = "memory.events";
const MEMORY_PROMPT_SAFETY_EXTENSION_ID: &str = "memory.prompt_safety";

/// Maximum page size accepted by the projection service.
///
/// `ProjectionRequest.limit` is reserved for product adapters; a caller-
/// controlled limit must not be allowed to force the durable log to scan
/// or return an arbitrarily large page. Requests above this bound are
/// rejected with [`ProjectionError::InvalidRequest`] before any read.
pub const MAX_PROJECTION_PAGE_LIMIT: usize = 1_000;

/// Scoped projection request authority.
///
/// The stream key selects the durable `(tenant, user, agent)` partition. The
/// read scope tightens access within that partition so product callers cannot
/// observe neighboring project/thread/process records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionScope {
    pub stream: EventStreamKey,
    pub read_scope: ReadScope,
}

impl ProjectionScope {
    pub fn from_resource_scope(scope: &ResourceScope) -> Self {
        Self {
            stream: EventStreamKey::from_scope(scope),
            read_scope: ReadScope {
                project_id: scope.project_id.clone(),
                mission_id: scope.mission_id.clone(),
                thread_id: scope.thread_id.clone(),
                process_id: None,
            },
        }
    }

    pub fn for_process(scope: &ResourceScope, process_id: ProcessId) -> Self {
        Self {
            stream: EventStreamKey::from_scope(scope),
            read_scope: ReadScope {
                project_id: scope.project_id.clone(),
                mission_id: scope.mission_id.clone(),
                thread_id: scope.thread_id.clone(),
                process_id: Some(process_id),
            },
        }
    }
}

/// Cursor envelope for projection consumers.
///
/// This first slice is runtime-event backed. The wrapper keeps callers from
/// treating raw durable cursors as a stable product API and leaves room for
/// audit/materialized checkpoints later.
///
/// Cursors are **scope-bound**: every cursor carries the
/// [`ProjectionScope`] under which it was minted. The durable stream is
/// partitioned by `(tenant, user, agent)` while project / mission /
/// thread / process filtering happens inside the read filter, so a cursor
/// returned for thread B may have a runtime value that lies inside the
/// shared stream of thread A. Replaying it under thread A's scope without
/// scope-matching would silently skip thread A's earlier events. Resume
/// rejects mismatched-scope cursors with
/// [`ProjectionError::RebaseRequired`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProjectionCursor {
    pub runtime: EventCursor,
    pub scope: ProjectionScope,
}

impl ProjectionCursor {
    /// Construct a cursor bound to `scope` at the given runtime position.
    ///
    /// Production callers should let the service mint cursors via
    /// [`EventProjectionService::snapshot`] / [`EventProjectionService::updates`]
    /// and pass them straight back into the next request. Direct construction
    /// is provided for tests and adapters that already hold authority for
    /// the scope they pass in.
    pub fn for_scope(scope: ProjectionScope, runtime: EventCursor) -> Self {
        Self { runtime, scope }
    }

    /// Cursor that precedes every record in `scope`.
    pub fn origin_for_scope(scope: ProjectionScope) -> Self {
        Self {
            runtime: EventCursor::origin(),
            scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionRequest {
    pub scope: ProjectionScope,
    pub after: Option<ProjectionCursor>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSnapshot {
    pub timeline: ThreadTimeline,
    pub runs: Vec<RunStatusProjection>,
    pub capability_activities: Vec<CapabilityActivityProjection>,
    pub next_cursor: ProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionReplay {
    pub updates: Vec<TimelineEntry>,
    pub runs: Vec<RunStatusProjection>,
    pub capability_activities: Vec<CapabilityActivityProjection>,
    pub next_cursor: ProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadTimeline {
    pub entries: Vec<TimelineEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub cursor: EventCursor,
    pub event_id: ironclaw_events::RuntimeEventId,
    pub timestamp: Timestamp,
    pub kind: TimelineEntryKind,
    pub invocation_id: InvocationId,
    pub thread_id: Option<ThreadId>,
    pub capability_id: CapabilityId,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
    /// Sanitized hook metadata. Populated only when `kind` is one of the
    /// `Hook*` variants — for other kinds these fields are `None`.
    /// Each field is a *closed-vocabulary* label (no free-form text), so
    /// replay consumers can pattern-match on the actual hook that
    /// fired/failed without burning audit budget on operator-supplied
    /// reason strings (henrypark133 Concerning #6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_point: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_trust_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_failure_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_failure_disposition: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineEntryKind {
    DispatchRequested,
    RuntimeSelected,
    DispatchSucceeded,
    DispatchFailed,
    ModelStarted,
    ModelCompleted,
    ModelFailed,
    AssistantReplyFinalized,
    LoopCompleted,
    LoopCancelled,
    LoopFailed,
    ProcessStarted,
    ProcessCompleted,
    ProcessFailed,
    ProcessKilled,
    HookDispatched,
    HookDecisionEmitted,
    HookFailed,
}

impl From<RuntimeEventKind> for TimelineEntryKind {
    fn from(kind: RuntimeEventKind) -> Self {
        match kind {
            RuntimeEventKind::DispatchRequested => Self::DispatchRequested,
            RuntimeEventKind::RuntimeSelected => Self::RuntimeSelected,
            RuntimeEventKind::DispatchSucceeded => Self::DispatchSucceeded,
            RuntimeEventKind::DispatchFailed => Self::DispatchFailed,
            RuntimeEventKind::ModelStarted => Self::ModelStarted,
            RuntimeEventKind::ModelCompleted => Self::ModelCompleted,
            RuntimeEventKind::ModelFailed => Self::ModelFailed,
            RuntimeEventKind::AssistantReplyFinalized => Self::AssistantReplyFinalized,
            RuntimeEventKind::LoopCompleted => Self::LoopCompleted,
            RuntimeEventKind::LoopCancelled => Self::LoopCancelled,
            RuntimeEventKind::LoopFailed => Self::LoopFailed,
            RuntimeEventKind::ProcessStarted => Self::ProcessStarted,
            RuntimeEventKind::ProcessCompleted => Self::ProcessCompleted,
            RuntimeEventKind::ProcessFailed => Self::ProcessFailed,
            RuntimeEventKind::ProcessKilled => Self::ProcessKilled,
            RuntimeEventKind::HookDispatched => Self::HookDispatched,
            RuntimeEventKind::HookDecisionEmitted => Self::HookDecisionEmitted,
            RuntimeEventKind::HookFailed => Self::HookFailed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStatusProjection {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub thread_id: Option<ThreadId>,
    pub status: RunProjectionStatus,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub error_kind: Option<String>,
    pub last_cursor: EventCursor,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunProjectionStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
    Killed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityActivityProjection {
    pub invocation_id: InvocationId,
    pub capability_id: CapabilityId,
    pub thread_id: Option<ThreadId>,
    pub status: CapabilityActivityStatus,
    pub provider: Option<ExtensionId>,
    pub runtime: Option<RuntimeKind>,
    pub process_id: Option<ProcessId>,
    pub output_bytes: Option<u64>,
    pub error_kind: Option<String>,
    pub last_cursor: EventCursor,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityActivityStatus {
    Started,
    Running,
    Completed,
    Failed,
    Killed,
}

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("projection request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error(
        "projection rebase required: requested runtime cursor {requested:?} cannot replay from earliest retained runtime cursor {earliest:?}"
    )]
    RebaseRequired {
        // Boxed because `ProjectionCursor` carries the full
        // `ProjectionScope` (stream + read scope) and inlining both
        // into the error variant balloons every `Result` size on the
        // happy path. Construction sites use `Box::new(..)`.
        requested: Box<ProjectionCursor>,
        earliest: Box<ProjectionCursor>,
    },
    #[error("projection source failed during {operation}")]
    Source { operation: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuditProjectionCursor {
    pub audit: EventCursor,
    pub scope: ProjectionScope,
}

impl AuditProjectionCursor {
    pub fn for_scope(scope: ProjectionScope, audit: EventCursor) -> Self {
        Self { audit, scope }
    }

    pub fn origin_for_scope(scope: ProjectionScope) -> Self {
        Self {
            audit: EventCursor::origin(),
            scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditProjectionRequest {
    pub scope: ProjectionScope,
    pub after: Option<AuditProjectionCursor>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditProjectionSnapshot {
    pub entries: Vec<AuditProjectionEntry>,
    pub next_cursor: AuditProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditProjectionReplay {
    pub entries: Vec<AuditProjectionEntry>,
    pub next_cursor: AuditProjectionCursor,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditProjectionEntry {
    pub cursor: EventCursor,
    pub event_id: AuditEventId,
    pub timestamp: Timestamp,
    pub stage: AuditProjectionStage,
    pub correlation_id: CorrelationId,
    pub invocation_id: InvocationId,
    pub thread_id: Option<ThreadId>,
    pub process_id: Option<ProcessId>,
    pub approval_request_id: Option<ApprovalRequestId>,
    pub extension_id: Option<ExtensionId>,
    pub action_kind: String,
    pub action_target: Option<String>,
    pub decision_kind: String,
    pub result_status: Option<String>,
    pub output_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryAuditProjectionMetadata>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryAuditProjectionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_path_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_text: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_path_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_count: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditProjectionStage {
    Before,
    After,
    Denied,
    ApprovalRequested,
    ApprovalResolved,
    ResourceReserved,
    ResourceReconciled,
    ResourceReleased,
}

impl From<AuditStage> for AuditProjectionStage {
    fn from(stage: AuditStage) -> Self {
        match stage {
            AuditStage::Before => Self::Before,
            AuditStage::After => Self::After,
            AuditStage::Denied => Self::Denied,
            AuditStage::ApprovalRequested => Self::ApprovalRequested,
            AuditStage::ApprovalResolved => Self::ApprovalResolved,
            AuditStage::ResourceReserved => Self::ResourceReserved,
            AuditStage::ResourceReconciled => Self::ResourceReconciled,
            AuditStage::ResourceReleased => Self::ResourceReleased,
        }
    }
}

#[derive(Debug, Error)]
pub enum AuditProjectionError {
    #[error("audit projection request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error(
        "audit projection rebase required: requested audit cursor {requested:?} cannot replay from earliest retained audit cursor {earliest:?}"
    )]
    RebaseRequired {
        requested: Box<AuditProjectionCursor>,
        earliest: Box<AuditProjectionCursor>,
    },
    #[error("audit projection source failed during {operation}")]
    Source { operation: &'static str },
}

/// Durable audit adapter for memory-owned redacted event seams.
#[derive(Clone)]
pub struct DurableMemoryAuditSink {
    audit: Arc<dyn AuditSink>,
}

impl DurableMemoryAuditSink {
    pub fn new(audit: Arc<dyn AuditSink>) -> Self {
        Self { audit }
    }
}

impl std::fmt::Debug for DurableMemoryAuditSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableMemoryAuditSink")
            .field("audit", &"<audit_sink>")
            .finish()
    }
}

#[async_trait]
impl MemorySignificantEventSink for DurableMemoryAuditSink {
    async fn record_memory_significant_event(
        &self,
        event: MemorySignificantEvent,
    ) -> Result<(), MemoryEventSinkError> {
        let record = memory_significant_audit(event)?;
        self.audit
            .emit_audit(record)
            .await
            .map_err(|error| memory_audit_error(error.to_string()))
    }
}

#[async_trait]
impl PromptWriteSafetyEventSink for DurableMemoryAuditSink {
    async fn record_prompt_write_safety_event(
        &self,
        event: PromptWriteSafetyEvent,
    ) -> Result<(), MemoryEventSinkError> {
        let record = prompt_write_safety_audit(event)?;
        self.audit
            .emit_audit(record)
            .await
            .map_err(|error| memory_audit_error(error.to_string()))
    }
}

fn memory_significant_audit(
    event: MemorySignificantEvent,
) -> Result<AuditEnvelope, MemoryEventSinkError> {
    let audit_context = resolve_memory_audit_context(&event.scope, event.audit_context.as_ref())?;
    let resource_scope = audit_context.resource_scope;
    Ok(AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: audit_context.correlation_id,
        stage: AuditStage::After,
        timestamp: Utc::now(),
        tenant_id: resource_scope.tenant_id,
        user_id: resource_scope.user_id,
        agent_id: resource_scope.agent_id,
        project_id: resource_scope.project_id,
        mission_id: resource_scope.mission_id,
        thread_id: resource_scope.thread_id,
        invocation_id: resource_scope.invocation_id,
        process_id: None,
        approval_request_id: None,
        extension_id: Some(memory_events_extension_id()?),
        action: ActionSummary {
            kind: memory_significant_action_kind(event.kind).to_string(),
            target: None,
            effects: memory_significant_effects(event.kind),
        },
        decision: DecisionSummary {
            kind: "memory_event_recorded".to_string(),
            reason: None,
            actor: None,
        },
        result: Some(ActionResultSummary {
            success: true,
            status: Some(encode_memory_significant_metadata(&event)),
            output_bytes: event.byte_count,
        }),
    })
}

fn prompt_write_safety_audit(
    event: PromptWriteSafetyEvent,
) -> Result<AuditEnvelope, MemoryEventSinkError> {
    let audit_context = resolve_memory_audit_context(&event.scope, event.audit_context.as_ref())?;
    let resource_scope = audit_context.resource_scope;
    Ok(AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: audit_context.correlation_id,
        stage: match event.kind {
            PromptWriteSafetyEventKind::Rejected => AuditStage::Denied,
            _ => AuditStage::After,
        },
        timestamp: Utc::now(),
        tenant_id: resource_scope.tenant_id,
        user_id: resource_scope.user_id,
        agent_id: resource_scope.agent_id,
        project_id: resource_scope.project_id,
        mission_id: resource_scope.mission_id,
        thread_id: resource_scope.thread_id,
        invocation_id: resource_scope.invocation_id,
        process_id: None,
        approval_request_id: None,
        extension_id: Some(memory_prompt_safety_extension_id()?),
        action: ActionSummary {
            kind: prompt_write_action_kind(event.operation).to_string(),
            target: None,
            effects: vec![EffectKind::WriteFilesystem],
        },
        decision: DecisionSummary {
            kind: event
                .reason_code
                .map(prompt_safety_reason_projection_kind)
                .unwrap_or_else(|| prompt_safety_event_kind_label(event.kind))
                .to_string(),
            reason: None,
            actor: None,
        },
        result: Some(prompt_safety_result(&event)),
    })
}

#[derive(Clone)]
struct ResolvedMemoryAuditContext {
    resource_scope: ResourceScope,
    correlation_id: CorrelationId,
}

fn resolve_memory_audit_context(
    scope: &MemoryDocumentScope,
    audit_context: Option<&MemoryAuditContext>,
) -> Result<ResolvedMemoryAuditContext, MemoryEventSinkError> {
    if let Some(audit_context) = audit_context {
        if !resource_scope_matches_memory_scope(&audit_context.resource_scope, scope) {
            return Err(memory_audit_error(
                "memory audit context scope does not match memory event scope",
            ));
        }
        return Ok(ResolvedMemoryAuditContext {
            resource_scope: audit_context.resource_scope.clone(),
            correlation_id: audit_context.correlation_id,
        });
    }

    Ok(ResolvedMemoryAuditContext {
        resource_scope: ResourceScope {
            tenant_id: TenantId::new(scope.tenant_id()).map_err(|error| {
                memory_audit_error(format!("invalid memory tenant id: {error}"))
            })?,
            user_id: UserId::new(scope.user_id())
                .map_err(|error| memory_audit_error(format!("invalid memory user id: {error}")))?,
            agent_id: scope
                .agent_id()
                .map(AgentId::new)
                .transpose()
                .map_err(|error| memory_audit_error(format!("invalid memory agent id: {error}")))?,
            project_id: scope
                .project_id()
                .map(ProjectId::new)
                .transpose()
                .map_err(|error| {
                    memory_audit_error(format!("invalid memory project id: {error}"))
                })?,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        correlation_id: CorrelationId::new(),
    })
}

fn resource_scope_matches_memory_scope(
    resource_scope: &ResourceScope,
    memory_scope: &MemoryDocumentScope,
) -> bool {
    resource_scope.tenant_id.as_str() == memory_scope.tenant_id()
        && resource_scope.user_id.as_str() == memory_scope.user_id()
        && resource_scope.agent_id.as_ref().map(AgentId::as_str) == memory_scope.agent_id()
        && resource_scope.project_id.as_ref().map(ProjectId::as_str) == memory_scope.project_id()
}

fn memory_significant_action_kind(kind: MemorySignificantEventKind) -> &'static str {
    match kind {
        MemorySignificantEventKind::DocumentWritten => "memory_document_written",
        MemorySignificantEventKind::DocumentDeleted => "memory_document_deleted",
        MemorySignificantEventKind::DocumentIndexed => "memory_document_indexed",
        MemorySignificantEventKind::SearchPerformed => "memory_search_performed",
        MemorySignificantEventKind::LayerRedirected => "memory_layer_redirected",
    }
}

fn memory_events_extension_id() -> Result<ExtensionId, MemoryEventSinkError> {
    ExtensionId::new(MEMORY_EVENTS_EXTENSION_ID)
        .map_err(|error| memory_audit_error(format!("invalid memory events extension id: {error}")))
}

fn memory_prompt_safety_extension_id() -> Result<ExtensionId, MemoryEventSinkError> {
    ExtensionId::new(MEMORY_PROMPT_SAFETY_EXTENSION_ID).map_err(|error| {
        memory_audit_error(format!(
            "invalid memory prompt-safety extension id: {error}"
        ))
    })
}

fn memory_significant_effects(kind: MemorySignificantEventKind) -> Vec<EffectKind> {
    match kind {
        MemorySignificantEventKind::DocumentWritten
        | MemorySignificantEventKind::DocumentIndexed => {
            vec![EffectKind::WriteFilesystem]
        }
        MemorySignificantEventKind::DocumentDeleted => vec![EffectKind::DeleteFilesystem],
        MemorySignificantEventKind::SearchPerformed => vec![EffectKind::ReadFilesystem],
        MemorySignificantEventKind::LayerRedirected => vec![EffectKind::WriteFilesystem],
    }
}

fn prompt_write_action_kind(operation: PromptWriteOperation) -> &'static str {
    match operation {
        PromptWriteOperation::Write => "write_file",
        PromptWriteOperation::Append => "append_file",
        PromptWriteOperation::Patch => "patch_file",
        PromptWriteOperation::Import => "memory_import",
        PromptWriteOperation::Seed => "memory_seed",
        PromptWriteOperation::ProfileUpdate => "profile_update",
        PromptWriteOperation::AdminSystemPromptUpdate => "admin_system_prompt_update",
    }
}

fn prompt_safety_reason_projection_kind(reason: PromptSafetyReasonCode) -> &'static str {
    match reason {
        PromptSafetyReasonCode::HighRiskPromptInjection => "prompt_high_risk",
        PromptSafetyReasonCode::CriticalPromptInjection => "prompt_critical",
        PromptSafetyReasonCode::PromptWritePolicyUnavailable => "prompt_policy_unavailable",
        PromptSafetyReasonCode::PromptWritePolicyMisconfigured => "prompt_policy_misconfigured",
        PromptSafetyReasonCode::ProtectedPathRegistryUnavailable => "protected_registry_missing",
        PromptSafetyReasonCode::PromptWriteBypassNotAllowed => "prompt_bypass_denied",
        PromptSafetyReasonCode::PromptWriteSafetyEventUnavailable => "prompt_event_unavailable",
    }
}

fn prompt_safety_event_kind_label(kind: PromptWriteSafetyEventKind) -> &'static str {
    match kind {
        PromptWriteSafetyEventKind::Checked => "prompt_write_safety_checked",
        PromptWriteSafetyEventKind::Warned => "prompt_write_safety_warned",
        PromptWriteSafetyEventKind::Rejected => "prompt_write_safety_rejected",
        PromptWriteSafetyEventKind::BypassAllowed => "prompt_write_safety_bypass_allowed",
    }
}

fn prompt_safety_metadata_status(kind: PromptWriteSafetyEventKind) -> &'static str {
    match kind {
        PromptWriteSafetyEventKind::Checked => "checked",
        PromptWriteSafetyEventKind::Warned => "warned",
        PromptWriteSafetyEventKind::Rejected => "rejected",
        PromptWriteSafetyEventKind::BypassAllowed => "bypass_allowed",
    }
}

fn prompt_safety_result(event: &PromptWriteSafetyEvent) -> ActionResultSummary {
    ActionResultSummary {
        success: event.kind != PromptWriteSafetyEventKind::Rejected,
        status: Some(encode_prompt_safety_metadata(event)),
        output_bytes: None,
    }
}

fn encode_memory_significant_metadata(event: &MemorySignificantEvent) -> String {
    let mut pairs = Vec::new();
    push_metadata_pair(&mut pairs, "status", event.status.as_str().to_string());
    if let Some(path_hash) = &event.relative_path_hash {
        push_metadata_pair(&mut pairs, "path_hash", path_hash.clone());
    }
    if let Some(chunk_count) = event.chunk_count {
        push_metadata_pair(&mut pairs, "chunks", chunk_count.to_string());
    }
    if let Some(result_count) = event.result_count {
        push_metadata_pair(&mut pairs, "results", result_count.to_string());
    }
    if let Some(full_text) = event.full_text {
        push_metadata_pair(&mut pairs, "full_text", full_text.to_string());
    }
    if let Some(vector) = event.vector {
        push_metadata_pair(&mut pairs, "vector", vector.to_string());
    }
    encode_metadata_pairs("memory_event:v1", pairs)
}

fn encode_prompt_safety_metadata(event: &PromptWriteSafetyEvent) -> String {
    let mut pairs = Vec::new();
    push_metadata_pair(
        &mut pairs,
        "status",
        prompt_safety_metadata_status(event.kind).to_string(),
    );
    if let Some(path_hash) = &event.relative_path_hash {
        push_metadata_pair(&mut pairs, "path_hash", path_hash.clone());
    }
    if let Some(path_class) = &event.protected_path_class {
        push_metadata_pair(
            &mut pairs,
            "protected_path_class",
            path_class.as_str().to_string(),
        );
    }
    if let Some(reason) = event.reason_code {
        push_metadata_pair(&mut pairs, "reason", reason.as_str().to_string());
    }
    if let Some(severity) = event.severity {
        push_metadata_pair(&mut pairs, "severity", severity.as_str().to_string());
    }
    if event.finding_count > 0 {
        push_metadata_pair(&mut pairs, "findings", event.finding_count.to_string());
    }
    encode_metadata_pairs("memory_prompt_safety:v1", pairs)
}

fn push_metadata_pair(pairs: &mut Vec<String>, key: &str, value: String) {
    pairs.push(format!("{key}={value}"));
}

fn encode_metadata_pairs(prefix: &str, pairs: Vec<String>) -> String {
    if pairs.is_empty() {
        return prefix.to_string();
    }
    format!("{prefix};{}", pairs.join(";"))
}

fn memory_audit_error(reason: impl Into<String>) -> MemoryEventSinkError {
    MemoryEventSinkError::new(reason)
}

#[async_trait]
pub trait AuditProjectionService: Send + Sync {
    async fn snapshot(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionSnapshot, AuditProjectionError>;

    async fn updates(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionReplay, AuditProjectionError>;
}

#[derive(Clone)]
pub struct ReplayAuditProjectionService {
    audit_log: Arc<dyn DurableAuditLog>,
}

impl ReplayAuditProjectionService {
    pub fn new<T>(audit_log: Arc<T>) -> Self
    where
        T: DurableAuditLog + 'static,
    {
        let audit_log: Arc<dyn DurableAuditLog> = audit_log;
        Self { audit_log }
    }

    pub fn from_audit_log(audit_log: Arc<dyn DurableAuditLog>) -> Self {
        Self { audit_log }
    }

    async fn read_audit(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<ProjectedAuditPage, AuditProjectionError> {
        if request.limit == 0 {
            return Err(AuditProjectionError::InvalidRequest {
                reason: "limit must be greater than zero",
            });
        }
        if request.limit > MAX_PROJECTION_PAGE_LIMIT {
            return Err(AuditProjectionError::InvalidRequest {
                reason: "limit exceeds MAX_PROJECTION_PAGE_LIMIT",
            });
        }
        if let Some(cursor) = request.after.as_ref()
            && cursor.scope != request.scope
        {
            return Err(AuditProjectionError::RebaseRequired {
                requested: Box::new(cursor.clone()),
                earliest: Box::new(AuditProjectionCursor::origin_for_scope(
                    request.scope.clone(),
                )),
            });
        }
        let fetch_limit =
            request
                .limit
                .checked_add(1)
                .ok_or(AuditProjectionError::InvalidRequest {
                    reason: "limit is too large",
                })?;
        let after = request.after.as_ref().map(|cursor| cursor.audit);
        let replay = self
            .audit_log
            .read_after_cursor(
                &request.scope.stream,
                &request.scope.read_scope,
                after,
                fetch_limit,
            )
            .await
            .map_err(|error| map_audit_projection_error(error, "audit replay", &request.scope))?;
        let mut entries = replay.entries;
        let truncated = entries.len() > request.limit;
        if truncated {
            entries.truncate(request.limit);
        }
        let next_cursor = if truncated {
            entries
                .last()
                .map(|entry| entry.cursor)
                .unwrap_or_else(|| after.unwrap_or_else(EventCursor::origin))
        } else {
            replay.next_cursor
        };
        Ok(ProjectedAuditPage {
            entries,
            next_cursor: AuditProjectionCursor::for_scope(request.scope.clone(), next_cursor),
            truncated,
        })
    }
}

impl std::fmt::Debug for ReplayAuditProjectionService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplayAuditProjectionService")
            .field("audit_log", &"<durable_audit_log>")
            .finish()
    }
}

#[async_trait]
impl AuditProjectionService for ReplayAuditProjectionService {
    async fn snapshot(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionSnapshot, AuditProjectionError> {
        let page = self.read_audit(request).await?;
        Ok(AuditProjectionSnapshot {
            entries: project_audit_entries(&page.entries),
            next_cursor: page.next_cursor,
            truncated: page.truncated,
        })
    }

    async fn updates(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionReplay, AuditProjectionError> {
        let page = self.read_audit(request).await?;
        Ok(AuditProjectionReplay {
            entries: project_audit_entries(&page.entries),
            next_cursor: page.next_cursor,
            truncated: page.truncated,
        })
    }
}

#[async_trait]
pub trait EventProjectionService: Send + Sync {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError>;

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStreamResume {
    Snapshot {
        snapshot: Box<ProjectionSnapshot>,
        rebased_from: Option<ProjectionCursor>,
        earliest_available: Option<ProjectionCursor>,
    },
    Updates(ProjectionReplay),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStreamResume {
    Snapshot {
        snapshot: Box<AuditProjectionSnapshot>,
        rebased_from: Option<AuditProjectionCursor>,
        earliest_available: Option<AuditProjectionCursor>,
    },
    Updates(AuditProjectionReplay),
}

/// Transport-agnostic facade for scoped Reborn projection replay.
///
/// Product transports should enter through this manager rather than reaching
/// directly into durable logs. The manager intentionally keeps runtime and
/// audit projections domain-specific: it validates and routes requests to the
/// owning projection services without flattening them into a generic event
/// union DTO.
#[derive(Clone)]
pub struct EventStreamManager {
    runtime_projection: Arc<dyn EventProjectionService>,
    audit_projection: Arc<dyn AuditProjectionService>,
}

impl EventStreamManager {
    pub fn new<R, A>(runtime_projection: Arc<R>, audit_projection: Arc<A>) -> Self
    where
        R: EventProjectionService + 'static,
        A: AuditProjectionService + 'static,
    {
        let runtime_projection: Arc<dyn EventProjectionService> = runtime_projection;
        let audit_projection: Arc<dyn AuditProjectionService> = audit_projection;
        Self {
            runtime_projection,
            audit_projection,
        }
    }

    pub fn from_services(
        runtime_projection: Arc<dyn EventProjectionService>,
        audit_projection: Arc<dyn AuditProjectionService>,
    ) -> Self {
        Self {
            runtime_projection,
            audit_projection,
        }
    }

    pub async fn runtime_snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        self.runtime_projection.snapshot(request).await
    }

    pub async fn runtime_updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        self.runtime_projection.updates(request).await
    }

    pub async fn runtime_resume(
        &self,
        request: ProjectionRequest,
    ) -> Result<RuntimeStreamResume, ProjectionError> {
        if request.after.is_none() {
            let snapshot = self.runtime_projection.snapshot(request).await?;
            return Ok(RuntimeStreamResume::Snapshot {
                snapshot: Box::new(snapshot),
                rebased_from: None,
                earliest_available: None,
            });
        }

        if let Some(cursor) = request.after.as_ref()
            && cursor.scope != request.scope
        {
            return Err(ProjectionError::RebaseRequired {
                requested: Box::new(cursor.clone()),
                earliest: Box::new(ProjectionCursor::origin_for_scope(request.scope)),
            });
        }

        let snapshot_request = ProjectionRequest {
            scope: request.scope.clone(),
            after: None,
            limit: request.limit,
        };
        match self.runtime_projection.updates(request).await {
            Ok(replay) => Ok(RuntimeStreamResume::Updates(replay)),
            Err(ProjectionError::RebaseRequired {
                requested,
                earliest,
            }) => {
                let snapshot = self.runtime_projection.snapshot(snapshot_request).await?;
                Ok(RuntimeStreamResume::Snapshot {
                    snapshot: Box::new(snapshot),
                    rebased_from: Some(*requested),
                    earliest_available: Some(*earliest),
                })
            }
            Err(error) => Err(error),
        }
    }

    pub async fn audit_snapshot(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionSnapshot, AuditProjectionError> {
        self.audit_projection.snapshot(request).await
    }

    pub async fn audit_updates(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditProjectionReplay, AuditProjectionError> {
        self.audit_projection.updates(request).await
    }

    pub async fn audit_resume(
        &self,
        request: AuditProjectionRequest,
    ) -> Result<AuditStreamResume, AuditProjectionError> {
        if request.after.is_none() {
            let snapshot = self.audit_projection.snapshot(request).await?;
            return Ok(AuditStreamResume::Snapshot {
                snapshot: Box::new(snapshot),
                rebased_from: None,
                earliest_available: None,
            });
        }

        if let Some(cursor) = request.after.as_ref()
            && cursor.scope != request.scope
        {
            return Err(AuditProjectionError::RebaseRequired {
                requested: Box::new(cursor.clone()),
                earliest: Box::new(AuditProjectionCursor::origin_for_scope(request.scope)),
            });
        }

        let snapshot_request = AuditProjectionRequest {
            scope: request.scope.clone(),
            after: None,
            limit: request.limit,
        };
        match self.audit_projection.updates(request).await {
            Ok(replay) => Ok(AuditStreamResume::Updates(replay)),
            Err(AuditProjectionError::RebaseRequired {
                requested,
                earliest,
            }) => {
                let snapshot = self.audit_projection.snapshot(snapshot_request).await?;
                Ok(AuditStreamResume::Snapshot {
                    snapshot: Box::new(snapshot),
                    rebased_from: Some(*requested),
                    earliest_available: Some(*earliest),
                })
            }
            Err(error) => Err(error),
        }
    }
}

impl std::fmt::Debug for EventStreamManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventStreamManager")
            .field("runtime_projection", &"<event_projection_service>")
            .field("audit_projection", &"<audit_projection_service>")
            .finish()
    }
}

#[derive(Clone)]
pub struct ReplayEventProjectionService {
    runtime_log: Arc<dyn DurableEventLog>,
}

impl ReplayEventProjectionService {
    pub fn new<T>(runtime_log: Arc<T>) -> Self
    where
        T: DurableEventLog + 'static,
    {
        let runtime_log: Arc<dyn DurableEventLog> = runtime_log;
        Self { runtime_log }
    }

    pub fn from_runtime_log(runtime_log: Arc<dyn DurableEventLog>) -> Self {
        Self { runtime_log }
    }

    async fn read_runtime(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectedRuntimePage, ProjectionError> {
        if request.limit == 0 {
            return Err(ProjectionError::InvalidRequest {
                reason: "limit must be greater than zero",
            });
        }
        if request.limit > MAX_PROJECTION_PAGE_LIMIT {
            return Err(ProjectionError::InvalidRequest {
                reason: "limit exceeds MAX_PROJECTION_PAGE_LIMIT",
            });
        }
        // Reject cursors that were minted under a different scope. The
        // durable stream is partitioned by `(tenant, user, agent)`, so a
        // sibling thread/project/process within the same stream can mint
        // a runtime cursor that the durable log accepts but that would
        // silently skip records the requested scope had not yet seen.
        // Force the consumer to rebase against a snapshot instead of
        // returning a partial replay.
        if let Some(cursor) = request.after.as_ref()
            && cursor.scope != request.scope
        {
            return Err(ProjectionError::RebaseRequired {
                requested: Box::new(cursor.clone()),
                earliest: Box::new(ProjectionCursor::origin_for_scope(request.scope.clone())),
            });
        }
        let fetch_limit = request
            .limit
            .checked_add(1)
            .ok_or(ProjectionError::InvalidRequest {
                reason: "limit is too large",
            })?;
        let after = request.after.as_ref().map(|cursor| cursor.runtime);
        let replay = self
            .runtime_log
            .read_after_cursor(
                &request.scope.stream,
                &request.scope.read_scope,
                after,
                fetch_limit,
            )
            .await
            .map_err(|error| {
                map_projection_error(error, after, "runtime replay", &request.scope)
            })?;
        let mut entries = replay.entries;
        let truncated = entries.len() > request.limit;
        if truncated {
            entries.truncate(request.limit);
        }
        let next_cursor = if truncated {
            entries
                .last()
                .map(|entry| entry.cursor)
                .unwrap_or_else(|| after.unwrap_or_else(EventCursor::origin))
        } else {
            replay.next_cursor
        };
        Ok(ProjectedRuntimePage {
            entries,
            next_cursor: ProjectionCursor::for_scope(request.scope.clone(), next_cursor),
            truncated,
        })
    }

    /// Fold the entire scoped runtime stream into the current run-state
    /// projection for every invocation visible under `scope`.
    ///
    /// `snapshot()` uses this so the `runs` projection always reflects the
    /// current scoped stream head, independent of how the timeline page was
    /// paginated. Without this, a `snapshot(limit=1)` whose page contains
    /// only `DispatchRequested` for a run that has already terminated would
    /// surface a `Running` `RunStatusProjection` while the terminal event
    /// sits unread on the next page — silently shipping stale run state to
    /// consumers that use snapshots to rebase after a replay gap.
    ///
    /// The same bounded-memory contract applies: pages are folded
    /// incrementally, allocation is `O(scoped runs)` regardless of stream
    /// length, and scanning more than [`STATE_REPLAY_MAX_EVENTS`] events
    /// surfaces [`ProjectionError::RebaseRequired`] instead of silently
    /// returning a partial run-state view.
    async fn fold_runtime_to_head(
        &self,
        scope: &ProjectionScope,
    ) -> Result<RuntimeProjectionState, ProjectionError> {
        let mut state = RuntimeProjectionState::default();
        let mut after: Option<EventCursor> = None;
        let mut scanned: usize = 0;
        loop {
            let replay = self
                .runtime_log
                .read_after_cursor(
                    &scope.stream,
                    &scope.read_scope,
                    after,
                    STATE_REPLAY_PAGE_LIMIT,
                )
                .await
                .map_err(|error| {
                    map_projection_error(error, after, "snapshot run-state replay", scope)
                })?;
            if replay.entries.is_empty() {
                break;
            }
            for entry in &replay.entries {
                scanned = scanned.saturating_add(1);
                if scanned > STATE_REPLAY_MAX_EVENTS {
                    return Err(ProjectionError::RebaseRequired {
                        requested: Box::new(ProjectionCursor::origin_for_scope(scope.clone())),
                        earliest: Box::new(ProjectionCursor::for_scope(
                            scope.clone(),
                            entry.cursor,
                        )),
                    });
                }
                state.apply(entry);
            }
            if after == Some(replay.next_cursor) {
                // The durable log made no progress — stream exhausted.
                break;
            }
            after = Some(replay.next_cursor);
        }
        Ok(state)
    }

    /// Fold the runtime-event prefix `(origin, until]` for `scope` into the
    /// run-state projection for the invocations identified by `touched`.
    ///
    /// This is the bounded-memory replacement for collecting the entire
    /// prefix into a `Vec`. The fold visits each page in sequence and only
    /// retains state for invocations the caller already saw in the current
    /// page, so allocation is `O(touched.len())` regardless of how many
    /// runtime events the stream has produced. A hard cap of
    /// [`STATE_REPLAY_MAX_EVENTS`] events scanned per call protects against
    /// pathological histories — when exceeded, the caller is told to rebase.
    async fn fold_runtime_prefix(
        &self,
        scope: &ProjectionScope,
        until: EventCursor,
        touched: &HashSet<InvocationId>,
    ) -> Result<RuntimeProjectionState, ProjectionError> {
        let mut state = RuntimeProjectionState::default();
        if touched.is_empty() || until == EventCursor::origin() {
            return Ok(state);
        }

        let mut after = None;
        let mut scanned: usize = 0;
        loop {
            let replay = self
                .runtime_log
                .read_after_cursor(
                    &scope.stream,
                    &scope.read_scope,
                    after,
                    STATE_REPLAY_PAGE_LIMIT,
                )
                .await
                .map_err(|error| {
                    map_projection_error(error, after, "runtime state replay", scope)
                })?;
            if replay.entries.is_empty() {
                break;
            }

            for entry in &replay.entries {
                if entry.cursor > until {
                    return Ok(state);
                }
                scanned = scanned.saturating_add(1);
                if scanned > STATE_REPLAY_MAX_EVENTS {
                    return Err(ProjectionError::RebaseRequired {
                        requested: Box::new(ProjectionCursor::for_scope(scope.clone(), until)),
                        earliest: Box::new(ProjectionCursor::for_scope(
                            scope.clone(),
                            entry.cursor,
                        )),
                    });
                }
                if touched.contains(&entry.record.scope.invocation_id) {
                    state.apply(entry);
                }
                if entry.cursor >= until {
                    return Ok(state);
                }
            }

            if replay.next_cursor >= until || after == Some(replay.next_cursor) {
                break;
            }
            after = Some(replay.next_cursor);
        }
        Ok(state)
    }
}

impl std::fmt::Debug for ReplayEventProjectionService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplayEventProjectionService")
            .field("runtime_log", &"<durable_event_log>")
            .finish()
    }
}

#[async_trait]
impl EventProjectionService for ReplayEventProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        let scope = request.scope.clone();
        let page = self.read_runtime(request).await?;
        let timeline = project_timeline(&page.entries);
        // Snapshot's `runs` always reflect the current scoped stream head,
        // not just the events present in `timeline`. A truncated timeline
        // page (or a `limit=1` request) would otherwise surface a stale
        // `Running` status for a run whose terminal event lives on the
        // next page — see PR #3212 review feedback (discussion_r3195454963).
        let folded = self.fold_runtime_to_head(&scope).await?;
        let (mut runs, mut capability_activities) = folded.into_parts();
        sort_runs_for_projection(&mut runs);
        sort_capability_activities_for_projection(&mut capability_activities);
        Ok(ProjectionSnapshot {
            timeline,
            runs,
            capability_activities,
            next_cursor: page.next_cursor,
            truncated: page.truncated,
        })
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        let scope = request.scope.clone();
        let page = self.read_runtime(request).await?;
        let touched_runs = page
            .entries
            .iter()
            .map(|entry| entry.record.scope.invocation_id)
            .collect::<HashSet<_>>();
        let (mut runs, mut capability_activities) = if touched_runs.is_empty() {
            (Vec::new(), Vec::new())
        } else {
            let folded = self
                .fold_runtime_prefix(&scope, page.next_cursor.runtime, &touched_runs)
                .await?;
            folded.into_parts()
        };
        sort_runs_for_projection(&mut runs);
        sort_capability_activities_for_projection(&mut capability_activities);
        Ok(ProjectionReplay {
            updates: project_timeline(&page.entries).entries,
            runs,
            capability_activities,
            next_cursor: page.next_cursor,
            truncated: page.truncated,
        })
    }
}

struct ProjectedAuditPage {
    entries: Vec<EventLogEntry<AuditEnvelope>>,
    next_cursor: AuditProjectionCursor,
    truncated: bool,
}

fn project_audit_entries(entries: &[EventLogEntry<AuditEnvelope>]) -> Vec<AuditProjectionEntry> {
    entries.iter().map(project_audit_entry).collect()
}

fn project_audit_entry(entry: &EventLogEntry<AuditEnvelope>) -> AuditProjectionEntry {
    let audit = &entry.record;
    let action_kind = sanitize_error_kind(audit.action.kind.clone());
    let output_bytes = audit.result.as_ref().and_then(|result| result.output_bytes);
    let memory = audit
        .result
        .as_ref()
        .and_then(|result| result.status.as_deref())
        .and_then(|status| parse_memory_audit_metadata(status, output_bytes));
    let result_status = if let Some(memory) = &memory {
        memory.status.clone()
    } else {
        audit
            .result
            .as_ref()
            .and_then(|result| result.status.as_deref())
            .map(sanitize_audit_status)
    };
    AuditProjectionEntry {
        cursor: entry.cursor,
        event_id: audit.event_id,
        timestamp: audit.timestamp,
        stage: audit.stage.into(),
        correlation_id: audit.correlation_id,
        invocation_id: audit.invocation_id,
        thread_id: audit.thread_id.clone(),
        process_id: audit.process_id,
        approval_request_id: audit.approval_request_id,
        extension_id: audit.extension_id.clone(),
        action_target: safe_audit_action_target(&action_kind, audit.action.target.as_ref()),
        action_kind,
        decision_kind: sanitize_error_kind(audit.decision.kind.clone()),
        result_status,
        output_bytes,
        memory,
    }
}

fn parse_memory_audit_metadata(
    status: &str,
    output_bytes: Option<u64>,
) -> Option<MemoryAuditProjectionMetadata> {
    let mut segments = status.split(';');
    let prefix = segments.next()?;
    if prefix != "memory_event:v1" && prefix != "memory_prompt_safety:v1" {
        return None;
    }

    let mut metadata = MemoryAuditProjectionMetadata {
        byte_count: output_bytes,
        ..MemoryAuditProjectionMetadata::default()
    };
    for segment in segments {
        let (key, value) = segment.split_once('=')?;
        match key {
            "status" => metadata.status = sanitize_memory_metadata_label(value),
            "path_hash" => metadata.relative_path_hash = sanitize_memory_path_hash(value),
            "chunks" => metadata.chunk_count = sanitize_memory_metadata_u64(value),
            "results" => metadata.result_count = sanitize_memory_metadata_u64(value),
            "full_text" => metadata.full_text = sanitize_memory_metadata_bool(value),
            "vector" => metadata.vector = sanitize_memory_metadata_bool(value),
            "protected_path_class" => {
                metadata.protected_path_class = sanitize_memory_metadata_label(value)
            }
            "reason" => metadata.reason_code = sanitize_memory_metadata_label(value),
            "severity" => metadata.severity = sanitize_memory_metadata_label(value),
            "findings" => metadata.finding_count = sanitize_memory_metadata_u64(value),
            _ => return None,
        }
    }
    metadata.status.as_ref()?;
    Some(metadata)
}

fn sanitize_memory_metadata_label(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > 128 {
        return None;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        .then(|| value.to_string())
}

fn sanitize_memory_path_hash(value: &str) -> Option<String> {
    let hash = value.strip_prefix("sha256:").unwrap_or(value);
    (hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn sanitize_memory_metadata_u64(value: &str) -> Option<u64> {
    value.parse::<u64>().ok()
}

fn sanitize_memory_metadata_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn sanitize_audit_status(status: &str) -> String {
    let mut seen = HashSet::new();
    let mut sanitized = String::new();

    for (index, label) in status.split(',').enumerate() {
        if index >= OBLIGATION_EVALUATION_ORDER.len() {
            return UNCLASSIFIED_ERROR_KIND.to_string();
        }

        let Some(kind) = obligation_kind_from_status_label(label) else {
            return UNCLASSIFIED_ERROR_KIND.to_string();
        };
        if !seen.insert(kind) {
            return UNCLASSIFIED_ERROR_KIND.to_string();
        }

        if index > 0 {
            sanitized.push(',');
        }
        sanitized.push_str(label);
    }

    if seen.is_empty() {
        UNCLASSIFIED_ERROR_KIND.to_string()
    } else {
        sanitized
    }
}

fn obligation_kind_from_status_label(label: &str) -> Option<ObligationKind> {
    OBLIGATION_EVALUATION_ORDER
        .iter()
        .copied()
        .find(|kind| obligation_status_label(*kind) == label)
}

fn obligation_status_label(kind: ObligationKind) -> &'static str {
    match kind {
        ObligationKind::ReserveResources => "reserve_resources",
        ObligationKind::UseScopedMounts => "use_scoped_mounts",
        ObligationKind::ApplyNetworkPolicy => "apply_network_policy",
        ObligationKind::InjectSecretOnce => "inject_secret_once",
        ObligationKind::AuditBefore => "audit_before",
        ObligationKind::RedactOutput => "redact_output",
        ObligationKind::EnforceResourceCeiling => "enforce_resource_ceiling",
        ObligationKind::EnforceOutputLimit => "enforce_output_limit",
        ObligationKind::AuditAfter => "audit_after",
    }
}

fn safe_audit_action_target(action_kind: &str, target: Option<&String>) -> Option<String> {
    match action_kind {
        "dispatch" | "spawn_capability" => target.and_then(|target| {
            CapabilityId::new(target.clone())
                .ok()
                .map(|capability| capability.into_string())
        }),
        _ => None,
    }
}

struct ProjectedRuntimePage {
    entries: Vec<EventLogEntry<RuntimeEvent>>,
    next_cursor: ProjectionCursor,
    truncated: bool,
}

fn project_timeline(entries: &[EventLogEntry<RuntimeEvent>]) -> ThreadTimeline {
    ThreadTimeline {
        entries: entries.iter().map(project_timeline_entry).collect(),
    }
}

fn project_timeline_entry(entry: &EventLogEntry<RuntimeEvent>) -> TimelineEntry {
    let event = &entry.record;
    TimelineEntry {
        cursor: entry.cursor,
        event_id: event.event_id,
        timestamp: event.timestamp,
        kind: event.kind.into(),
        invocation_id: event.scope.invocation_id,
        thread_id: event.scope.thread_id.clone(),
        capability_id: event.capability_id.clone(),
        provider: event.provider.clone(),
        runtime: event.runtime,
        process_id: event.process_id,
        output_bytes: event.output_bytes,
        error_kind: event.error_kind.clone().map(sanitize_error_kind),
        hook_id: event.hook_id.clone(),
        hook_point: event.hook_point.clone(),
        hook_trust_class: event.hook_trust_class.clone(),
        hook_decision: event.hook_decision.clone(),
        hook_failure_category: event.hook_failure_category.clone(),
        hook_failure_disposition: event.hook_failure_disposition.clone(),
    }
}

fn map_audit_projection_error(
    error: EventError,
    operation: &'static str,
    scope: &ProjectionScope,
) -> AuditProjectionError {
    match error {
        EventError::ReplayGap {
            requested,
            earliest,
        } => AuditProjectionError::RebaseRequired {
            requested: Box::new(AuditProjectionCursor::for_scope(scope.clone(), requested)),
            earliest: Box::new(AuditProjectionCursor::for_scope(scope.clone(), earliest)),
        },
        EventError::InvalidReplayRequest { .. } => AuditProjectionError::InvalidRequest {
            reason: "invalid durable replay request",
        },
        EventError::Serialize { .. } | EventError::Sink { .. } | EventError::DurableLog { .. } => {
            AuditProjectionError::Source { operation }
        }
    }
}

fn map_projection_error(
    error: EventError,
    _requested_after: Option<EventCursor>,
    operation: &'static str,
    scope: &ProjectionScope,
) -> ProjectionError {
    match error {
        EventError::ReplayGap {
            requested,
            earliest,
        } => ProjectionError::RebaseRequired {
            requested: Box::new(ProjectionCursor::for_scope(scope.clone(), requested)),
            earliest: Box::new(ProjectionCursor::for_scope(scope.clone(), earliest)),
        },
        EventError::InvalidReplayRequest { .. } => ProjectionError::InvalidRequest {
            reason: "invalid durable replay request",
        },
        EventError::Serialize { .. } | EventError::Sink { .. } | EventError::DurableLog { .. } => {
            ProjectionError::Source { operation }
        }
    }
}
