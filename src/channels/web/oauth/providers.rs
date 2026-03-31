//! OAuth provider trait and implementations (Google, GitHub).

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::{OAuthError, OAuthUserProfile};

/// Trait for OAuth providers.
///
/// Each provider knows how to build an authorization URL and exchange an
/// authorization code for a user profile.
#[async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Provider name (e.g. `google`, `github`).
    fn name(&self) -> &str;

    /// Build the authorization URL for redirecting the user.
    fn authorization_url(&self, callback_url: &str, state: &str, code_challenge: &str) -> String;

    /// Exchange an authorization code for a user profile.
    async fn exchange_code(
        &self,
        code: &str,
        callback_url: &str,
        code_verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError>;
}

// ── Google (OIDC) ────────────────────────────────────────────────────────

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

pub struct GoogleProvider {
    client_id: String,
    client_secret: SecretString,
    /// Optional hosted domain restriction (Google Workspace).
    allowed_hd: Option<String>,
    http: reqwest::Client,
}

impl GoogleProvider {
    pub fn new(client_id: String, client_secret: SecretString, allowed_hd: Option<String>) -> Self {
        Self {
            client_id,
            client_secret,
            allowed_hd,
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct GoogleTokenResponse {
    id_token: Option<String>,
    #[allow(dead_code)]
    access_token: String,
}

#[derive(Deserialize, serde::Serialize)]
struct GoogleIdTokenClaims {
    sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
    picture: Option<String>,
    /// Google Workspace hosted domain (e.g. `company.com`).
    hd: Option<String>,
}

#[async_trait]
impl OAuthProvider for GoogleProvider {
    fn name(&self) -> &str {
        "google"
    }

    fn authorization_url(&self, callback_url: &str, state: &str, code_challenge: &str) -> String {
        let mut url = format!(
            "{GOOGLE_AUTH_URL}?\
             response_type=code\
             &client_id={client_id}\
             &redirect_uri={redirect_uri}\
             &scope={scope}\
             &state={state}\
             &code_challenge={code_challenge}\
             &code_challenge_method=S256\
             &access_type=online",
            client_id = urlencoding::encode(&self.client_id),
            redirect_uri = urlencoding::encode(callback_url),
            scope = urlencoding::encode("openid email profile"),
            state = urlencoding::encode(state),
            code_challenge = urlencoding::encode(code_challenge),
        );
        // Hint Google to show only accounts from this hosted domain.
        if let Some(ref hd) = self.allowed_hd {
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
        // Exchange the authorization code for tokens.
        let resp = self
            .http
            .post(GOOGLE_TOKEN_URL)
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
            .map_err(|e| OAuthError::CodeExchange(e.to_string()))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::CodeExchange(format!(
                "Google token endpoint returned error: {body}"
            )));
        }

        let token_resp: GoogleTokenResponse = resp
            .json()
            .await
            .map_err(|e| OAuthError::CodeExchange(e.to_string()))?;

        // Decode the id_token JWT to extract user profile claims.
        // We received this directly from Google over TLS, so we skip
        // signature verification (the token is authentic by transport).
        let id_token = token_resp.id_token.ok_or_else(|| {
            OAuthError::CodeExchange("Google did not return an id_token".to_string())
        })?;

        // Validate the id_token JWT claims. We skip signature verification
        // because the token was received directly from Google over TLS.
        // However, we MUST validate `aud` to prevent token substitution from
        // a different OAuth client.
        let mut validation = jsonwebtoken::Validation::default();
        validation.insecure_disable_signature_validation();
        validation.set_audience(&[&self.client_id]);

        let token_data = jsonwebtoken::decode::<GoogleIdTokenClaims>(
            &id_token,
            &jsonwebtoken::DecodingKey::from_secret(&[]),
            &validation,
        )
        .map_err(|e| OAuthError::ProfileFetch(format!("Failed to decode id_token: {e}")))?;

        let claims = token_data.claims;

        // Server-side hosted domain validation — the `hd` URL parameter is
        // only a UI hint; a user could bypass it by editing the URL.
        if let Some(ref required_hd) = self.allowed_hd {
            match claims.hd.as_deref() {
                Some(hd) if hd.eq_ignore_ascii_case(required_hd) => {}
                _ => {
                    return Err(OAuthError::ProfileFetch(format!(
                        "Account is not from the required domain '{required_hd}'"
                    )));
                }
            }
        }

        Ok(OAuthUserProfile {
            provider_user_id: claims.sub.clone(),
            email: claims.email.clone(),
            email_verified: claims.email_verified.unwrap_or(false),
            display_name: claims.name.clone(),
            avatar_url: claims.picture.clone(),
            raw: serde_json::to_value(&claims).unwrap_or_default(),
        })
    }
}

// ── GitHub ────────────────────────────────────────────────────────────────

const GITHUB_AUTH_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_URL: &str = "https://api.github.com/user";
const GITHUB_EMAILS_URL: &str = "https://api.github.com/user/emails";

pub struct GitHubProvider {
    client_id: String,
    client_secret: SecretString,
    http: reqwest::Client,
}

impl GitHubProvider {
    pub fn new(client_id: String, client_secret: SecretString) -> Self {
        Self {
            client_id,
            client_secret,
            http: reqwest::Client::builder()
                .user_agent("IronClaw")
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

#[derive(Deserialize)]
struct GitHubTokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct GitHubUser {
    id: u64,
    login: String,
    name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct GitHubEmail {
    email: String,
    verified: bool,
    primary: bool,
}

#[async_trait]
impl OAuthProvider for GitHubProvider {
    fn name(&self) -> &str {
        "github"
    }

    fn authorization_url(&self, callback_url: &str, state: &str, _code_challenge: &str) -> String {
        // GitHub does not support PKCE; CSRF is protected via the state param.
        format!(
            "{GITHUB_AUTH_URL}?\
             client_id={client_id}\
             &redirect_uri={redirect_uri}\
             &scope={scope}\
             &state={state}",
            client_id = urlencoding::encode(&self.client_id),
            redirect_uri = urlencoding::encode(callback_url),
            scope = urlencoding::encode("read:user user:email"),
            state = urlencoding::encode(state),
        )
    }

    async fn exchange_code(
        &self,
        code: &str,
        _callback_url: &str,
        _code_verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError> {
        // Exchange the code for an access token.
        let resp = self
            .http
            .post(GITHUB_TOKEN_URL)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.expose_secret()),
                ("code", code),
            ])
            .send()
            .await
            .map_err(|e| OAuthError::CodeExchange(e.to_string()))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::CodeExchange(format!(
                "GitHub token endpoint error: {body}"
            )));
        }

        let token_resp: GitHubTokenResponse = resp
            .json()
            .await
            .map_err(|e| OAuthError::CodeExchange(e.to_string()))?;

        // Fetch user profile.
        let user: GitHubUser = self
            .http
            .get(GITHUB_USER_URL)
            .header(
                "Authorization",
                format!("Bearer {}", token_resp.access_token),
            )
            .send()
            .await
            .map_err(|e| OAuthError::ProfileFetch(e.to_string()))?
            .json()
            .await
            .map_err(|e| OAuthError::ProfileFetch(e.to_string()))?;

        // Fetch verified emails (the profile may not include one).
        let emails: Vec<GitHubEmail> = self
            .http
            .get(GITHUB_EMAILS_URL)
            .header(
                "Authorization",
                format!("Bearer {}", token_resp.access_token),
            )
            .send()
            .await
            .map_err(|e| OAuthError::ProfileFetch(e.to_string()))?
            .json()
            .await
            .map_err(|e| OAuthError::ProfileFetch(format!("Failed to parse GitHub emails: {e}")))?;

        // Pick the primary verified email, or any verified email.
        let verified_email = emails
            .iter()
            .filter(|e| e.verified)
            .find(|e| e.primary)
            .or_else(|| emails.iter().find(|e| e.verified));

        let (email, email_verified) = match verified_email {
            Some(e) => (Some(e.email.clone()), true),
            None => (user.email.clone(), false),
        };

        let raw = serde_json::json!({
            "id": user.id,
            "login": user.login,
            "name": user.name,
            "avatar_url": user.avatar_url,
        });

        Ok(OAuthUserProfile {
            provider_user_id: user.id.to_string(),
            email,
            email_verified,
            display_name: user.name.or(Some(user.login)),
            avatar_url: user.avatar_url,
            raw,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google_authorization_url_format() {
        let provider = GoogleProvider::new(
            "test-client-id".to_string(),
            SecretString::from("test-secret".to_string()),
            None,
        );

        let url = provider.authorization_url(
            "https://example.com/auth/callback/google",
            "csrf-state-123",
            "challenge-abc",
        );

        assert!(url.starts_with(GOOGLE_AUTH_URL));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("state=csrf-state-123"));
        assert!(url.contains("code_challenge=challenge-abc"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("scope=openid"));
        assert!(!url.contains("&hd="));
    }

    #[test]
    fn test_google_authorization_url_includes_hd() {
        let provider = GoogleProvider::new(
            "test-client-id".to_string(),
            SecretString::from("test-secret".to_string()),
            Some("company.com".to_string()),
        );

        let url = provider.authorization_url(
            "https://example.com/auth/callback/google",
            "csrf-state-123",
            "challenge-abc",
        );

        assert!(url.contains("&hd=company.com"));
    }

    #[test]
    fn test_github_authorization_url_format() {
        let provider = GitHubProvider::new(
            "gh-client-id".to_string(),
            SecretString::from("gh-secret".to_string()),
        );

        let url = provider.authorization_url(
            "https://example.com/auth/callback/github",
            "csrf-state-456",
            "ignored-challenge",
        );

        assert!(url.starts_with(GITHUB_AUTH_URL));
        assert!(url.contains("client_id=gh-client-id"));
        assert!(url.contains("state=csrf-state-456"));
        assert!(url.contains("scope=read%3Auser"));
        // GitHub ignores code_challenge, verify it's not included
        assert!(!url.contains("code_challenge="));
    }
}
