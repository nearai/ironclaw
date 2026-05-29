//! GitHub OAuth provider for the WebChat v2 login surface.
//!
//! Mirrors the v1 behavior in
//! `src/channels/web/oauth/providers.rs::GitHubProvider`:
//!
//! - Authorization URL uses scopes `read:user user:email`. GitHub's
//!   OAuth App flow does NOT support PKCE, so the `code_challenge`
//!   the trait passes is ignored — CSRF is protected solely by the
//!   `state` parameter the router mints and verifies on callback.
//! - Code exchange POSTs to GitHub's token endpoint (asking for a
//!   JSON response), then reads the authenticated `/user` profile and
//!   `/user/emails` list with the returned bearer token. The
//!   verified-email preference matches v1: primary verified email
//!   first, then any verified email, then the (unverified) profile
//!   email with `email_verified = false` so the downstream
//!   [`UserDirectory`](super::user_directory::UserDirectory) can fail
//!   closed on an unverified address.

use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::config::GitHubOAuthConfig;
use super::error::OAuthError;
use super::profile::OAuthUserProfile;
use super::provider::OAuthProvider;
use super::provider_name::OAuthProviderName;

const GITHUB_AUTH_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_URL: &str = "https://api.github.com/user";
const GITHUB_EMAILS_URL: &str = "https://api.github.com/user/emails";

/// Per-call timeout applied by the `reqwest::Client` to each
/// individual GitHub HTTP request (token, user, emails). Bounds a
/// single stalled call. The default `reqwest::Client` has no timeout,
/// which would let a hung GitHub response pin the callback handler
/// indefinitely.
const GITHUB_HTTP_TIMEOUT: Duration = Duration::from_secs(8);

/// Overall budget for the full `exchange_code` chain (token -> user ->
/// emails). The per-call timeout bounds one request; without a wrapping
/// budget, three sequential 8s stalls could pin a Tokio task for ~24s,
/// so the whole exchange is wrapped in this shorter ceiling. Whichever
/// limit trips first fails the exchange closed.
const GITHUB_EXCHANGE_BUDGET: Duration = Duration::from_secs(20);

/// GitHub OAuth provider.
pub struct GitHubProvider {
    /// Cached provider name. Constructed once at provider build time
    /// so `OAuthProvider::name()` is allocation-free and returns the
    /// same instance on every call (the URL `{provider}` segment from
    /// the callback is compared against this exact value).
    name: OAuthProviderName,
    client_id: String,
    client_secret: SecretString,
    http: reqwest::Client,
    /// Overridable for tests; production callers leave these at the
    /// real GitHub endpoints.
    auth_endpoint: String,
    token_endpoint: String,
    user_endpoint: String,
    emails_endpoint: String,
}

impl GitHubProvider {
    /// Build a provider from an operator-supplied
    /// [`GitHubOAuthConfig`] using the real GitHub endpoints.
    pub fn new(config: GitHubOAuthConfig) -> Self {
        Self::with_endpoints_inner(
            config,
            GITHUB_AUTH_URL,
            GITHUB_TOKEN_URL,
            GITHUB_USER_URL,
            GITHUB_EMAILS_URL,
        )
    }

    /// Test-only constructor: lets the caller-level test harness
    /// substitute the GitHub endpoints with a local mock server. The
    /// `dev-in-memory-session` feature gate keeps the helper out of
    /// production builds for the same reason the in-memory session
    /// store is gated. Mirrors `GoogleProvider::with_endpoints`.
    #[cfg(any(test, feature = "dev-in-memory-session"))]
    pub fn with_endpoints(
        config: GitHubOAuthConfig,
        auth_endpoint: impl Into<String>,
        token_endpoint: impl Into<String>,
        user_endpoint: impl Into<String>,
        emails_endpoint: impl Into<String>,
    ) -> Self {
        Self::with_endpoints_inner(
            config,
            auth_endpoint,
            token_endpoint,
            user_endpoint,
            emails_endpoint,
        )
    }

