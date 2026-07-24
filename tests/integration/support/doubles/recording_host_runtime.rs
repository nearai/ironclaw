/// Test double substituting the production `HostRuntime` impl
/// (`DefaultHostRuntime`, `crates/ironclaw_host_runtime/src/production.rs`).
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_host_api::{ApprovalRequestId, ResourceScope};
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, HostRuntime, HostRuntimeError,
    HostRuntimeHealth, HostRuntimeStatus, RuntimeApprovalResume, RuntimeCapabilityOutcome,
    RuntimeInvocation, RuntimeStatusRequest,
    VisibleCapabilityRequest as RuntimeVisibleCapabilityRequest,
    VisibleCapabilitySurface as RuntimeVisibleCapabilitySurface,
};

pub(crate) struct RecordingHostRuntime {
    inner: Arc<dyn HostRuntime>,
    pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
}

impl RecordingHostRuntime {
    pub(crate) fn new(
        inner: Arc<dyn HostRuntime>,
        pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
    ) -> Self {
        Self {
            inner,
            pending_approval_scopes,
        }
    }
}

#[async_trait]
impl HostRuntime for RecordingHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeInvocation,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.0.resource_scope.clone();
        let outcome = self.inner.invoke_capability(request).await?;
        if let RuntimeCapabilityOutcome::ApprovalRequired(gate) = &outcome {
            self.pending_approval_scopes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(gate.approval_request_id, scope);
        }
        Ok(outcome)
    }

    async fn spawn_capability(
        &self,
        request: RuntimeInvocation,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.0.resource_scope.clone();
        let outcome = self.inner.spawn_capability(request).await?;
        if let RuntimeCapabilityOutcome::ApprovalRequired(gate) = &outcome {
            self.pending_approval_scopes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(gate.approval_request_id, scope);
        }
        Ok(outcome)
    }

    async fn resume_capability(
        &self,
        request: RuntimeApprovalResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.resume_capability(request).await
    }

    /// C-JOURNEY: forward auth-resume to the real runtime. `auth_resume_capability`
    /// is a DEFAULTED trait method whose default fails loudly, so a wrapper
    /// that forgets to forward it fails visibly — this wrapper's missing
    /// forward was latent until the first auth→resolve→re-dispatch journey
    /// drove it. Mirrors `invoke_capability`'s ApprovalRequired scope
    /// recording because `auth_resume_json` can itself raise an approval gate.
    async fn auth_resume_capability(
        &self,
        request: ironclaw_host_runtime::RuntimeAuthResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.0.resource_scope.clone();
        let outcome = self.inner.auth_resume_capability(request).await?;
        if let RuntimeCapabilityOutcome::ApprovalRequired(gate) = &outcome {
            self.pending_approval_scopes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(gate.approval_request_id, scope);
        }
        Ok(outcome)
    }

    async fn resume_spawn_capability(
        &self,
        request: RuntimeApprovalResume,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.resume_spawn_capability(request).await
    }

    /// Forward the denied-gate terminalization to the real runtime — like
    /// `auth_resume_capability` above, `decline_auth_capability` is a
    /// DEFAULTED trait method whose default fails closed, so a wrapper that
    /// forgets to forward it kills every denied-auth-resume run.
    async fn decline_auth_capability(
        &self,
        request: ironclaw_host_runtime::RuntimeAuthDecline,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.decline_auth_capability(request).await
    }

    async fn visible_capabilities(
        &self,
        request: RuntimeVisibleCapabilityRequest,
    ) -> Result<RuntimeVisibleCapabilitySurface, HostRuntimeError> {
        self.inner.visible_capabilities(request).await
    }

    async fn cancel_work(
        &self,
        request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        self.inner.cancel_work(request).await
    }

    async fn runtime_status(
        &self,
        request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        self.inner.runtime_status(request).await
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        self.inner.health().await
    }
}
