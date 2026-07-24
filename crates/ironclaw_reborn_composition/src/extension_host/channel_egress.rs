//! Channel egress transport: executes policy-approved channel vendor calls
//! (`ironclaw_extension_host::egress::ApprovedChannelEgress`) over the host
//! runtime HTTP egress — secret-material resolution, declared credential
//! injection (header / query / path placeholder), SSRF-safe transport with
//! the network policy pinned to the approved host, and response caps.
//!
//! Policy (allowlisting, header screening, handle equality) already ran in
//! `ironclaw_extension_host`; this module only resolves material and drives
//! the transport. Adapters never see secret bytes.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_host::egress::{ApprovedChannelEgress, ChannelEgressTransport};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, NetworkPolicy, NetworkScheme, NetworkTargetPattern,
    ResourceScope, RestrictedEgressError, RestrictedEgressResponse, RuntimeHttpEgressReasonCode,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle, TrustClass,
};
use ironclaw_host_runtime::{
    HostRuntimeCredentialMaterial, HostRuntimeHttpEgressPort, HostRuntimeHttpEgressRequest,
};
use ironclaw_secrets::SecretMaterial;
#[cfg(test)]
use ironclaw_secrets::SecretStorePort;

/// Fixed capability id channel vendor calls are attributed to in egress
/// events/audit (mirrors the retiring per-vendor egress capability ids).
const CHANNEL_EGRESS_CAPABILITY_ID: &str = "channel.egress";

/// Resolves secret material for a channel-declared credential handle.
///
/// The generic implementation reads the scoped secret store (where
/// administrator secret fields are stored under their handles); bridges
/// for legacy per-vendor setup storage implement the same port until their
/// storage migrates (P6).
#[async_trait]
pub(crate) trait ChannelEgressCredentialsPort: Send + Sync {
    async fn channel_secret(
        &self,
        extension_id: &str,
        installation_id: &str,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, ChannelEgressCredentialError>;
}

/// Typed credential-resolution failures (never carry secret material).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ChannelEgressCredentialError {
    #[error("channel credential backend unavailable")]
    Unavailable,
}

/// Generic credentials port over the scoped secret store.
#[cfg(test)]
pub(crate) struct SecretStoreChannelEgressCredentials {
    store: Arc<dyn SecretStorePort>,
    scope_template: ResourceScope,
}

#[cfg(test)]
impl SecretStoreChannelEgressCredentials {
    pub(crate) fn new(store: Arc<dyn SecretStorePort>, scope_template: ResourceScope) -> Self {
        Self {
            store,
            scope_template,
        }
    }
}

#[async_trait]
#[cfg(test)]
impl ChannelEgressCredentialsPort for SecretStoreChannelEgressCredentials {
    async fn channel_secret(
        &self,
        _extension_id: &str,
        _installation_id: &str,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, ChannelEgressCredentialError> {
        let lease = match self.store.lease_once(&self.scope_template, handle).await {
            Ok(lease) => lease,
            Err(_) => return Ok(None),
        };
        match self.store.consume(&self.scope_template, lease.id).await {
            Ok(material) => Ok(Some(material)),
            Err(_) => Err(ChannelEgressCredentialError::Unavailable),
        }
    }
}

/// Production credential bridge over the same effective configuration
/// resolver used by setup, OAuth, activation, pairing, and ingress.
pub(crate) struct AdminConfigurationEgressCredentials {
    admin_configuration_resolver: Arc<
        crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver,
    >,
}

impl AdminConfigurationEgressCredentials {
    pub(crate) fn new(
        admin_configuration_resolver: Arc<
            crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver,
        >,
    ) -> Self {
        Self {
            admin_configuration_resolver,
        }
    }
}

#[async_trait]
impl ChannelEgressCredentialsPort for AdminConfigurationEgressCredentials {
    async fn channel_secret(
        &self,
        extension_id: &str,
        _installation_id: &str,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, ChannelEgressCredentialError> {
        let extension_id = ExtensionId::new(extension_id)
            .map_err(|_| ChannelEgressCredentialError::Unavailable)?;
        self.admin_configuration_resolver
            .secret_material(&extension_id, handle)
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "effective channel egress credential unavailable");
                ChannelEgressCredentialError::Unavailable
            })
    }
}

