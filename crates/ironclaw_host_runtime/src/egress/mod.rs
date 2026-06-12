mod credential;
mod host_port;
mod pipeline;
mod sanitize;

use async_trait::async_trait;
use ironclaw_events::{SecurityAuditEvent, SecurityAuditSink, SecurityBoundary, SecurityDecision};
use ironclaw_host_api::{
    CapabilityId, NetworkPolicy, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
};
use ironclaw_network::{NetworkHttpEgress, NetworkHttpError};
use ironclaw_safety::LeakDetector;
use ironclaw_secrets::SecretStore;
use std::{fmt, sync::Arc};

use crate::obligations::{NetworkObligationPolicyStore, RuntimeSecretInjectionStore};
use crate::{ToolCallHttpEgress, http_body::RuntimeHttpBodyStore};

const NO_EXPOSURE_SENSITIVE_HEADER_DENIED_CODE: &str = "no_exposure_sensitive_header_denied";
const NO_EXPOSURE_MANUAL_CREDENTIALS_DENIED_CODE: &str = "no_exposure_manual_credentials_denied";
const NO_EXPOSURE_REQUEST_LEAK_BLOCKED_CODE: &str = "no_exposure_request_leak_blocked";
const NO_EXPOSURE_RESPONSE_LEAK_BLOCKED_CODE: &str = "no_exposure_response_leak_blocked";

pub use host_port::{
    HostRuntimeCredentialMaterial, HostRuntimeHttpEgressPort, HostRuntimeHttpEgressRequest,
    RuntimeSecretMaterialStager, RuntimeSecretStageError,
};

#[derive(Clone)]
pub struct HostHttpEgressService<N, S> {
    network: N,
    secrets: S,
    leak_detector: Arc<LeakDetector>,
    network_policy_store: Arc<NetworkObligationPolicyStore>,
    secret_injections: Arc<RuntimeSecretInjectionStore>,
    security_audit_sink: Option<Arc<dyn SecurityAuditSink>>,
    unsafe_raw_diagnostics_allowed: bool,
    body_store: Arc<dyn RuntimeHttpBodyStore>,
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
            .field("network_policy_store", &self.network_policy_store)
            .field("secret_injections", &self.secret_injections)
            .field("security_audit_sink", &self.security_audit_sink.is_some())
            .field(
                "unsafe_raw_diagnostics_allowed",
                &self.unsafe_raw_diagnostics_allowed,
            )
            .field("body_store", &self.body_store)
            .finish()
    }
}

impl<N, S> HostHttpEgressService<N, S> {
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
            network_policy_store,
            secret_injections,
            security_audit_sink: None,
            unsafe_raw_diagnostics_allowed: false,
            body_store,
        }
    }

    pub(crate) fn with_security_audit_sink(mut self, sink: Arc<dyn SecurityAuditSink>) -> Self {
        self.security_audit_sink = Some(sink);
        self
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
        Arc::ptr_eq(&self.network_policy_store, expected_network_policy_store)
            && Arc::ptr_eq(&self.secret_injections, expected_secret_injections)
    }

    pub fn with_body_store(mut self, store: Arc<dyn RuntimeHttpBodyStore>) -> Self {
        self.body_store = store;
        self
    }

    pub(super) fn network_policy_for_request(
        &self,
        request: &mut RuntimeHttpEgressRequest,
    ) -> Result<NetworkPolicy, PipelineError> {
        self.network_policy_store
            .get(&request.scope, &request.capability_id)
            .ok_or_else(|| {
                PipelineError::pre_transport(RuntimeHttpEgressError::Network {
                    reason: "network_policy_missing".to_string(),
                    request_bytes: 0,
                    response_bytes: 0,
                })
            })
    }

    fn discard_staged_policy(&self, scope: &ResourceScope, capability_id: &CapabilityId) {
        self.network_policy_store
            .discard_for_capability(scope, capability_id);
    }

    fn discard_staged_secret_injections(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) {
        if let Err(error) = self
            .secret_injections
            .discard_for_capability(scope, capability_id)
        {
            tracing::debug!(
                error = ?error,
                capability_id = %capability_id,
                "runtime HTTP egress failed to discard staged secret injections"
            );
        }
    }

    pub(super) fn validate_credential_sources_for_request(
        &self,
        request: &RuntimeHttpEgressRequest,
    ) -> Result<(), PipelineError> {
        credential::validate_sources_for_request(request).map_err(PipelineError::pre_transport)
    }

    pub(super) fn secret_injections(&self) -> Option<&RuntimeSecretInjectionStore> {
        Some(self.secret_injections.as_ref())
    }

    pub(super) fn network(&self) -> &N {
        &self.network
    }

    pub(super) fn secrets(&self) -> &S {
        &self.secrets
    }

    pub(super) fn leak_detector(&self) -> &LeakDetector {
        &self.leak_detector
    }

    pub(super) fn unsafe_raw_diagnostics_allowed(&self) -> bool {
        self.unsafe_raw_diagnostics_allowed
    }

    pub(super) fn body_store(&self) -> &dyn RuntimeHttpBodyStore {
        self.body_store.as_ref()
    }

    fn record_no_exposure_block(
        &self,
        error: &RuntimeHttpEgressError,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) {
        let Some(code) = no_exposure_audit_code(error) else {
            return;
        };
        let Some(sink) = &self.security_audit_sink else {
            return;
        };
        sink.record(
            SecurityAuditEvent::new(
                SecurityBoundary::NoExposureGuard,
                SecurityDecision::Blocked,
                code,
            )
            .with_capability_id(capability_id.clone())
            .with_scope(scope.clone()),
        );
    }
}

