//! Host-mediated Telegram Bot API protocol HTTP egress.
//!
//! The Telegram adapter renders only a constrained, token-free `EgressRequest`
//! (declared host `api.telegram.org`, origin-form path `/sendMessage` or
//! `/sendChatAction`, JSON body, and an opaque `telegram_bot_token` credential
//! handle). This module is the host side: it validates the request against the
//! adapter's declared egress policy, resolves the opaque handle to the bot
//! token, and issues the real Bot API call.
//!
//! Telegram differs from Slack in *where* the credential goes. Slack injects a
//! bearer token via an `Authorization` header. Telegram's Bot API puts the
//! token in the URL path: `/bot<TOKEN>/sendMessage`. Bot tokens contain a `:`
//! (`<id>:<secret>`), which the host's `PathPlaceholder` credential injection
//! rejects as a non-unreserved path segment, so the token cannot be injected by
//! the shared runtime credential path. Instead the bridge constructs the
//! tokenized URL itself and passes **no** credential injections. The token
//! therefore appears only in the constructed URL handed to the network layer —
//! never in the adapter's `EgressRequest`, never in a header, never in this
//! module's error or response values (every error here is a `RedactedString`
//! or a stable runtime reason code).

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, NetworkMethod, NetworkPolicy, NetworkTargetPattern,
    ResourceScope, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeKind, TrustClass,
};
use ironclaw_host_runtime::{HostRuntimeHttpEgressPort, HostRuntimeHttpEgressRequest};
use ironclaw_product_adapters::{
    EgressCredentialHandle, EgressRequest, EgressResponse, ProtocolHttpEgress,
    ProtocolHttpEgressError, RedactedString,
};
use ironclaw_wasm_product_adapters::{EgressPolicy, EgressPolicyError, EgressPolicyTarget};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

const TELEGRAM_EGRESS_TIMEOUT_MS: u32 = 10_000;
const TELEGRAM_EGRESS_RESPONSE_BODY_LIMIT_BYTES: u64 = 64 * 1024;
const TELEGRAM_EGRESS_CAPABILITY_ID: &str = "telegram.egress";
const TELEGRAM_EGRESS_EXTENSION_ID: &str = "ironclaw_telegram";

/// Environment variable that overrides the Bot API origin
/// (`https://api.telegram.org`) for tests so the egress bridge can target a
/// fake Telegram API server. Test-only; production never sets it.
pub(crate) const TELEGRAM_API_BASE_URL_ENV: &str = "IRONCLAW_TEST_TELEGRAM_API_BASE_URL";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum TelegramEgressCredentialError {
    #[error("unknown Telegram egress credential handle {handle}")]
    UnknownHandle { handle: String },
}

/// A resolved Telegram bot token. Holds the secret in a `SecretString` so it is
/// never accidentally logged or `Debug`-printed.
pub(crate) struct TelegramEgressCredential {
    bot_token: SecretString,
}

impl TelegramEgressCredential {
    pub(crate) fn bot_token(token: impl Into<String>) -> Self {
        Self {
            bot_token: SecretString::from(token.into()),
        }
    }

    fn as_bot_token(&self) -> &str {
        self.bot_token.expose_secret()
    }
}

#[async_trait]
pub(crate) trait TelegramEgressCredentialProvider: Send + Sync {
    async fn resolve_telegram_egress_credential(
        &self,
        handle: &EgressCredentialHandle,
    ) -> Result<TelegramEgressCredential, TelegramEgressCredentialError>;
}

pub(crate) struct StaticTelegramEgressCredentialProvider {
    handle: EgressCredentialHandle,
    credential: TelegramEgressCredential,
}

impl StaticTelegramEgressCredentialProvider {
    pub(crate) fn new(handle: EgressCredentialHandle, bot_token: impl Into<String>) -> Self {
        Self {
            handle,
            credential: TelegramEgressCredential::bot_token(bot_token),
        }
    }
}

