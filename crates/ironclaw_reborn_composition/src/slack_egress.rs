//! Host-mediated Slack protocol HTTP egress.
//!
//! The Slack adapter renders only a constrained `EgressRequest` containing the
//! declared host, origin-form path, headers, body, and opaque credential handle.
//! This module is the host side: it validates the request against the adapter's
//! declared egress policy, resolves the opaque handle to a bearer token, and
//! delegates authorization plus runtime credential injection to the shared host
//! HTTP egress port.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, NetworkMethod, NetworkPolicy, NetworkScheme,
    NetworkTargetPattern, ResourceScope, RuntimeCredentialTarget, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle, TrustClass,
};
use ironclaw_host_runtime::{
    HostRuntimeCredentialMaterial, HostRuntimeHttpEgressPort, HostRuntimeHttpEgressRequest,
};
use ironclaw_product_adapters::{
    EgressCredentialHandle, EgressRequest, EgressResponse, ProtocolHttpEgress,
    ProtocolHttpEgressError, RedactedString,
};
use ironclaw_secrets::SecretMaterial;
use ironclaw_wasm_product_adapters::{EgressPolicy, EgressPolicyError, EgressPolicyTarget};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

const SLACK_EGRESS_TIMEOUT_MS: u32 = 10_000;
const SLACK_EGRESS_RESPONSE_BODY_LIMIT_BYTES: u64 = 64 * 1024;
const SLACK_EGRESS_CAPABILITY_ID: &str = "slack.egress";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlackEgressCredentialError {
    #[error("unknown Slack egress credential handle {handle}")]
    UnknownHandle { handle: String },
    #[error("Slack egress credential handle {handle} is not authorized")]
    UnauthorizedHandle { handle: String },
    #[error("Slack egress credential backend unavailable")]
    Unavailable,
}

pub struct SlackEgressCredential {
    bearer_token: SecretString,
}

impl SlackEgressCredential {
    pub fn bearer_token(token: impl Into<String>) -> Self {
        Self {
            bearer_token: SecretString::from(token.into()),
        }
    }

    fn as_bearer_token(&self) -> &str {
        self.bearer_token.expose_secret()
    }
}

#[async_trait]
pub trait SlackEgressCredentialProvider: Send + Sync {
    async fn resolve_slack_egress_credential(
        &self,
        handle: &EgressCredentialHandle,
    ) -> Result<SlackEgressCredential, SlackEgressCredentialError>;
}

pub struct StaticSlackEgressCredentialProvider {
    handle: EgressCredentialHandle,
    credential: SlackEgressCredential,
}

impl StaticSlackEgressCredentialProvider {
    pub fn new(handle: EgressCredentialHandle, bearer_token: impl Into<String>) -> Self {
        Self {
            handle,
            credential: SlackEgressCredential::bearer_token(bearer_token),
        }
    }
}

#[async_trait]
impl SlackEgressCredentialProvider for StaticSlackEgressCredentialProvider {
    async fn resolve_slack_egress_credential(
        &self,
        handle: &EgressCredentialHandle,
    ) -> Result<SlackEgressCredential, SlackEgressCredentialError> {
        if handle == &self.handle {
            Ok(SlackEgressCredential::bearer_token(
                self.credential.as_bearer_token().to_string(),
            ))
        } else {
            Err(SlackEgressCredentialError::UnknownHandle {
                handle: handle.as_str().to_string(),
            })
        }
    }
}

pub struct SlackProtocolHttpEgress {
    host_egress: HostRuntimeHttpEgressPort,
    credentials: Arc<dyn SlackEgressCredentialProvider>,
    policy: EgressPolicy,
    scope_template: ResourceScope,
}

impl SlackProtocolHttpEgress {
    pub fn new(
        host_egress: HostRuntimeHttpEgressPort,
        credentials: Arc<dyn SlackEgressCredentialProvider>,
        policy: EgressPolicy,
        scope_template: ResourceScope,
    ) -> Self {
        Self {
            host_egress,
            credentials,
            policy,
            scope_template,
        }
    }
}

