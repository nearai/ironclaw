mod credential;
mod pipeline;
mod sanitize;

use ironclaw_host_api::{
    CapabilityId, NetworkPolicy, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
};
use ironclaw_network::{NetworkHttpEgress, NetworkHttpError};
use ironclaw_safety::LeakDetector;
use ironclaw_secrets::SecretStore;
use std::{fmt, sync::Arc};

use crate::http_body::{RuntimeHttpBodyStore, UnsupportedRuntimeHttpBodyStore};
use crate::obligations::{NetworkObligationPolicyStore, RuntimeSecretInjectionStore};

#[derive(Debug, Clone)]
enum NetworkPolicySource {
    Production {
        network_policy_store: Arc<NetworkObligationPolicyStore>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
    },
    MissingStagedForTests,
}

#[derive(Clone)]
pub struct HostHttpEgressService<N, S> {
    pub(super) network: N,
    pub(super) secrets: S,
    pub(super) leak_detector: Arc<LeakDetector>,
    network_policy_source: NetworkPolicySource,
    pub(super) unsafe_raw_diagnostics_allowed: bool,
    pub(super) body_store: Arc<dyn RuntimeHttpBodyStore>,
}

impl<N, S> fmt::Debug for HostHttpEgressService<N, S>
where
    N: fmt::Debug,
    S: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostHttpEgressService")
            .field("network", &self.network)
            .field("secrets", &self.secrets)
            .field("leak_detector", &"<shared>")
            .field("network_policy_source", &self.network_policy_source)
            .field(
                "unsafe_raw_diagnostics_allowed",
                &self.unsafe_raw_diagnostics_allowed,
            )
            .field("body_store", &self.body_store)
            .finish()
    }
}

impl<N, S> HostHttpEgressService<N, S> {
    /// Construct host HTTP egress without staged handoff stores.
    ///
    /// This service rejects requests before transport until production
    /// composition supplies staged network-policy and secret-injection stores.
    pub fn new(network: N, secrets: S) -> Self {
        Self {
            network,
            secrets,
            leak_detector: Arc::new(LeakDetector::new()),
            network_policy_source: NetworkPolicySource::MissingStagedForTests,
            unsafe_raw_diagnostics_allowed: false,
            body_store: Arc::new(UnsupportedRuntimeHttpBodyStore),
        }
    }

    pub(crate) fn production(
        network: N,
        secrets: S,
        network_policy_store: Arc<NetworkObligationPolicyStore>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
        body_store: Arc<dyn RuntimeHttpBodyStore>,
    ) -> Self {
        Self {
            network,
            secrets,
            leak_detector: Arc::new(LeakDetector::new()),
            network_policy_source: NetworkPolicySource::Production {
                network_policy_store,
                secret_injections,
            },
            unsafe_raw_diagnostics_allowed: false,
            body_store,
        }
    }

    pub(crate) fn with_unsafe_raw_diagnostics_allowed(mut self, allowed: bool) -> Self {
        self.unsafe_raw_diagnostics_allowed = allowed;
        self
    }

    pub(crate) fn is_production_wired_with(
        &self,
        expected_network_policy_store: &Arc<NetworkObligationPolicyStore>,
        expected_secret_injections: &Arc<RuntimeSecretInjectionStore>,
    ) -> bool {
        match &self.network_policy_source {
            NetworkPolicySource::Production {
                network_policy_store,
                secret_injections,
            } => {
                Arc::ptr_eq(network_policy_store, expected_network_policy_store)
                    && Arc::ptr_eq(secret_injections, expected_secret_injections)
            }
            NetworkPolicySource::MissingStagedForTests => false,
        }
    }

    pub fn with_body_store(mut self, store: Arc<dyn RuntimeHttpBodyStore>) -> Self {
        self.body_store = store;
        self
    }

