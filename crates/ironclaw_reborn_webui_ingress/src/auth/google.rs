//! Google OAuth (OIDC) provider for the WebChat v2 login surface.
//!
//! Mirrors the v1 behavior in
//! `src/channels/web/oauth/providers.rs::GoogleProvider`:
//!
//! - Authorization URL uses OIDC scopes `openid email profile`, PKCE
//!   S256, and an optional `hd=` hosted-domain hint.
//! - Code exchange POSTs to the Google token endpoint with the PKCE
//!   verifier; the returned `id_token` is decoded WITHOUT signature
//!   verification (the token arrived over TLS directly from Google)
//!   but `aud` (client id) and `iss` are still validated to prevent
//!   token substitution.
//! - When the operator set [`GoogleOAuthConfig::allowed_hd`], the
//!   callback rejects any ID token whose `hd` claim does not match —
//!   the URL hint is a UX nudge, not a security boundary.

use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::config::GoogleOAuthConfig;
use super::error::{OAuthError, ProviderInitError};
use super::profile::OAuthUserProfile;
use super::provider::OAuthProvider;
use super::provider_http::{describe_transport_error, read_capped_body, sanitize_error_code};
use super::provider_name::OAuthProviderName;

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_ISSUER: &str = "https://accounts.google.com";
/// Per-request DEFAULT timeout on the Google token endpoint. The default
/// `reqwest::Client` has no timeout, which would let a hung Google
/// response pin the callback handler indefinitely. Operators on a slow /
/// cross-border path can override it via `GoogleOAuthConfig::http_timeout`
/// (the reborn CLI exposes `IRONCLAW_REBORN_WEBUI_OAUTH_HTTP_TIMEOUT_SECS`).
const GOOGLE_HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// Google OIDC provider.
pub struct GoogleProvider {
    /// Cached provider name. Constructed once at provider build time
    /// so `OAuthProvider::name()` is allocation-free and returns the
    /// same instance on every call (the URL `{provider}` segment
    /// from the callback is compared against this exact value).
    name: OAuthProviderName,
    client_id: String,
    client_secret: SecretString,
    allowed_hd: Option<String>,
    http: reqwest::Client,
    /// Overridable for tests; production callers leave it at the
    /// default `https://oauth2.googleapis.com/token`.
    token_endpoint: String,
    /// Overridable for tests; production callers leave it at the
    /// default `https://accounts.google.com/o/oauth2/v2/auth`.
    auth_endpoint: String,
}

impl GoogleProvider {
    /// Build a provider from an operator-supplied
    /// [`GoogleOAuthConfig`] using the real Google endpoints.
    ///
    /// Fallible for the same reason as
    /// [`GitHubProvider::new`](super::github::GitHubProvider::new): the
    /// `reqwest::Client` build can fail if the rustls / tokio runtime
    /// cannot initialize. Surfacing it as a `Result` lets the host
    /// composition layer fail startup loudly rather than silently
    /// shipping a client with no timeout (a hung token endpoint would
    /// otherwise pin the callback task) or panicking in a constructor.
    pub fn new(config: GoogleOAuthConfig) -> Result<Self, ProviderInitError> {
        Self::with_endpoints_inner(config, GOOGLE_AUTH_URL, GOOGLE_TOKEN_URL)
    }

    /// Test-only constructor: lets the caller-level test harness
    /// substitute the auth / token endpoint URLs with a local mock
    /// server. The `dev-in-memory-session` feature gate keeps the
    /// helper out of production builds for the same reason the
    /// in-memory session store is gated.
    #[cfg(any(test, feature = "dev-in-memory-session"))]
    pub fn with_endpoints(
        config: GoogleOAuthConfig,
        auth_endpoint: impl Into<String>,
        token_endpoint: impl Into<String>,
    ) -> Result<Self, ProviderInitError> {
        Self::with_endpoints_inner(config, auth_endpoint, token_endpoint)
    }

