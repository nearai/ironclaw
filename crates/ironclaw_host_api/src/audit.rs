//! Audit envelope contracts for durable provenance.
//!
//! [`AuditEnvelope`] is the redacted, durable record shape for authorization
//! decisions and externally visible side effects. It carries scope, correlation,
//! action summary, decision summary, and optional result metadata without raw
//! secrets or raw host paths. Service crates are responsible for persisting and
//! emitting these envelopes at the required before/after/denied stages.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    AuditEventId, CorrelationId, DenyReason, EffectKind, ExecutionContext, ExtensionId,
    InvocationId, MissionId, ProcessId, ProjectId, TenantId, ThreadId, Timestamp, UserId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEnvelope {
    pub event_id: AuditEventId,
    pub correlation_id: CorrelationId,
    pub stage: AuditStage,
    pub timestamp: Timestamp,

    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub invocation_id: InvocationId,
    pub process_id: Option<ProcessId>,
    pub extension_id: ExtensionId,

    pub action: ActionSummary,
    pub decision: DecisionSummary,
    pub result: Option<ActionResultSummary>,
}

impl AuditEnvelope {
    pub fn denied(
        ctx: &ExecutionContext,
        stage: AuditStage,
        action: ActionSummary,
        reason: DenyReason,
    ) -> Self {
        Self {
            event_id: AuditEventId::new(),
            correlation_id: ctx.correlation_id,
            stage,
            timestamp: Utc::now(),
            tenant_id: ctx.tenant_id.clone(),
            user_id: ctx.user_id.clone(),
            project_id: ctx.project_id.clone(),
            mission_id: ctx.mission_id.clone(),
            thread_id: ctx.thread_id.clone(),
            invocation_id: ctx.invocation_id,
            process_id: ctx.process_id,
            extension_id: ctx.extension_id.clone(),
            action,
            decision: DecisionSummary {
                kind: "deny".to_string(),
                reason: Some(reason),
            },
            result: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStage {
    Before,
    After,
    Denied,
    ApprovalRequested,
    ApprovalResolved,
    ResourceReserved,
    ResourceReconciled,
    ResourceReleased,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSummary {
    pub kind: String,
    pub target: Option<String>,
    pub effects: Vec<EffectKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionSummary {
    pub kind: String,
    pub reason: Option<DenyReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResultSummary {
    pub success: bool,
    pub status: Option<String>,
    pub output_bytes: Option<u64>,
}
