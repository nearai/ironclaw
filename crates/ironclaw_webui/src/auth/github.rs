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
use super::error::{OAuthError, ProviderInitError};
use super::profile::OAuthUserProfile;
use super::provider::OAuthProvider;
use super::provider_http::{describe_transport_error, read_capped_body, sanitize_error_code};
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
///
/// This is the DEFAULT; operators on a slow / cross-border path to
/// `github.com` can override it via `GitHubOAuthConfig::http_timeout`
/// (the reborn CLI exposes `IRONCLAW_REBORN_WEBUI_OAUTH_HTTP_TIMEOUT_SECS`).
/// The overall `exchange_budget` is derived from the effective per-call
/// timeout at construction so the "budget >= sum of calls" invariant
/// holds for any value.
const GITHUB_HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// Overall budget for the full `exchange_code` chain (token -> user ->
/// emails). The per-call timeout bounds one request; without a wrapping
/// budget, three sequential stalls could still pin a Tokio task for the
/// sum of every call's timeout, so the whole exchange is wrapped in this
/// ceiling as a backstop. Whichever limit trips first fails closed.
///
/// Floor for the derived budget. The constructor computes
/// `effective_timeout * 3 + 2s` and takes the max with this floor, so the
/// invariant `budget >= 3 * per_call` always holds while a single
/// genuinely-slow stage still fails with the precise per-call timeout
/// rather than the generic budget error.
const GITHUB_EXCHANGE_BUDGET_FLOOR: Duration = Duration::from_secs(20);

/// Defensive cap on the `/user/emails` list. A real account has a
/// handful of addresses; anything past this is treated as abuse /
/// misbehavior. Enforced *during* deserialization (see
/// [`CappedGitHubEmails`]) so a hostile endpoint cannot make serde
/// inflate an arbitrary number of owned `GitHubEmail` structs before a
/// post-hoc length check could reject them — allocation is bounded to
/// `MAX_GITHUB_EMAILS + 1` entries regardless of how many it streams
/// (the [`read_capped_body`] byte-cap bounds the raw transfer; this
/// bounds the parsed structures).
const MAX_GITHUB_EMAILS: usize = 100;

/// GitHub OAuth provider.
///
/// # PKCE is deliberately absent
///
/// GitHub's OAuth App flow does not implement PKCE: its authorization
/// endpoint ignores `code_challenge` and its token endpoint rejects a
/// `code_verifier`
/// (<https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps>).
/// So the `code_challenge` the router computes is intentionally dropped
/// in [`authorization_url`](GitHubProvider::authorization_url) rather
/// than sent.
///
/// **Accepted residual risk:** without PKCE there is no
/// authorization-code → client binding, so CSRF protection rests
/// *entirely* on the `state` parameter the router mints and verifies
/// (single-use, TTL-bounded, cross-provider-replay-guarded — see
/// `CLAUDE.md` §Security invariants). A future maintainer must NOT add
/// PKCE here expecting it to help (GitHub will ignore or reject it),
/// and must NOT copy this no-PKCE shape to a provider that *does*
/// support PKCE (where dropping it would be a real downgrade).
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
    /// Overall ceiling for the full `exchange_code` chain. Defaults to
    /// [`GITHUB_EXCHANGE_BUDGET`]; tests shrink it to exercise the
    /// timeout branch deterministically.
    exchange_budget: Duration,
}

