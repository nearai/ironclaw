//! Host-mediated Slack protocol HTTP egress.
//!
//! The Slack adapter renders only a constrained `EgressRequest` containing the
//! declared host, origin-form path, headers, body, and opaque credential handle.
//! This module is the host side: it validates the request against the adapter's
//! declared egress policy, resolves the opaque handle to a bearer token, injects
//! authorization, and sends the request through the shared network policy egress.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityId, NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeKind,
};
use ironclaw_product_adapters::{
    EgressCredentialHandle, EgressRequest, EgressResponse, ProtocolHttpEgress,
    ProtocolHttpEgressError, RedactedString,
};
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
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    credentials: Arc<dyn SlackEgressCredentialProvider>,
    policy: EgressPolicy,
    scope: ResourceScope,
}

impl SlackProtocolHttpEgress {
    pub fn new(
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
        credentials: Arc<dyn SlackEgressCredentialProvider>,
        policy: EgressPolicy,
        scope: ResourceScope,
    ) -> Self {
        Self {
            runtime_http_egress,
            credentials,
            policy,
            scope,
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

        let mut headers = request
            .headers()
            .iter()
            .map(|header| (header.name().to_string(), header.value().to_string()))
            .collect::<Vec<_>>();
        if let Some(handle) = request.credential_handle() {
            let credential = self
                .credentials
                .resolve_slack_egress_credential(handle)
                .await
                .map_err(map_credential_error)?;
            let authorization = bearer_authorization_value(&credential)?;
            headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
            headers.push(("authorization".to_string(), authorization));
        }

        let capability_id = CapabilityId::new(SLACK_EGRESS_CAPABILITY_ID).map_err(|error| {
            ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(format!("invalid Slack egress capability id: {error}")),
            }
        })?;
        let response = self
            .runtime_http_egress
            .execute(RuntimeHttpEgressRequest {
                runtime: RuntimeKind::FirstParty,
                scope: self.scope.clone(),
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
            })
            .await
            .map_err(map_runtime_http_error)?;

        Ok(EgressResponse::new(response.status, response.body))
    }
}

fn bearer_authorization_value(
    credential: &SlackEgressCredential,
) -> Result<String, ProtocolHttpEgressError> {
    let token = credential.as_bearer_token();
    if token.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        return Err(ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new("Slack bearer token contains control characters"),
        });
    }
    Ok(format!("Bearer {token}"))
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
    use ironclaw_host_api::{
        RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED, RuntimeHttpEgressResponse,
        RuntimeHttpSavedBody,
    };
    use ironclaw_product_adapters::{
        DeclaredEgressHost, DeclaredEgressTarget, EgressCredentialHandle, EgressMethod, EgressPath,
    };

    use super::*;

    #[derive(Default)]
    struct RecordingRuntimeHttpEgress {
        requests: Mutex<Vec<RuntimeHttpEgressRequest>>,
    }

    impl RecordingRuntimeHttpEgress {
        fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
            self.requests
                .lock()
                .expect("runtime HTTP requests lock")
                .clone()
        }
    }

    #[async_trait]
    impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            self.requests
                .lock()
                .expect("runtime HTTP requests lock")
                .push(request);
            Ok(RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                body: br#"{\"ok\":true}"#.to_vec(),
                saved_body: None::<RuntimeHttpSavedBody>,
                request_bytes: 0,
                response_bytes: 0,
                redaction_applied: false,
            })
        }
    }

    struct FailingRuntimeHttpEgress {
        error: RuntimeHttpEgressError,
    }

    #[async_trait]
    impl RuntimeHttpEgress for FailingRuntimeHttpEgress {
        async fn execute(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            Err(self.error.clone())
        }
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

    fn slack_egress(runtime_http: Arc<RecordingRuntimeHttpEgress>) -> SlackProtocolHttpEgress {
        let handle = slack_handle();
        SlackProtocolHttpEgress::new(
            runtime_http,
            Arc::new(StaticSlackEgressCredentialProvider::new(
                handle.clone(),
                "xoxb-secret",
            )),
            EgressPolicy::new([DeclaredEgressTarget::new(slack_host(), Some(handle))]),
            ResourceScope::system(),
        )
    }

    fn slack_egress_with_runtime(
        runtime_http: Arc<dyn RuntimeHttpEgress>,
    ) -> SlackProtocolHttpEgress {
        let handle = slack_handle();
        SlackProtocolHttpEgress::new(
            runtime_http,
            Arc::new(StaticSlackEgressCredentialProvider::new(
                handle.clone(),
                "xoxb-secret",
            )),
            EgressPolicy::new([DeclaredEgressTarget::new(slack_host(), Some(handle))]),
            ResourceScope::system(),
        )
    }

    #[tokio::test]
    async fn slack_protocol_http_egress_validates_policy_and_injects_bearer() {
        let runtime_http = Arc::new(RecordingRuntimeHttpEgress::default());
        let egress = slack_egress(Arc::clone(&runtime_http));

        let response = egress
            .send(slack_request(slack_handle()))
            .await
            .expect("slack egress should succeed");

        assert_eq!(response.status(), 200);
        let requests = runtime_http.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, "https://slack.com/api/chat.postMessage");
        assert_eq!(requests[0].method, NetworkMethod::Post);
        assert_eq!(requests[0].body, br#"{"channel":"D1","text":"hi"}"#);
        let auth_headers = requests[0]
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .collect::<Vec<_>>();
        assert_eq!(auth_headers.len(), 1);
        assert_eq!(auth_headers[0].1, "Bearer xoxb-secret");
    }

    #[tokio::test]
    async fn slack_protocol_http_egress_rejects_control_chars_in_bearer_before_network() {
        let runtime_http = Arc::new(RecordingRuntimeHttpEgress::default());
        let handle = slack_handle();
        let egress = SlackProtocolHttpEgress::new(
            runtime_http.clone(),
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
        assert!(runtime_http.requests().is_empty());
    }

    #[tokio::test]
    async fn slack_protocol_http_egress_rejects_unknown_handle_before_network() {
        let runtime_http = Arc::new(RecordingRuntimeHttpEgress::default());
        let unknown = EgressCredentialHandle::new("other_token").expect("other handle");
        let egress = SlackProtocolHttpEgress::new(
            runtime_http.clone(),
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
        assert!(runtime_http.requests().is_empty());
    }

    #[tokio::test]
    async fn slack_protocol_http_egress_maps_runtime_http_failures() {
        let cases = [
            (
                RuntimeHttpEgressError::Request {
                    reason: "invalid_url".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                "request-denied",
                RuntimeErrorExpectation::PolicyDenied,
            ),
            (
                RuntimeHttpEgressError::Network {
                    reason: "policy_denied".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                "policy-denied",
                RuntimeErrorExpectation::PolicyDenied,
            ),
            (
                RuntimeHttpEgressError::Response {
                    reason: RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED.to_string(),
                    request_bytes: 12,
                    response_bytes: 65_536,
                },
                "body-limit",
                RuntimeErrorExpectation::LeakDetected,
            ),
            (
                RuntimeHttpEgressError::Network {
                    reason: "dns_failure".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                "network",
                RuntimeErrorExpectation::Network,
            ),
        ];

        for (runtime_error, label, expectation) in cases {
            let runtime_http = Arc::new(FailingRuntimeHttpEgress {
                error: runtime_error,
            });
            let egress = slack_egress_with_runtime(runtime_http);
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