/// Wraps the generic scoped-store credentials port with a registration seam
/// that integration proofs use to inject a static `(extension, handle) →
/// material` mapping ahead of the store — standing in for administrator
/// configuration
/// secret storage until the configure surface lands (P6/H). The registration
/// mechanism (`bridges` + [`register`](Self::register)) is `test-support`
/// only; a production build never registers a bridge and resolves straight
/// through `fallback`.
#[cfg(feature = "test-support")]
pub(crate) struct BridgedChannelEgressCredentials {
    bridges: std::sync::RwLock<Vec<Arc<dyn ChannelEgressCredentialsPort>>>,
    fallback: Arc<dyn ChannelEgressCredentialsPort>,
}

#[cfg(feature = "test-support")]
impl BridgedChannelEgressCredentials {
    pub(crate) fn new(fallback: Arc<dyn ChannelEgressCredentialsPort>) -> Self {
        Self {
            bridges: std::sync::RwLock::new(Vec::new()),
            fallback,
        }
    }

    pub(crate) fn register(&self, bridge: Arc<dyn ChannelEgressCredentialsPort>) {
        self.bridges
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(bridge);
    }
}

#[cfg(feature = "test-support")]
#[async_trait]
impl ChannelEgressCredentialsPort for BridgedChannelEgressCredentials {
    async fn channel_secret(
        &self,
        extension_id: &str,
        installation_id: &str,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, ChannelEgressCredentialError> {
        let bridges: Vec<Arc<dyn ChannelEgressCredentialsPort>> = self
            .bridges
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        for bridge in bridges {
            if let Some(material) = bridge
                .channel_secret(extension_id, installation_id, handle)
                .await?
            {
                return Ok(Some(material));
            }
        }
        self.fallback
            .channel_secret(extension_id, installation_id, handle)
            .await
    }
}

/// Fixed `(extension_id, handle) → material` mapping, registered as a bridge
/// by integration proofs standing in for administrator secret storage
/// until the configure surface lands (P6/H). Test-support only.
#[cfg(feature = "test-support")]
pub(crate) struct StaticChannelEgressCredentials {
    entries: Vec<(String, String, SecretMaterial)>,
}

#[cfg(feature = "test-support")]
impl StaticChannelEgressCredentials {
    pub(crate) fn new(entries: Vec<(String, String, SecretMaterial)>) -> Self {
        Self { entries }
    }
}

#[cfg(feature = "test-support")]
#[async_trait]
impl ChannelEgressCredentialsPort for StaticChannelEgressCredentials {
    async fn channel_secret(
        &self,
        extension_id: &str,
        _installation_id: &str,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, ChannelEgressCredentialError> {
        Ok(self
            .entries
            .iter()
            .find(|(extension, entry_handle, _)| {
                extension == extension_id && entry_handle == handle.as_str()
            })
            .map(|(_, _, material)| material.clone()))
    }
}

/// The production [`ChannelEgressTransport`]: host runtime egress with the
/// network policy pinned to the approved host.
pub(crate) struct HostRuntimeChannelEgressTransport {
    host_egress: HostRuntimeHttpEgressPort,
    credentials: Arc<dyn ChannelEgressCredentialsPort>,
    scope_template: ResourceScope,
}

impl HostRuntimeChannelEgressTransport {
    pub(crate) fn new(
        host_egress: HostRuntimeHttpEgressPort,
        credentials: Arc<dyn ChannelEgressCredentialsPort>,
        scope_template: ResourceScope,
    ) -> Self {
        Self {
            host_egress,
            credentials,
            scope_template,
        }
    }

