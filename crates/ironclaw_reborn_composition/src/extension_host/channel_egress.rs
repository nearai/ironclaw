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
use ironclaw_secrets::{SecretMaterial, SecretStore};

/// Fixed capability id channel vendor calls are attributed to in egress
/// events/audit (mirrors the retiring per-vendor egress capability ids).
const CHANNEL_EGRESS_CAPABILITY_ID: &str = "channel.egress";

/// Resolves secret material for a channel-declared credential handle.
///
/// The generic implementation reads the scoped secret store (where
/// `[channel.config]` secret fields are stored under their handles); bridges
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
pub(crate) struct SecretStoreChannelEgressCredentials {
    store: Arc<dyn SecretStore>,
    scope_template: ResourceScope,
}

impl SecretStoreChannelEgressCredentials {
    pub(crate) fn new(store: Arc<dyn SecretStore>, scope_template: ResourceScope) -> Self {
        Self {
            store,
            scope_template,
        }
    }
}

#[async_trait]
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

        let credentials = match &approved.credential {
            None => Vec::new(),
            Some(credential) => {
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
                vec![HostRuntimeCredentialMaterial {
                    handle: credential.handle.clone(),
                    material,
                    target: credential.target.clone(),
                    required: true,
                }]
            }
        };

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
