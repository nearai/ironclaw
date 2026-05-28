use ironclaw_auth::{AuthProductError, OAuthTokenResponse, ProviderScope};
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct GoogleTokenResponseBody {
    access_token: SecretString,
    #[serde(default)]
    refresh_token: Option<SecretString>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    token_type: Option<String>,
}

#[derive(Debug)]
pub(super) struct ParsedGoogleTokenResponse {
    pub(super) response: OAuthTokenResponse,
    pub(super) scope_was_present: bool,
}

pub(super) fn parse_token_response(
    body: &[u8],
) -> Result<ParsedGoogleTokenResponse, AuthProductError> {
    let parsed: GoogleTokenResponseBody =
        serde_json::from_slice(body).map_err(|_| AuthProductError::TokenExchangeFailed)?;
    let response_scope = parsed
        .scope
        .as_deref()
        .filter(|scope| !scope.trim().is_empty());
    let scope_was_present = response_scope.is_some();
    let response = OAuthTokenResponse::new(
        parsed.access_token,
        parsed.refresh_token,
        response_scope,
        parsed.expires_in,
    )
    .map_err(|_| AuthProductError::TokenExchangeFailed)?;

    let _ = parsed.token_type;
    Ok(ParsedGoogleTokenResponse {
        response,
        scope_was_present,
    })
}

pub(super) fn scopes_for_exchange(
    token_response: &ParsedGoogleTokenResponse,
) -> Result<Vec<ProviderScope>, AuthProductError> {
    if token_response.scope_was_present {
        Ok(token_response.response.scopes.clone())
    } else {
        Err(AuthProductError::TokenExchangeFailed)
    }
}

pub(super) fn scopes_for_refresh(
    token_response: &ParsedGoogleTokenResponse,
    existing_scopes: &[ProviderScope],
) -> Vec<ProviderScope> {
    if token_response.scope_was_present {
        token_response.response.scopes.clone()
    } else {
        existing_scopes.to_vec()
    }
}