    fn with_endpoints_inner(
        config: GoogleOAuthConfig,
        auth_endpoint: impl Into<String>,
        token_endpoint: impl Into<String>,
    ) -> Result<Self, ProviderInitError> {
        let http = reqwest::Client::builder()
            .timeout(config.http_timeout.unwrap_or(GOOGLE_HTTP_TIMEOUT))
            .build()
            .map_err(|err| ProviderInitError(err.to_string()))?;
        Ok(Self {
            name: OAuthProviderName::new("google").expect("\"google\" satisfies the grammar"), // safety: literal satisfies OAuthProviderName grammar (lowercase ascii, 6 chars); checked by `OAuthProviderName::accepts_lowercase_alphanumeric`
            client_id: config.client_id,
            client_secret: config.client_secret,
            allowed_hd: config.allowed_hd,
            http,
            token_endpoint: token_endpoint.into(),
            auth_endpoint: auth_endpoint.into(),
        })
    }
}

#[derive(Deserialize)]
struct GoogleTokenResponse {
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct GoogleTokenErrorResponse {
    error: Option<String>,
}

#[derive(Deserialize)]
struct GoogleIdTokenClaims {
    sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
    exp: i64,
    /// Google Workspace hosted domain claim (e.g. `company.com`).
    hd: Option<String>,
}

#[async_trait]
impl OAuthProvider for GoogleProvider {
    fn name(&self) -> &OAuthProviderName {
        &self.name
    }

    fn authorization_url(&self, callback_url: &str, state: &str, code_challenge: &str) -> String {
        let mut url = format!(
            "{auth}?response_type=code&client_id={client_id}&redirect_uri={redirect}&scope={scope}&state={state}&code_challenge={challenge}&code_challenge_method=S256&access_type=online",
            auth = self.auth_endpoint,
            client_id = urlencoding::encode(&self.client_id),
            redirect = urlencoding::encode(callback_url),
            scope = urlencoding::encode("openid email profile"),
            state = urlencoding::encode(state),
            challenge = urlencoding::encode(code_challenge),
        );
        if let Some(hd) = &self.allowed_hd {
            url.push_str(&format!("&hd={}", urlencoding::encode(hd)));
        }
        url
    }

    async fn exchange_code(
        &self,
        code: &str,
        callback_url: &str,
        code_verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError> {
        let resp = self
            .http
            .post(&self.token_endpoint)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", callback_url),
                ("client_id", &self.client_id),
                ("client_secret", self.client_secret.expose_secret()),
                ("code_verifier", code_verifier),
            ])
            .send()
            .await
            .map_err(|err| OAuthError::CodeExchange(describe_transport_error(&err)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            // Read through the shared size cap (same hardened path the
            // GitHub provider uses) so a misconfigured / overridden token
            // endpoint cannot force an unbounded allocation in the callback
            // task. `reqwest` applies no body cap of its own.
            let body = read_capped_body(resp).await.unwrap_or_default();
            let error_code = serde_json::from_slice::<GoogleTokenErrorResponse>(&body)
                .ok()
                .and_then(|error| error.error);
            if let Some(error_code) = error_code {
                // Sanitize the provider-supplied code (attacker-
                // influenceable via an overridden token endpoint) before
                // it lands in an error string that gets logged — same
                // log-injection guard as the GitHub provider.
                let safe_error = sanitize_error_code(&error_code);
                return Err(OAuthError::CodeExchange(format!(
                    "Google token endpoint returned {status} ({safe_error})"
                )));
            }
            tracing::debug!(
                %status,
                body = %String::from_utf8_lossy(&body),
                "google token endpoint returned non-success response"
            );
            return Err(OAuthError::CodeExchange(format!(
                "Google token endpoint returned {status}"
            )));
        }

        let body = read_capped_body(resp)
            .await
            .map_err(OAuthError::CodeExchange)?;
        let tokens: GoogleTokenResponse = serde_json::from_slice(&body)
            .map_err(|err| OAuthError::CodeExchange(err.to_string()))?;
        let id_token = tokens.id_token.ok_or_else(|| {
            OAuthError::CodeExchange("Google did not return an id_token".to_string())
        })?;

        // Skip signature verification — token was received over TLS
        // directly from Google. Validate `aud` (prevents another
        // Google client substituting tokens), `iss` (defense in
        // depth against a forged TLS termination), and `exp` so
        // expired tokens still fail closed.
        let mut validation = jsonwebtoken::Validation::default();
        validation.insecure_disable_signature_validation();
        validation.set_audience(&[&self.client_id]);
        validation.set_issuer(&[GOOGLE_ISSUER, "accounts.google.com"]);
        validation.validate_exp = true;

        let token_data = jsonwebtoken::decode::<GoogleIdTokenClaims>(
            &id_token,
            &jsonwebtoken::DecodingKey::from_secret(&[]),
            &validation,
        )
        .map_err(|err| OAuthError::ProfileFetch(format!("decode id_token: {err}")))?;
        let claims = token_data.claims;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| OAuthError::ProfileFetch(format!("decode id_token: {err}")))?
            .as_secs() as i64;
        if claims.exp <= now {
            return Err(OAuthError::ProfileFetch(format!(
                "decode id_token: token expired at {}",
                claims.exp
            )));
        }