#[async_trait]
impl ProtocolHttpEgress for SlackProtocolHttpEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        self.policy
            .check(EgressPolicyTarget {
                host: request.host(),
                credential_handle: request.credential_handle(),
            })
            .map_err(map_egress_policy_error)?;

        if request
            .headers()
            .iter()
            .any(|header| header.name().eq_ignore_ascii_case("authorization"))
        {
            return Err(ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(
                    "Slack adapter requests must use credential handles, not Authorization headers",
                ),
            });
        }
        let headers = request
            .headers()
            .iter()
            .map(|header| (header.name().to_string(), header.value().to_string()))
            .collect::<Vec<_>>();

        let capability_id = CapabilityId::new(SLACK_EGRESS_CAPABILITY_ID).map_err(|error| {
            ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(format!("invalid Slack egress capability id: {error}")),
            }
        })?;
        let credentials = self
            .credential_material(request.credential_handle())
            .await?;
        let scope = self.request_scope();
        let response = self
            .host_egress
            .execute(HostRuntimeHttpEgressRequest {
                extension_id: slack_extension_id()?,
                trust: TrustClass::System,
                request: RuntimeHttpEgressRequest {
                    runtime: RuntimeKind::FirstParty,
                    scope,
                    capability_id,
                    method: network_method(request.method().as_str())?,
                    url: format!(
                        "https://{}{}",
                        request.host().as_str(),
                        request.path().as_str()
                    ),
                    headers,
                    body: request.body().to_vec(),
                    network_policy: slack_network_policy(request.host().as_str()),
                    credential_injections: Vec::new(),
                    response_body_limit: Some(SLACK_EGRESS_RESPONSE_BODY_LIMIT_BYTES),
                    save_body_to: None,
                    timeout_ms: Some(SLACK_EGRESS_TIMEOUT_MS),
                },
                credentials,
            })
            .await
            .map_err(map_runtime_http_error)?;

        Ok(EgressResponse::new(response.status, response.body))
    }
}

impl SlackProtocolHttpEgress {
    fn request_scope(&self) -> ResourceScope {
        let mut scope = self.scope_template.clone();
        scope.invocation_id = InvocationId::new();
        scope
    }

    async fn credential_material(
        &self,
        handle: Option<&EgressCredentialHandle>,
    ) -> Result<Vec<HostRuntimeCredentialMaterial>, ProtocolHttpEgressError> {
        let Some(handle) = handle else {
            return Ok(Vec::new());
        };
        let credential = self
            .credentials
            .resolve_slack_egress_credential(handle)
            .await
            .map_err(map_credential_error)?;
        validate_bearer_token(&credential)?;
        let secret_handle = SecretHandle::new(handle.as_str()).map_err(|error| {
            ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(format!(
                    "invalid Slack egress credential handle: {error}"
                )),
            }
        })?;
        Ok(vec![HostRuntimeCredentialMaterial {
            handle: secret_handle,
            material: SecretMaterial::from(credential.as_bearer_token().to_string()),
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            required: true,
        }])
    }
}

fn validate_bearer_token(
    credential: &SlackEgressCredential,
) -> Result<(), ProtocolHttpEgressError> {
    let token = credential.as_bearer_token();
    if token.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return Err(ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new("Slack bearer token contains control characters"),
        });
    }
    Ok(())
}

fn slack_network_policy(host: &str) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: host.to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

fn slack_extension_id() -> Result<ExtensionId, ProtocolHttpEgressError> {
    ExtensionId::new("ironclaw_slack").map_err(|error| ProtocolHttpEgressError::PolicyDenied {
        reason: RedactedString::new(format!("invalid Slack egress extension id: {error}")),
    })
}

