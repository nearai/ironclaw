//! Slack slash-command body parser + ack helpers.
//!
//! Slack POSTs slash commands as `application/x-www-form-urlencoded` to the
//! URL declared in the app manifest (`/api/channels/slack/slash` for
//! IronClaw). The receiver MUST acknowledge within 3 seconds; the actual
//! agent reply is delivered out-of-band by POSTing to `response_url` (a
//! per-invocation URL Slack supplies in the body, valid for 30 minutes /
//! 5 deliveries).
//!
//! This module owns the parser and the ack-payload builders; the HTTP
//! handler in `src/channels/web/handlers/slack.rs` plumbs them together.
//! Splitting parser from handler keeps the parser pure (no axum, no DB)
//! and unit-testable against fixed fixtures.
//!
//! ## Why URL-decode here, not at the axum layer
//!
//! `axum::Form<T>` deserializes the body and discards the raw bytes — but
//! we need the raw bytes to verify the HMAC signature (Slack signs the
//! exact bytes that hit the wire, before any decoding). So the handler
//! reads `axum::body::Bytes`, hands them to `slack::sig::verify`, and
//! only then hands them to this parser.

/// Parsed slash-command request body. Subset of fields IronClaw actually
/// uses; Slack's full payload has ~15 fields, most of which are noise for
/// the agent (e.g. `team_domain`, `is_enterprise_install`).
#[derive(Debug, Clone)]
pub struct SlashCommandRequest {
    /// Slack workspace id (`T…`). The receiver looks this up in
    /// `channel_identities` to find the bound IronClaw user.
    pub team_id: String,
    /// Slack user id (`U…`) of whoever invoked the command. Used by the
    /// per-user pairing flow to ask "who is this?" on first contact.
    pub user_id: String,
    /// Channel where the command was invoked (`C…` / `D…` / `G…`).
    pub channel_id: String,
    /// Command name including the leading slash, e.g. `/ironclaw`.
    pub command: String,
    /// User-supplied prompt, possibly empty.
    pub text: String,
    /// One-shot per-invocation reply URL, valid for 30 min / 5 deliveries.
    pub response_url: String,
    /// Trigger id, valid for 3 seconds, required to open modals later.
    pub trigger_id: String,
    /// Optional Enterprise Grid id (`E…`); empty for non-grid workspaces.
    pub enterprise_id: String,
}