#[async_trait]
impl TelegramEgressCredentialProvider for StaticTelegramEgressCredentialProvider {
    async fn resolve_telegram_egress_credential(
        &self,
        handle: &EgressCredentialHandle,
    ) -> Result<TelegramEgressCredential, TelegramEgressCredentialError> {
        if handle == &self.handle {
            Ok(TelegramEgressCredential::bot_token(
                self.credential.as_bot_token().to_string(),
            ))
        } else {
            Err(TelegramEgressCredentialError::UnknownHandle {
                handle: handle.as_str().to_string(),
            })
        }
    }
}

pub(crate) struct TelegramProtocolHttpEgress {
    host_egress: HostRuntimeHttpEgressPort,
    credentials: Arc<dyn TelegramEgressCredentialProvider>,
    policy: EgressPolicy,
    scope_template: ResourceScope,
    /// Optional Bot API origin override (scheme://host[:port]), validated at
    /// construction. `None` => `https://<request host>`.
    base_url_override: Option<String>,
}

impl TelegramProtocolHttpEgress {
    pub(crate) fn new(
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
            base_url_override: None,
        }
    }

    /// Override the Bot API origin (test-only; for the fake Telegram API). The
    /// override must be a bare `scheme://host[:port]` with no path, query, or
    /// userinfo. Returns an error (rather than silently ignoring) so a
    /// misconfigured test fails loudly instead of hitting the real Bot API.
    pub(crate) fn with_base_url_override(
        mut self,
        base_url: impl Into<String>,
    ) -> Result<Self, TelegramEgressConfigError> {
        let base_url = base_url.into();
        validate_base_url_override(&base_url)?;
        self.base_url_override = Some(base_url.trim_end_matches('/').to_string());
        Ok(self)
    }

    /// Build the override from `TELEGRAM_API_BASE_URL_ENV` if set. No-op when the
    /// env var is unset (production path).
    pub(crate) fn with_base_url_override_from_env(self) -> Result<Self, TelegramEgressConfigError> {
        match std::env::var(TELEGRAM_API_BASE_URL_ENV) {
            Ok(value) if !value.trim().is_empty() => self.with_base_url_override(value),
            _ => Ok(self),
        }
    }

    fn request_scope(&self) -> ResourceScope {
        let mut scope = self.scope_template.clone();
        scope.invocation_id = InvocationId::new();
        scope
    }

    /// Resolve the Bot API origin for this request: the validated override, or
    /// `https://<declared host>`.
    fn origin_for(&self, request_host: &str) -> String {
        match &self.base_url_override {
            Some(base) => base.clone(),
            None => format!("https://{request_host}"),
        }
    }
}

#[async_trait]
impl ProtocolHttpEgress for TelegramProtocolHttpEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> Result<EgressResponse, ProtocolHttpEgressError> {
        // 1. Policy gate: the (declared host, credential handle) pair must be
        //    declared by the adapter's egress policy. This runs before any
        //    token resolution or URL construction.
        self.policy
            .check(EgressPolicyTarget {
                host: request.host(),
                credential_handle: request.credential_handle(),
            })
            .map_err(map_egress_policy_error)?;

        // 2. Defense in depth: the Telegram adapter must never smuggle an
        //    Authorization header; the token belongs in the URL path only.
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

        // 3. Resolve the opaque handle to the bot token and validate it before
        //    it is concatenated into the URL path.
        let Some(handle) = request.credential_handle() else {
            return Err(ProtocolHttpEgressError::PolicyDenied {
                reason: RedactedString::new(
                    "Telegram egress request is missing a credential handle",
                ),
            });
        };
        let credential = self
            .credentials
            .resolve_telegram_egress_credential(handle)
            .await
            .map_err(map_credential_error)?;
        validate_bot_token(&credential)?;

