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
    Action, AgentId, ApprovalRequest, AuditEventId, CapabilityId, CorrelationId, DenyReason,
    EffectKind, ExecutionContext, ExtensionId, InvocationId, MissionId, NetworkMethod, Principal,
    ProcessId, ProjectId, SecretUseMode, TenantId, ThreadId, Timestamp, UserId,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEnvelope {
    pub event_id: AuditEventId,
    pub correlation_id: CorrelationId,
    pub stage: AuditStage,
    pub timestamp: Timestamp,

    pub tenant_id: TenantId,
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    pub mission_id: Option<MissionId>,
    pub thread_id: Option<ThreadId>,
    pub invocation_id: InvocationId,
    pub process_id: Option<ProcessId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_request_id: Option<crate::ApprovalRequestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<ExtensionId>,

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
            agent_id: ctx.agent_id.clone(),
            project_id: ctx.project_id.clone(),
            mission_id: ctx.mission_id.clone(),
            thread_id: ctx.thread_id.clone(),
            invocation_id: ctx.invocation_id,
            process_id: ctx.process_id,
            approval_request_id: None,
            extension_id: Some(ctx.extension_id.clone()),
            action,
            decision: DecisionSummary {
                kind: "deny".to_string(),
                reason: Some(reason),
                actor: None,
            },
            result: None,
        }
    }

    pub fn approval_resolved(
        scope: &crate::ResourceScope,
        request: &ApprovalRequest,
        resolved_by: Principal,
        decision: impl Into<String>,
    ) -> Self {
        let decision = decision.into();
        Self {
            event_id: AuditEventId::new(),
            correlation_id: request.correlation_id,
            stage: AuditStage::ApprovalResolved,
            timestamp: Utc::now(),
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: scope.mission_id.clone(),
            thread_id: scope.thread_id.clone(),
            invocation_id: scope.invocation_id,
            process_id: None,
            approval_request_id: Some(request.id),
            extension_id: extension_from_principal(&request.requested_by),
            action: ActionSummary::from_action(request.action.as_ref()),
            decision: DecisionSummary {
                kind: decision,
                reason: None,
                actor: Some(resolved_by),
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

impl ActionSummary {
    pub fn from_action(action: &Action) -> Self {
        match action {
            Action::ReadFile { path } => Self::new(
                "read_file",
                Some(path.as_str().to_string()),
                vec![EffectKind::ReadFilesystem],
            ),
            Action::ListDir { path } => Self::new(
                "list_dir",
                Some(path.as_str().to_string()),
                vec![EffectKind::ReadFilesystem],
            ),
            Action::WriteFile { path, .. } => Self::new(
                "write_file",
                Some(path.as_str().to_string()),
                vec![EffectKind::WriteFilesystem],
            ),
            Action::DeleteFile { path } => Self::new(
                "delete_file",
                Some(path.as_str().to_string()),
                vec![EffectKind::DeleteFilesystem],
            ),
            Action::Dispatch { capability, .. } => {
                Self::capability("dispatch", capability, vec![EffectKind::DispatchCapability])
            }
            Action::SpawnCapability { capability, .. } => Self::capability(
                "spawn_capability",
                capability,
                vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
            ),
            Action::UseSecret { handle, mode } => Self::new(
                "use_secret",
                Some(secret_target(handle.as_str(), mode)),
                vec![EffectKind::UseSecret],
            ),
            Action::Network { target, method, .. } => Self::new(
                "network",
                Some(network_target(method, target.host.as_str(), target.port)),
                vec![EffectKind::Network],
            ),
            Action::ReserveResources { .. } => {
                Self::new("reserve_resources", None, vec![EffectKind::ModifyBudget])
            }
            Action::Approve { request } => Self::new(
                "approve",
                Some(request.id.to_string()),
                vec![EffectKind::ModifyApproval],
            ),
            Action::ExtensionLifecycle {
                extension_id,
                operation,
            } => Self::new(
                "extension_lifecycle",
                Some(format!("{}:{operation}", extension_id.as_str())),
                vec![EffectKind::ModifyExtension],
            ),
            Action::EmitExternalEffect { effect } => {
                Self::new("emit_external_effect", None, vec![*effect])
            }
        }
    }

    fn capability(
        kind: impl Into<String>,
        capability: &CapabilityId,
        effects: Vec<EffectKind>,
    ) -> Self {
        Self::new(kind, Some(capability.as_str().to_string()), effects)
    }

    fn new(kind: impl Into<String>, target: Option<String>, effects: Vec<EffectKind>) -> Self {
        Self {
            kind: kind.into(),
            target,
            effects,
        }
    }
}

fn extension_from_principal(principal: &Principal) -> Option<ExtensionId> {
    match principal {
        Principal::Extension(extension_id) => Some(extension_id.clone()),
        _ => None,
    }
}

fn network_target(method: &NetworkMethod, host: &str, port: Option<u16>) -> String {
    match port {
        Some(port) => format!("{method}:{host}:{port}"),
        None => format!("{method}:{host}"),
    }
}

fn secret_target(handle: &str, mode: &SecretUseMode) -> String {
    format!("{handle}:{mode}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionSummary {
    pub kind: String,
    pub reason: Option<DenyReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Principal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResultSummary {
    pub success: bool,
    pub status: Option<String>,
    pub output_bytes: Option<u64>,
}