impl GitHubProvider {
    /// Build a provider from an operator-supplied
    /// [`GitHubOAuthConfig`] using the real GitHub endpoints.
    ///
    /// Fallible: the `reqwest::Client` build can fail if the rustls /
    /// tokio runtime cannot initialize. Surfacing that as a `Result`
    /// (rather than panicking in the constructor) lets the host
    /// composition layer fail startup loudly. A silent fallback to
    /// `reqwest::Client::new()` is deliberately NOT used — it would
    /// drop the timeout and `User-Agent` (unbounded hang + GitHub
    /// 403s) and panic on the same fault anyway.
    pub fn new(config: GitHubOAuthConfig) -> Result<Self, ProviderInitError> {
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
    /// store is gated.
    #[cfg(any(test, feature = "dev-in-memory-session"))]
    pub fn with_endpoints(
        config: GitHubOAuthConfig,
        auth_endpoint: impl Into<String>,
        token_endpoint: impl Into<String>,
        user_endpoint: impl Into<String>,
        emails_endpoint: impl Into<String>,
    ) -> Result<Self, ProviderInitError> {
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
    ) -> Result<Self, ProviderInitError> {
        let effective_timeout = config.http_timeout.unwrap_or(GITHUB_HTTP_TIMEOUT);
        // Derive the overall budget from the effective per-call timeout so
        // the "budget >= 3 calls" invariant holds for any configured value,
        // never below the floor.
        let exchange_budget = effective_timeout
            .saturating_mul(3)
            .saturating_add(Duration::from_secs(2))
            .max(GITHUB_EXCHANGE_BUDGET_FLOOR);
        let http = reqwest::Client::builder()
            .timeout(effective_timeout)
            // GitHub's API rejects requests without a User-Agent
            // header (HTTP 403). Set one on the client so every call
            // carries it.
            .user_agent("IronClaw-WebChat-v2")
            .build()
            .map_err(|err| ProviderInitError(err.to_string()))?;
        Ok(Self {
            name: OAuthProviderName::new("github").expect("\"github\" satisfies the grammar"), // safety: literal satisfies OAuthProviderName grammar (lowercase ascii, 6 chars); checked by `OAuthProviderName::accepts_lowercase_alphanumeric`
            client_id: config.client_id,
            client_secret: config.client_secret,
            http,
            auth_endpoint: auth_endpoint.into(),
            token_endpoint: token_endpoint.into(),
            user_endpoint: user_endpoint.into(),
            emails_endpoint: emails_endpoint.into(),
            exchange_budget,
        })
    }

    /// Test / dev-only: shrink the overall exchange budget so the
    /// timeout branch can be exercised against a blackhole endpoint
    /// without a 20-second wait. Gated like [`Self::with_endpoints`]
    /// (the `dev-in-memory-session` feature) rather than `#[cfg(test)]`
    /// so caller-level tests in `tests/` — a separate crate that cannot
    /// see `#[cfg(test)]` items — can reach it too.
    #[cfg(any(test, feature = "dev-in-memory-session"))]
    pub fn with_exchange_budget(mut self, budget: Duration) -> Self {
        self.exchange_budget = budget;
        self
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

/// `Vec<GitHubEmail>` that refuses to grow past [`MAX_GITHUB_EMAILS`]
/// *while* deserializing. A post-hoc `.len()` check would let serde
/// allocate every entry of a hostile multi-thousand-element list first;
/// this stops at the first element past the cap, so peak allocation is
/// bounded to `MAX_GITHUB_EMAILS + 1` structs.
struct CappedGitHubEmails(Vec<GitHubEmail>);

impl<'de> Deserialize<'de> for CappedGitHubEmails {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EmailsVisitor;

        impl<'de> serde::de::Visitor<'de> for EmailsVisitor {
            type Value = Vec<GitHubEmail>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "an array of at most {MAX_GITHUB_EMAILS} GitHub emails")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut emails = Vec::new();
                while let Some(email) = seq.next_element::<GitHubEmail>()? {
                    if emails.len() >= MAX_GITHUB_EMAILS {
                        return Err(serde::de::Error::custom(format!(
                            "GitHub returned an implausible number of emails (>{MAX_GITHUB_EMAILS})"
                        )));
                    }
                    emails.push(email);
                }
                Ok(emails)
            }
        }

        deserializer
            .deserialize_seq(EmailsVisitor)
            .map(CappedGitHubEmails)
    }
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
            self.exchange_budget,
            self.do_exchange_code(code, callback_url),
        )
        .await
        .map_err(|_| OAuthError::CodeExchange("GitHub OAuth exchange timed out".to_string()))?
    }
}

impl GitHubProvider {
    /// Inner exchange body, wrapped by [`OAuthProvider::exchange_code`]
    /// in an overall timeout budget. Exchanges the code for a token,
    /// then fetches `/user` and `/user/emails` and projects the result
    /// to an [`OAuthUserProfile`].
    ///
    /// The three calls run sequentially on purpose: it keeps
    /// [`GITHUB_EXCHANGE_BUDGET`] a meaningful ceiling (the budget only
    /// binds across sequential stages — fanning the two profile reads
    /// out concurrently would cap the worst case at ~2 per-call
    /// timeouts, below the budget, leaving it dead). The saved
    /// round-trip is not worth a dead safeguard on this cold path.
    async fn do_exchange_code(
        &self,
        code: &str,
        callback_url: &str,
    ) -> Result<OAuthUserProfile, OAuthError> {
        let access_token = self.exchange_code_for_token(code, callback_url).await?;
        let user = self.fetch_user(&access_token).await?;
        let emails = self.fetch_verified_emails(&access_token).await?;

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

        // Carry ALL verified emails (primary first, for deterministic
        // selection downstream) so a host admission allowlist can match a
        // verified secondary address even when the primary is off-list.
        let mut verified_emails: Vec<String> = Vec::new();
        if let Some(primary) = emails.iter().find(|e| e.verified && e.primary) {
            verified_emails.push(primary.email.clone());
        }
        for entry in emails.iter().filter(|e| e.verified && !e.primary) {
            verified_emails.push(entry.email.clone());
        }

        Ok(OAuthUserProfile {
            provider_user_id: user.id.to_string(),
            email,
            email_verified,
            verified_emails,
            display_name: user.name.or(Some(user.login)),
        })
    }