        // 4. Build the tokenized Bot API URL. The token lives ONLY here.
        let origin = self.origin_for(request.host().as_str());
        let url = format!(
            "{origin}/bot{token}{path}",
            token = credential.as_bot_token(),
            path = request.path().as_str()
        );
        let policy_host = host_of(&url).ok_or_else(|| ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new("Telegram egress URL did not parse to a host"),
        })?;

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

        let response = self
            .host_egress
            .execute(HostRuntimeHttpEgressRequest {
                extension_id: telegram_extension_id()?,
                trust: TrustClass::System,
                request: RuntimeHttpEgressRequest {
                    runtime: RuntimeKind::FirstParty,
                    scope: self.request_scope(),
                    capability_id,
                    method: network_method(request.method().as_str())?,
                    url,
                    headers,
                    body: request.body().to_vec(),
                    network_policy: telegram_network_policy(&policy_host),
                    // The token is in the URL; no runtime credential injection.
                    credential_injections: Vec::new(),
                    response_body_limit: Some(TELEGRAM_EGRESS_RESPONSE_BODY_LIMIT_BYTES),
                    save_body_to: None,
                    timeout_ms: Some(TELEGRAM_EGRESS_TIMEOUT_MS),
                },
                // No host-side credential material: the token is not injected.
                credentials: Vec::new(),
            })
            .await
            .map_err(map_runtime_http_error)?;

        Ok(EgressResponse::new(response.status, response.body))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum TelegramEgressConfigError {
    #[error("Telegram API base url override must not be empty")]
    Empty,
    #[error("Telegram API base url override must use http or https scheme")]
    BadScheme,
    #[error("Telegram API base url override must not contain userinfo, path, query, or fragment")]
    NotBareOrigin,
}

/// Validate a `scheme://host[:port]` origin override: http/https scheme, a
/// non-empty host, and no userinfo / path / query / fragment.
fn validate_base_url_override(base_url: &str) -> Result<(), TelegramEgressConfigError> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(TelegramEgressConfigError::Empty);
    }
    let rest = match trimmed.strip_prefix("https://") {
        Some(rest) => rest,
        None => trimmed
            .strip_prefix("http://")
            .ok_or(TelegramEgressConfigError::BadScheme)?,
    };
    let authority = rest.trim_end_matches('/');
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
    {
        return Err(TelegramEgressConfigError::NotBareOrigin);
    }
    Ok(())
}

/// Reject bot tokens that would break the URL path or smuggle extra path
/// segments / query / fragment. Telegram tokens are `<id>:<secret>` where the
/// secret is URL-safe base64-ish; `:` `-` `_` are legal path chars and are
/// allowed, but `/ ? #`, whitespace, and control bytes are not.
fn validate_bot_token(
    credential: &TelegramEgressCredential,
) -> Result<(), ProtocolHttpEgressError> {
    let token = credential.as_bot_token();
    if token.is_empty() {
        return Err(ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new("Telegram bot token is empty"),
        });
    }
    if token.bytes().any(|byte| {
        byte < 0x20 || byte == 0x7f || matches!(byte, b'/' | b'?' | b'#' | b' ' | b'\t' | b'\\')
    }) {
        return Err(ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new("Telegram bot token contains disallowed characters"),
        });
    }
    Ok(())
}

/// Extract the host (without scheme/port) from a `scheme://host[:port]/...` URL.
fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://")?.1;
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _)| host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn telegram_network_policy(host: &str) -> NetworkPolicy {
    let loopback = host == "127.0.0.1" || host == "::1" || host.eq_ignore_ascii_case("localhost");
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: host.to_string(),
            port: None,
        }],
        // The Bot API is public; private IPs are denied in production. A
        // loopback host can only appear via the test-only base-url override, so
        // permit private ranges in exactly that case.
        deny_private_ip_ranges: !loopback,
        max_egress_bytes: None,
    }
}

