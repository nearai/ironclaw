use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityId, ResourceScope, RuntimeCredentialAuthRequirement, sha256_digest_token,
};
use ironclaw_host_runtime::{
    CancelRuntimeWorkOutcome, CancelRuntimeWorkRequest, HostRuntime, HostRuntimeError,
    HostRuntimeHealth, HostRuntimeStatus, RuntimeAuthGate, RuntimeBlockedReason,
    RuntimeCapabilityAuthResumeRequest, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, RuntimeGateId, RuntimeStatusRequest, VisibleCapabilityRequest,
    VisibleCapabilitySurface,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeCredentialReauthRequest {
    pub(crate) capability_id: CapabilityId,
    pub(crate) credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
}

/// Composition-owned handoff between the runtime HTTP recovery wrapper and the
/// outer host-runtime outcome wrapper.
///
/// The two wrappers are separated by the capability runtime call boundary:
/// `RuntimeHttpEgress` observes the 401 response inside a tool invocation, while
/// `HostRuntime` is the layer that can return `AuthRequired` to the loop. Keep
/// this bridge private to Reborn composition so it cannot become a generic host
/// event bus.
#[derive(Clone, Default)]
pub(crate) struct RuntimeCredentialReauthBridge {
    inner: Arc<Mutex<Vec<RuntimeCredentialReauthRecord>>>,
}

impl RuntimeCredentialReauthBridge {
    pub(crate) fn record_recovered_auth_required(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    ) {
        if credential_requirements.is_empty() {
            return;
        }
        let record = RuntimeCredentialReauthRecord {
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            credential_requirements,
        };
        match self.inner.lock() {
            Ok(mut records) => records.push(record),
            Err(poisoned) => poisoned.into_inner().push(record),
        }
    }

    pub(crate) fn take_recovered_auth_required(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<RuntimeCredentialReauthRequest> {
        let mut records = match self.inner.lock() {
            Ok(records) => records,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut credential_requirements = Vec::new();
        records.retain(|record| {
            let matches = &record.scope == scope && &record.capability_id == capability_id;
            if matches {
                credential_requirements.extend(record.credential_requirements.clone());
            }
            !matches
        });
        if credential_requirements.is_empty() {
            return None;
        }
        Some(RuntimeCredentialReauthRequest {
            capability_id: capability_id.clone(),
            credential_requirements,
        })
    }
}

#[derive(Debug, Clone)]
struct RuntimeCredentialReauthRecord {
    scope: ResourceScope,
    capability_id: CapabilityId,
    credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
}

pub(crate) struct RuntimeCredentialReauthHostRuntime {
    inner: Arc<dyn HostRuntime>,
    bridge: Arc<RuntimeCredentialReauthBridge>,
}

impl RuntimeCredentialReauthHostRuntime {
    pub(crate) fn new(
        inner: Arc<dyn HostRuntime>,
        bridge: Arc<RuntimeCredentialReauthBridge>,
    ) -> Self {
        Self { inner, bridge }
    }

    fn apply_reauth(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        fallback: RuntimeCapabilityOutcome,
    ) -> RuntimeCapabilityOutcome {
        self.bridge
            .take_recovered_auth_required(scope, capability_id)
            .map(auth_required_outcome)
            .unwrap_or(fallback)
    }

    fn finish_reauth_call(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        outcome: Result<RuntimeCapabilityOutcome, HostRuntimeError>,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        match outcome {
            Ok(outcome) => Ok(self.apply_reauth(scope, capability_id, outcome)),
            Err(error) => {
                self.bridge
                    .take_recovered_auth_required(scope, capability_id);
                Err(error)
            }
        }
    }
}

#[async_trait]
impl HostRuntime for RuntimeCredentialReauthHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.context.resource_scope.clone();
        let capability_id = request.capability_id.clone();
        let outcome = self.inner.invoke_capability(request).await;
        self.finish_reauth_call(&scope, &capability_id, outcome)
    }

    async fn spawn_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.context.resource_scope.clone();
        let capability_id = request.capability_id.clone();
        let outcome = self.inner.spawn_capability(request).await;
        self.finish_reauth_call(&scope, &capability_id, outcome)
    }

    async fn resume_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.context.resource_scope.clone();
        let capability_id = request.capability_id.clone();
        let outcome = self.inner.resume_capability(request).await;
        self.finish_reauth_call(&scope, &capability_id, outcome)
    }

    async fn auth_resume_capability(
        &self,
        request: RuntimeCapabilityAuthResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.context.resource_scope.clone();
        let capability_id = request.capability_id.clone();
        let outcome = self.inner.auth_resume_capability(request).await;
        self.finish_reauth_call(&scope, &capability_id, outcome)
    }

    async fn resume_spawn_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.context.resource_scope.clone();
        let capability_id = request.capability_id.clone();
        let outcome = self.inner.resume_spawn_capability(request).await;
        self.finish_reauth_call(&scope, &capability_id, outcome)
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, HostRuntimeError> {
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

fn auth_required_outcome(request: RuntimeCredentialReauthRequest) -> RuntimeCapabilityOutcome {
    RuntimeCapabilityOutcome::AuthRequired(RuntimeAuthGate {
        gate_id: stable_auth_gate_id(&request.capability_id, &request.credential_requirements),
        capability_id: request.capability_id,
        reason: RuntimeBlockedReason::AuthRequired,
        required_secrets: Vec::new(),
        credential_requirements: request.credential_requirements,
    })
}

fn stable_auth_gate_id(
    capability_id: &CapabilityId,
    credential_requirements: &[RuntimeCredentialAuthRequirement],
) -> RuntimeGateId {
    let mut parts = vec![format!("capability={}", capability_id.as_str())];
    let mut requirements = credential_requirements
        .iter()
        .map(|requirement| {
            let mut scopes = requirement.provider_scopes.clone();
            scopes.sort();
            format!(
                "credential={}:{}:{}",
                requirement.provider.as_str(),
                requirement.requester_extension.as_str(),
                scopes.join(",")
            )
        })
        .collect::<Vec<_>>();
    requirements.sort();
    parts.extend(requirements);

    let digest = sha256_digest_token(parts.join("\n").as_bytes());
    let suffix = digest.strip_prefix("sha256:").unwrap_or(&digest);
    RuntimeGateId::from_stable_suffix(&format!("auth-{suffix}"))
        .unwrap_or_else(|_| RuntimeGateId::new())
}
