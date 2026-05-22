use std::{net::IpAddr, sync::Arc};

use ironclaw_host_api::{
    NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceScope,
};
use ironclaw_network::{NetworkHttpEgress, NetworkHttpRequest};
use ironclaw_run_state::RunStateStore;
use ironclaw_secrets::SecretStore;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde_json::json;
use url::Url;

use crate::{
    OAuthError, OAuthResumeNotifier, OAuthStateStore, ProviderRegistry, TokenPersister, TokenSet,
};

const DEFAULT_OAUTH_CALLBACK_BASE: &str = "http://127.0.0.1:0";
const OAUTH_RESPONSE_LIMIT: u64 = 1024 * 1024;

#[derive(Clone)]
pub enum ProviderMode {
    Brokered {
        broker_url: Url,
        broker_auth: SecretString,
    },
    Direct,
}

#[derive(Clone)]
pub struct OAuthRuntime {
    flow: OAuthFlow,
}

impl OAuthRuntime {
    pub fn builder(
        providers: Arc<ProviderRegistry>,
        egress: Arc<dyn NetworkHttpEgress>,
        secrets: Arc<dyn SecretStore>,
    ) -> OAuthRuntimeBuilder {
        OAuthRuntimeBuilder::new(providers, egress, secrets)
    }

    pub fn from_env(
        providers: Arc<ProviderRegistry>,
        egress: Arc<dyn NetworkHttpEgress>,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, OAuthError> {
        Self::builder(providers, egress, secrets).from_env()
    }

    pub fn flow(&self) -> OAuthFlow {
        self.flow.clone()
    }

    pub async fn start(
        &self,
        provider_id: &str,
        scopes: Vec<String>,
        scope: ResourceScope,
    ) -> Result<StartedFlow, OAuthError> {
        self.flow.start(provider_id, scopes, scope).await
    }

    pub async fn exchange(
        &self,
        provider_id: &str,
        code: String,
        state: String,
    ) -> Result<(), OAuthError> {
        self.flow.exchange(provider_id, code, state).await
    }

    pub async fn refresh_if_needed(
        &self,
        provider_id: &str,
        scope: &ResourceScope,
    ) -> Result<TokenSet, OAuthError> {
        self.flow.refresh(provider_id, scope).await
    }
}

#[derive(Clone)]
pub struct OAuthFlow {
    inner: Arc<OAuthRuntimeInner>,
}

struct OAuthRuntimeInner {
    mode: ProviderMode,
    state: Arc<OAuthStateStore>,
    providers: Arc<ProviderRegistry>,
    egress: Arc<dyn NetworkHttpEgress>,
    tokens: TokenPersister,
    resume: Arc<OAuthResumeNotifier>,
    run_state: Option<Arc<dyn RunStateStore>>,
    redirect_base_url: Url,
    allow_loopback_broker_for_tests: bool,
}

pub struct OAuthRuntimeBuilder {
    mode: Option<ProviderMode>,
    state: Arc<OAuthStateStore>,
    providers: Arc<ProviderRegistry>,
    egress: Arc<dyn NetworkHttpEgress>,
    secrets: Arc<dyn SecretStore>,
    resume: Arc<OAuthResumeNotifier>,
    run_state: Option<Arc<dyn RunStateStore>>,
    redirect_base_url: Option<Url>,
    allow_loopback_broker_for_tests: bool,
}

impl OAuthRuntimeBuilder {
    pub fn new(
        providers: Arc<ProviderRegistry>,
        egress: Arc<dyn NetworkHttpEgress>,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            mode: None,
            state: Arc::new(OAuthStateStore::new()),
            providers,
            egress,
            secrets,
            resume: Arc::new(OAuthResumeNotifier::default()),
            run_state: None,
            redirect_base_url: None,
            allow_loopback_broker_for_tests: false,
        }
    }

    pub fn mode(mut self, mode: ProviderMode) -> Self {
        self.mode = Some(mode);
        self
    }

    pub fn state(mut self, state: Arc<OAuthStateStore>) -> Self {
        self.state = state;
        self
    }

    pub fn resume(mut self, resume: Arc<OAuthResumeNotifier>) -> Self {
        self.resume = resume;
        self
    }

    pub fn run_state(mut self, run_state: Arc<dyn RunStateStore>) -> Self {
        self.run_state = Some(run_state);
        self
    }

    pub fn redirect_base_url(mut self, redirect_base_url: Url) -> Self {
        self.redirect_base_url = Some(redirect_base_url);
        self
    }

    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn allow_loopback_broker_for_tests(mut self) -> Self {
        self.allow_loopback_broker_for_tests = true;
        self
    }

