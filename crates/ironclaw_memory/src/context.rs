//! Host-resolved scoped memory context.

use ironclaw_host_api::{CorrelationId, ResourceScope};

use crate::events::MemoryAuditContext;
use crate::path::MemoryDocumentScope;
use crate::safety::PromptSafetyAllowanceId;

/// Host-resolved scoped context passed to memory backends.
///
/// Backends receive this context after the host has parsed and authorized the
/// virtual path. They must not infer broader tenant/user/project authority from
/// their own configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryContext {
    scope: MemoryDocumentScope,
    invocation_id: Option<String>,
    audit_context: Option<MemoryAuditContext>,
    prompt_write_safety_allowance: Option<PromptSafetyAllowanceId>,
}

impl MemoryContext {
    pub fn new(scope: MemoryDocumentScope) -> Self {
        Self {
            scope,
            invocation_id: None,
            audit_context: None,
            prompt_write_safety_allowance: None,
        }
    }

    pub fn with_invocation_id(mut self, invocation_id: impl Into<String>) -> Self {
        self.invocation_id = Some(invocation_id.into());
        self
    }

    pub fn with_audit_context(
        mut self,
        resource_scope: ResourceScope,
        correlation_id: CorrelationId,
    ) -> Self {
        self.invocation_id = Some(resource_scope.invocation_id.to_string());
        self.audit_context = Some(MemoryAuditContext::new(resource_scope, correlation_id));
        self
    }

    pub fn with_prompt_write_safety_allowance(
        mut self,
        allowance: PromptSafetyAllowanceId,
    ) -> Self {
        self.prompt_write_safety_allowance = Some(allowance);
        self
    }

    pub fn scope(&self) -> &MemoryDocumentScope {
        &self.scope
    }

    pub fn invocation_id(&self) -> Option<&str> {
        self.invocation_id.as_deref()
    }

    pub fn audit_context(&self) -> Option<&MemoryAuditContext> {
        self.audit_context.as_ref()
    }

    pub fn prompt_write_safety_allowance(&self) -> Option<&PromptSafetyAllowanceId> {
        self.prompt_write_safety_allowance.as_ref()
    }
}
