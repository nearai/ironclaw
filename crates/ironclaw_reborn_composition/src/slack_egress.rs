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
