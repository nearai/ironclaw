//! Slack workspace-install OAuth helpers.
//!
//! Slack's bot install flow has a slightly different shape from the
//! generic user-profile OAuth flow already handled by
//! `src/channels/web/handlers/auth.rs`:
//!
//!   * The token endpoint is `oauth.v2.access`, not `oauth.access` /
//!     `token`. The response carries a *bot* token plus team metadata,
//!     not a user profile.
//!   * `client_id` + `client_secret` are sent via HTTP Basic auth
//!     (Slack also accepts them as form fields, but Basic is canonical
//!     and lets us hand the secret to reqwest's `basic_auth` so it
//!     never appears in any access log).
//!   * No PKCE — Slack's bot OAuth doesn't support code verifiers.
//!
//! The HTTP redirect handlers in `auth.rs` plumb these helpers; pulling
//! them out keeps the URL builder + form encoder pure and unit-testable
//! without spinning up a mock HTTP server.
//!
//! ## Authorize URL
//!
//! `https://slack.com/oauth/v2/authorize?client_id=…&scope=…&redirect_uri=…&state=…`
//!
//! `scope` is comma-separated (Slack's convention), not space-separated
//! like RFC 6749 specifies. We build it from `MINIMAL_BOT_SCOPES`.
//!
//! ## Token endpoint
//!
//! `POST https://slack.com/api/oauth.v2.access`
//!   body: `code=<auth code>&redirect_uri=<same as authorize>`
//!   auth: `Basic base64(client_id:client_secret)`
//!
//! Slack's API base is parameterised so tests can swap in a mockito server.

use secrecy::{ExposeSecret, SecretString};

use super::manifest::MINIMAL_BOT_SCOPES;
use super::oauth::{OAuthV2AccessResponse, SlackOAuthError, parse_oauth_v2_access};

/// Production Slack API base URL. Tests inject a mockito URL.
pub const SLACK_API_BASE: &str = "https://slack.com";

/// Build the workspace-install authorize URL the operator's browser
/// is redirected to. `state` is an opaque CSRF token IronClaw issued
/// (typically a 32-byte hex-encoded random) and validates on callback.
pub fn authorize_url(client_id: &str, redirect_uri: &str, state: &str) -> String {
    let scope = MINIMAL_BOT_SCOPES.join(",");
    let mut url = url::Url::parse(&format!("{SLACK_API_BASE}/oauth/v2/authorize"))
        .expect("hard-coded URL is well-formed"); // safety: SLACK_API_BASE is a hard-coded HTTPS literal — Url::parse cannot fail on this input.
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("scope", &scope)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("state", state);
    url.to_string()
}

/// Body for the `oauth.v2.access` POST. Returned as a string rather
/// than a structured form so the caller can pass it straight through
/// reqwest without extra allocations or feature dependencies on
/// reqwest's `form` constructor.
pub fn oauth_v2_access_form_body(code: &str, redirect_uri: &str) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    s.append_pair("code", code);
    s.append_pair("redirect_uri", redirect_uri);
    s.finish()
}

/// Errors from the OAuth code → bot token exchange. Distinct from
/// [`SlackOAuthError`] so the redirect handler can map "Slack rejected
/// the install" (operator misconfiguration) separately from "transport
/// error" (network blip — retryable).
#[derive(Debug, thiserror::Error)]
pub enum ExchangeError {
    #[error("HTTP request to Slack failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Slack returned a malformed response: {0}")]
    BadResponse(#[from] SlackOAuthError),
}

/// Exchange the authorization `code` for a bot token. The result is the
/// already-validated [`OAuthV2AccessResponse`] (so the caller doesn't
/// need to re-check `ok==true`).
///
/// `api_base` lets tests inject a mockito server URL; production code
/// passes [`SLACK_API_BASE`].
pub async fn exchange_code(
    http: &reqwest::Client,
    api_base: &str,
    code: &str,
    client_id: &str,
    client_secret: &SecretString,
    redirect_uri: &str,
) -> Result<OAuthV2AccessResponse, ExchangeError> {
    let body = oauth_v2_access_form_body(code, redirect_uri);
    let response = http
        .post(format!("{api_base}/api/oauth.v2.access"))
        .basic_auth(client_id, Some(client_secret.expose_secret()))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    let raw = response.text().await?;
    Ok(parse_oauth_v2_access(&raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_carries_minimal_scopes_comma_separated() {
        let url = authorize_url(
            "12345.67890",
            "https://ironclaw.example.com/auth/slack/install/callback",
            "abc123state",
        );
        // Slack uses comma-separated scopes, not RFC-6749 spaces.
        let expected_scope = MINIMAL_BOT_SCOPES.join(",");
        assert!(
            url.contains(&format!("scope={}", urlencoding::encode(&expected_scope))),
            "scope query missing comma-separated minimal scopes: {url}"
        );
        assert!(url.contains("client_id=12345.67890"));
        assert!(url.contains("state=abc123state"));
        assert!(url.starts_with("https://slack.com/oauth/v2/authorize?"));
    }

    #[test]
    fn authorize_url_percent_encodes_redirect_uri() {
        let url = authorize_url(
            "id",
            "https://ironclaw.example.com/auth/slack/install/callback",
            "s",
        );
        // The colon and slashes in the redirect_uri must be percent-encoded
        // inside the query string. URL-encoding produces %3A and %2F.
        assert!(
            url.contains("redirect_uri=https%3A%2F%2Fironclaw.example.com%2Fauth%2Fslack%2Finstall%2Fcallback"),
            "redirect_uri not percent-encoded in {url}"
        );
    }

    #[test]
    fn oauth_v2_access_form_body_encodes_code_and_redirect() {
        let body = oauth_v2_access_form_body("0wbg9.abc/cd", "https://ironclaw.example.com/cb?x=y");
        // form-urlencoded encodes `/`, `:`, `?`, `=` etc.
        assert!(body.contains("code=0wbg9.abc%2Fcd"), "got {body}");
        assert!(
            body.contains("redirect_uri=https%3A%2F%2Fironclaw.example.com%2Fcb%3Fx%3Dy"),
            "got {body}"
        );
    }
}
