use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_capabilities::{
    CapabilityObligationHandler, CapabilityObligationPhase, CapabilityObligationRequest,
};
use ironclaw_host_api::{
    CapabilityId, CapabilitySet, ExecutionContext, ExtensionId, MountView, Obligation,
    ResourceEstimate, ResourceScope, RuntimeCredentialInjection, RuntimeCredentialSource,
    RuntimeCredentialTarget, RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse, RuntimeKind, SecretHandle, TrustClass,
};
use ironclaw_secrets::SecretMaterial;

use crate::obligations::{NetworkObligationPolicyStore, RuntimeSecretInjectionStore};

/// Canonical host-runtime one-shot secret material staging port.
///
/// This is for host-owned adapters that already hold trusted secret material
/// and need the shared runtime HTTP egress to inject it without exposing the
/// material through request headers.
#[derive(Clone)]
pub struct RuntimeSecretMaterialStager {
    secret_injection_store: Arc<RuntimeSecretInjectionStore>,
}

/// Alias for [`ironclaw_host_api::CredentialStageError`].
pub type RuntimeSecretStageError = ironclaw_host_api::CredentialStageError;

impl RuntimeSecretMaterialStager {
    pub(crate) fn new(secret_injection_store: Arc<RuntimeSecretInjectionStore>) -> Self {
        Self {
            secret_injection_store,
        }
    }

    pub async fn stage_secret_material_once(
        &self,
        target_scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
        material: SecretMaterial,
    ) -> Result<(), RuntimeSecretStageError> {
        self.secret_injection_store
            .insert(target_scope, capability_id, handle, material)
            .map_err(|_| RuntimeSecretStageError::Backend)
    }

    fn discard_secret_material_for_capability(
        &self,
        target_scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Result<(), RuntimeHttpEgressError> {
        self.secret_injection_store
            .discard_for_capability(target_scope, capability_id)
            .map_err(|_| RuntimeHttpEgressError::Credential {
                reason: "host-staged credential cleanup failed".to_string(),
            })
    }
}

#[derive(Clone)]
pub struct HostRuntimeHttpEgressPort {
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    obligation_handler: Arc<dyn CapabilityObligationHandler>,
    network_policies: Arc<NetworkObligationPolicyStore>,
    secret_stager: RuntimeSecretMaterialStager,
}

struct StagedEgressLease {
    network_policies: Arc<NetworkObligationPolicyStore>,
    secret_stager: RuntimeSecretMaterialStager,
    scope: ResourceScope,
    capability_id: CapabilityId,
    includes_secrets: bool,
    active: bool,
}

impl StagedEgressLease {
    fn new(
        network_policies: Arc<NetworkObligationPolicyStore>,
        secret_stager: RuntimeSecretMaterialStager,
        scope: ResourceScope,
        capability_id: CapabilityId,
        includes_secrets: bool,
    ) -> Self {
        Self {
            network_policies,
            secret_stager,
            scope,
            capability_id,
            includes_secrets,
            active: true,
        }
    }

    fn cleanup(&mut self) -> Result<(), RuntimeHttpEgressError> {
        if !self.active {
            return Ok(());
        }
        self.network_policies
            .discard_for_capability(&self.scope, &self.capability_id);
        let secret_result = self.includes_secrets.then(|| {
            self.secret_stager
                .discard_secret_material_for_capability(&self.scope, &self.capability_id)
        });
        self.active = false;
        secret_result.transpose().map(|_| ())
    }

    fn finish(
        mut self,
        result: Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let cleanup = self.cleanup();
        match (result, cleanup) {
            (Ok(response), Ok(())) => Ok(response),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(cleanup_error)) => {
                tracing::debug!(
                    error = ?cleanup_error,
                    capability_id = %self.capability_id,
                    "host-mediated HTTP egress cleanup also failed after request failure"
                );
                Err(error)
            }
        }
    }
}

impl Drop for StagedEgressLease {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            tracing::debug!(
                error = ?error,
                capability_id = %self.capability_id,
                "host-mediated HTTP egress cancellation cleanup failed"
            );
        }
    }
}

/// Runtime-HTTP view of [`HostRuntimeHttpEgressPort`] bound to one explicit
/// extension identity and trust class.
///
/// Product surfaces that initiate HTTP work outside capability dispatch use
/// this adapter so the host port stages the request's network obligation before
/// the canonical runtime egress service performs transport.
#[derive(Clone)]
pub struct BoundHostRuntimeHttpEgress {
    port: HostRuntimeHttpEgressPort,
    extension_id: ExtensionId,
    trust: TrustClass,
}