    fn request_scope(&self) -> ResourceScope {
        let mut scope = self.scope_template.clone();
        scope.invocation_id = InvocationId::new();
        scope
    }
}

#[async_trait]
impl ChannelEgressTransport for HostRuntimeChannelEgressTransport {
    async fn execute(
        &self,
        approved: ApprovedChannelEgress,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        let extension_id = ExtensionId::new(&approved.extension_id).map_err(|_| {
            RestrictedEgressError::Transport {
                reason: "invalid extension id for channel egress".to_string(),
            }
        })?;
        let capability_id = CapabilityId::new(CHANNEL_EGRESS_CAPABILITY_ID).map_err(|_| {
            RestrictedEgressError::Transport {
                reason: "invalid channel egress capability id".to_string(),
            }
        })?;

        let mut credentials = Vec::new();
        // The primary declared credential plus every declared body credential
        // this call opted into resolve through the same port and ride the
        // same host-runtime injection path; each is required, fail-closed.
        let approved_credentials = approved
            .credential
            .iter()
            .chain(approved.body_credentials.iter());
        for credential in approved_credentials {
            let material = self
                .credentials
                .channel_secret(
                    &approved.extension_id,
                    &approved.installation_id,
                    &credential.handle,
                )
                .await
                .map_err(|error| RestrictedEgressError::Transport {
                    reason: error.to_string(),
                })?;
            let Some(material) = material else {
                return Err(RestrictedEgressError::AuthRequired {
                    required_secrets: vec![credential.handle.clone()],
                    credential_requirements: Vec::new(),
                });
            };
            credentials.push(HostRuntimeCredentialMaterial {
                handle: credential.handle.clone(),
                material,
                target: credential.target.clone(),
                required: true,
            });
        }

        let response = self
            .host_egress
            .execute(HostRuntimeHttpEgressRequest {
                extension_id,
                trust: TrustClass::System,
                request: RuntimeHttpEgressRequest {
                    runtime: RuntimeKind::FirstParty,
                    scope: self.request_scope(),
                    capability_id,
                    method: approved.method,
                    url: approved.url,
                    headers: approved.headers,
                    body: approved.body,
                    network_policy: NetworkPolicy {
                        allowed_targets: vec![NetworkTargetPattern {
                            scheme: Some(NetworkScheme::Https),
                            host_pattern: approved.host,
                            port: None,
                        }],
                        deny_private_ip_ranges: true,
                        max_egress_bytes: None,
                    },
                    credential_injections: Vec::new(),
                    response_body_limit: Some(approved.response_body_limit),
                    save_body_to: None,
                    timeout_ms: Some(u32::try_from(approved.timeout_ms).unwrap_or(u32::MAX)),
                },
                credentials,
            })
            .await
            .map_err(map_runtime_http_error)?;

        Ok(RestrictedEgressResponse {
            status: response.status,
            body: response.body,
        })
    }
}

fn map_runtime_http_error(
    error: ironclaw_host_api::RuntimeHttpEgressError,
) -> RestrictedEgressError {
    match error.reason_code() {
        RuntimeHttpEgressReasonCode::PolicyDenied | RuntimeHttpEgressReasonCode::RequestDenied => {
            RestrictedEgressError::PolicyDenied
        }
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RestrictedEgressError::ResponseTooLarge
        }
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RestrictedEgressError::AuthRequired {
            required_secrets: Vec::new(),
            credential_requirements: Vec::new(),
        },
        RuntimeHttpEgressReasonCode::NetworkError | RuntimeHttpEgressReasonCode::ResponseError => {
            RestrictedEgressError::Transport {
                reason: error.stable_runtime_reason().to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_authorization::GrantAuthorizer;
    use ironclaw_extension_host::egress::{ApprovedChannelCredential, ApprovedChannelEgress};
    use ironclaw_extensions::ExtensionRegistry;
    use ironclaw_filesystem::DiskFilesystem;
    use ironclaw_host_api::{InvocationId, NetworkMethod, RuntimeCredentialTarget, UserId};
    use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
    use ironclaw_network::{
        NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
    };
    use ironclaw_processes::in_memory_backed_process_services;
    use ironclaw_resources::InMemoryResourceGovernor;
    use ironclaw_secrets::{SecretStore, SecretStorePort};
    use secrecy::SecretString;

    use super::*;

    struct RecordingNetworkHttpEgress {
        requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
        response: Result<NetworkHttpResponse, NetworkHttpError>,
    }

    impl RecordingNetworkHttpEgress {
        fn ok() -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                response: Ok(NetworkHttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: br#"{"ok":true}"#.to_vec(),
                    usage: NetworkUsage {
                        request_bytes: 0,
                        response_bytes: 11,
                        resolved_ip: None,
                    },
                }),
            }
        }

        fn requests(&self) -> Arc<Mutex<Vec<NetworkHttpRequest>>> {
            Arc::clone(&self.requests)
        }
    }