    pub(super) fn network_policy_for_request(
        &self,
        request: &mut RuntimeHttpEgressRequest,
    ) -> Result<NetworkPolicy, PipelineError> {
        match &self.network_policy_source {
            NetworkPolicySource::Production {
                network_policy_store,
                ..
            } => network_policy_store
                .get(&request.scope, &request.capability_id)
                .ok_or_else(|| {
                    PipelineError::pre_transport(RuntimeHttpEgressError::Network {
                        reason: "network_policy_missing".to_string(),
                        request_bytes: 0,
                        response_bytes: 0,
                    })
                }),
            NetworkPolicySource::MissingStagedForTests => Err(PipelineError::pre_transport(
                RuntimeHttpEgressError::Network {
                    reason: "network_policy_missing".to_string(),
                    request_bytes: 0,
                    response_bytes: 0,
                },
            )),
        }
    }

    fn discard_staged_policy(&self, scope: &ResourceScope, capability_id: &CapabilityId) {
        if let NetworkPolicySource::Production {
            network_policy_store,
            ..
        } = &self.network_policy_source
        {
            network_policy_store.discard_for_capability(scope, capability_id);
        }
    }

    pub(super) fn validate_credential_sources_for_request(
        &self,
        request: &RuntimeHttpEgressRequest,
    ) -> Result<(), PipelineError> {
        credential::validate_sources_for_request(request, self.allows_direct_secret_lease())
            .map_err(PipelineError::pre_transport)
    }

    pub(super) fn secret_injections(&self) -> Option<&RuntimeSecretInjectionStore> {
        match &self.network_policy_source {
            NetworkPolicySource::Production {
                secret_injections, ..
            } => Some(secret_injections.as_ref()),
            NetworkPolicySource::MissingStagedForTests => None,
        }
    }

    pub(super) fn allows_direct_secret_lease(&self) -> bool {
        false
    }
}

impl<N, S> RuntimeHttpEgress for HostHttpEgressService<N, S>
where
    N: NetworkHttpEgress,
    S: SecretStore,
{
    fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let scope = request.scope.clone();
        let capability_id = request.capability_id.clone();
        let result = pipeline::execute(self, request);
        match result {
            Ok(response) => Ok(response),
            Err(error) => {
                if error.should_discard_staged_policy() {
                    self.discard_staged_policy(&scope, &capability_id);
                }
                Err(error.into_inner())
            }
        }
    }
}

pub(super) struct PipelineError {
    error: RuntimeHttpEgressError,
    discard_staged_policy: bool,
}

impl PipelineError {
    pub(super) fn pre_transport(error: RuntimeHttpEgressError) -> Self {
        Self {
            error,
            discard_staged_policy: true,
        }
    }

    pub(super) fn post_transport(error: RuntimeHttpEgressError) -> Self {
        Self {
            error,
            discard_staged_policy: false,
        }
    }

    fn should_discard_staged_policy(&self) -> bool {
        self.discard_staged_policy
    }

    fn into_inner(self) -> RuntimeHttpEgressError {
        self.error
    }
}

pub(super) fn runtime_network_error(
    unsafe_raw_diagnostics_allowed: bool,
    error: NetworkHttpError,
) -> RuntimeHttpEgressError {
    log_raw_network_http_error_for_local_diagnostics(unsafe_raw_diagnostics_allowed, &error);
    RuntimeHttpEgressError::Network {
        reason: error.stable_reason().to_string(),
        request_bytes: error.request_bytes(),
        response_bytes: error.response_bytes(),
    }
}

fn log_raw_network_http_error_for_local_diagnostics(
    unsafe_raw_diagnostics_allowed: bool,
    error: &NetworkHttpError,
) {
    if !crate::unsafe_raw_http_diagnostics_enabled(unsafe_raw_diagnostics_allowed) {
        return;
    }

    tracing::warn!(
        network_error_kind = error.kind().as_str(),
        raw_network_error = %error,
        unsafe_raw_diagnostics = true,
        "unsafe raw HTTP egress error diagnostic enabled"
    );
}

pub(super) fn runtime_response(
    response: ironclaw_network::NetworkHttpResponse,
    redaction_applied: bool,
    saved_body: Option<ironclaw_host_api::RuntimeHttpSavedBody>,
) -> RuntimeHttpEgressResponse {
    RuntimeHttpEgressResponse {
        status: response.status,
        headers: response.headers,
        body: response.body,
        saved_body,
        request_bytes: response.usage.request_bytes,
        response_bytes: response.usage.response_bytes,
        redaction_applied,
    }
}