    /// Step 1: exchange the authorization code for an access token,
    /// wrapped in [`SecretString`] so an accidental `{:?}` / tracing
    /// capture of an intermediate holder cannot expose it in plaintext.
    async fn exchange_code_for_token(
        &self,
        code: &str,
        callback_url: &str,
    ) -> Result<SecretString, OAuthError> {
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
            .map_err(|err| OAuthError::CodeExchange(describe_transport_error(&err)))?;

        if !resp.status().is_success() {
            // Deliberately do NOT read the body: it is untrusted
            // external content of unbounded size (a misconfigured /
            // non-HTTPS `token_endpoint` override could point at a
            // hostile server) and would only ever go to a log. The
            // status code alone is enough to diagnose the failure.
            let status = resp.status();
            tracing::debug!(%status, "github token endpoint returned non-success response");
            return Err(OAuthError::CodeExchange(format!(
                "GitHub token endpoint returned {status}"
            )));
        }

        let body = read_capped_body(resp)
            .await
            .map_err(OAuthError::CodeExchange)?;
        let token: GitHubTokenResponse = serde_json::from_slice(&body)
            .map_err(|err| OAuthError::CodeExchange(err.to_string()))?;
        // GitHub signals failure in the 200 body, not via status. An
        // empty `error` string is not a real failure — filter it out so
        // `{"error":"","access_token":"…"}` from a proxy/middleware does
        // not reject an otherwise-valid token.
        if let Some(error) = token.error.filter(|e| !e.is_empty()) {
            // Only the sanitized opaque error code is logged; GitHub's
            // human-readable `error_description` can carry
            // user-identifiable context and is never deserialized.
            let safe_error = sanitize_error_code(&error);
            tracing::debug!(
                error = %safe_error,
                "github token endpoint returned an error in the response body"
            );
            return Err(OAuthError::CodeExchange(format!(
                "GitHub token endpoint returned error: {safe_error}"
            )));
        }
        token
            .access_token
            // An empty-string token is as useless as a missing one —
            // treat it as missing rather than letting `Some("")` slip
            // through and surface later as a misleading 401 ProfileFetch.
            .filter(|t| !t.is_empty())
            .map(SecretString::from)
            .ok_or_else(|| OAuthError::CodeExchange("GitHub did not return an access_token".into()))
    }

    /// Step 2a: fetch the authenticated user profile.
    async fn fetch_user(&self, access_token: &SecretString) -> Result<GitHubUser, OAuthError> {
        let resp = self
            .http
            .get(&self.user_endpoint)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .bearer_auth(access_token.expose_secret())
            .send()
            .await
            .map_err(|err| OAuthError::ProfileFetch(describe_transport_error(&err)))?;
        if !resp.status().is_success() {
            return Err(OAuthError::ProfileFetch(format!(
                "GitHub user endpoint returned {}",
                resp.status()
            )));
        }
        let body = read_capped_body(resp)
            .await
            .map_err(OAuthError::ProfileFetch)?;
        serde_json::from_slice(&body).map_err(|err| OAuthError::ProfileFetch(err.to_string()))
    }