    fn with_endpoints_inner(
        config: GitHubOAuthConfig,
        auth_endpoint: impl Into<String>,
        token_endpoint: impl Into<String>,
        user_endpoint: impl Into<String>,
        emails_endpoint: impl Into<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(GITHUB_HTTP_TIMEOUT)
            // GitHub's API rejects requests without a User-Agent
            // header (HTTP 403). Set one on the client so every call
            // carries it.
            .user_agent("IronClaw-WebChat-v2")
            .build()
            .expect("reqwest client build failed: TLS/runtime init is unrecoverable"); // safety: ClientBuilder::build only fails on unrecoverable rustls/tokio init; a silent Client::new() fallback would drop the timeout + User-Agent (unbounded hang, GitHub 403s) and panics on the same fault anyway
        Self {
            name: OAuthProviderName::new("github").expect("\"github\" satisfies the grammar"), // safety: literal satisfies OAuthProviderName grammar (lowercase ascii, 6 chars); checked by `OAuthProviderName::accepts_lowercase_alphanumeric`
            client_id: config.client_id,
            client_secret: config.client_secret,
            http,
            auth_endpoint: auth_endpoint.into(),
            token_endpoint: token_endpoint.into(),
            user_endpoint: user_endpoint.into(),
            emails_endpoint: emails_endpoint.into(),
        }
    }
}

/// GitHub's token endpoint answers `200 OK` even on failure, encoding
/// the failure as `{ "error": ... }` rather than a non-2xx status, so
/// we must inspect the body either way. The human-readable
/// `error_description` is deliberately NOT deserialized: it can carry
/// user-identifiable context and we never want it in a log line.
#[derive(Deserialize)]
struct GitHubTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct GitHubUser {
    id: u64,
    login: String,
    name: Option<String>,
    email: Option<String>,
}

#[derive(Deserialize)]
struct GitHubEmail {
    email: String,
    verified: bool,
    primary: bool,
}

#[async_trait]
impl OAuthProvider for GitHubProvider {
    fn name(&self) -> &OAuthProviderName {
        &self.name
    }

    fn authorization_url(&self, callback_url: &str, state: &str, _code_challenge: &str) -> String {
        // GitHub does not support PKCE; the `code_challenge` arg is
        // intentionally ignored. CSRF is protected via `state`.
        format!(
            "{auth}?response_type=code&client_id={client_id}&redirect_uri={redirect}&scope={scope}&state={state}",
            auth = self.auth_endpoint,
            client_id = urlencoding::encode(&self.client_id),
            redirect = urlencoding::encode(callback_url),
            scope = urlencoding::encode("read:user user:email"),
            state = urlencoding::encode(state),
        )
    }

