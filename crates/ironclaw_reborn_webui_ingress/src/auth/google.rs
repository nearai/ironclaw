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

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::config::GoogleOAuthConfig;
use super::error::OAuthError;
use super::profile::OAuthUserProfile;

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_ISSUER: &str = "https://accounts.google.com";
/// Per-request timeout on the Google token endpoint. The default
/// `reqwest::Client` has no timeout, which would let a hung Google
/// response pin the callback handler indefinitely. 10s comfortably
/// covers the worst-case TLS handshake + token exchange while
/// failing loud on a real outage.
const GOOGLE_HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Provider trait — the route handlers dispatch by provider name and
/// never depend on a concrete provider impl. Google is the only
/// implementation today; GitHub / NEAR will add their own impls under
/// the same trait without changing the route handlers.
#[async_trait]
pub trait OAuthProvider: Send + Sync + 'static {
    /// Stable provider identifier exposed on `/auth/providers` and
    /// matched against the `{provider}` path segment on login /
    /// callback. Lowercase, ASCII.
    fn name(&self) -> &'static str;

    /// Build the provider-side authorization URL the browser is
    /// redirected to. `callback_url` is the v2-owned
    /// `/auth/callback/{provider}` URL; `state` is the CSRF token
    /// stored in the pending-flow cache; `code_challenge` is the
    /// PKCE S256 challenge.
    fn authorization_url(&self, callback_url: &str, state: &str, code_challenge: &str) -> String;

    /// Exchange the authorization code returned by the provider for
    /// a normalized [`OAuthUserProfile`].
    async fn exchange_code(
        &self,
        code: &str,
        callback_url: &str,
        code_verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError>;
}

/// Google OIDC provider.
pub struct GoogleProvider {
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
    pub fn new(config: GoogleOAuthConfig) -> Self {
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
    ) -> Self {
        Self::with_endpoints_inner(config, auth_endpoint, token_endpoint)
    }

    fn with_endpoints_inner(
        config: GoogleOAuthConfig,
        auth_endpoint: impl Into<String>,
        token_endpoint: impl Into<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(GOOGLE_HTTP_TIMEOUT)
            .build()
            // Builder failure here means rustls / tokio runtime is
            // genuinely broken; fall back to the default client so
            // we still surface a real OAuthError on the request
            // rather than a constructor panic.
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            client_id: config.client_id,
            client_secret: config.client_secret,
            allowed_hd: config.allowed_hd,
            http,
            token_endpoint: token_endpoint.into(),
            auth_endpoint: auth_endpoint.into(),
        }
    }
}

#[derive(Deserialize)]
struct GoogleTokenResponse {
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct GoogleIdTokenClaims {
    sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
    /// Google Workspace hosted domain claim (e.g. `company.com`).
    hd: Option<String>,
}

#[async_trait]
impl OAuthProvider for GoogleProvider {
    fn name(&self) -> &'static str {
        "google"
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
            .map_err(|err| OAuthError::CodeExchange(err.to_string()))?;

        if !resp.status().is_success() {
            // Body is logged at the route handler via tracing; never
            // echoed back to the browser.
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default(); // silent-ok: error already non-success, body only used for the operator tracing line
            return Err(OAuthError::CodeExchange(format!(
                "Google token endpoint returned {status}: {body}"
            )));
        }

        let tokens: GoogleTokenResponse = resp
            .json()
            .await
            .map_err(|err| OAuthError::CodeExchange(err.to_string()))?;
        let id_token = tokens.id_token.ok_or_else(|| {
            OAuthError::CodeExchange("Google did not return an id_token".to_string())
        })?;

        // Skip signature verification — token was received over TLS
        // directly from Google. Validate `aud` (prevents another
        // Google client substituting tokens) and `iss` (defense in
        // depth against a forged TLS termination).
        let mut validation = jsonwebtoken::Validation::default();
        validation.insecure_disable_signature_validation();
        validation.set_audience(&[&self.client_id]);
        validation.set_issuer(&[GOOGLE_ISSUER, "accounts.google.com"]);

        let token_data = jsonwebtoken::decode::<GoogleIdTokenClaims>(
            &id_token,
            &jsonwebtoken::DecodingKey::from_secret(&[]),
            &validation,
        )
        .map_err(|err| OAuthError::ProfileFetch(format!("decode id_token: {err}")))?;
        let claims = token_data.claims;

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

        Ok(OAuthUserProfile {
            provider_user_id: claims.sub,
            email: claims.email,
            email_verified: claims.email_verified.unwrap_or(false),
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
        }
    }

    #[test]
    fn authorization_url_includes_required_oidc_params() {
        let provider = GoogleProvider::new(cfg(None));
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
        let provider = GoogleProvider::new(cfg(Some("company.com")));
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

    fn make_id_token(client_id: &'static str, hd: Option<&'static str>) -> String {
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
            exp: now + 600,
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
        async fn handler(Json(body): Json<serde_json::Value>) -> Json<MockTokenResponse> {
            let _ = body; // form params aren't validated in the mock
            unreachable!("axum form extractor required, replaced below")
        }
        let _ = handler;

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
        let id_token = make_id_token(client_id, None);
        let addr = spawn_mock_token_endpoint(id_token).await;
        let endpoint = format!("http://{addr}/token");

        let provider =
            GoogleProvider::with_endpoints(cfg(None), "https://example.invalid/auth", endpoint);
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
        let id_token = make_id_token(client_id, Some("attacker.example"));
        let addr = spawn_mock_token_endpoint(id_token).await;
        let endpoint = format!("http://{addr}/token");

        let provider = GoogleProvider::with_endpoints(
            cfg(Some("company.com")),
            "https://example.invalid/auth",
            endpoint,
        );
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
}
