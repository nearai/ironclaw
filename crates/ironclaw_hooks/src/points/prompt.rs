//! Context for the `before_prompt` / `before_context` hook points.

use ironclaw_host_api::TenantId;

/// Read-only context handed to a prompt-mutator hook.
///
/// First slice intentionally exposes minimal information: tenant scope and a
/// hint about how much byte budget remains for snippet additions. Richer
/// fields (current snippet count, identity-message presence, capability
/// surface descriptors) become available as the dispatcher composes with the
/// Reborn host port middleware.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BeforePromptHookContext {
    pub tenant_id: TenantId,
    /// Bytes still available in the prompt-bundle snippet budget. Mutator
    /// hooks must keep their `HookPatch::AddSnippet.byte_count` under this.
    pub remaining_snippet_byte_budget: u32,
}

impl BeforePromptHookContext {
    pub fn new(tenant_id: TenantId, remaining_snippet_byte_budget: u32) -> Self {
        Self {
            tenant_id,
            remaining_snippet_byte_budget,
        }
    }
}