    async fn exchange_code(
        &self,
        code: &str,
        callback_url: &str,
        _code_verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError> {
        // Bound the whole token -> user -> emails chain, not just each
        // individual call. The per-call `GITHUB_HTTP_TIMEOUT` bounds one
        // request; this wraps the sequence so a partially-degraded
        // GitHub cannot pin a Tokio task for the sum of every call's
        // timeout.
        tokio::time::timeout(
            GITHUB_EXCHANGE_BUDGET,
            self.do_exchange_code(code, callback_url),
        )
        .await
        .map_err(|_| OAuthError::CodeExchange("GitHub OAuth exchange timed out".to_string()))?
    }
}

impl GitHubProvider {
    /// Inner exchange body, wrapped by [`OAuthProvider::exchange_code`]
    /// in an overall timeout budget. Runs the GitHub token -> user ->
    /// emails sequence and projects the result to an
    /// [`OAuthUserProfile`].
    async fn do_exchange_code(
        &self,
        code: &str,
        callback_url: &str,
    ) -> Result<OAuthUserProfile, OAuthError> {
        // 1. Exchange the authorization code for an access token.
        let resp = self
            .http
            .post(&self.token_endpoint)
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.expose_secret()),
                ("code", code),
                ("redirect_uri", callback_url),
            ])
            .send()
            .await
            .map_err(|err| OAuthError::CodeExchange(err.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::debug!(
                %status,
                body = %body,
                "github token endpoint returned non-success response"
            );
            return Err(OAuthError::CodeExchange(format!(
                "GitHub token endpoint returned {status}"
            )));
        }

        let token: GitHubTokenResponse = resp
            .json()
            .await
            .map_err(|err| OAuthError::CodeExchange(err.to_string()))?;
        // GitHub signals failure in the 200 body, not via status.
        if let Some(error) = token.error {
            // Only the opaque error code is logged; GitHub's
            // human-readable `error_description` can carry
            // user-identifiable context and is never deserialized.
            tracing::debug!(
                error = %error,
                "github token endpoint returned an error in the response body"
            );
            return Err(OAuthError::CodeExchange(format!(
                "GitHub token endpoint returned error: {error}"
            )));
        }
        let access_token = token.access_token.ok_or_else(|| {
            OAuthError::CodeExchange("GitHub did not return an access_token".to_string())
        })?;

        // 2. Fetch the authenticated user profile.
        let user_resp = self
            .http
            .get(&self.user_endpoint)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|err| OAuthError::ProfileFetch(err.to_string()))?;
        if !user_resp.status().is_success() {
            let status = user_resp.status();
            return Err(OAuthError::ProfileFetch(format!(
                "GitHub user endpoint returned {status}"
            )));
        }
        let user: GitHubUser = user_resp
            .json()
            .await
            .map_err(|err| OAuthError::ProfileFetch(err.to_string()))?;

        // 3. Fetch verified emails (the profile may not include one).
        let emails_resp = self
            .http
            .get(&self.emails_endpoint)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|err| OAuthError::ProfileFetch(err.to_string()))?;
        if !emails_resp.status().is_success() {
            let status = emails_resp.status();
            return Err(OAuthError::ProfileFetch(format!(
                "GitHub emails endpoint returned {status}"
            )));
        }
        let emails: Vec<GitHubEmail> = emails_resp
            .json()
            .await
            .map_err(|err| OAuthError::ProfileFetch(err.to_string()))?;

        // Prefer the primary verified email, then any verified email,
        // and only fall back to the unverified profile email if no
        // verified address exists — flagging it as unverified so the
        // user directory can reject it.
        let verified_email = emails
            .iter()
            .filter(|e| e.verified)
            .find(|e| e.primary)
            .or_else(|| emails.iter().find(|e| e.verified));
        let (email, email_verified) = match verified_email {
            Some(e) => (Some(e.email.clone()), true),
            None => (user.email.clone(), false),
        };

        Ok(OAuthUserProfile {
            provider_user_id: user.id.to_string(),
            email,
            email_verified,
            display_name: user.name.or(Some(user.login)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::extract::Form;
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    use serde::Serialize;
    use std::collections::HashMap;
    use std::net::SocketAddr;

    fn cfg() -> GitHubOAuthConfig {
        GitHubOAuthConfig {
            client_id: "gh-client-id".to_string(),
            client_secret: SecretString::from("gh-client-secret".to_string()),
        }
    }

    #[test]
    fn authorization_url_includes_required_params_and_ignores_pkce() {
        let provider = GitHubProvider::new(cfg());
        let url = provider.authorization_url(
            "https://example.com/auth/callback/github",
            "csrf-token",
            "pkce-challenge-ignored",
        );
        assert!(url.starts_with(GITHUB_AUTH_URL));
        assert!(url.contains("client_id=gh-client-id"));
        // `read:user user:email`, URL-encoded.
        assert!(url.contains("scope=read%3Auser%20user%3Aemail"));
        assert!(url.contains("state=csrf-token"));
        // GitHub does not support PKCE — the challenge must NOT leak
        // into the authorization URL.
        assert!(!url.contains("code_challenge"));
        assert!(!url.contains("pkce-challenge-ignored"));
    }

    #[test]
    fn name_is_github() {
        let provider = GitHubProvider::new(cfg());
        assert_eq!(provider.name().as_str(), "github");
    }

    // ── mock GitHub endpoints ─────────────────────────────────────────

    #[derive(Serialize, Clone)]
    struct MockEmail {
        email: &'static str,
        verified: bool,
        primary: bool,
    }

    #[derive(Clone)]
    struct MockGitHub {
        /// Raw body returned by the token endpoint (so tests can send
        /// malformed JSON / error bodies as well as success).
        token_body: String,
        token_status: StatusCode,
        user_status: StatusCode,
        user_body: serde_json::Value,
        emails_status: StatusCode,
        emails: Vec<MockEmail>,
    }

    impl MockGitHub {
        fn success() -> Self {
            Self {
                token_body: r#"{"access_token":"gho_fake","token_type":"bearer"}"#.to_string(),
                token_status: StatusCode::OK,
                user_status: StatusCode::OK,
                user_body: serde_json::json!({
                    "id": 4242,
                    "login": "octocat",
                    "name": "The Octocat",
                    "email": null,
                }),
                emails_status: StatusCode::OK,
                emails: vec![
                    MockEmail {
                        email: "secondary@example.com",
                        verified: true,
                        primary: false,
                    },
                    MockEmail {
                        email: "primary@example.com",
                        verified: true,
                        primary: true,
                    },
                ],
            }
        }
    }

    /// Aborts the spawned mock-server task when the test's binding
    /// goes out of scope, so neither the task nor its `TcpListener`
    /// outlives the test that created it.
    struct AbortOnDrop(tokio::task::JoinHandle<()>);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    async fn spawn_mock(mock: MockGitHub) -> (SocketAddr, AbortOnDrop) {
        let token_mock = mock.clone();
        let user_mock = mock.clone();
        let emails_mock = mock.clone();

        let router = axum::Router::new()
            .route(
                "/token",
                post(move |_: Form<HashMap<String, String>>| {
                    let mock = token_mock.clone();
                    async move {
                        axum::response::Response::builder()
                            .status(mock.token_status)
                            .header(axum::http::header::CONTENT_TYPE, "application/json")
                            .body(axum::body::Body::from(mock.token_body))
                            .expect("token response")
                    }
                }),
            )
            .route(
                "/user",
                get(move || {
                    let mock = user_mock.clone();
                    async move { (mock.user_status, Json(mock.user_body)).into_response() }
                }),
            )
            .route(
                "/emails",
                get(move || {
                    let mock = emails_mock.clone();
                    async move { (mock.emails_status, Json(mock.emails)).into_response() }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (addr, AbortOnDrop(handle))
    }

    use axum::response::IntoResponse;

    fn provider_for(addr: SocketAddr) -> GitHubProvider {
        GitHubProvider::with_endpoints(
            cfg(),
            "https://github.test/login/oauth/authorize",
            format!("http://{addr}/token"),
            format!("http://{addr}/user"),
            format!("http://{addr}/emails"),
        )
    }

    async fn exchange(addr: SocketAddr) -> Result<OAuthUserProfile, OAuthError> {
        provider_for(addr)
            .exchange_code(
                "fake-code",
                "https://example.com/auth/callback/github",
                "ignored-verifier",
            )
            .await
    }

    #[tokio::test]
    async fn exchange_code_prefers_primary_verified_email() {
        let (addr, _server) = spawn_mock(MockGitHub::success()).await;
        let profile = exchange(addr).await.expect("exchange success");
        assert_eq!(profile.provider_user_id, "4242");
        assert_eq!(profile.email.as_deref(), Some("primary@example.com"));
        assert!(profile.email_verified);
        assert_eq!(profile.display_name.as_deref(), Some("The Octocat"));
    }

    #[tokio::test]
    async fn exchange_code_falls_back_to_any_verified_email() {
        let mut mock = MockGitHub::success();
        // No primary flagged — the only verified address must win.
        mock.emails = vec![
            MockEmail {
                email: "unverified@example.com",
                verified: false,
                primary: true,
            },
            MockEmail {
                email: "verified@example.com",
                verified: true,
                primary: false,
            },
        ];
        let (addr, _server) = spawn_mock(mock).await;
        let profile = exchange(addr).await.expect("exchange success");
        assert_eq!(profile.email.as_deref(), Some("verified@example.com"));
        assert!(profile.email_verified);
    }

    #[tokio::test]
    async fn exchange_code_falls_back_to_unverified_profile_email() {
        let mut mock = MockGitHub::success();
        mock.user_body = serde_json::json!({
            "id": 7,
            "login": "octocat",
            "name": null,
            "email": "profile@example.com",
        });
        // No verified emails at all.
        mock.emails = vec![MockEmail {
            email: "profile@example.com",
            verified: false,
            primary: true,
        }];
        let (addr, _server) = spawn_mock(mock).await;
        let profile = exchange(addr).await.expect("exchange success");
        assert_eq!(profile.email.as_deref(), Some("profile@example.com"));
        assert!(
            !profile.email_verified,
            "unverified GitHub email must not be marked verified",
        );
        // No name → falls back to the login handle.
        assert_eq!(profile.display_name.as_deref(), Some("octocat"));
    }

    #[tokio::test]
    async fn exchange_code_rejects_token_error_body_returned_with_200() {
        let mut mock = MockGitHub::success();
        // GitHub's documented failure shape: HTTP 200 + error body.
        mock.token_body =
            r#"{"error":"bad_verification_code","error_description":"should not leak"}"#
                .to_string();
        let (addr, _server) = spawn_mock(mock).await;
        let err = exchange(addr).await.expect_err("must reject error body");
        assert!(
            matches!(&err, OAuthError::CodeExchange(msg) if msg.contains("bad_verification_code")),
            "expected CodeExchange referencing the error code, got {err:?}",
        );
        assert!(
            !format!("{err:?}").contains("should not leak"),
            "the human-readable description must not be echoed: {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_non_2xx_token_response() {
        let mut mock = MockGitHub::success();
        mock.token_status = StatusCode::INTERNAL_SERVER_ERROR;
        mock.token_body = "boom".to_string();
        let (addr, _server) = spawn_mock(mock).await;
        let err = exchange(addr).await.expect_err("must reject 5xx");
        assert!(
            matches!(&err, OAuthError::CodeExchange(msg) if msg.contains("500")),
            "expected status-only CodeExchange, got {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_token_response_without_access_token() {
        let mut mock = MockGitHub::success();
        mock.token_body = r#"{"token_type":"bearer"}"#.to_string();
        let (addr, _server) = spawn_mock(mock).await;
        let err = exchange(addr).await.expect_err("must reject missing token");
        assert!(
            matches!(&err, OAuthError::CodeExchange(msg) if msg.contains("access_token")),
            "expected CodeExchange referencing access_token, got {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_user_endpoint_failure() {
        let mut mock = MockGitHub::success();
        mock.user_status = StatusCode::UNAUTHORIZED;
        let (addr, _server) = spawn_mock(mock).await;
        let err = exchange(addr).await.expect_err("must reject user 401");
        assert!(
            matches!(&err, OAuthError::ProfileFetch(msg) if msg.contains("401")),
            "expected ProfileFetch for user endpoint failure, got {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_emails_endpoint_failure() {
        let mut mock = MockGitHub::success();
        mock.emails_status = StatusCode::INTERNAL_SERVER_ERROR;
        let (addr, _server) = spawn_mock(mock).await;
        let err = exchange(addr).await.expect_err("must reject emails 5xx");
        assert!(
            matches!(&err, OAuthError::ProfileFetch(msg) if msg.contains("500")),
            "expected ProfileFetch for emails endpoint failure, got {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_returns_none_email_when_no_verified_and_no_profile_email() {
        // No verified emails AND a null profile email exercises the
        // `None => (user.email.clone(), false)` arm with a truly absent
        // address — the profile carries `email: None`, which is a valid
        // (directory-handled) shape, NOT an error: `OAuthUserProfile.email`
        // is `Option` by contract and `UserDirectory` falls back to the
        // provider-unique id when no verified email is present.
        let mut mock = MockGitHub::success();
        mock.user_body = serde_json::json!({
            "id": 9,
            "login": "ghost",
            "name": null,
            "email": null,
        });
        mock.emails = vec![];
        let (addr, _server) = spawn_mock(mock).await;
        let profile = exchange(addr).await.expect("exchange success");
        assert!(
            profile.email.is_none(),
            "email must be None when no verified email and no profile email exist",
        );
        assert!(!profile.email_verified);
        assert_eq!(profile.display_name.as_deref(), Some("ghost"));
    }

    #[test]
    fn authorization_url_encodes_state_with_special_characters() {
        let provider = GitHubProvider::new(cfg());
        // base64url / JWT-style state can contain `+`, `/`, `=`, and
        // spaces — all of which must be percent-encoded so the router
        // recovers the exact value on callback (state is GitHub's only
        // CSRF mechanism since PKCE is absent).
        let state = "abc+def/ghi=jkl mno";
        let url = provider.authorization_url("https://example.com/cb", state, "ignored");
        let encoded = urlencoding::encode(state);
        assert!(
            url.contains(&format!("state={encoded}")),
            "state must appear percent-encoded; got {url}",
        );
        // The raw special characters must not leak into the query.
        assert!(
            !url.contains("state=abc+def"),
            "+ must be percent-encoded: {url}"
        );
        assert!(
            !url.contains("ghi=jkl"),
            "= inside the state value must be percent-encoded: {url}",
        );
    }
}
