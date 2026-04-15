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
        validation.set_issuer(&["https://accounts.google.com"]);

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

        tracing::debug!(
            sub = %claims.sub,
            picture = ?claims.picture,
            name = ?claims.name,
            "Google id_token claims decoded"
        );

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
                .user_agent(&crate::config::platform_display_name())
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
        callback_url: &str,
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
                ("redirect_uri", callback_url),
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

// ── Apple Sign In ─────────────────────────────────────────────────────────

const APPLE_AUTH_URL: &str = "https://appleid.apple.com/auth/authorize";
const APPLE_TOKEN_URL: &str = "https://appleid.apple.com/auth/token";

/// Apple Sign In provider.
///
/// Apple uses OIDC but requires a JWT `client_secret` (ES256-signed, 6-month
/// lifetime) and sends the callback as a POST with `response_mode=form_post`.
pub struct AppleProvider {
    client_id: String,
    team_id: String,
    key_id: String,
    private_key_pem: SecretString,
    http: reqwest::Client,
}

impl AppleProvider {
    pub fn new(
        client_id: String,
        team_id: String,
        key_id: String,
        private_key_pem: SecretString,
    ) -> Self {
        Self {
            client_id,
            team_id,
            key_id,
            private_key_pem,
            http: reqwest::Client::new(),
        }
    }

    /// Generate a JWT client_secret for Apple's token endpoint.
    ///
    /// Apple requires the client_secret to be an ES256-signed JWT with:
    /// - `iss`: Team ID
    /// - `sub`: Client (Services) ID
    /// - `aud`: `https://appleid.apple.com`
    /// - `iat`: current time
    /// - `exp`: up to 6 months from now
    fn generate_client_secret(&self) -> Result<String, OAuthError> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header};

        let now = chrono::Utc::now().timestamp() as u64;
        let claims = serde_json::json!({
            "iss": self.team_id,
            "sub": self.client_id,
            "aud": "https://appleid.apple.com",
            "iat": now,
            "exp": now + 86400 * 180, // 6 months
        });

        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());

        let key = EncodingKey::from_ec_pem(self.private_key_pem.expose_secret().as_bytes())
            .map_err(|e| OAuthError::CodeExchange(format!("Invalid Apple private key: {e}")))?;

        jsonwebtoken::encode(&header, &claims, &key).map_err(|e| {
            OAuthError::CodeExchange(format!("Failed to sign Apple client_secret: {e}"))
        })
    }
}

#[derive(Deserialize)]
struct AppleTokenResponse {
    id_token: String,
}

#[derive(Deserialize, serde::Serialize)]
struct AppleIdTokenClaims {
    sub: String,
    email: Option<String>,
    email_verified: Option<serde_json::Value>,
}

#[async_trait]
impl OAuthProvider for AppleProvider {
    fn name(&self) -> &str {
        "apple"
    }

    fn authorization_url(&self, callback_url: &str, state: &str, _code_challenge: &str) -> String {
        // Apple requires response_mode=form_post (callback is a POST, not GET).
        format!(
            "{APPLE_AUTH_URL}?\
             response_type=code\
             &response_mode=form_post\
             &client_id={client_id}\
             &redirect_uri={redirect_uri}\
             &scope={scope}\
             &state={state}",
            client_id = urlencoding::encode(&self.client_id),
            redirect_uri = urlencoding::encode(callback_url),
            scope = urlencoding::encode("name email"),
            state = urlencoding::encode(state),
        )
    }

    async fn exchange_code(
        &self,
        code: &str,
        callback_url: &str,
        _code_verifier: &str,
    ) -> Result<OAuthUserProfile, OAuthError> {
        let client_secret = self.generate_client_secret()?;

        let resp = self
            .http
            .post(APPLE_TOKEN_URL)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", callback_url),
                ("client_id", &self.client_id),
                ("client_secret", &client_secret),
            ])
            .send()
            .await
            .map_err(|e| OAuthError::CodeExchange(e.to_string()))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(OAuthError::CodeExchange(format!(
                "Apple token endpoint error: {body}"
            )));
        }

        let token_resp: AppleTokenResponse = resp
            .json()
            .await
            .map_err(|e| OAuthError::CodeExchange(e.to_string()))?;

        // Decode the id_token — we skip signature verification because
        // the token was received directly from Apple over TLS. We validate
        // `aud` to prevent token substitution.
        let mut validation = jsonwebtoken::Validation::default();
        validation.insecure_disable_signature_validation();
        validation.set_audience(&[&self.client_id]);
        validation.set_issuer(&["https://appleid.apple.com"]);

        let token_data = jsonwebtoken::decode::<AppleIdTokenClaims>(
            &token_resp.id_token,
            &jsonwebtoken::DecodingKey::from_secret(&[]),
            &validation,
        )
        .map_err(|e| OAuthError::ProfileFetch(format!("Failed to decode Apple id_token: {e}")))?;

        let claims = token_data.claims;

        // Apple's email_verified can be a string "true"/"false" or a boolean.
        let email_verified = match &claims.email_verified {
            Some(serde_json::Value::Bool(b)) => *b,
            Some(serde_json::Value::String(s)) => s == "true",
            _ => false,
        };

        Ok(OAuthUserProfile {
            provider_user_id: claims.sub.clone(),
            email: claims.email.clone(),
            email_verified,
            display_name: None, // Apple sends name only on first auth via POST user field
            avatar_url: None,
            raw: serde_json::to_value(&claims).unwrap_or_default(),
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

    #[test]
    fn test_apple_authorization_url_format() {
        let provider = AppleProvider::new(
            "com.example.myapp".to_string(),
            "TEAM123456".to_string(),
            "KEY1234567".to_string(),
            SecretString::from("fake-key".to_string()),
        );

        let url = provider.authorization_url(
            "https://example.com/auth/callback/apple",
            "csrf-state-789",
            "ignored-challenge",
        );

        assert!(url.starts_with(APPLE_AUTH_URL));
        assert!(url.contains("client_id=com.example.myapp"));
        assert!(url.contains("response_mode=form_post"));
        assert!(url.contains("state=csrf-state-789"));
        assert!(url.contains("scope=name"));
        assert!(!url.contains("code_challenge="));
    }
}