pub struct HostRuntimeHttpEgressRequest {
    pub extension_id: ExtensionId,
    pub trust: TrustClass,
    pub request: RuntimeHttpEgressRequest,
    pub credentials: Vec<HostRuntimeCredentialMaterial>,
}

pub struct HostRuntimeCredentialMaterial {
    pub handle: SecretHandle,
    pub material: SecretMaterial,
    pub target: RuntimeCredentialTarget,
    pub required: bool,
}

impl HostRuntimeHttpEgressPort {
    pub(crate) fn new(
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
        obligation_handler: Arc<dyn CapabilityObligationHandler>,
        network_policies: Arc<NetworkObligationPolicyStore>,
        secret_stager: RuntimeSecretMaterialStager,
    ) -> Self {
        Self {
            runtime_http_egress,
            obligation_handler,
            network_policies,
            secret_stager,
        }
    }

    pub fn bind(&self, extension_id: ExtensionId, trust: TrustClass) -> BoundHostRuntimeHttpEgress {
        BoundHostRuntimeHttpEgress {
            port: self.clone(),
            extension_id,
            trust,
        }
    }

    pub async fn execute(
        &self,
        mut request: HostRuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        if !request.request.credential_injections.is_empty() {
            return Err(RuntimeHttpEgressError::Credential {
                reason: "host-mediated HTTP egress does not accept caller-provided credential injections"
                    .to_string(),
            });
        }
        self.authorize_network_egress(&request).await?;
        let staged_credentials = !request.credentials.is_empty();
        let lease = StagedEgressLease::new(
            Arc::clone(&self.network_policies),
            self.secret_stager.clone(),
            request.request.scope.clone(),
            request.request.capability_id.clone(),
            staged_credentials,
        );
        if let Err(error) = self
            .stage_credentials(&mut request.request, request.credentials)
            .await
        {
            return lease.finish(Err(error));
        }
        let result = self.runtime_http_egress.execute(request.request).await;
        lease.finish(result)
    }

    async fn stage_credentials(
        &self,
        request: &mut RuntimeHttpEgressRequest,
        credentials: Vec<HostRuntimeCredentialMaterial>,
    ) -> Result<(), RuntimeHttpEgressError> {
        for credential in credentials {
            self.secret_stager
                .stage_secret_material_once(
                    &request.scope,
                    &request.capability_id,
                    &credential.handle,
                    credential.material,
                )
                .await
                .map_err(|_| RuntimeHttpEgressError::Credential {
                    reason: "host credential material could not be staged".to_string(),
                })?;
            request
                .credential_injections
                .push(RuntimeCredentialInjection {
                    handle: credential.handle,
                    source: RuntimeCredentialSource::StagedObligation {
                        capability_id: request.capability_id.clone(),
                    },
                    target: credential.target,
                    required: credential.required,
                });
        }
        Ok(())
    }

    async fn authorize_network_egress(
        &self,
        request: &HostRuntimeHttpEgressRequest,
    ) -> Result<(), RuntimeHttpEgressError> {
        let context = execution_context_for_host_http_egress(
            &request.request.scope,
            request.extension_id.clone(),
            request.request.runtime,
            request.trust,
        )?;
        let estimate = ResourceEstimate {
            network_egress_bytes: request.request.network_policy.max_egress_bytes,
            ..ResourceEstimate::default()
        };
        self.obligation_handler
            .satisfy(CapabilityObligationRequest {
                phase: CapabilityObligationPhase::Invoke,
                context: &context,
                capability_id: &request.request.capability_id,
                estimate: &estimate,
                obligations: &[Obligation::ApplyNetworkPolicy {
                    policy: request.request.network_policy.clone(),
                }],
            })
            .await
            .map_err(|error| RuntimeHttpEgressError::Request {
                reason: format!("host network egress policy was not authorized: {error}"),
                request_bytes: 0,
                response_bytes: 0,
            })
    }
}

#[async_trait]
impl RuntimeHttpEgress for BoundHostRuntimeHttpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.port
            .execute(HostRuntimeHttpEgressRequest {
                extension_id: self.extension_id.clone(),
                trust: self.trust,
                request,
                credentials: Vec::new(),
            })
            .await
    }
}