        // Server-side hosted-domain check. The URL `hd=` parameter
        // is only a UX hint — the user can bypass it by editing the
        // URL. Reject anything whose `hd` claim does not match the
        // operator-configured allow value.
        if let Some(required) = &self.allowed_hd {
            match claims.hd.as_deref() {
                Some(hd) if hd.eq_ignore_ascii_case(required) => {}
                _ => {
                    return Err(OAuthError::Denied(format!(
                        "account is not from the required hosted domain {required:?}"
                    )));
                }
            }
        }

        let email = claims.email;
        let email_verified = claims.email_verified.unwrap_or(false);
        // Google's ID token carries a single email; surface it as a verified
        // address only when the `email_verified` claim is true.
        let verified_emails = match (&email, email_verified) {
            (Some(addr), true) => vec![addr.clone()],
            _ => Vec::new(),
        };
        Ok(OAuthUserProfile {
            provider_user_id: claims.sub,
            email,
            email_verified,
            verified_emails,
            display_name: claims.name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::routing::post;
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
    use serde::Serialize;
    use std::net::SocketAddr;

    fn cfg(hd: Option<&str>) -> GoogleOAuthConfig {
        GoogleOAuthConfig {
            client_id: "client-id-123".to_string(),
            client_secret: SecretString::from("client-secret-xyz".to_string()),
            allowed_hd: hd.map(str::to_string),
            http_timeout: None,
        }
    }

    #[test]
    fn authorization_url_includes_required_oidc_params() {
        let provider = GoogleProvider::new(cfg(None)).expect("build provider");
        let url = provider.authorization_url(
            "https://example.com/auth/callback/google",
            "csrf-token",
            "pkce-challenge",
        );
        assert!(url.starts_with(GOOGLE_AUTH_URL));
        assert!(url.contains("client_id=client-id-123"));
        assert!(url.contains("scope=openid"));
        assert!(url.contains("code_challenge=pkce-challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=csrf-token"));
        assert!(!url.contains("&hd="));
    }

    #[test]
    fn authorization_url_appends_hd_hint_when_restricted() {
        let provider = GoogleProvider::new(cfg(Some("company.com"))).expect("build provider");
        let url = provider.authorization_url(
            "https://example.com/auth/callback/google",
            "csrf-token",
            "pkce-challenge",
        );
        assert!(url.contains("&hd=company.com"));
    }

    // ── mock Google token endpoint ────────────────────────────────────

    #[derive(Serialize)]
    struct MockTokenResponse {
        id_token: String,
        access_token: String,
    }

    #[derive(Serialize)]
    struct MockIdTokenClaims {
        sub: &'static str,
        email: &'static str,
        email_verified: bool,
        name: &'static str,
        aud: &'static str,
        iss: &'static str,
        iat: i64,
        exp: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        hd: Option<&'static str>,
    }

    fn make_id_token(client_id: &'static str, hd: Option<&'static str>, exp: i64) -> String {
        // Sign with HS256 + a dummy secret — `GoogleProvider` disables
        // signature verification, so any well-formed JWT decodes
        // successfully as long as the claims pass audience+issuer
        // validation.
        let now = chrono::Utc::now().timestamp();
        let claims = MockIdTokenClaims {
            sub: "google-sub-123",
            email: "alice@example.com",
            email_verified: true,
            name: "Alice Example",
            aud: client_id,
            iss: "https://accounts.google.com",
            iat: now,
            exp,
            hd,
        };
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(b"unused-test-secret"),
        )
        .expect("encode JWT")
    }

    async fn spawn_mock_token_endpoint(id_token: String) -> SocketAddr {
        let router = axum::Router::new().route(
            "/token",
            post(
                move |axum::extract::Form(_): axum::extract::Form<
                    std::collections::HashMap<String, String>,
                >| {
                    let id_token = id_token.clone();
                    async move {
                        Json(MockTokenResponse {
                            id_token,
                            access_token: "ya29.fake".to_string(),
                        })
                    }
                },
            ),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        addr
    }

    #[tokio::test]
    async fn exchange_code_decodes_id_token_into_profile() {
        let client_id: &'static str = "client-id-123";
        let id_token = make_id_token(client_id, None, chrono::Utc::now().timestamp() + 600);
        let addr = spawn_mock_token_endpoint(id_token).await;
        let endpoint = format!("http://{addr}/token");

        let provider =
            GoogleProvider::with_endpoints(cfg(None), "https://example.invalid/auth", endpoint)
                .expect("build provider");
        let profile = provider
            .exchange_code(
                "fake-auth-code",
                "https://example.com/auth/callback/google",
                "fake-verifier",
            )
            .await
            .expect("exchange success");

        assert_eq!(profile.provider_user_id, "google-sub-123");
        assert_eq!(profile.email.as_deref(), Some("alice@example.com"));
        assert!(profile.email_verified);
        assert_eq!(profile.display_name.as_deref(), Some("Alice Example"));
    }

    #[tokio::test]
    async fn exchange_code_rejects_mismatched_hosted_domain() {
        let client_id: &'static str = "client-id-123";
        let id_token = make_id_token(
            client_id,
            Some("attacker.example"),
            chrono::Utc::now().timestamp() + 600,
        );
        let addr = spawn_mock_token_endpoint(id_token).await;
        let endpoint = format!("http://{addr}/token");

        let provider = GoogleProvider::with_endpoints(
            cfg(Some("company.com")),
            "https://example.invalid/auth",
            endpoint,
        )
        .expect("build provider");
        let err = provider
            .exchange_code(
                "fake-auth-code",
                "https://example.com/auth/callback/google",
                "fake-verifier",
            )
            .await
            .expect_err("hd mismatch must reject");
        assert!(
            matches!(err, OAuthError::Denied(_)),
            "expected Denied, got {err:?}",
        );
    }

    // ── token endpoint failure shapes (reviewer finding #10) ──────────
    //
    // Coverage was previously limited to the success path plus the
    // hd-claim rejection. The reviewer flagged three uncovered
    // branches in `exchange_code`: non-2xx HTTP, malformed token
    // JSON, and a 200 response that lacks `id_token`. Each must
    // return `OAuthError::CodeExchange`, not a panic or a silent
    // bad-profile.

    /// Spawn a mock token endpoint that always answers with the
    /// supplied (status, body) pair. Used by the failure-shape
    /// tests below to simulate Google misbehaving.
    async fn spawn_token_endpoint_returning(
        status: axum::http::StatusCode,
        body: &'static str,
        content_type: &'static str,
    ) -> SocketAddr {
        let router = axum::Router::new().route(
            "/token",
            post(
                move |_: axum::extract::Form<std::collections::HashMap<String, String>>| async move {
                    axum::response::Response::builder()
                        .status(status)
                        .header(axum::http::header::CONTENT_TYPE, content_type)
                        .body(axum::body::Body::from(body))
                        .expect("response")
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        addr
    }

    async fn run_exchange_against(endpoint: String) -> OAuthError {
        let provider =
            GoogleProvider::with_endpoints(cfg(None), "https://example.invalid/auth", endpoint)
                .expect("build provider");
        provider
            .exchange_code(
                "fake-auth-code",
                "https://example.com/auth/callback/google",
                "fake-verifier",
            )
            .await
            .expect_err("expected error from misbehaving token endpoint")
    }

    #[tokio::test]
    async fn exchange_code_rejects_non_2xx_token_response() {
        let addr = spawn_token_endpoint_returning(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":"server_error","error_description":"should not leak"}"#,
            "application/json",
        )
        .await;
        let err = run_exchange_against(format!("http://{addr}/token")).await;
        assert!(
            matches!(&err, OAuthError::CodeExchange(msg) if msg.contains("500") && msg.contains("server_error")),
            "expected sanitized CodeExchange for non-2xx response, got {err:?}",
        );
        assert!(
            !format!("{err:?}").contains("should not leak"),
            "raw token error body leaked into error: {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_malformed_token_json() {
        let addr = spawn_token_endpoint_returning(
            axum::http::StatusCode::OK,
            "not actually json {{{",
            "application/json",
        )
        .await;
        let err = run_exchange_against(format!("http://{addr}/token")).await;
        assert!(
            matches!(err, OAuthError::CodeExchange(_)),
            "expected CodeExchange for malformed JSON, got {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_falls_back_to_status_only_for_non_json_error_body() {
        let addr = spawn_token_endpoint_returning(
            axum::http::StatusCode::BAD_GATEWAY,
            "<html>bad gateway</html>",
            "text/html",
        )
        .await;
        let err = run_exchange_against(format!("http://{addr}/token")).await;
        assert!(
            matches!(&err, OAuthError::CodeExchange(msg) if msg.contains("502")),
            "expected status-only CodeExchange for non-JSON error body, got {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_response_without_id_token() {
        // 200 + valid JSON but no `id_token` field. Code path is
        // the `tokens.id_token.ok_or_else(...)` branch.
        let addr = spawn_token_endpoint_returning(
            axum::http::StatusCode::OK,
            r#"{"access_token":"ya29.fake"}"#,
            "application/json",
        )
        .await;
        let err = run_exchange_against(format!("http://{addr}/token")).await;
        assert!(
            matches!(&err, OAuthError::CodeExchange(msg) if msg.contains("id_token")),
            "expected CodeExchange referencing id_token, got {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_expired_id_token_even_without_signature_validation() {
        let client_id: &'static str = "client-id-123";
        let id_token = make_id_token(client_id, None, chrono::Utc::now().timestamp() - 60);
        let addr = spawn_mock_token_endpoint(id_token).await;
        let endpoint = format!("http://{addr}/token");

        let provider =
            GoogleProvider::with_endpoints(cfg(None), "https://example.invalid/auth", endpoint)
                .expect("build provider");
        let err = provider
            .exchange_code(
                "fake-auth-code",
                "https://example.com/auth/callback/google",
                "fake-verifier",
            )
            .await
            .expect_err("expired id_token must be rejected");
        assert!(
            matches!(&err, OAuthError::ProfileFetch(_)),
            "expected ProfileFetch for expired token, got {err:?}",
        );
    }
}
