//! Parser for Slack's `oauth.v2.access` response.
//!
//! The first commit on this branch only lands the parser — the redirect
//! handler that calls it lives in `src/channels/web/handlers/auth.rs` and
//! is wired in a follow-up commit. Splitting the parser out keeps it pure
//! (easy to unit-test against fixed fixtures) and lets the handler reuse
//! it without bringing in `axum` for tests.
//!
//! Response shape (Slack docs, June 2024):
//! ```json
//! {
//!   "ok": true,
//!   "access_token": "xoxb-...",
//!   "token_type": "bot",
//!   "scope": "chat:write,app_mentions:read,...",
//!   "bot_user_id": "U0KRQLJ9H",
//!   "app_id": "A0KRD7HC3",
//!   "team": { "id": "T9TK3CUKW", "name": "Slack Pickleball Team" },
//!   "enterprise": { "id": "E12345", "name": "Acme Inc" }
//! }
//! ```
//! `enterprise` is null for non-Enterprise-Grid workspaces.

use serde::{Deserialize, Serialize};

/// Successful `oauth.v2.access` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthV2AccessResponse {
    pub ok: bool,
    /// Bot User OAuth Token (`xoxb-…`). Empty string allowed only when `ok=false`.
    #[serde(default)]
    pub access_token: String,
    /// Always `"bot"` for the bot-token install path.
    #[serde(default)]
    pub token_type: String,
    /// Comma-separated scopes the workspace approved. May be a subset of
    /// what the manifest requested if the user de-selected optional scopes.
    #[serde(default)]
    pub scope: String,
    /// Slack user ID of the bot user installed in this workspace.
    #[serde(default)]
    pub bot_user_id: String,
    /// Slack-side app id (`A…`).
    #[serde(default)]
    pub app_id: String,
    pub team: Option<SlackTeam>,
    #[serde(default)]
    pub enterprise: Option<SlackEnterprise>,
    /// Present on `ok=false` responses.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackTeam {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackEnterprise {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// Parser error variants. Distinguished so the install command can give
/// the operator a precise message ("Slack rejected the install: …") rather
/// than a generic JSON parse failure.
#[derive(Debug, thiserror::Error)]
pub enum SlackOAuthError {
    #[error("malformed oauth.v2.access response: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("Slack rejected the install: {0}")]
    NotOk(String),
    #[error("Slack returned ok=true but no team object — required for workspace install")]
    MissingTeam,
    #[error("Slack returned ok=true but no access_token — required to call chat.postMessage")]
    MissingAccessToken,
}

/// Parse a raw `oauth.v2.access` response body and validate that it
/// carries the fields IronClaw needs to register a workspace install.
///
/// Returns the parsed response on success; on failure returns a typed
/// error the install command surfaces to the operator.
pub fn parse_oauth_v2_access(body: &str) -> Result<OAuthV2AccessResponse, SlackOAuthError> {
    let parsed: OAuthV2AccessResponse = serde_json::from_str(body)?;
    if !parsed.ok {
        return Err(SlackOAuthError::NotOk(
            parsed
                .error
                .clone()
                .unwrap_or_else(|| "unknown_error".into()),
        ));
    }
    if parsed.team.is_none() {
        return Err(SlackOAuthError::MissingTeam);
    }
    if parsed.access_token.is_empty() {
        return Err(SlackOAuthError::MissingAccessToken);
    }
    Ok(parsed)
}

/// Extract the workspace identifier the install command persists as
/// `channel_identities.external_id`. For Enterprise Grid installs we
/// prefer the enterprise id (cross-workspace install); otherwise the
/// team id.
pub fn workspace_external_id(resp: &OAuthV2AccessResponse) -> Option<&str> {
    if let Some(ref ent) = resp.enterprise
        && !ent.id.is_empty()
    {
        return Some(ent.id.as_str());
    }
    resp.team.as_ref().map(|t| t.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_OK: &str = r#"{
        "ok": true,
        "access_token": "xoxb-FAKE-FIXTURE-NOT-A-REAL-TOKEN",
        "token_type": "bot",
        "scope": "chat:write,app_mentions:read,im:history,im:write,commands",
        "bot_user_id": "U0KRQLJ9H",
        "app_id": "A0KRD7HC3",
        "team": { "id": "T9TK3CUKW", "name": "Pickleball" }
    }"#;

    const FIXTURE_ENTERPRISE: &str = r#"{
        "ok": true,
        "access_token": "xoxb-FAKE-FIXTURE-2",
        "token_type": "bot",
        "scope": "chat:write",
        "bot_user_id": "U1",
        "app_id": "A1",
        "team": { "id": "T9TK3CUKW", "name": "Pickleball" },
        "enterprise": { "id": "E12345", "name": "Acme" }
    }"#;

    const FIXTURE_NOT_OK: &str = r#"{ "ok": false, "error": "invalid_code" }"#;

    #[test]
    fn parses_bot_token_shape() {
        let resp = parse_oauth_v2_access(FIXTURE_OK).expect("parses");
        assert!(resp.ok);
        assert_eq!(resp.access_token, "xoxb-FAKE-FIXTURE-NOT-A-REAL-TOKEN");
        assert_eq!(resp.token_type, "bot");
        assert_eq!(resp.bot_user_id, "U0KRQLJ9H");
        assert_eq!(resp.team.as_ref().unwrap().id, "T9TK3CUKW");
        assert_eq!(workspace_external_id(&resp), Some("T9TK3CUKW"));
    }

    #[test]
    fn enterprise_grid_prefers_enterprise_id() {
        let resp = parse_oauth_v2_access(FIXTURE_ENTERPRISE).expect("parses");
        assert_eq!(workspace_external_id(&resp), Some("E12345"));
    }

    #[test]
    fn surfaces_slack_error_message() {
        let err = parse_oauth_v2_access(FIXTURE_NOT_OK).unwrap_err();
        match err {
            SlackOAuthError::NotOk(msg) => assert_eq!(msg, "invalid_code"),
            other => panic!("expected NotOk, got {other:?}"),
        }
    }

    #[test]
    fn rejects_response_with_no_team() {
        let body = r#"{ "ok": true, "access_token": "xoxb-FAKE-X", "token_type": "bot" }"#;
        let err = parse_oauth_v2_access(body).unwrap_err();
        assert!(matches!(err, SlackOAuthError::MissingTeam));
    }

    #[test]
    fn rejects_response_with_empty_token() {
        let body = r#"{ "ok": true, "access_token": "", "team": { "id": "T1" } }"#;
        let err = parse_oauth_v2_access(body).unwrap_err();
        assert!(matches!(err, SlackOAuthError::MissingAccessToken));
    }
}