    #[async_trait]
    impl NetworkHttpEgress for RecordingNetworkHttpEgress {
        async fn execute(
            &self,
            request: NetworkHttpRequest,
        ) -> Result<NetworkHttpResponse, NetworkHttpError> {
            self.requests
                .lock()
                .expect("network HTTP requests lock")
                .push(request);
            self.response.clone()
        }
    }

    fn test_host_runtime_services() -> HostRuntimeServices<
        DiskFilesystem,
        InMemoryResourceGovernor,
        ironclaw_processes::ProcessStore<ironclaw_filesystem::InMemoryBackend>,
        ironclaw_processes::ProcessResultStore<ironclaw_filesystem::InMemoryBackend>,
    > {
        HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            Arc::new(DiskFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            in_memory_backed_process_services(),
            CapabilitySurfaceVersion::new("surface-v1").expect("surface version"),
        )
    }

    fn host_egress_port(
        network: RecordingNetworkHttpEgress,
    ) -> (
        HostRuntimeHttpEgressPort,
        Arc<Mutex<Vec<NetworkHttpRequest>>>,
    ) {
        let requests = network.requests();
        let services = test_host_runtime_services()
            .with_secret_store(Arc::new(SecretStore::ephemeral()))
            .try_with_host_http_egress(network)
            .expect("host HTTP egress should wire");
        let port = services
            .host_runtime_http_egress_port()
            .expect("host runtime HTTP egress port should be configured");
        (port, requests)
    }

    fn test_scope() -> ResourceScope {
        ResourceScope::local_default(
            UserId::new("channel-egress-user").unwrap(),
            InvocationId::new(),
        )
        .unwrap()
    }

    async fn seeded_credentials(
        scope: &ResourceScope,
        handle: &SecretHandle,
        value: &str,
    ) -> Arc<dyn ChannelEgressCredentialsPort> {
        let store = Arc::new(SecretStore::ephemeral());
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretString::from(value.to_string()),
                None,
            )
            .await
            .expect("seed channel secret");
        Arc::new(SecretStoreChannelEgressCredentials::new(
            store as Arc<dyn SecretStorePort>,
            scope.clone(),
        ))
    }

    fn approved(
        url: &str,
        host: &str,
        credential: Option<ApprovedChannelCredential>,
    ) -> ApprovedChannelEgress {
        ApprovedChannelEgress {
            extension_id: "vendorx".to_string(),
            installation_id: "inst-1".to_string(),
            method: NetworkMethod::Post,
            url: url.to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: b"{}".to_vec(),
            host: host.to_string(),
            credential,
            body_credentials: Vec::new(),
            response_body_limit: 64 * 1024,
            timeout_ms: 5_000,
        }
    }