    pub fn from_env(self) -> Result<OAuthRuntime, OAuthError> {
        let mut builder = self;
        if builder.mode.is_none() {
            builder.mode = provider_mode_from_env()?;
        }
        if builder.redirect_base_url.is_none() {
            let base = std::env::var("OAUTH_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_OAUTH_CALLBACK_BASE.to_string());
            builder.redirect_base_url =
                Some(
                    Url::parse(&base).map_err(|error| OAuthError::InvalidConfig {
                        reason: format!("OAUTH_BASE_URL is invalid: {error}"),
                    })?,
                );
        }
        builder.build()
    }

    pub fn build(self) -> Result<OAuthRuntime, OAuthError> {
        let mode = self.mode.unwrap_or(ProviderMode::Direct);
        if let ProviderMode::Brokered { broker_url, .. } = &mode {
            validate_oauth_endpoint(broker_url, self.allow_loopback_broker_for_tests)?;
        }
        let redirect_base_url = self
            .redirect_base_url
            .unwrap_or_else(default_redirect_base_url);
        Ok(OAuthRuntime {
            flow: OAuthFlow {
                inner: Arc::new(OAuthRuntimeInner {
                    mode,
                    state: self.state,
                    providers: self.providers,
                    egress: self.egress,
                    tokens: TokenPersister::new(self.secrets),
                    resume: self.resume,
                    run_state: self.run_state,
                    redirect_base_url,
                    allow_loopback_broker_for_tests: self.allow_loopback_broker_for_tests,
                }),
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedFlow {
    pub flow_id: uuid::Uuid,
    pub provider_id: String,
    pub credential_name: String,
    pub scopes: Vec<String>,
    pub redirect_uri: String,
    pub oauth_url: String,
}

impl OAuthFlow {
    pub fn credential_name(&self, provider_id: &str) -> Result<String, OAuthError> {
        Ok(self
            .inner
            .providers
            .get(provider_id)?
            .credential_name()
            .to_string())
    }

    pub fn resume_notifier(&self) -> Arc<OAuthResumeNotifier> {
        self.inner.resume.clone()
    }

    pub async fn start(
        &self,
        provider_id: &str,
        scopes: Vec<String>,
        scope: ResourceScope,
    ) -> Result<StartedFlow, OAuthError> {
        let provider = self.inner.providers.get(provider_id)?;
        validate_oauth_endpoint(
            &Url::parse(provider.auth_url()).map_err(|error| OAuthError::InvalidConfig {
                reason: format!("provider auth_url is invalid: {error}"),
            })?,
            false,
        )?;
        validate_oauth_endpoint(
            &Url::parse(provider.token_url()).map_err(|error| OAuthError::InvalidConfig {
                reason: format!("provider token_url is invalid: {error}"),
            })?,
            self.inner.allow_loopback_broker_for_tests,
        )?;
        let redirect_uri = callback_url(&self.inner.redirect_base_url, provider_id)?;
        let (state, pending) = self.inner.state.create(
            provider_id.to_string(),
            scopes.clone(),
            scope,
            redirect_uri.clone(),
        )?;
        let oauth_url =
            provider.build_authorize_url(&state, &pending.code_challenge, &scopes, &redirect_uri);
        Ok(StartedFlow {
            flow_id: pending.flow_id,
            provider_id: provider_id.to_string(),
            credential_name: provider.credential_name().to_string(),
            scopes,
            redirect_uri,
            oauth_url,
        })
    }

    pub async fn exchange(
        &self,
        provider_id: &str,
        code: String,
        state: String,
    ) -> Result<(), OAuthError> {
        let pending = self.inner.state.take(&state)?;
        if pending.provider_id != provider_id {
            return Err(OAuthError::ProviderMismatch {
                expected: pending.provider_id,
                actual: provider_id.to_string(),
            });
        }
        let provider = self.inner.providers.get(provider_id)?;
        let token_set = match &self.inner.mode {
            ProviderMode::Brokered {
                broker_url,
                broker_auth,
            } => {
                let url = broker_url.join("/oauth/exchange").map_err(|error| {
                    OAuthError::InvalidConfig {
                        reason: format!("broker exchange URL is invalid: {error}"),
                    }
                })?;
                let body = json!({
                    "code": code,
                    "redirect_uri": pending.redirect_uri,
                    "token_url": provider.token_url(),
                    "client_id": provider.public_client_id(),
                    "access_token_field": "access_token",
                });
                let response = self
                    .post_json(&pending.scope, url, Some(broker_auth), &body)
                    .await?;
                provider.parse_token_response(&response)?
            }
            ProviderMode::Direct => {
                let token_url = Url::parse(provider.token_url()).map_err(|error| {
                    OAuthError::InvalidConfig {
                        reason: format!("provider token_url is invalid: {error}"),
                    }
                })?;
                validate_oauth_endpoint(&token_url, false)?;
                let secret = provider.direct_client_secret().ok_or_else(|| {
                    OAuthError::IncompleteConfig {
                        provider: provider_id.to_string(),
                        reason: "direct mode requires client secret".to_string(),
                    }
                })?;
                let body = {
                    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
                    serializer
                        .append_pair("grant_type", "authorization_code")
                        .append_pair("code", &code)
                        .append_pair("redirect_uri", &pending.redirect_uri)
                        .append_pair("client_id", provider.public_client_id())
                        .append_pair("client_secret", secret.expose_secret())
                        .append_pair("code_verifier", &pending.code_verifier);
                    serializer.finish()
                };
                let response = self
                    .post_form(&pending.scope, token_url, None, body)
                    .await?;
                provider.parse_token_response(&response)?
            }
        };
        self.inner
            .tokens
            .persist(&pending.scope, provider.credential_name(), &token_set)
            .await?;
        if let Some(run_state) = &self.inner.run_state {
            self.inner
                .resume
                .notify_blocked_auth(
                    run_state.as_ref(),
                    provider.credential_name(),
                    &pending.scope,
                    pending.flow_id,
                )
                .await?;
        } else {
            self.inner
                .resume
                .notify(provider.credential_name(), pending.scope, pending.flow_id);
        }
        Ok(())
    }

    pub async fn refresh(
        &self,
        provider_id: &str,
        scope: &ResourceScope,
    ) -> Result<TokenSet, OAuthError> {
        let provider = self.inner.providers.get(provider_id)?;
        let refresh_token = self
            .inner
            .tokens
            .load_refresh_token(scope, provider.credential_name())
            .await?
            .ok_or_else(|| OAuthError::MissingRefreshToken {
                credential_name: provider.credential_name().to_string(),
            })?;
        let token_set = match &self.inner.mode {
            ProviderMode::Brokered {
                broker_url,
                broker_auth,
            } => {
                let url = broker_url.join("/oauth/refresh").map_err(|error| {
                    OAuthError::InvalidConfig {
                        reason: format!("broker refresh URL is invalid: {error}"),
                    }
                })?;
                let body = json!({
                    "refresh_token": refresh_token.expose_secret(),
                    "token_url": provider.token_url(),
                    "client_id": provider.public_client_id(),
                    "provider": provider.provider_id(),
                });
                let response = self.post_json(scope, url, Some(broker_auth), &body).await?;
                provider.parse_token_response(&response)?
            }
            ProviderMode::Direct => {
                let token_url = Url::parse(provider.token_url()).map_err(|error| {
                    OAuthError::InvalidConfig {
                        reason: format!("provider token_url is invalid: {error}"),
                    }
                })?;
                validate_oauth_endpoint(&token_url, false)?;
                let secret = provider.direct_client_secret().ok_or_else(|| {
                    OAuthError::IncompleteConfig {
                        provider: provider_id.to_string(),
                        reason: "direct mode requires client secret".to_string(),
                    }
                })?;
                let body = {
                    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
                    serializer
                        .append_pair("grant_type", "refresh_token")
                        .append_pair("refresh_token", refresh_token.expose_secret())
                        .append_pair("client_id", provider.public_client_id())
                        .append_pair("client_secret", secret.expose_secret());
                    serializer.finish()
                };
                let response = self.post_form(scope, token_url, None, body).await?;
                provider.parse_token_response(&response)?
            }
        };
        self.inner
            .tokens
            .persist(scope, provider.credential_name(), &token_set)
            .await?;
        Ok(token_set)
    }

    async fn post_json<T: Serialize>(
        &self,
        scope: &ResourceScope,
        url: Url,
        bearer: Option<&SecretString>,
        body: &T,
    ) -> Result<serde_json::Value, OAuthError> {
        let body = serde_json::to_vec(body)?;
        let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
        if let Some(bearer) = bearer {
            headers.push((
                "authorization".to_string(),
                format!("Bearer {}", bearer.expose_secret()),
            ));
        }
        self.post(scope, url, headers, body).await
    }

    async fn post_form(
        &self,
        scope: &ResourceScope,
        url: Url,
        bearer: Option<&SecretString>,
        body: String,
    ) -> Result<serde_json::Value, OAuthError> {
        let mut headers = vec![(
            "content-type".to_string(),
            "application/x-www-form-urlencoded".to_string(),
        )];
        if let Some(bearer) = bearer {
            headers.push((
                "authorization".to_string(),
                format!("Bearer {}", bearer.expose_secret()),
            ));
        }
        self.post(scope, url, headers, body.into_bytes()).await
    }

    async fn post(
        &self,
        scope: &ResourceScope,
        url: Url,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<serde_json::Value, OAuthError> {
        let policy = network_policy_for_url(&url, self.inner.allow_loopback_broker_for_tests)?;
        let response = self.inner.egress.execute(NetworkHttpRequest {
            scope: scope.clone(),
            method: NetworkMethod::Post,
            url: url.to_string(),
            headers,
            body,
            policy,
            response_body_limit: Some(OAUTH_RESPONSE_LIMIT),
            timeout_ms: Some(30_000),
        })?;
        if !(200..300).contains(&response.status) {
            return Err(OAuthError::HttpStatus {
                status: response.status,
                reason: redacted_http_failure_reason(&response.body),
            });
        }
        Ok(serde_json::from_slice(&response.body)?)
    }
}

pub fn broker_auth_from_env() -> Option<SecretString> {
    std::env::var("IRONCLAW_OAUTH_PROXY_AUTH_TOKEN")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("GATEWAY_AUTH_TOKEN")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .map(SecretString::from)
}

fn provider_mode_from_env() -> Result<Option<ProviderMode>, OAuthError> {
    let Some(broker_url) = std::env::var("IRONCLAW_OAUTH_EXCHANGE_URL")
        .ok()
        .filter(|value| !value.is_empty())
    else {
        return Ok(Some(ProviderMode::Direct));
    };
    let broker_url = Url::parse(&broker_url).map_err(|error| OAuthError::InvalidConfig {
        reason: format!("IRONCLAW_OAUTH_EXCHANGE_URL is invalid: {error}"),
    })?;
    let broker_auth = broker_auth_from_env().ok_or_else(|| OAuthError::IncompleteConfig {
        provider: "broker".to_string(),
        reason: "broker mode requires IRONCLAW_OAUTH_PROXY_AUTH_TOKEN or GATEWAY_AUTH_TOKEN"
            .to_string(),
    })?;
    Ok(Some(ProviderMode::Brokered {
        broker_url,
        broker_auth,
    }))
}

fn callback_url(base: &Url, provider_id: &str) -> Result<String, OAuthError> {
    Ok(base
        .join(&format!("/auth/callback/{provider_id}"))
        .map_err(|error| OAuthError::InvalidConfig {
            reason: format!("OAuth callback URL is invalid: {error}"),
        })?
        .to_string())
}

fn default_redirect_base_url() -> Url {
    // Safety: static literal is a valid URL and covered by tests through all
    // builder paths that do not pass an explicit base.
    Url::parse(DEFAULT_OAUTH_CALLBACK_BASE).expect("static callback base URL is valid")
}

fn redacted_http_failure_reason(body: &[u8]) -> String {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return "oauth endpoint returned non-success response".to_string();
    };
    value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .filter(|reason| is_stable_error_reason(reason))
        .unwrap_or("oauth endpoint returned non-success response")
        .to_string()
}

fn is_stable_error_reason(reason: &str) -> bool {
    !reason.is_empty()
        && reason.len() <= 80
        && reason
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn network_policy_for_url(url: &Url, allow_loopback: bool) -> Result<NetworkPolicy, OAuthError> {
    validate_oauth_endpoint(url, allow_loopback)?;
    let host = url.host_str().ok_or_else(|| OAuthError::InvalidConfig {
        reason: "OAuth endpoint host is required".to_string(),
    })?;
    let scheme = match url.scheme() {
        "https" => NetworkScheme::Https,
        "http" if allow_loopback && is_loopback_host(host) => NetworkScheme::Http,
        other => {
            return Err(OAuthError::rejected_url(
                url,
                format!("unsupported scheme {other}"),
            ));
        }
    };
    Ok(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(scheme),
            host_pattern: host.to_ascii_lowercase(),
            port: url.port(),
        }],
        deny_private_ip_ranges: !allow_loopback,
        max_egress_bytes: Some(2 * OAUTH_RESPONSE_LIMIT),
    })
}

fn validate_oauth_endpoint(url: &Url, allow_loopback: bool) -> Result<(), OAuthError> {
    let host = url.host_str().ok_or_else(|| OAuthError::InvalidConfig {
        reason: "OAuth endpoint host is required".to_string(),
    })?;
    if url.scheme() == "https" {
        // ok
    } else if !(allow_loopback && url.scheme() == "http" && is_loopback_host(host)) {
        return Err(OAuthError::rejected_url(
            url,
            "OAuth endpoints must use HTTPS",
        ));
    }
    if !allow_loopback && is_private_or_loopback_host(host) {
        return Err(OAuthError::rejected_url(
            url,
            "private or loopback endpoint is not allowed",
        ));
    }
    Ok(())
}

fn is_private_or_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(is_private_or_loopback_ip)
}

fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || {
                    let [first, second, ..] = ip.octets();
                    first == 100 && (64..=127).contains(&second)
                }
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8
        }
    }
}