fn execution_context_for_host_http_egress(
    scope: &ResourceScope,
    extension_id: ExtensionId,
    runtime: RuntimeKind,
    trust: TrustClass,
) -> Result<ExecutionContext, RuntimeHttpEgressError> {
    let context = ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: ironclaw_host_api::CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: scope.mission_id.clone(),
        thread_id: scope.thread_id.clone(),
        extension_id,
        runtime,
        trust,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope: scope.clone(),
    };
    context
        .validate()
        .map_err(|error| RuntimeHttpEgressError::Credential {
            reason: format!("invalid host HTTP egress context: {error}"),
        })?;
    Ok(context)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use ironclaw_capabilities::{
        CapabilityObligationError, CapabilityObligationFailureKind, CapabilityObligationRequest,
    };
    use ironclaw_host_api::{
        InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
        RuntimeHttpEgressResponse, UserId,
    };
    use ironclaw_network::{PolicyNetworkHttpEgress, ReqwestNetworkTransport};
    use ironclaw_secrets::InMemorySecretStore;

    use super::*;

    struct AllowObligations;

    #[async_trait]
    impl CapabilityObligationHandler for AllowObligations {
        async fn satisfy(
            &self,
            _request: CapabilityObligationRequest<'_>,
        ) -> Result<(), CapabilityObligationError> {
            Ok(())
        }
    }

    struct RecordingObligations {
        network_policies: Arc<NetworkObligationPolicyStore>,
        satisfy_calls: AtomicUsize,
    }

    #[async_trait]
    impl CapabilityObligationHandler for RecordingObligations {
        async fn satisfy(
            &self,
            request: CapabilityObligationRequest<'_>,
        ) -> Result<(), CapabilityObligationError> {
            self.satisfy_calls.fetch_add(1, Ordering::SeqCst);
            let policy = request
                .obligations
                .iter()
                .find_map(|obligation| match obligation {
                    Obligation::ApplyNetworkPolicy { policy } => Some(policy.clone()),
                    _ => None,
                });
            if let Some(policy) = policy {
                self.network_policies.insert(
                    &request.context.resource_scope,
                    request.capability_id,
                    policy,
                );
            }
            Ok(())
        }
    }

    struct DenyNetworkObligations;

    #[async_trait]
    impl CapabilityObligationHandler for DenyNetworkObligations {
        async fn satisfy(
            &self,
            _request: CapabilityObligationRequest<'_>,
        ) -> Result<(), CapabilityObligationError> {
            Err(CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Network,
            })
        }
    }

    struct RecordingRuntimeHttpEgress {
        calls: AtomicUsize,
        response: Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>,
    }

    impl RecordingRuntimeHttpEgress {
        fn ok() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                response: Ok(RuntimeHttpEgressResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: Vec::new(),
                    saved_body: None,
                    request_bytes: 0,
                    response_bytes: 0,
                    redaction_applied: false,
                }),
            }
        }

        fn failing() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                response: Err(RuntimeHttpEgressError::Network {
                    reason: "network_error".to_string(),
                    request_bytes: 17,
                    response_bytes: 0,
                }),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
        async fn execute(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.response.clone()
        }
    }

    struct BlockingRuntimeHttpEgress {
        started: tokio::sync::Notify,
        entered: AtomicBool,
    }

    impl BlockingRuntimeHttpEgress {
        fn new() -> Self {
            Self {
                started: tokio::sync::Notify::new(),
                entered: AtomicBool::new(false),
            }
        }

        async fn wait_until_started(&self) {
            if self.entered.load(Ordering::SeqCst) {
                return;
            }
            self.started.notified().await;
        }
    }

    #[async_trait]
    impl RuntimeHttpEgress for BlockingRuntimeHttpEgress {
        async fn execute(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            self.entered.store(true, Ordering::SeqCst);
            self.started.notify_waiters();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn host_runtime_http_egress_cancellation_drops_staged_policy_and_credentials() {
        let network_policies = Arc::new(NetworkObligationPolicyStore::new());
        let obligations = Arc::new(RecordingObligations {
            network_policies: Arc::clone(&network_policies),
            satisfy_calls: AtomicUsize::new(0),
        });
        let secret_injections = secret_store();
        let blocking = Arc::new(BlockingRuntimeHttpEgress::new());
        let port = HostRuntimeHttpEgressPort::new(
            blocking.clone(),
            obligations,
            Arc::clone(&network_policies),
            RuntimeSecretMaterialStager::new(Arc::clone(&secret_injections)),
        );
        let mut request = host_request();
        let scope = request.request.scope.clone();
        let capability_id = request.request.capability_id.clone();
        let handle = secret_handle();
        request.credentials.push(HostRuntimeCredentialMaterial {
            handle: handle.clone(),
            material: SecretMaterial::from("cancellation-secret"),
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        });

        let task = tokio::spawn(async move { port.execute(request).await });
        blocking.wait_until_started().await;
        assert!(network_policies.contains(&scope, &capability_id));
        assert!(
            secret_injections
                .clone_material(&scope, &capability_id, &handle)
                .expect("staged secret store is readable")
                .is_some()
        );

        task.abort();
        let cancelled = task.await.expect_err("request task is cancelled");
        assert!(cancelled.is_cancelled());
        assert!(!network_policies.contains(&scope, &capability_id));
        assert!(
            secret_injections
                .clone_material(&scope, &capability_id, &handle)
                .expect("staged secret store is readable after cancellation")
                .is_none()
        );
    }

    #[tokio::test]
    async fn bound_host_runtime_http_egress_rejects_target_outside_exact_staged_policy() {
        let network_policies = Arc::new(NetworkObligationPolicyStore::new());
        let obligations = Arc::new(RecordingObligations {
            network_policies: Arc::clone(&network_policies),
            satisfy_calls: AtomicUsize::new(0),
        });
        let secret_injections = secret_store();
        let canonical_egress = Arc::new(crate::HostHttpEgressService::production(
            PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::new(
                std::time::Duration::from_secs(1),
            )),
            InMemorySecretStore::new(),
            Arc::clone(&network_policies),
            Arc::clone(&secret_injections),
            Arc::new(crate::http_body::UnsupportedRuntimeHttpBodyStore),
        ));
        let bound = HostRuntimeHttpEgressPort::new(
            canonical_egress,
            obligations,
            Arc::clone(&network_policies),
            RuntimeSecretMaterialStager::new(secret_injections),
        )
        .bind(
            ExtensionId::new("registered-provider").expect("extension id"),
            TrustClass::Sandbox,
        );
        let mut request = host_request().request;
        let scope = request.scope.clone();
        let capability_id = request.capability_id.clone();
        request.url = "https://different.example.test/mcp".to_string();

        let error = bound
            .execute(request)
            .await
            .expect_err("different host must fail before transport");

        assert!(
            matches!(error, RuntimeHttpEgressError::Network { ref reason, .. } if reason.contains("policy")),
            "unexpected failure: {error:?}"
        );
        assert!(!network_policies.contains(&scope, &capability_id));
    }

    #[tokio::test]
    async fn bound_host_runtime_http_egress_cleans_network_policy_on_success_and_failure() {
        let network_policies = Arc::new(NetworkObligationPolicyStore::new());
        let obligations = Arc::new(RecordingObligations {
            network_policies: Arc::clone(&network_policies),
            satisfy_calls: AtomicUsize::new(0),
        });
        let success = HostRuntimeHttpEgressPort::new(
            Arc::new(RecordingRuntimeHttpEgress::ok()),
            obligations.clone(),
            Arc::clone(&network_policies),
            RuntimeSecretMaterialStager::new(secret_store()),
        )
        .bind(
            ExtensionId::new("test_extension").expect("extension id"),
            TrustClass::Sandbox,
        );
        let success_request = host_request().request;
        let success_scope = success_request.scope.clone();
        let success_capability = success_request.capability_id.clone();
        success
            .execute(success_request)
            .await
            .expect("bound host egress succeeds");
        assert!(!network_policies.contains(&success_scope, &success_capability));

        let failure = HostRuntimeHttpEgressPort::new(
            Arc::new(RecordingRuntimeHttpEgress::failing()),
            obligations.clone(),
            Arc::clone(&network_policies),
            RuntimeSecretMaterialStager::new(secret_store()),
        )
        .bind(
            ExtensionId::new("test_extension").expect("extension id"),
            TrustClass::Sandbox,
        );
        let failure_request = host_request().request;
        let failure_scope = failure_request.scope.clone();
        let failure_capability = failure_request.capability_id.clone();
        failure
            .execute(failure_request)
            .await
            .expect_err("transport failure remains visible");
        assert!(!network_policies.contains(&failure_scope, &failure_capability));

        assert_eq!(obligations.satisfy_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn host_runtime_http_egress_rejects_caller_provided_credential_injections() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::ok());
        let port = host_port(egress.clone(), Arc::new(AllowObligations), secret_store());
        let mut request = host_request();
        request
            .request
            .credential_injections
            .push(RuntimeCredentialInjection {
                handle: secret_handle(),
                source: RuntimeCredentialSource::StagedObligation {
                    capability_id: capability_id(),
                },
                target: RuntimeCredentialTarget::Header {
                    name: "authorization".to_string(),
                    prefix: Some("Bearer ".to_string()),
                },
                required: true,
            });

        let error = port
            .execute(request)
            .await
            .expect_err("caller-provided credential injections must be rejected");

        assert!(matches!(error, RuntimeHttpEgressError::Credential { .. }));
        assert_eq!(egress.calls(), 0);
    }

    #[tokio::test]
    async fn host_runtime_http_egress_maps_network_policy_denial_to_request_error() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::ok());
        let port = host_port(
            egress.clone(),
            Arc::new(DenyNetworkObligations),
            secret_store(),
        );

        let error = port
            .execute(host_request())
            .await
            .expect_err("policy denial must fail before runtime egress");

        assert!(matches!(error, RuntimeHttpEgressError::Request { .. }));
        assert_eq!(error.stable_runtime_reason(), "request_denied");
        assert_eq!(egress.calls(), 0);
    }

    #[tokio::test]
    async fn host_runtime_http_egress_discards_staged_secret_after_delegate_failure() {
        let store = secret_store();
        let egress = Arc::new(RecordingRuntimeHttpEgress::failing());
        let port = host_port(egress, Arc::new(AllowObligations), Arc::clone(&store));
        let mut request = host_request();
        let scope = request.request.scope.clone();
        let capability_id = request.request.capability_id.clone();
        let handle = secret_handle();
        request.credentials.push(HostRuntimeCredentialMaterial {
            handle: handle.clone(),
            material: SecretMaterial::from("host-held-token"),
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        });

        let error = port
            .execute(request)
            .await
            .expect_err("delegate egress failure should bubble");

        assert!(matches!(error, RuntimeHttpEgressError::Network { .. }));
        assert!(
            store
                .take(&scope, &capability_id, &handle)
                .expect("staged secret store should be readable")
                .is_none(),
            "host-staged material should be discarded after delegate failure"
        );
    }

    fn host_port(
        egress: Arc<dyn RuntimeHttpEgress>,
        obligations: Arc<dyn CapabilityObligationHandler>,
        store: Arc<RuntimeSecretInjectionStore>,
    ) -> HostRuntimeHttpEgressPort {
        HostRuntimeHttpEgressPort::new(
            egress,
            obligations,
            Arc::new(NetworkObligationPolicyStore::new()),
            RuntimeSecretMaterialStager::new(store),
        )
    }

    fn host_request() -> HostRuntimeHttpEgressRequest {
        HostRuntimeHttpEgressRequest {
            extension_id: ExtensionId::new("test_extension").expect("extension id"),
            trust: TrustClass::System,
            request: RuntimeHttpEgressRequest {
                runtime: RuntimeKind::FirstParty,
                scope: scope(),
                capability_id: capability_id(),
                method: NetworkMethod::Get,
                url: "https://api.example.test/v1".to_string(),
                headers: Vec::new(),
                body: Vec::new(),
                network_policy: network_policy(),
                credential_injections: Vec::new(),
                response_body_limit: None,
                save_body_to: None,
                timeout_ms: None,
            },
            credentials: Vec::new(),
        }
    }

    fn scope() -> ResourceScope {
        ResourceScope::local_default(
            UserId::new("user:test").expect("user id"),
            InvocationId::new(),
        )
        .expect("scope")
    }

    fn capability_id() -> CapabilityId {
        CapabilityId::new("test.host_http").expect("capability id")
    }

    fn secret_handle() -> SecretHandle {
        SecretHandle::new("host-held-token").expect("secret handle")
    }

    fn secret_store() -> Arc<RuntimeSecretInjectionStore> {
        Arc::new(RuntimeSecretInjectionStore::new())
    }

    fn network_policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.example.test".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(1024),
        }
    }
}
