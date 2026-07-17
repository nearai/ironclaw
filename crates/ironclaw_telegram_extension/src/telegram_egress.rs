//! Host-mediated Telegram protocol HTTP egress.
//!
//! The Telegram adapter renders only a constrained `EgressRequest` containing
//! the declared host, origin-form method path (e.g. `/sendMessage`), headers,
//! body, and opaque credential handle. This module is the host side: it
//! validates the request against the adapter's declared egress policy,
//! resolves the opaque handle to the current bot token, and delegates
//! authorization plus runtime credential injection to the shared host HTTP
//! egress port. Unlike Slack (bearer header), the Telegram Bot API carries the
//! token in the URL path, so the URL built here contains only the literal
//! `{telegram_bot_token}` placeholder and the mediated host egress substitutes
//! the material via [`RuntimeCredentialTarget::PathPlaceholder`] — the raw
//! token never appears in any URL string this module constructs.

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

use crate::telegram_setup::TelegramSetupService;

const TELEGRAM_EGRESS_TIMEOUT_MS: u32 = 10_000;
const TELEGRAM_EGRESS_RESPONSE_BODY_LIMIT_BYTES: u64 = 64 * 1024;
const TELEGRAM_EGRESS_CAPABILITY_ID: &str = "telegram.egress";
/// Opaque credential handle the adapter renders; doubles as the literal URL
/// placeholder the mediated egress substitutes.
pub const TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE: &str = "telegram_bot_token";

/// Resolves the opaque `telegram_bot_token` handle to the current bot token
/// at send time. The dynamic setup-service-backed implementation re-reads the
/// durable setup record per send, so a WebUI token rotation takes effect on
/// the next outbound message without a rebuild.
#[async_trait]
pub trait TelegramEgressCredentialProvider: Send + Sync {
    async fn resolve_telegram_bot_token(
        &self,
        handle: &EgressCredentialHandle,
    ) -> Result<SecretString, ProtocolHttpEgressError>;
}

/// Production provider over [`TelegramSetupService::bot_token`].
pub struct SetupServiceTelegramEgressCredentialProvider {
    setup_service: Arc<TelegramSetupService>,
}

impl SetupServiceTelegramEgressCredentialProvider {
    pub fn new(setup_service: Arc<TelegramSetupService>) -> Self {
        Self { setup_service }
    }
}

#[async_trait]
impl TelegramEgressCredentialProvider for SetupServiceTelegramEgressCredentialProvider {
    async fn resolve_telegram_bot_token(
        &self,
        handle: &EgressCredentialHandle,
    ) -> Result<SecretString, ProtocolHttpEgressError> {
        if handle.as_str() != TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE {
            return Err(ProtocolHttpEgressError::UnknownCredentialHandle {
                handle: handle.as_str().to_string(),
            });
        }
        self.setup_service
            .bot_token()
            .await
            .map_err(|error| {
                tracing::debug!(%error, "telegram bot token resolution failed for egress");
                ProtocolHttpEgressError::Network(RedactedString::new(
                    "telegram bot token unavailable",
                ))
            })?
            .ok_or_else(|| {
                ProtocolHttpEgressError::Network(RedactedString::new(
                    "telegram bot token unavailable",
                ))
            })
    }
}

pub struct TelegramProtocolHttpEgress {
    host_egress: HostRuntimeHttpEgressPort,
    credentials: Arc<dyn TelegramEgressCredentialProvider>,
    policy: EgressPolicy,
    scope_template: ResourceScope,
}

impl TelegramProtocolHttpEgress {
    pub fn new(
        host_egress: HostRuntimeHttpEgressPort,
        credentials: Arc<dyn TelegramEgressCredentialProvider>,
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
impl ProtocolHttpEgress for TelegramProtocolHttpEgress {
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
                    "Telegram adapter requests must use credential handles, not Authorization headers",
                ),
            });
        }
        let headers = request
            .headers()
            .iter()
            .map(|header| (header.name().to_string(), header.value().to_string()))
            .collect::<Vec<_>>();

        let capability_id = CapabilityId::new(TELEGRAM_EGRESS_CAPABILITY_ID).map_err(|error| {
            ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(format!(
                    "invalid Telegram egress capability id: {error}"
                )),
            }
        })?;
        let credentials = self
            .credential_material(request.credential_handle())
            .await?;
        let scope = self.request_scope();
        let response = self
            .host_egress
            .execute(HostRuntimeHttpEgressRequest {
                extension_id: telegram_extension_id()?,
                trust: TrustClass::System,
                request: RuntimeHttpEgressRequest {
                    runtime: RuntimeKind::FirstParty,
                    scope,
                    capability_id,
                    method: network_method(request.method().as_str())?,
                    url: telegram_bot_api_url(request.host().as_str(), request.path().as_str()),
                    headers,
                    body: request.body().to_vec(),
                    network_policy: telegram_network_policy(request.host().as_str()),
                    credential_injections: Vec::new(),
                    response_body_limit: Some(TELEGRAM_EGRESS_RESPONSE_BODY_LIMIT_BYTES),
                    save_body_to: None,
                    timeout_ms: Some(TELEGRAM_EGRESS_TIMEOUT_MS),
                },
                credentials,
            })
            .await
            .map_err(map_runtime_http_error)?;

        Ok(EgressResponse::new(response.status, response.body))
    }
}