    /// Step 2b: fetch the user's verified emails. Rejects an
    /// implausibly large list as a defensive bound on the work done
    /// for one login (a normal account has a handful of addresses).
    async fn fetch_verified_emails(
        &self,
        access_token: &SecretString,
    ) -> Result<Vec<GitHubEmail>, OAuthError> {
        let resp = self
            .http
            .get(&self.emails_endpoint)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .bearer_auth(access_token.expose_secret())
            .send()
            .await
            .map_err(|err| OAuthError::ProfileFetch(describe_transport_error(&err)))?;
        if !resp.status().is_success() {
            return Err(OAuthError::ProfileFetch(format!(
                "GitHub emails endpoint returned {}",
                resp.status()
            )));
        }
        let body = read_capped_body(resp)
            .await
            .map_err(OAuthError::ProfileFetch)?;
        // `CappedGitHubEmails` enforces `MAX_GITHUB_EMAILS` mid-parse, so
        // an oversized list is rejected before serde allocates past the
        // cap — no separate post-deserialize length check needed.
        let emails: CappedGitHubEmails = serde_json::from_slice(&body)
            .map_err(|err| OAuthError::ProfileFetch(err.to_string()))?;
        Ok(emails.0)
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
            http_timeout: None,
        }
    }

    #[test]
    fn authorization_url_includes_required_params_and_ignores_pkce() {
        let provider = GitHubProvider::new(cfg()).expect("build provider");
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
        let provider = GitHubProvider::new(cfg()).expect("build provider");
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

    /// Bind an ephemeral loopback port, serve `router`, and return the
    /// address plus a drop guard that aborts the server. Shared by the
    /// full-mock and blackhole spawns below.
    async fn spawn_server(router: axum::Router) -> (SocketAddr, AbortOnDrop) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (addr, AbortOnDrop(handle))
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

        spawn_server(router).await
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
        .expect("build provider")
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
    async fn exchange_code_rejects_oversized_token_response() {
        use super::super::provider_http::MAX_RESPONSE_BYTES;
        let mut mock = MockGitHub::success();
        // A body past the shared cap must be rejected by
        // `read_capped_body` before serde parses anything.
        mock.token_body = "x".repeat(MAX_RESPONSE_BYTES + 1);
        let (addr, _server) = spawn_mock(mock).await;
        let err = exchange(addr)
            .await
            .expect_err("must reject an oversized response body");
        assert!(
            matches!(&err, OAuthError::CodeExchange(msg) if msg.contains("exceeds")),
            "expected CodeExchange for oversized body, got {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_rejects_empty_string_access_token() {
        let mut mock = MockGitHub::success();
        // `Some("")` must be treated as a missing token, not sent as a
        // `Bearer ` header that surfaces later as a misleading 401.
        mock.token_body = r#"{"access_token":"","token_type":"bearer"}"#.to_string();
        let (addr, _server) = spawn_mock(mock).await;
        let err = exchange(addr)
            .await
            .expect_err("empty access_token must be rejected");
        assert!(
            matches!(&err, OAuthError::CodeExchange(msg) if msg.contains("access_token")),
            "expected CodeExchange for empty access_token, got {err:?}",
        );
    }

    #[tokio::test]
    async fn exchange_code_ignores_empty_error_field_with_valid_token() {
        let mut mock = MockGitHub::success();
        // An empty `error` string from a proxy/middleware must NOT fail
        // an otherwise-valid token exchange.
        mock.token_body =
            r#"{"error":"","access_token":"gho_valid","token_type":"bearer"}"#.to_string();
        let (addr, _server) = spawn_mock(mock).await;
        let profile = exchange(addr)
            .await
            .expect("empty error must not reject a valid token");
        assert_eq!(profile.provider_user_id, "4242");
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
    async fn exchange_code_rejects_implausible_email_count() {
        let mut mock = MockGitHub::success();
        // One past the cap. The count guard does not inspect content,
        // so a single repeated literal is enough to exercise it.
        mock.emails = vec![
            MockEmail {
                email: "x@example.com",
                verified: false,
                primary: false,
            };
            MAX_GITHUB_EMAILS + 1
        ];
        let (addr, _server) = spawn_mock(mock).await;
        let err = exchange(addr)
            .await
            .expect_err("must reject an implausible email count");
        assert!(
            matches!(&err, OAuthError::ProfileFetch(msg) if msg.contains("implausible")),
            "expected ProfileFetch for implausible email count, got {err:?}",
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
        let provider = GitHubProvider::new(cfg()).expect("build provider");
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

    /// Spawn a mock GitHub whose `/token` endpoint accepts the request
    /// but never responds (a blackhole), so the only thing that can end
    /// the exchange is the overall budget timer.
    async fn spawn_blackhole_token() -> (SocketAddr, AbortOnDrop) {
        async fn blackhole() -> Json<serde_json::Value> {
            std::future::pending::<()>().await;
            unreachable!("blackhole endpoint never resolves")
        }
        let router = axum::Router::new().route(
            "/token",
            post(|_: Form<HashMap<String, String>>| blackhole()),
        );
        spawn_server(router).await
    }

    #[tokio::test]
    async fn exchange_code_aborts_when_overall_budget_exceeded() {
        // The token endpoint never responds. With a tiny exchange
        // budget the overall `tokio::time::timeout` wins long before the
        // per-call reqwest timeout, so the exchange fails with the
        // budget's "timed out" error. Locks in that a regression
        // removing/raising the wrapper would surface as a hang.
        let (addr, _server) = spawn_blackhole_token().await;
        let provider = GitHubProvider::with_endpoints(
            cfg(),
            "https://github.test/login/oauth/authorize",
            format!("http://{addr}/token"),
            format!("http://{addr}/user"),
            format!("http://{addr}/emails"),
        )
        .expect("build provider")
        .with_exchange_budget(Duration::from_millis(150));
        let err = provider
            .exchange_code(
                "fake-code",
                "https://example.com/auth/callback/github",
                "ignored-verifier",
            )
            .await
            .expect_err("overall budget must trip");
        assert!(
            matches!(&err, OAuthError::CodeExchange(msg) if msg.contains("timed out")),
            "expected the overall-budget timeout error, got {err:?}",
        );
    }
}