#[async_trait]
impl<N, S> RuntimeHttpEgress for HostHttpEgressService<N, S>
where
    N: NetworkHttpEgress + Send + Sync,
    S: SecretStore + Send + Sync,
{
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let scope = request.scope.clone();
        let capability_id = request.capability_id.clone();
        let result = pipeline::execute(self, request).await;
        match result {
            Ok(response) => Ok(response),
            Err(error) => {
                self.record_no_exposure_block(error.error(), &scope, &capability_id);
                if error.should_discard_staged_policy() {
                    self.discard_staged_policy(&scope, &capability_id);
                }
                if error.should_discard_staged_secret_injections() {
                    self.discard_staged_secret_injections(&scope, &capability_id);
                }
                Err(error.into_inner())
            }
        }
    }
}

#[async_trait]
impl<N, S> ToolCallHttpEgress for HostHttpEgressService<N, S>
where
    N: NetworkHttpEgress + Send + Sync,
    S: SecretStore + Send + Sync,
{
    async fn execute_for_model_visible_output(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let scope = request.scope.clone();
        let capability_id = request.capability_id.clone();
        let result = pipeline::execute_for_model_visible_output(self, request).await;
        match result {
            Ok(response) => Ok(response),
            Err(error) => {
                self.record_no_exposure_block(error.error(), &scope, &capability_id);
                if error.should_discard_staged_policy() {
                    self.discard_staged_policy(&scope, &capability_id);
                }
                if error.should_discard_staged_secret_injections() {
                    self.discard_staged_secret_injections(&scope, &capability_id);
                }
                Err(error.into_inner())
            }
        }
    }
}

pub(super) struct PipelineError {
    error: RuntimeHttpEgressError,
    discard_staged_policy: bool,
    discard_staged_secret_injections: bool,
}

impl PipelineError {
    pub(super) fn pre_transport(error: RuntimeHttpEgressError) -> Self {
        Self {
            error,
            discard_staged_policy: true,
            discard_staged_secret_injections: true,
        }
    }

    pub(super) fn pre_transport_keep_staged_secrets(error: RuntimeHttpEgressError) -> Self {
        Self {
            error,
            discard_staged_policy: true,
            discard_staged_secret_injections: false,
        }
    }

    pub(super) fn post_transport(error: RuntimeHttpEgressError) -> Self {
        Self {
            error,
            discard_staged_policy: false,
            discard_staged_secret_injections: false,
        }
    }

    fn should_discard_staged_policy(&self) -> bool {
        self.discard_staged_policy
    }

    fn should_discard_staged_secret_injections(&self) -> bool {
        self.discard_staged_secret_injections
    }

    fn into_inner(self) -> RuntimeHttpEgressError {
        self.error
    }

    fn error(&self) -> &RuntimeHttpEgressError {
        &self.error
    }
}