/// Parser failure modes. Distinct so the handler can map to the right
/// HTTP status (`400` for malformed body, `422` for missing required
/// fields).
#[derive(Debug, thiserror::Error)]
pub enum SlashParseError {
    #[error("slash command body is not valid UTF-8: {0}")]
    NotUtf8(#[from] std::str::Utf8Error),
    #[error("slash command body is missing required field: {0}")]
    MissingField(&'static str),
}

/// Parse a slash-command body. Slack guarantees fields like `team_id`,
/// `user_id`, `command`, `response_url`, and `trigger_id` are always
/// present — but we treat them as required at the parser layer rather
/// than panicking later.
///
/// Uses `url::form_urlencoded::parse` rather than serde so we don't pull
/// in a new direct dep just for a flat string-to-string decode.
pub fn parse_slash_command(body: &[u8]) -> Result<SlashCommandRequest, SlashParseError> {
    // form_urlencoded::parse takes &[u8] but happily decodes invalid UTF-8
    // into U+FFFD; we want a hard failure so we revalidate up front.
    let _ = std::str::from_utf8(body)?;

    let mut team_id = String::new();
    let mut user_id = String::new();
    let mut channel_id = String::new();
    let mut command = String::new();
    let mut text = String::new();
    let mut response_url = String::new();
    let mut trigger_id = String::new();
    let mut enterprise_id = String::new();

    for (key, value) in url::form_urlencoded::parse(body) {
        match key.as_ref() {
            "team_id" => team_id = value.into_owned(),
            "user_id" => user_id = value.into_owned(),
            "channel_id" => channel_id = value.into_owned(),
            "command" => command = value.into_owned(),
            "text" => text = value.into_owned(),
            "response_url" => response_url = value.into_owned(),
            "trigger_id" => trigger_id = value.into_owned(),
            "enterprise_id" => enterprise_id = value.into_owned(),
            // Drop the rest (token, team_domain, channel_name, user_name,
            // api_app_id, is_enterprise_install) — IronClaw doesn't use them
            // and ignoring unknown keys keeps us forward-compatible with
            // future Slack additions.
            _ => {}
        }
    }

    if team_id.is_empty() {
        return Err(SlashParseError::MissingField("team_id"));
    }
    if user_id.is_empty() {
        return Err(SlashParseError::MissingField("user_id"));
    }
    if command.is_empty() {
        return Err(SlashParseError::MissingField("command"));
    }
    if response_url.is_empty() {
        return Err(SlashParseError::MissingField("response_url"));
    }

    Ok(SlashCommandRequest {
        team_id,
        user_id,
        channel_id,
        command,
        text,
        response_url,
        trigger_id,
        enterprise_id,
    })
}

/// Effective workspace id: prefer `enterprise_id` over `team_id` when
/// present (matches the Enterprise Grid handling in
/// [`crate::channels::slack::oauth::workspace_external_id`]).
pub fn effective_workspace_id(req: &SlashCommandRequest) -> &str {
    if !req.enterprise_id.is_empty() {
        &req.enterprise_id
    } else {
        &req.team_id
    }
}

/// Build the immediate ack payload Slack expects within 3 seconds.
///
/// Slack supports two `response_type` values:
///   * `ephemeral` (default) — only the invoker sees the message.
///     Right for "I'm thinking…" messages so the channel doesn't get
///     spammed by every slash invocation.
///   * `in_channel` — visible to everyone in the channel.
///
/// We use `ephemeral` for the placeholder; the eventual agent reply
/// (delivered via `response_url`) can promote to `in_channel` based on
/// the user's prompt.
pub fn ack_payload(text: &str) -> serde_json::Value {
    serde_json::json!({
        "response_type": "ephemeral",
        "text": text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-shape fixture — the field set Slack documents for slash
    /// commands as of 2026. Order is alphabetised because that's what
    /// Slack's encoder happens to emit.
    const FIXTURE: &[u8] = b"\
api_app_id=A0KRD7HC3&\
channel_id=C2147483705&\
channel_name=test&\
command=%2Fironclaw&\
enterprise_id=&\
is_enterprise_install=false&\
response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2F1234%2F5678%2Fxxxx&\
team_domain=example&\
team_id=T0001&\
text=hello+world&\
token=gIkuvaNzQIHg97ATvDxqgjtO&\
trigger_id=13345224609.738474920.8088930838d88f008e0&\
user_id=U2147483697&\
user_name=alice";

    #[test]
    fn parses_slash_command_fixture() {
        let req = parse_slash_command(FIXTURE).expect("fixture parses");
        assert_eq!(req.team_id, "T0001");
        assert_eq!(req.user_id, "U2147483697");
        assert_eq!(req.channel_id, "C2147483705");
        assert_eq!(req.command, "/ironclaw");
        assert_eq!(req.text, "hello world");
        assert!(req.response_url.starts_with("https://hooks.slack.com/"));
        assert!(req.trigger_id.starts_with("13345224609"));
        // Non-Enterprise install — enterprise_id is empty, fallback to team_id.
        assert!(req.enterprise_id.is_empty());
        assert_eq!(effective_workspace_id(&req), "T0001");
    }

    #[test]
    fn enterprise_grid_uses_enterprise_id_for_workspace_lookup() {
        // Same fixture but with enterprise_id populated.
        let body = b"\
team_id=T0001&\
user_id=U1&\
channel_id=C1&\
command=%2Fironclaw&\
text=&\
response_url=https%3A%2F%2Fexample.com&\
trigger_id=t&\
enterprise_id=E12345";
        let req = parse_slash_command(body).expect("parses");
        assert_eq!(effective_workspace_id(&req), "E12345");
    }

    #[test]
    fn rejects_body_missing_team_id() {
        // Drop `team_id`; serde_urlencoded fills it with default ""
        let body = b"\
user_id=U1&\
channel_id=C1&\
command=%2Fironclaw&\
text=&\
response_url=https%3A%2F%2Fexample.com&\
trigger_id=t";
        let err = parse_slash_command(body).unwrap_err();
        assert!(matches!(err, SlashParseError::MissingField("team_id")));
    }

    #[test]
    fn rejects_body_missing_response_url() {
        let body = b"\
team_id=T1&\
user_id=U1&\
channel_id=C1&\
command=%2Fironclaw&\
text=&\
trigger_id=t";
        let err = parse_slash_command(body).unwrap_err();
        assert!(matches!(err, SlashParseError::MissingField("response_url")));
    }

    #[test]
    fn ack_payload_emits_ephemeral_default() {
        let p = ack_payload("on it");
        assert_eq!(p["response_type"], "ephemeral");
        assert_eq!(p["text"], "on it");
    }
}
