use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_oauth::{OAuthError, OAuthProvider, TokenSet};
use secrecy::SecretString;
use url::Url;

use crate::EnvConfig;

pub const BAKED_IN_GOOGLE_DESKTOP_CLIENT_ID: &str =
    "564604149681-efo25d43rs85v0tibdepsmdv5dsrhhr0.apps.googleusercontent.com";

#[derive(Debug, Clone)]
pub struct GoogleProvider {
    public_client_id: String,
    direct_client_secret: Option<SecretString>,
    allowed_hd: Option<String>,
}

impl GoogleProvider {
    pub fn from_env(broker_active: bool) -> Result<Option<Arc<Self>>, OAuthError> {
        let env = EnvConfig {
            oauth_broker_active: broker_active,
            google_client_id: std::env::var("GOOGLE_CLIENT_ID")
                .ok()
                .filter(|value| !value.is_empty()),
            google_client_secret: std::env::var("GOOGLE_CLIENT_SECRET")
                .ok()
                .filter(|value| !value.is_empty()),
            google_allowed_hd: std::env::var("GOOGLE_ALLOWED_HD")
                .ok()
                .filter(|value| !value.is_empty()),
        };
        Self::from_config(&env)
    }

    pub fn from_config(env: &EnvConfig) -> Result<Option<Arc<Self>>, OAuthError> {
        let public_client_id = env.google_client_id.clone().or_else(|| {
            env.oauth_broker_active
                .then(|| BAKED_IN_GOOGLE_DESKTOP_CLIENT_ID.to_string())
        });
        let Some(public_client_id) = public_client_id else {
            return Ok(None);
        };
        let direct_client_secret = if env.oauth_broker_active {
            None
        } else {
            Some(SecretString::from(
                env.google_client_secret
                    .clone()
                    .ok_or_else(|| OAuthError::IncompleteConfig {
                        provider: "google".to_string(),
                        reason: "direct mode requires GOOGLE_CLIENT_SECRET".to_string(),
                    })?,
            ))
        };
        Ok(Some(Arc::new(Self {
            public_client_id,
            direct_client_secret,
            allowed_hd: env.google_allowed_hd.clone(),
        })))
    }
}

#[async_trait]
impl OAuthProvider for GoogleProvider {
    fn provider_id(&self) -> &str {
        "google"
    }

    fn auth_url(&self) -> &str {
        "https://accounts.google.com/o/oauth2/v2/auth"
    }

    fn token_url(&self) -> &str {
        "https://oauth2.googleapis.com/token"
    }

    fn credential_name(&self) -> &str {
        "google_oauth_token"
    }

    fn public_client_id(&self) -> &str {
        &self.public_client_id
    }

    fn direct_client_secret(&self) -> Option<&SecretString> {
        self.direct_client_secret.as_ref()
    }

    fn build_authorize_url(
        &self,
        state: &str,
        code_challenge: &str,
        scopes: &[String],
        redirect_uri: &str,
    ) -> String {
        let mut url = Url::parse(self.auth_url()).expect("Google auth URL is a static valid URL");
        url.query_pairs_mut()
            .append_pair("client_id", &self.public_client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &scopes.join(" "))
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("include_granted_scopes", "true");
        if let Some(hd) = &self.allowed_hd {
            url.query_pairs_mut().append_pair("hd", hd);
        }
        url.to_string()
    }

    fn parse_token_response(&self, body: &serde_json::Value) -> Result<TokenSet, OAuthError> {
        let access_token = body
            .get("access_token")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| OAuthError::InvalidTokenResponse {
                reason: "access_token missing".to_string(),
            })?;
        let refresh_token = body
            .get("refresh_token")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        let expires_in = body.get("expires_in").and_then(serde_json::Value::as_u64);
        let scopes = body
            .get("scope")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .split_whitespace()
            .map(ToString::to_string)
            .collect();
        Ok(TokenSet::from_expires_in(
            access_token.to_string(),
            refresh_token,
            expires_in,
            scopes,
        ))
    }

    fn detect_scope_mismatch(&self, stored: &[String], required: &[String]) -> Vec<String> {
        required
            .iter()
            .filter(|scope| !stored.contains(scope))
            .cloned()
            .collect()
    }
}
