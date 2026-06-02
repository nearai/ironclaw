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
    NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
};
use ironclaw_network::{NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest};
use ironclaw_product_adapters::{
    EgressCredentialHandle, EgressRequest, EgressResponse, ProtocolHttpEgress,
    ProtocolHttpEgressError, RedactedString,
};
use ironclaw_wasm_product_adapters::{EgressPolicy, EgressPolicyError, EgressPolicyTarget};
use thiserror::Error;

const SLACK_EGRESS_TIMEOUT_MS: u32 = 10_000;
const SLACK_EGRESS_RESPONSE_BODY_LIMIT_BYTES: u64 = 64 * 1024;

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
    bearer_token: String,
}

impl SlackEgressCredential {
    pub fn bearer_token(token: impl Into<String>) -> Self {
        Self {
            bearer_token: token.into(),
        }
    }

    fn as_bearer_token(&self) -> &str {
        &self.bearer_token
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
    network: Arc<dyn NetworkHttpEgress>,
    credentials: Arc<dyn SlackEgressCredentialProvider>,
    policy: EgressPolicy,
    scope: ResourceScope,
}

impl SlackProtocolHttpEgress {
    pub fn new(
        network: Arc<dyn NetworkHttpEgress>,
        credentials: Arc<dyn SlackEgressCredentialProvider>,
        policy: EgressPolicy,
        scope: ResourceScope,
    ) -> Self {
        Self {
            network,
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
            headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
            headers.push((
                "authorization".to_string(),
                format!("Bearer {}", credential.as_bearer_token()),
            ));
        }

        let response = self
            .network
            .execute(NetworkHttpRequest {
                scope: self.scope.clone(),
                method: network_method(request.method().as_str())?,
                url: format!(
                    "https://{}{}",
                    request.host().as_str(),
                    request.path().as_str()
                ),
                headers,
                body: request.body().to_vec(),
                policy: slack_network_policy(request.host().as_str()),
                response_body_limit: Some(SLACK_EGRESS_RESPONSE_BODY_LIMIT_BYTES),
                timeout_ms: Some(SLACK_EGRESS_TIMEOUT_MS),
            })
            .await
            .map_err(map_network_error)?;

        Ok(EgressResponse::new(response.status, response.body))
    }
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

fn map_network_error(error: NetworkHttpError) -> ProtocolHttpEgressError {
    match error {
        NetworkHttpError::PolicyDenied { reason, .. } => ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new(reason),
        },
        NetworkHttpError::InvalidUrl { reason, .. } => ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new(reason),
        },
        NetworkHttpError::ResponseBodyLimit { .. } => ProtocolHttpEgressError::LeakDetected,
        NetworkHttpError::Dns { reason, .. } | NetworkHttpError::Transport { reason, .. } => {
            ProtocolHttpEgressError::Network(RedactedString::new(reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_network::{NetworkHttpRequest, NetworkHttpResponse, NetworkUsage};
    use ironclaw_product_adapters::{
        DeclaredEgressHost, DeclaredEgressTarget, EgressCredentialHandle, EgressMethod, EgressPath,
    };

    use super::*;

    #[derive(Default)]
    struct RecordingNetwork {
        requests: Mutex<Vec<NetworkHttpRequest>>,
    }

    impl RecordingNetwork {
        fn requests(&self) -> Vec<NetworkHttpRequest> {
            self.requests.lock().expect("network requests lock").clone()
        }
    }

    #[async_trait]
    impl NetworkHttpEgress for RecordingNetwork {
        async fn execute(
            &self,
            request: NetworkHttpRequest,
        ) -> Result<NetworkHttpResponse, NetworkHttpError> {
            self.requests
                .lock()
                .expect("network requests lock")
                .push(request);
            Ok(NetworkHttpResponse {
                status: 200,
                headers: Vec::new(),
                body: br#"{\"ok\":true}"#.to_vec(),
                usage: NetworkUsage::default(),
            })
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

    fn slack_egress(network: Arc<RecordingNetwork>) -> SlackProtocolHttpEgress {
        let handle = slack_handle();
        SlackProtocolHttpEgress::new(
            network,
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
        let network = Arc::new(RecordingNetwork::default());
        let egress = slack_egress(Arc::clone(&network));

        let response = egress
            .send(slack_request(slack_handle()))
            .await
            .expect("slack egress should succeed");

        assert_eq!(response.status(), 200);
        let requests = network.requests();
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
    async fn slack_protocol_http_egress_rejects_unknown_handle_before_network() {
        let network = Arc::new(RecordingNetwork::default());
        let unknown = EgressCredentialHandle::new("other_token").expect("other handle");
        let egress = SlackProtocolHttpEgress::new(
            network.clone(),
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
        assert!(network.requests().is_empty());
    }
}