impl TelegramProtocolHttpEgress {
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
            // Every Bot API URL embeds the token placeholder, so an
            // unauthenticated Telegram egress cannot be substituted.
            return Err(ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(
                    "Telegram egress requires the bot token credential handle",
                ),
            });
        };
        let token = self.credentials.resolve_telegram_bot_token(handle).await?;
        validate_bot_token(&token)?;
        let secret_handle = SecretHandle::new(handle.as_str()).map_err(|error| {
            ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(format!(
                    "invalid Telegram egress credential handle: {error}"
                )),
            }
        })?;
        Ok(vec![HostRuntimeCredentialMaterial {
            handle: secret_handle,
            material: SecretMaterial::from(token.expose_secret().to_string()),
            target: RuntimeCredentialTarget::PathPlaceholder {
                placeholder: TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE.to_string(),
            },
            required: true,
        }])
    }
}

/// Bot API URL shape: the token travels as a path segment, so the URL string
/// carries only the literal `{telegram_bot_token}` placeholder — never raw
/// token material — and the host egress substitutes the credential via
/// [`RuntimeCredentialTarget::PathPlaceholder`]. Mirrors
/// [`crate::telegram_bot_api::HostEgressTelegramBotApi`]'s URL
/// construction so setup-time and delivery-time egress cannot drift.
fn telegram_bot_api_url(host: &str, path: &str) -> String {
    format!("https://{host}/bot{{{TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE}}}{path}")
}

fn validate_bot_token(token: &SecretString) -> Result<(), ProtocolHttpEgressError> {
    if token
        .expose_secret()
        .bytes()
        .any(|byte| byte < 0x20 || byte == 0x7f)
    {
        return Err(ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new("Telegram bot token contains control characters"),
        });
    }
    Ok(())
}