fn network_method(method: &str) -> Result<NetworkMethod, ProtocolHttpEgressError> {
    match method {
        "GET" => Ok(NetworkMethod::Get),
        "POST" => Ok(NetworkMethod::Post),
        "PUT" => Ok(NetworkMethod::Put),
        "PATCH" => Ok(NetworkMethod::Patch),
        "DELETE" => Ok(NetworkMethod::Delete),
        _ => Err(ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new("unsupported Slack egress HTTP method"),
        }),
    }
}

fn map_egress_policy_error(error: EgressPolicyError) -> ProtocolHttpEgressError {
    match error {
        EgressPolicyError::UndeclaredHost { host } => ProtocolHttpEgressError::UndeclaredHost {
            host: host.as_str().to_string(),
        },
        EgressPolicyError::UnauthorizedCredentialHandle { handle }
        | EgressPolicyError::CredentialHandleNotPairedWithHost { handle, .. } => {
            ProtocolHttpEgressError::UnauthorizedCredentialHandle {
                handle: handle.as_str().to_string(),
            }
        }
        EgressPolicyError::UnauthenticatedEgressNotDeclared { .. } => {
            ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new("unauthenticated Slack egress is not declared"),
            }
        }
    }
}

fn map_credential_error(error: SlackEgressCredentialError) -> ProtocolHttpEgressError {
    match error {
        SlackEgressCredentialError::UnknownHandle { handle } => {
            ProtocolHttpEgressError::UnknownCredentialHandle { handle }
        }
        SlackEgressCredentialError::UnauthorizedHandle { handle } => {
            ProtocolHttpEgressError::UnauthorizedCredentialHandle { handle }
        }
        SlackEgressCredentialError::Unavailable => ProtocolHttpEgressError::Network(
            RedactedString::new("Slack credential backend unavailable"),
        ),
    }
}