fn telegram_extension_id() -> Result<ExtensionId, ProtocolHttpEgressError> {
    ExtensionId::new(TELEGRAM_EGRESS_EXTENSION_ID).map_err(|error| {
        ProtocolHttpEgressError::PolicyDenied {
            reason: RedactedString::new(format!("invalid Telegram egress extension id: {error}")),
        }
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

fn map_credential_error(error: TelegramEgressCredentialError) -> ProtocolHttpEgressError {
    match error {
        TelegramEgressCredentialError::UnknownHandle { handle } => {
            ProtocolHttpEgressError::UnknownCredentialHandle { handle }
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
        DeclaredEgressHost, DeclaredEgressTarget, EgressCredentialHandle, EgressMethod, EgressPath,
    };
    use ironclaw_resources::InMemoryResourceGovernor;
    use ironclaw_secrets::InMemorySecretStore;

    use super::*;

    // A real Telegram bot token shape: `<bot_id>:<url-safe-secret>`. The `:` is
    // a legal path char, so it stays in the URL path unmodified.
    const TEST_BOT_TOKEN: &str = "123456789:AAExampleTokenWith-Dashes_andUnderscores";

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
                    body: br#"{"ok":true,"result":{"message_id":1}}"#.to_vec(),
                    usage: NetworkUsage {
                        request_bytes: 0,
                        response_bytes: 0,
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

    fn telegram_host() -> DeclaredEgressHost {
        DeclaredEgressHost::new("api.telegram.org").expect("telegram host")
    }

    fn telegram_handle() -> EgressCredentialHandle {
        EgressCredentialHandle::new("telegram_bot_token").expect("telegram handle")
    }

    fn send_message_request(handle: EgressCredentialHandle) -> EgressRequest {
        EgressRequest::new(
            telegram_host(),
            EgressMethod::post(),
            EgressPath::new("/sendMessage").expect("telegram path"),
        )
        .with_body(br#"{"chat_id":-100,"text":"hi"}"#.to_vec())
        .with_credential_handle(Some(handle))
    }

    fn telegram_egress_with(
        network: RecordingNetworkHttpEgress,
        token: &str,
    ) -> (
        TelegramProtocolHttpEgress,
        Arc<Mutex<Vec<NetworkHttpRequest>>>,
    ) {
        let (host_egress, requests) = host_egress_port(network);
        let handle = telegram_handle();
        let egress = TelegramProtocolHttpEgress::new(
            host_egress,
            Arc::new(StaticTelegramEgressCredentialProvider::new(
                handle.clone(),
                token.to_string(),
            )),
            EgressPolicy::new([DeclaredEgressTarget::new(telegram_host(), Some(handle))]),
            ResourceScope::system(),
        );
        (egress, requests)
    }

    #[tokio::test]
    async fn telegram_egress_embeds_token_in_path_and_never_in_headers() {
        let (egress, recorded) =
            telegram_egress_with(RecordingNetworkHttpEgress::ok(), TEST_BOT_TOKEN);

        let response = egress
            .send(send_message_request(telegram_handle()))
            .await
            .expect("telegram egress should succeed");

        assert_eq!(response.status(), 200);
        let requests = recorded.lock().expect("requests lock");
        assert_eq!(requests.len(), 1);
        // The token rides in the URL path as `/bot<TOKEN>/sendMessage`.
        assert_eq!(
            requests[0].url,
            format!("https://api.telegram.org/bot{TEST_BOT_TOKEN}/sendMessage")
        );
        assert_eq!(requests[0].method, NetworkMethod::Post);
        assert_eq!(requests[0].body, br#"{"chat_id":-100,"text":"hi"}"#);
        // No Authorization header (Slack-style bearer injection must not happen).
        assert!(
            !requests[0]
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("authorization")),
            "Telegram egress must not set an Authorization header"
        );
    }

    #[tokio::test]
    async fn telegram_egress_base_url_override_targets_fake_host() {
        let (host_egress, recorded) = host_egress_port(RecordingNetworkHttpEgress::ok());
        let handle = telegram_handle();
        let egress = TelegramProtocolHttpEgress::new(
            host_egress,
            Arc::new(StaticTelegramEgressCredentialProvider::new(
                handle.clone(),
                TEST_BOT_TOKEN.to_string(),
            )),
            EgressPolicy::new([DeclaredEgressTarget::new(telegram_host(), Some(handle))]),
            ResourceScope::system(),
        )
        .with_base_url_override("http://127.0.0.1:8089")
        .expect("loopback override is a bare origin");

        egress
            .send(send_message_request(telegram_handle()))
            .await
            .expect("override egress should succeed");

        let requests = recorded.lock().expect("requests lock");
        assert_eq!(
            requests[0].url,
            format!("http://127.0.0.1:8089/bot{TEST_BOT_TOKEN}/sendMessage")
        );
        // Loopback override allows private-range targets for the fake server.
        assert!(!requests[0].policy.deny_private_ip_ranges);
        assert_eq!(
            requests[0].policy.allowed_targets[0].host_pattern,
            "127.0.0.1"
        );
    }

    #[tokio::test]
    async fn telegram_egress_rejects_control_chars_in_token_before_network() {
        let (egress, recorded) =
            telegram_egress_with(RecordingNetworkHttpEgress::ok(), "123:tok\r\nX-Injected: 1");

        let error = egress
            .send(send_message_request(telegram_handle()))
            .await
            .expect_err("token with control chars must fail before network");

        assert!(matches!(
            error,
            ProtocolHttpEgressError::PolicyDenied { .. }
        ));
        assert!(recorded.lock().expect("requests lock").is_empty());
        // The redacted error must not leak the token material.
        assert!(!format!("{error:?}").contains("123:tok"));
    }

    #[tokio::test]
    async fn telegram_egress_rejects_unknown_handle_before_network() {
        let (host_egress, recorded) = host_egress_port(RecordingNetworkHttpEgress::ok());
        let configured = telegram_handle();
        let unknown = EgressCredentialHandle::new("other_token").expect("other handle");
        let egress = TelegramProtocolHttpEgress::new(
            host_egress,
            Arc::new(StaticTelegramEgressCredentialProvider::new(
                configured,
                TEST_BOT_TOKEN.to_string(),
            )),
            EgressPolicy::new([DeclaredEgressTarget::new(
                telegram_host(),
                Some(unknown.clone()),
            )]),
            ResourceScope::system(),
        );

        let error = egress
            .send(send_message_request(unknown))
            .await
            .expect_err("unknown handle must fail before network");

        assert!(matches!(
            error,
            ProtocolHttpEgressError::UnknownCredentialHandle { .. }
        ));
        assert!(recorded.lock().expect("requests lock").is_empty());
    }

    #[tokio::test]
    async fn telegram_egress_maps_runtime_http_failures() {
        let cases = [
            (
                NetworkHttpError::PolicyDenied {
                    reason: "policy_denied".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                "policy-denied",
            ),
            (
                NetworkHttpError::ResponseBodyLimit {
                    limit: 65_536,
                    request_bytes: 12,
                    response_bytes: 65_536,
                    partial_response: None,
                },
                "body-limit",
            ),
            (
                NetworkHttpError::Dns {
                    reason: "dns_failure".to_string(),
                    request_bytes: 12,
                    response_bytes: 0,
                },
                "network",
            ),
        ];

        for (network_error, label) in cases {
            let (egress, _) = telegram_egress_with(
                RecordingNetworkHttpEgress::failing(network_error),
                TEST_BOT_TOKEN,
            );
            let error = match egress.send(send_message_request(telegram_handle())).await {
                Ok(response) => panic!("{label} case should fail, got {response:?}"),
                Err(error) => error,
            };
            // Whatever the mapping, the token must never appear in the error.
            assert!(
                !format!("{error:?}").contains(TEST_BOT_TOKEN),
                "{label}: error leaked the bot token"
            );
            match label {
                "policy-denied" => {
                    assert!(matches!(
                        error,
                        ProtocolHttpEgressError::PolicyDenied { .. }
                    ))
                }
                "body-limit" => assert!(matches!(error, ProtocolHttpEgressError::LeakDetected)),
                _ => assert!(matches!(error, ProtocolHttpEgressError::Network(_))),
            }
        }
    }

    #[test]
    fn base_url_override_rejects_non_origin_values() {
        assert!(validate_base_url_override("https://api.telegram.org").is_ok());
        assert!(validate_base_url_override("http://127.0.0.1:8089").is_ok());
        assert_eq!(
            validate_base_url_override(""),
            Err(TelegramEgressConfigError::Empty)
        );
        assert_eq!(
            validate_base_url_override("ftp://api.telegram.org"),
            Err(TelegramEgressConfigError::BadScheme)
        );
        assert_eq!(
            validate_base_url_override("https://api.telegram.org/bot123"),
            Err(TelegramEgressConfigError::NotBareOrigin)
        );
        assert_eq!(
            validate_base_url_override("https://user:pass@api.telegram.org"),
            Err(TelegramEgressConfigError::NotBareOrigin)
        );
    }
}