    #[tokio::test]
    async fn header_injection_reaches_the_network_request() {
        let scope = test_scope();
        let handle = SecretHandle::new("vendor_bot_token").unwrap();
        let credentials = seeded_credentials(&scope, &handle, "xoxb-secret-token").await;
        let (port, requests) = host_egress_port(RecordingNetworkHttpEgress::ok());
        let transport = HostRuntimeChannelEgressTransport::new(port, credentials, scope);

        let response = transport
            .execute(approved(
                "https://vendor.example/api/chat.postMessage",
                "vendor.example",
                Some(ApprovedChannelCredential {
                    handle: handle.clone(),
                    target: RuntimeCredentialTarget::Header {
                        name: "authorization".to_string(),
                        prefix: Some("Bearer ".to_string()),
                    },
                }),
            ))
            .await
            .expect("transport executes");
        assert_eq!(response.status, 200);

        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].url,
            "https://vendor.example/api/chat.postMessage"
        );
        let authorization = recorded[0]
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .expect("authorization header injected host-side");
        assert_eq!(authorization.1, "Bearer xoxb-secret-token");
        // The network policy is pinned to the approved host with private-IP
        // denial.
        assert!(recorded[0].policy.deny_private_ip_ranges);
        assert_eq!(recorded[0].policy.allowed_targets.len(), 1);
        assert_eq!(
            recorded[0].policy.allowed_targets[0].host_pattern,
            "vendor.example"
        );
        assert_eq!(recorded[0].response_body_limit, Some(64 * 1024));
    }

    #[tokio::test]
    async fn path_placeholder_injection_substitutes_the_secret_host_side() {
        let scope = test_scope();
        let handle = SecretHandle::new("vendor_bot_token").unwrap();
        let credentials = seeded_credentials(&scope, &handle, "123456:telegram-token").await;
        let (port, requests) = host_egress_port(RecordingNetworkHttpEgress::ok());
        let transport = HostRuntimeChannelEgressTransport::new(port, credentials, scope);

        transport
            .execute(approved(
                "https://vendor.example/bot{token}/sendMessage",
                "vendor.example",
                Some(ApprovedChannelCredential {
                    handle,
                    target: RuntimeCredentialTarget::PathPlaceholder {
                        placeholder: "token".to_string(),
                    },
                }),
            ))
            .await
            .expect("transport executes");

        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(
            recorded[0].url, "https://vendor.example/bot123456:telegram-token/sendMessage",
            "the placeholder is substituted host-side; the adapter never saw the token"
        );
    }

    #[tokio::test]
    async fn body_json_pointer_credential_is_resolved_into_the_wire_body() {
        let scope = test_scope();
        let handle = SecretHandle::new("vendor_webhook_secret").unwrap();
        let credentials = seeded_credentials(&scope, &handle, "wh-sentinel-secret").await;
        let (port, requests) = host_egress_port(RecordingNetworkHttpEgress::ok());
        let transport = HostRuntimeChannelEgressTransport::new(port, credentials, scope);

        let mut plan = approved(
            "https://vendor.example/api/setWebhook",
            "vendor.example",
            None,
        );
        plan.body = br#"{"url":"https://hooks.example/updates"}"#.to_vec();
        plan.body_credentials = vec![ApprovedChannelCredential {
            handle,
            target: RuntimeCredentialTarget::BodyJsonPointer {
                pointer: "/secret_token".to_string(),
            },
        }];
        transport.execute(plan).await.expect("transport executes");

        let recorded = requests.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&recorded[0].body).unwrap();
        assert_eq!(
            body["secret_token"], "wh-sentinel-secret",
            "the resolved secret VALUE is inserted host-side; the adapter never saw it"
        );
        assert_eq!(body["url"], "https://hooks.example/updates");
    }

    #[tokio::test]
    async fn missing_secret_material_fails_closed_as_auth_required() {
        let scope = test_scope();
        let handle = SecretHandle::new("vendor_bot_token").unwrap();
        // Credentials port over an EMPTY store: no material seeded.
        let credentials: Arc<dyn ChannelEgressCredentialsPort> =
            Arc::new(SecretStoreChannelEgressCredentials::new(
                Arc::new(SecretStore::ephemeral()) as Arc<dyn SecretStorePort>,
                scope.clone(),
            ));
        let (port, requests) = host_egress_port(RecordingNetworkHttpEgress::ok());
        let transport = HostRuntimeChannelEgressTransport::new(port, credentials, scope);

        let error = transport
            .execute(approved(
                "https://vendor.example/api/x",
                "vendor.example",
                Some(ApprovedChannelCredential {
                    handle: handle.clone(),
                    target: RuntimeCredentialTarget::Header {
                        name: "authorization".to_string(),
                        prefix: Some("Bearer ".to_string()),
                    },
                }),
            ))
            .await
            .expect_err("missing material fails closed");
        match error {
            RestrictedEgressError::AuthRequired {
                required_secrets, ..
            } => assert_eq!(required_secrets, vec![handle]),
            other => panic!("expected AuthRequired, got {other:?}"),
        }
        assert!(
            requests.lock().unwrap().is_empty(),
            "no network activity without credential material"
        );
    }
}