/// Map a pipeline error to a no-exposure security-audit code.
///
/// The variant match is exhaustive on purpose: adding a new
/// `RuntimeHttpEgressError` variant is a compile error here, forcing an
/// explicit decision about whether it represents a no-exposure block. The
/// reason strings come from the shared `sanitize::REASON_*` constants used by
/// the producer sites, so a reason rename cannot desynchronize producer and
/// mapper. Reasons not listed are not no-exposure blocks by design (e.g.
/// network policy or body-store failures).
fn no_exposure_audit_code(error: &RuntimeHttpEgressError) -> Option<&'static str> {
    match error {
        RuntimeHttpEgressError::Request { reason, .. } => match reason.as_str() {
            sanitize::REASON_SENSITIVE_HEADER_DENIED => {
                Some(NO_EXPOSURE_SENSITIVE_HEADER_DENIED_CODE)
            }
            sanitize::REASON_MANUAL_CREDENTIALS_DENIED => {
                Some(NO_EXPOSURE_MANUAL_CREDENTIALS_DENIED_CODE)
            }
            sanitize::REASON_CREDENTIAL_LEAK_BLOCKED => Some(NO_EXPOSURE_REQUEST_LEAK_BLOCKED_CODE),
            _ => None,
        },
        RuntimeHttpEgressError::Response { reason, .. } => match reason.as_str() {
            sanitize::REASON_RESPONSE_LEAK_BLOCKED => Some(NO_EXPOSURE_RESPONSE_LEAK_BLOCKED_CODE),
            _ => None,
        },
        RuntimeHttpEgressError::Credential { .. } | RuntimeHttpEgressError::Network { .. } => None,
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

    tracing::debug!(
        network_error_kind = error.kind().as_str(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        InvocationId, NetworkMethod, NetworkPolicy as ApiNetworkPolicy, RuntimeKind, TenantId,
        UserId,
    };
    use ironclaw_network::{NetworkHttpResponse, NetworkUsage};

    /// Test-only token shaped like an API key so the leak detector fires.
    const LEAKY_SECRET: &str = "sk-proj-test1234567890abcdefghij";

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn sample_request() -> RuntimeHttpEgressRequest {
        RuntimeHttpEgressRequest {
            runtime: RuntimeKind::Script,
            scope: sample_scope(),
            capability_id: CapabilityId::new("runtime.http").unwrap(),
            method: NetworkMethod::Post,
            url: "https://api.example.test/v1/run".to_string(),
            headers: vec![],
            body: b"hello".to_vec(),
            network_policy: ApiNetworkPolicy {
                allowed_targets: vec![],
                deny_private_ip_ranges: true,
                max_egress_bytes: Some(4096),
            },
            credential_injections: vec![],
            response_body_limit: Some(4096),
            save_body_to: None,
            timeout_ms: None,
        }
    }

    /// Regression guard for the producer/mapper contract: every no-exposure
    /// block emitted by the `sanitize` producers must map to a non-`None`
    /// audit code. If a producer reason string or error shape changes without
    /// updating `no_exposure_audit_code`, these tests fail instead of audit
    /// recording silently disappearing.
    #[test]
    fn sensitive_header_producer_maps_to_audit_code() {
        let mut request = sample_request();
        request
            .headers
            .push(("authorization".to_string(), "Bearer token".to_string()));

        let error = sanitize::validate_runtime_request(&request, &LeakDetector::new())
            .expect_err("sensitive header should be denied");

        assert_eq!(
            no_exposure_audit_code(&error),
            Some(NO_EXPOSURE_SENSITIVE_HEADER_DENIED_CODE)
        );
    }

    #[test]
    fn manual_credentials_producer_maps_to_audit_code() {
        let mut request = sample_request();
        request.url = "https://user:pass@api.example.test/v1/run".to_string();

        let error = sanitize::validate_runtime_request(&request, &LeakDetector::new())
            .expect_err("manual URL credentials should be denied");

        assert_eq!(
            no_exposure_audit_code(&error),
            Some(NO_EXPOSURE_MANUAL_CREDENTIALS_DENIED_CODE)
        );
    }

    #[test]
    fn request_leak_producer_maps_to_audit_code() {
        let mut request = sample_request();
        request.body = LEAKY_SECRET.as_bytes().to_vec();

        let error = sanitize::validate_runtime_request(&request, &LeakDetector::new())
            .expect_err("credential-shaped request body should be blocked");

        assert_eq!(
            no_exposure_audit_code(&error),
            Some(NO_EXPOSURE_REQUEST_LEAK_BLOCKED_CODE)
        );
    }

    #[test]
    fn response_leak_producer_maps_to_audit_code() {
        let response = NetworkHttpResponse {
            status: 200,
            headers: vec![("x-note".to_string(), format!("leaked {LEAKY_SECRET}"))],
            body: br#"{"ok":true}"#.to_vec(),
            usage: NetworkUsage {
                request_bytes: 5,
                response_bytes: 11,
                resolved_ip: None,
            },
        };

        let error = sanitize::sanitize_runtime_response(response, &[], &LeakDetector::new())
            .expect_err("leaky response header should be blocked");

        assert_eq!(
            no_exposure_audit_code(&error),
            Some(NO_EXPOSURE_RESPONSE_LEAK_BLOCKED_CODE)
        );
    }

    #[test]
    fn non_no_exposure_errors_have_no_audit_code() {
        let error = RuntimeHttpEgressError::Network {
            reason: "network_policy_missing".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        };

        assert_eq!(no_exposure_audit_code(&error), None);
    }
}