fn telegram_network_policy(host: &str) -> NetworkPolicy {
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

fn telegram_extension_id() -> Result<ExtensionId, ProtocolHttpEgressError> {
    ExtensionId::new("ironclaw_telegram").map_err(|error| ProtocolHttpEgressError::PolicyDenied {
        reason: RedactedString::new(format!("invalid Telegram egress extension id: {error}")),
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
            reason: RedactedString::new("unsupported Telegram egress HTTP method"),
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
                reason: RedactedString::new("unauthenticated Telegram egress is not declared"),
            }
        }
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
        DeclaredEgressHost, DeclaredEgressTarget, EgressCredentialHandle, EgressHeader,
        EgressMethod, EgressPath,
    };
    use ironclaw_resources::InMemoryResourceGovernor;
    use ironclaw_secrets::InMemorySecretStore;

    use super::*;

    struct StaticTokenProvider {
        token: String,
    }

    #[async_trait]
    impl TelegramEgressCredentialProvider for StaticTokenProvider {
        async fn resolve_telegram_bot_token(
            &self,
            handle: &EgressCredentialHandle,
        ) -> Result<SecretString, ProtocolHttpEgressError> {
            if handle.as_str() != TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE {
                return Err(ProtocolHttpEgressError::UnknownCredentialHandle {
                    handle: handle.as_str().to_string(),
                });
            }
            Ok(SecretString::from(self.token.clone()))
        }
    }

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
                    body: br#"{"ok":true,"result":{}}"#.to_vec(),
                    usage: NetworkUsage {
                        request_bytes: 0,
                        response_bytes: 23,
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

    fn host_egress_port(network: RecordingNetworkHttpEgress) -> HostRuntimeHttpEgressPort {
        let services = test_host_runtime_services()
            .with_secret_store(Arc::new(InMemorySecretStore::new()))
            .try_with_host_http_egress(network)
            .expect("host HTTP egress should wire");
        services
            .host_runtime_http_egress_port()
            .expect("host runtime HTTP egress port should be configured")
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

    fn telegram_host() -> DeclaredEgressHost {
        DeclaredEgressHost::new("api.telegram.org").expect("telegram host")
    }

    fn telegram_handle() -> EgressCredentialHandle {
        EgressCredentialHandle::new(TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE).expect("telegram handle")
    }

    fn telegram_request(handle: EgressCredentialHandle) -> EgressRequest {
        EgressRequest::new(
            telegram_host(),
            EgressMethod::post(),
            EgressPath::new("/sendMessage").expect("telegram path"),
        )
        .with_body(br#"{"chat_id":42,"text":"hi"}"#.to_vec())
        .with_credential_handle(Some(handle))
    }

    fn telegram_egress_with_network(
        network: RecordingNetworkHttpEgress,
        token: &str,
    ) -> (
        TelegramProtocolHttpEgress,
        Arc<Mutex<Vec<NetworkHttpRequest>>>,
    ) {
        let recorded = network.requests();
        let host_egress = host_egress_port(network);
        let handle = telegram_handle();
        let egress = TelegramProtocolHttpEgress::new(
            host_egress,
            Arc::new(StaticTokenProvider {
                token: token.to_string(),
            }),
            EgressPolicy::new([DeclaredEgressTarget::new(telegram_host(), Some(handle))]),
            ResourceScope::system(),
        );
        (egress, recorded)
    }

    #[test]
    fn telegram_bot_api_url_carries_placeholder_and_never_raw_token() {
        // Exact equality pins both halves of the contract: the URL contains
        // the literal `/bot{telegram_bot_token}` placeholder segment and
        // nothing else — no token material can be embedded because the
        // builder never sees a token.
        assert_eq!(
            telegram_bot_api_url("api.telegram.org", "/sendMessage"),
            "https://api.telegram.org/bot{telegram_bot_token}/sendMessage"
        );
    }

    /// End-to-end mediated success path: the host-runtime `PathPlaceholder`
    /// injector substitutes the braced in-segment `bot{telegram_bot_token}`
    /// shape (Telegram tokens carry `:`, a legal pchar), so the send reaches
    /// the network layer with the real token in the dispatched URL path and
    /// the placeholder — never raw token material — in every runtime-visible
    /// request field.
    #[tokio::test]
    async fn telegram_protocol_http_egress_substitutes_token_and_dispatches() {
        let (egress, recorded) =
            telegram_egress_with_network(RecordingNetworkHttpEgress::ok(), "12345:secret-token");

        let response = egress
            .send(telegram_request(telegram_handle()))
            .await
            .expect("in-segment placeholder substitution dispatches");
        assert_eq!(response.status(), 200);

        let requests = recorded.lock().expect("network requests lock");
        assert_eq!(requests.len(), 1, "exactly one network dispatch");
        assert_eq!(
            requests[0].url, "https://api.telegram.org/bot12345:secret-token/sendMessage",
            "the dispatched URL carries the substituted token"
        );
    }

    #[test]
    fn telegram_egress_requests_cannot_carry_authorization_headers() {
        // The type layer already rejects host-owned headers at construction
        // (and on deserialize), so no adapter can hand this module an
        // Authorization header; the send-time guard in `send` stays as
        // defense in depth, mirroring the Slack egress.
        assert!(
            EgressHeader::new("Authorization", "Bearer sneaky").is_err(),
            "authorization headers must be rejected at the EgressHeader boundary"
        );
    }

    #[tokio::test]
    async fn telegram_protocol_http_egress_fails_closed_without_credential_handle() {
        let (egress, recorded) =
            telegram_egress_with_network(RecordingNetworkHttpEgress::ok(), "12345:secret-token");

        let request = EgressRequest::new(
            telegram_host(),
            EgressMethod::post(),
            EgressPath::new("/sendMessage").expect("telegram path"),
        )
        .with_credential_handle(None);
        let error = egress
            .send(request)
            .await
            .expect_err("handle-less telegram egress must fail closed");

        assert!(matches!(
            error,
            ProtocolHttpEgressError::PolicyDenied { .. }
        ));
        assert!(recorded.lock().expect("network requests lock").is_empty());
    }

    #[tokio::test]
    async fn telegram_protocol_http_egress_rejects_control_chars_in_token_before_network() {
        let (egress, recorded) = telegram_egress_with_network(
            RecordingNetworkHttpEgress::ok(),
            "12345:secret\r\ninjected",
        );

        let error = egress
            .send(telegram_request(telegram_handle()))
            .await
            .expect_err("control characters in the token must fail before network");

        assert!(matches!(
            error,
            ProtocolHttpEgressError::PolicyDenied { .. }
        ));
        assert!(recorded.lock().expect("network requests lock").is_empty());
    }

    #[tokio::test]
    async fn telegram_protocol_http_egress_rejects_unknown_handle_before_network() {
        let (egress, recorded) =
            telegram_egress_with_network(RecordingNetworkHttpEgress::ok(), "12345:secret-token");

        let unknown = EgressCredentialHandle::new("other_token").expect("other handle");
        let request = EgressRequest::new(
            telegram_host(),
            EgressMethod::post(),
            EgressPath::new("/sendMessage").expect("telegram path"),
        )
        .with_credential_handle(Some(unknown));
        let error = egress
            .send(request)
            .await
            .expect_err("unknown handle should fail before network");

        assert!(matches!(
            error,
            ProtocolHttpEgressError::UnauthorizedCredentialHandle { .. }
        ));
        assert!(recorded.lock().expect("network requests lock").is_empty());
    }
}
