//! Generic decorator that adds a single host-managed credential fallback to
//! any [`RuntimeCredentialAccountSelectionService`].
//!
//! [`ProductAuthRuntimeCredentialAccountSelector`] (the parent module) stays
//! provider-agnostic selection machinery. Provider-specific fallback wiring
//! (e.g. the NEAR AI MCP host-managed key) is composed here and supplied by
//! the caller — see `RebornProductAuthServices::runtime_credential_account_selection_service`.

use super::*;

/// A single "if the requester's own account is missing, try this scoped
/// host-managed account instead" rule.
///
/// Matches only when the request's provider and requester extension equal
/// `provider`/`requester_extension` exactly, and the request's runtime scope
/// is within `owner_scope` (same surface/tenant/agent, and same project
/// unless `owner_scope` is project-unscoped — see [`Self::scope_matches`]).
pub(crate) struct HostManagedCredentialFallbackRule {
    provider: AuthProviderId,
    requester_extension: ExtensionId,
    owner_scope: AuthProductScope,
}

impl HostManagedCredentialFallbackRule {
    pub(crate) fn new(
        provider: AuthProviderId,
        requester_extension: ExtensionId,
        owner_scope: AuthProductScope,
    ) -> Self {
        Self {
            provider,
            requester_extension,
            owner_scope,
        }
    }

    fn host_request_for(
        &self,
        request: &RuntimeCredentialAccountSelectionRequest,
    ) -> Option<RuntimeCredentialAccountSelectionRequest> {
        let requester_extension = request.lookup.requester_extension.as_ref()?;
        if request.lookup.provider != self.provider
            || requester_extension != &self.requester_extension
        {
            return None;
        }
        if !self.scope_matches(&request.runtime_scope) {
            return None;
        }
        Some(RuntimeCredentialAccountSelectionRequest::new(
            CredentialAccountSelectionRequest::new(
                self.owner_scope.clone(),
                request.lookup.provider.clone(),
            )
            .for_extension(requester_extension.clone()),
            self.owner_scope.clone(),
            request.setup.clone(),
            request.provider_scopes.clone(),
        ))
    }

    /// `owner_scope.resource.project_id == None` means the host credential is
    /// bootstrapped at tenant/agent granularity and is reusable from every
    /// project under that agent; a project-scoped host credential requires an
    /// exact project match.
    fn scope_matches(&self, runtime_scope: &AuthProductScope) -> bool {
        self.owner_scope.surface == runtime_scope.surface
            && self.owner_scope.resource.tenant_id == runtime_scope.resource.tenant_id
            && self.owner_scope.resource.agent_id == runtime_scope.resource.agent_id
            && (self.owner_scope.resource.project_id.is_none()
                || self.owner_scope.resource.project_id == runtime_scope.resource.project_id)
    }
}

/// Wraps any [`RuntimeCredentialAccountSelectionService`] and retries a
/// `CredentialMissing` runtime-resolution result against a single
/// [`HostManagedCredentialFallbackRule`] before giving up.
///
/// OAuth *binding* (`select_configured_account_for_binding`) is never
/// eligible for host-managed fallback — it always delegates straight
/// through, matching the un-decorated selector's behavior.
pub(crate) struct HostManagedRuntimeCredentialAccountSelector {
    inner: Arc<dyn RuntimeCredentialAccountSelectionService>,
    fallback: HostManagedCredentialFallbackRule,
}

impl HostManagedRuntimeCredentialAccountSelector {
    pub(crate) fn new(
        inner: Arc<dyn RuntimeCredentialAccountSelectionService>,
        fallback: HostManagedCredentialFallbackRule,
    ) -> Self {
        Self { inner, fallback }
    }
}

#[async_trait]
impl RuntimeCredentialAccountSelectionService for HostManagedRuntimeCredentialAccountSelector {
    async fn select_unique_configured_runtime_account(
        &self,
        request: RuntimeCredentialAccountSelectionRequest,
    ) -> Result<CredentialAccount, AuthProductError> {
        match self
            .inner
            .select_unique_configured_runtime_account(request.clone())
            .await
        {
            Err(AuthProductError::CredentialMissing) => {
                let Some(host_request) = self.fallback.host_request_for(&request) else {
                    return Err(AuthProductError::CredentialMissing);
                };
                self.inner
                    .select_unique_configured_runtime_account(host_request)
                    .await
            }
            result => result,
        }
    }

    async fn select_configured_account_for_binding(
        &self,
        lookup: CredentialAccountSelectionRequest,
        runtime_scope: AuthProductScope,
    ) -> Result<CredentialAccount, AuthProductError> {
        self.inner
            .select_configured_account_for_binding(lookup, runtime_scope)
            .await
    }
}