fn map_runtime_http_error(error: RuntimeHttpEgressError) -> ProtocolHttpEgressError {
    match error.reason_code() {
        ironclaw_host_api::RuntimeHttpEgressReasonCode::PolicyDenied
        | ironclaw_host_api::RuntimeHttpEgressReasonCode::RequestDenied => {
            ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(error.stable_runtime_reason()),
            }
        }
        ironclaw_host_api::RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            ProtocolHttpEgressError::LeakDetected
        }
        ironclaw_host_api::RuntimeHttpEgressReasonCode::CredentialUnavailable
        | ironclaw_host_api::RuntimeHttpEgressReasonCode::NetworkError
        | ironclaw_host_api::RuntimeHttpEgressReasonCode::ResponseError => {
            ProtocolHttpEgressError::Network(RedactedString::new(error.stable_runtime_reason()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_authorization::GrantAuthorizer;
    use ironclaw_extensions::ExtensionRegistry;
    use ironclaw_filesystem::LocalFilesystem;
    use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
    use ironclaw_network::{
        NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
    };
    use ironclaw_processes::{InMemoryProcessResultStore, InMemoryProcessStore, ProcessServices};
    use ironclaw_product_adapters::{
        DeclaredEgressHost, DeclaredEgressTarget, EgressCredentialHandle, EgressMethod, EgressPath,
    };
    use ironclaw_resources::InMemoryResourceGovernor;
    use ironclaw_secrets::InMemorySecretStore;

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
                    body: br#"{\"ok\":true}"#.to_vec(),
                    usage: NetworkUsage {
                        request_bytes: 0,
                        response_bytes: 11,
                        resolved_ip: None,
                    },
                }),
            }
        }

        fn failing(error: NetworkHttpError) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                response: Err(error),
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

    fn host_egress_port(
        network: RecordingNetworkHttpEgress,
    ) -> (
        HostRuntimeHttpEgressPort,
        Arc<Mutex<Vec<NetworkHttpRequest>>>,
    ) {
        let requests = network.requests();
        let services = test_host_runtime_services()
            .with_secret_store(Arc::new(InMemorySecretStore::new()))
            .try_with_host_http_egress(network)
            .expect("host HTTP egress should wire");
        let port = services
            .host_runtime_http_egress_port()
            .expect("host runtime HTTP egress port should be configured");
        (port, requests)
    }

    fn test_host_runtime_services() -> HostRuntimeServices<
        LocalFilesystem,
        InMemoryResourceGovernor,
        InMemoryProcessStore,
        InMemoryProcessResultStore,
    > {
        HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            Arc::new(LocalFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").expect("surface version"),
        )
    }

    fn slack_host() -> DeclaredEgressHost {
        DeclaredEgressHost::new("slack.com").expect("slack host")
    }

    fn slack_handle() -> EgressCredentialHandle {
        EgressCredentialHandle::new("slack_bot_token").expect("slack handle")
    }

    fn slack_request(handle: EgressCredentialHandle) -> EgressRequest {
        EgressRequest::new(
            slack_host(),
            EgressMethod::post(),
            EgressPath::new("/api/chat.postMessage").expect("slack path"),
        )
        .with_body(br#"{"channel":"D1","text":"hi"}"#.to_vec())
        .with_credential_handle(Some(handle))
    }

    fn slack_egress_with_network(
        network: RecordingNetworkHttpEgress,
    ) -> (SlackProtocolHttpEgress, Arc<Mutex<Vec<NetworkHttpRequest>>>) {
        let (host_egress, requests) = host_egress_port(network);
        let handle = slack_handle();
        let egress = SlackProtocolHttpEgress::new(
            host_egress,
            Arc::new(StaticSlackEgressCredentialProvider::new(
                handle.clone(),
                "xoxb-secret",
            )),
            EgressPolicy::new([DeclaredEgressTarget::new(slack_host(), Some(handle))]),
            ResourceScope::system(),
        );
        (egress, requests)
    }

    #[tokio::test]
    async fn slack_protocol_http_egress_validates_policy_and_host_injects_bearer() {
        let network = RecordingNetworkHttpEgress::ok();
        let recorded_requests = network.requests();
        let (host_egress, _) = host_egress_port(network);
        let handle = slack_handle();
        let egress = SlackProtocolHttpEgress::new(
            host_egress,
            Arc::new(StaticSlackEgressCredentialProvider::new(
                handle.clone(),
                "xoxb-secret",
            )),
            EgressPolicy::new([DeclaredEgressTarget::new(
                slack_host(),
                Some(handle.clone()),
            )]),
            ResourceScope::system(),
        );

        let response = egress
            .send(slack_request(handle))
            .await
            .expect("slack egress should succeed");

        assert_eq!(response.status(), 200);
        let requests = recorded_requests.lock().expect("network requests lock");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://slack.com/api/chat.postMessage");
        assert_eq!(requests[0].method, NetworkMethod::Post);
        assert_eq!(requests[0].body, br#"{"channel":"D1","text":"hi"}"#);
        assert_eq!(
            requests[0].policy.allowed_targets[0].host_pattern,
            "slack.com"
        );
        assert_eq!(
            requests[0]
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization")),
            Some(&(
                "authorization".to_string(),
                "Bearer xoxb-secret".to_string()
            ))
        );
    }

    #[tokio::test]
    async fn slack_protocol_http_egress_uses_fresh_invocation_scope_per_send() {
        let (egress, recorded_requests) =
            slack_egress_with_network(RecordingNetworkHttpEgress::ok());

        egress
            .send(slack_request(slack_handle()))
            .await
            .expect("first Slack egress should succeed");
        egress
            .send(slack_request(slack_handle()))
            .await
            .expect("second Slack egress should succeed");

        let requests = recorded_requests.lock().expect("network requests lock");
        assert_eq!(requests.len(), 2);
        assert_ne!(
            requests[0].scope.invocation_id, requests[1].scope.invocation_id,
            "each Slack protocol egress call must stage credentials in a per-request invocation scope"
        );
    }

    #[tokio::test]
    async fn slack_protocol_http_egress_rejects_control_chars_in_bearer_before_network() {
        let network = RecordingNetworkHttpEgress::ok();
        let recorded_requests = network.requests();
        let (host_egress, _) = host_egress_port(network);
        let handle = slack_handle();
        let egress = SlackProtocolHttpEgress::new(
            host_egress,
            Arc::new(StaticSlackEgressCredentialProvider::new(
                handle.clone(),
                "xoxb-secret\r\nX-Injected: true",
            )),
            EgressPolicy::new([DeclaredEgressTarget::new(
                slack_host(),
                Some(handle.clone()),
            )]),
            ResourceScope::system(),
        );

        let error = egress
            .send(slack_request(handle))
            .await
            .expect_err("invalid bearer token should fail before network");

        assert!(matches!(
            error,
            ProtocolHttpEgressError::PolicyDenied { .. }
        ));
        assert!(
            recorded_requests
                .lock()
                .expect("network requests lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn slack_protocol_http_egress_rejects_unknown_handle_before_network() {
        let network = RecordingNetworkHttpEgress::ok();
        let recorded_requests = network.requests();
        let (host_egress, _) = host_egress_port(network);
        let unknown = EgressCredentialHandle::new("other_token").expect("other handle");
        let egress = SlackProtocolHttpEgress::new(
            host_egress,
            Arc::new(StaticSlackEgressCredentialProvider::new(
                slack_handle(),
                "xoxb-secret",
            )),
            EgressPolicy::new([DeclaredEgressTarget::new(
                slack_host(),
                Some(unknown.clone()),
            )]),
            ResourceScope::system(),
        );

        let error = egress
            .send(slack_request(unknown))
            .await
            .expect_err("unknown handle should fail before network");

        assert!(matches!(
            error,
            ProtocolHttpEgressError::UnknownCredentialHandle { .. }
        ));
        assert!(
            recorded_requests
                .lock()
                .expect("network requests lock")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn slack_protocol_http_egress_maps_runtime_http_failures() {
        let cases = [
            (
                NetworkHttpError::InvalidUrl {
                    reason: "invalid_url".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                "request-denied",
                RuntimeErrorExpectation::Network,
            ),
            (
                NetworkHttpError::PolicyDenied {
                    reason: "policy_denied".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                "policy-denied",
                RuntimeErrorExpectation::PolicyDenied,
            ),
            (
                NetworkHttpError::ResponseBodyLimit {
                    limit: 65_536,
                    request_bytes: 12,
                    response_bytes: 65_536,
                    partial_response: None,
                },
                "body-limit",
                RuntimeErrorExpectation::LeakDetected,
            ),
            (
                NetworkHttpError::Dns {
                    reason: "dns_failure".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                "network",
                RuntimeErrorExpectation::Network,
            ),
        ];

        for (network_error, label, expectation) in cases {
            let (egress, _) =
                slack_egress_with_network(RecordingNetworkHttpEgress::failing(network_error));
            let error = match egress.send(slack_request(slack_handle())).await {
                Ok(response) => panic!("{label} case should fail, got {response:?}"),
                Err(error) => error,
            };

            expectation.assert_matches(error, label);
        }
    }

    #[derive(Clone, Copy)]
    enum RuntimeErrorExpectation {
        PolicyDenied,
        LeakDetected,
        Network,
    }

    impl RuntimeErrorExpectation {
        fn assert_matches(self, error: ProtocolHttpEgressError, label: &str) {
            match self {
                Self::PolicyDenied => assert!(
                    matches!(error, ProtocolHttpEgressError::PolicyDenied { .. }),
                    "{label}: expected policy denied, got {error:?}"
                ),
                Self::LeakDetected => assert!(
                    matches!(error, ProtocolHttpEgressError::LeakDetected),
                    "{label}: expected leak detected, got {error:?}"
                ),
                Self::Network => assert!(
                    matches!(error, ProtocolHttpEgressError::Network(_)),
                    "{label}: expected network error, got {error:?}"
                ),
            }
        }
    }
}
