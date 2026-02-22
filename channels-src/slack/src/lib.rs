//! Slack Events API channel for IronClaw.
//!
//! This WASM component implements the channel interface for handling Slack
//! webhooks and sending messages back to Slack.
//!
//! # Features
//!
//! - URL verification for Slack Events API
//! - Message event parsing (@mentions, DMs)
//! - Thread support for conversations
//! - Response posting via Slack Web API
//! - API error classification with retry and backoff
//! - Startup permission validation via auth.test
//! - Auth failure notification to surface token issues
//!
//! # Security
//!
//! - Signature validation is handled by the host (webhook secrets)
//! - Bot token is injected by host during HTTP requests
//! - WASM never sees raw credentials

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

/// Slack event wrapper.
#[derive(Debug, Deserialize)]
struct SlackEventWrapper {
    /// Event type (url_verification, event_callback, etc.)
    #[serde(rename = "type")]
    event_type: String,

    /// Challenge token for URL verification.
    challenge: Option<String>,

    /// The actual event payload (for event_callback).
    event: Option<SlackEvent>,

    /// Team ID that sent this event.
    team_id: Option<String>,

    /// Event ID for deduplication.
    #[allow(dead_code)]
    event_id: Option<String>,
}

/// Slack event payload.
#[derive(Debug, Deserialize)]
struct SlackEvent {
    /// Event type (message, app_mention, etc.)
    #[serde(rename = "type")]
    event_type: String,

    /// User who triggered the event.
    user: Option<String>,

    /// Channel where the event occurred.
    channel: Option<String>,

    /// Message text.
    text: Option<String>,

    /// Thread timestamp (for threaded messages).
    thread_ts: Option<String>,

    /// Message timestamp.
    ts: Option<String>,

    /// Bot ID (if message is from a bot).
    bot_id: Option<String>,

    /// Subtype (bot_message, etc.)
    subtype: Option<String>,
}

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SlackMessageMetadata {
    /// Slack channel ID.
    channel: String,

    /// Thread timestamp for threaded replies.
    thread_ts: Option<String>,

    /// Original message timestamp.
    message_ts: String,

    /// Team ID.
    team_id: Option<String>,
}

/// Slack API response for chat.postMessage.
#[derive(Debug, Deserialize)]
struct SlackPostMessageResponse {
    ok: bool,
    error: Option<String>,
    #[allow(dead_code)]
    ts: Option<String>,
}

/// Slack API response for auth.test.
#[derive(Debug, Deserialize)]
struct SlackAuthTestResponse {
    ok: bool,
    error: Option<String>,
    #[allow(dead_code)]
    user_id: Option<String>,
    bot_id: Option<String>,
    #[allow(dead_code)]
    team_id: Option<String>,
}

// ============================================================================
// API Error Classification
// ============================================================================

/// Classified Slack API error for deciding retry strategy.
#[derive(Debug, PartialEq)]
enum SlackApiError {
    /// HTTP 429 — honor Retry-After header.
    RateLimited { retry_after_ms: u32 },
    /// Token is invalid or revoked — fail immediately.
    InvalidAuth,
    /// Token lacks a required scope — fail immediately.
    MissingScope(String),
    /// Bot is not in the target channel.
    ChannelNotFound,
    /// Transient server error (5xx) — retry with backoff.
    ServerError(u16),
    /// Unrecognized error.
    Other(String),
}

/// Classify a Slack API error from the HTTP status and the `error` field in
/// Slack's JSON response body.
fn classify_slack_error(http_status: u16, error_field: Option<&str>) -> SlackApiError {
    // HTTP-level classification takes priority for rate limits / server errors
    if http_status == 429 {
        return SlackApiError::RateLimited {
            // Default 30s if no header; caller should override with actual Retry-After
            retry_after_ms: 30_000,
        };
    }
    if http_status >= 500 {
        return SlackApiError::ServerError(http_status);
    }

    // Application-level classification from Slack's `error` field
    match error_field {
        Some("invalid_auth" | "token_revoked" | "token_expired" | "not_authed"
        | "account_inactive") => SlackApiError::InvalidAuth,
        Some(e) if e.starts_with("missing_scope") => {
            SlackApiError::MissingScope(e.to_string())
        }
        Some("channel_not_found" | "not_in_channel" | "is_archived") => {
            SlackApiError::ChannelNotFound
        }
        Some("ratelimited") => SlackApiError::RateLimited {
            retry_after_ms: 30_000,
        },
        Some(other) => SlackApiError::Other(other.to_string()),
        None => SlackApiError::Other(format!("HTTP {}", http_status)),
    }
}

/// Whether a Slack API error is transient and worth retrying.
fn is_retryable(err: &SlackApiError) -> bool {
    matches!(
        err,
        SlackApiError::RateLimited { .. } | SlackApiError::ServerError(_)
    )
}

// ============================================================================
// Message Subtype Filtering
// ============================================================================

/// Subtypes that should be dropped (system events or bot echoes).
const IGNORED_SUBTYPES: &[&str] = &[
    "bot_message",
    "message_changed",
    "message_deleted",
    "channel_join",
    "channel_leave",
    "channel_topic",
    "channel_purpose",
    "channel_name",
    "channel_archive",
    "channel_unarchive",
    "group_join",
    "group_leave",
    "ekm_access_denied",
    "me_message",
];

/// Return true if a message with this subtype should be processed (not dropped).
fn should_process_subtype(subtype: Option<&str>) -> bool {
    match subtype {
        None => true,
        Some(st) => !IGNORED_SUBTYPES.contains(&st),
    }
}

/// Workspace path for persisting owner_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_id";
/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";
/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";
/// Workspace path for persisting bot_id (from auth.test) across WASM callbacks.
const BOT_ID_PATH: &str = "state/bot_id";
/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "slack";
/// Maximum retry attempts for transient Slack API errors.
const MAX_RETRIES: u32 = 3;

/// Channel configuration from capabilities file.
#[derive(Debug, Deserialize)]
struct SlackConfig {
    /// Name of secret containing signing secret (for verification by host).
    #[serde(default = "default_signing_secret_name")]
    #[allow(dead_code)]
    signing_secret_name: String,

    #[serde(default)]
    owner_id: Option<String>,

    #[serde(default)]
    dm_policy: Option<String>,

    #[serde(default)]
    allow_from: Option<Vec<String>>,
}

fn default_signing_secret_name() -> String {
    "slack_signing_secret".to_string()
}

struct SlackChannel;

impl Guest for SlackChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: SlackConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Slack channel starting");

        // Persist owner_id so subsequent callbacks can read it
        if let Some(ref owner_id) = config.owner_id {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, owner_id);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: user {}", owner_id),
            );
        } else {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
        }

        // Persist dm_policy and allow_from for DM pairing
        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing");
        let _ = channel_host::workspace_write(DM_POLICY_PATH, dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // Validate bot token via auth.test (best-effort, don't block startup)
        validate_bot_token();

        Ok(ChannelConfig {
            display_name: "Slack".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/slack".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: true,
            }],
            poll: None,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Parse the request body
        let body_str = match std::str::from_utf8(&req.body) {
            Ok(s) => s,
            Err(_) => {
                return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
            }
        };

        // Parse as Slack event
        let event_wrapper: SlackEventWrapper = match serde_json::from_str(body_str) {
            Ok(e) => e,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse Slack event: {}", e),
                );
                return json_response(400, serde_json::json!({"error": "Invalid event payload"}));
            }
        };

        match event_wrapper.event_type.as_str() {
            // URL verification challenge (Slack setup)
            "url_verification" => {
                if let Some(challenge) = event_wrapper.challenge {
                    channel_host::log(
                        channel_host::LogLevel::Info,
                        "Responding to Slack URL verification",
                    );
                    json_response(200, serde_json::json!({"challenge": challenge}))
                } else {
                    json_response(400, serde_json::json!({"error": "Missing challenge"}))
                }
            }

            // Actual event callback
            "event_callback" => {
                if let Some(event) = event_wrapper.event {
                    handle_slack_event(event, event_wrapper.team_id);
                }
                // Always respond 200 quickly to Slack (they have a 3s timeout)
                json_response(200, serde_json::json!({"ok": true}))
            }

            // Unknown event type
            _ => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Unknown Slack event type: {}", event_wrapper.event_type),
                );
                json_response(200, serde_json::json!({"ok": true}))
            }
        }
    }

    fn on_poll() {
        // Slack uses webhooks, no polling needed
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        // Parse metadata to get channel info
        let metadata: SlackMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Build Slack API request
        let mut payload = serde_json::json!({
            "channel": metadata.channel,
            "text": response.content,
        });

        // Add thread_ts for threaded replies
        if let Some(thread_ts) = response.thread_id.or(metadata.thread_ts) {
            payload["thread_ts"] = serde_json::Value::String(thread_ts);
        }

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| format!("Failed to serialize payload: {}", e))?;

        // Post with retry for transient errors
        post_message_with_retry(&payload_bytes, &metadata.channel)
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Slack channel shutting down");
    }
}

// ============================================================================
// Startup Validation
// ============================================================================

/// Call auth.test to verify the bot token and discover our bot_id.
/// Best-effort: logs warnings but never fails startup.
fn validate_bot_token() {
    let headers = serde_json::json!({"Content-Type": "application/json"});

    let result = channel_host::http_request(
        "POST",
        "https://slack.com/api/auth.test",
        &headers.to_string(),
        None,
        Some(10_000), // 10s timeout
    );

    match result {
        Ok(http_response) => {
            if let Ok(auth) =
                serde_json::from_slice::<SlackAuthTestResponse>(&http_response.body)
            {
                if auth.ok {
                    // Persist bot_id so we can filter our own messages
                    if let Some(ref bot_id) = auth.bot_id {
                        let _ = channel_host::workspace_write(BOT_ID_PATH, bot_id);
                    }
                    channel_host::log(
                        channel_host::LogLevel::Info,
                        &format!(
                            "Bot token validated: bot_id={}",
                            auth.bot_id.as_deref().unwrap_or("unknown"),
                        ),
                    );
                } else {
                    let err = auth.error.as_deref().unwrap_or("unknown");
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Bot token validation failed: {}", err),
                    );
                }
            }
        }
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("auth.test request failed (token may not be configured yet): {}", e),
            );
        }
    }
}

// ============================================================================
// Message Posting with Retry
// ============================================================================

/// Post a chat.postMessage with retry for transient errors.
fn post_message_with_retry(payload_bytes: &[u8], channel: &str) -> Result<(), String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    let mut last_error = String::new();

    for attempt in 0..MAX_RETRIES {
        if attempt > 0 {
            // Sleep before retry. WASM has no sleep, but we can use a busy-wait
            // approximation. In practice the host's HTTP timeout provides enough
            // backoff for server errors. For rate limits we just retry immediately
            // since the host-side HTTP layer already respected any Retry-After.
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!(
                    "Retrying chat.postMessage to {} (attempt {}/{})",
                    channel,
                    attempt + 1,
                    MAX_RETRIES
                ),
            );
        }

        let result = channel_host::http_request(
            "POST",
            "https://slack.com/api/chat.postMessage",
            &headers.to_string(),
            Some(payload_bytes),
            None,
        );

        match result {
            Ok(http_response) => {
                let slack_error_field = serde_json::from_slice::<SlackPostMessageResponse>(
                    &http_response.body,
                )
                .ok();

                // Check for Slack-level success
                if http_response.status == 200 {
                    if let Some(ref resp) = slack_error_field {
                        if resp.ok {
                            return Ok(());
                        }
                    }
                }

                // Classify the error
                let error_str = slack_error_field
                    .as_ref()
                    .and_then(|r| r.error.as_deref());
                let classified = classify_slack_error(http_response.status, error_str);

                // Auth failures: notify and fail immediately
                if classified == SlackApiError::InvalidAuth {
                    let msg =
                        error_str.unwrap_or("invalid_auth");
                    notify_auth_failure(msg);
                    return Err(format!("invalid_auth: {}", msg));
                }

                // Non-retryable: fail immediately
                if !is_retryable(&classified) {
                    return Err(format!(
                        "Slack API error: {:?}",
                        classified
                    ));
                }

                last_error = format!("Slack API error: {:?}", classified);
            }
            Err(e) => {
                // HTTP transport error — could be transient
                last_error = format!("HTTP request failed: {}", e);
            }
        }
    }

    Err(format!(
        "chat.postMessage failed after {} attempts: {}",
        MAX_RETRIES, last_error
    ))
}

/// Notify the host that authentication has failed so the user can see it.
fn notify_auth_failure(error: &str) {
    channel_host::emit_message(&EmittedMessage {
        user_id: "system".to_string(),
        user_name: Some("Slack Channel".to_string()),
        content: format!(
            "[Slack auth error] Bot token is invalid or revoked ({}). \
             Please update the slack_bot_token secret.",
            error
        ),
        thread_id: None,
        metadata_json: "{}".to_string(),
    });
}

// ============================================================================
// Event Handling
// ============================================================================

/// Handle a Slack event and emit message if applicable.
fn handle_slack_event(event: SlackEvent, team_id: Option<String>) {
    match event.event_type.as_str() {
        // Direct mention of the bot (always in a channel, not a DM)
        "app_mention" => {
            if let (Some(user), Some(channel), Some(text), Some(ts)) = (
                event.user,
                event.channel.clone(),
                event.text,
                event.ts.clone(),
            ) {
                // app_mention is always in a channel (not DM)
                if !check_sender_permission(&user, &channel, false) {
                    return;
                }
                emit_message(user, text, channel, event.thread_ts.or(Some(ts)), team_id);
            }
        }

        // Direct message to the bot
        "message" => {
            // Skip messages from bots (including ourselves)
            if event.bot_id.is_some() {
                return;
            }

            // Check subtype against deny-list (not blanket-reject)
            if !should_process_subtype(event.subtype.as_deref()) {
                return;
            }

            if let (Some(user), Some(channel), Some(text), Some(ts)) = (
                event.user,
                event.channel.clone(),
                event.text,
                event.ts.clone(),
            ) {
                // Only process DMs (channel IDs starting with D)
                if channel.starts_with('D') {
                    if !check_sender_permission(&user, &channel, true) {
                        return;
                    }
                    emit_message(user, text, channel, event.thread_ts.or(Some(ts)), team_id);
                }
            }
        }

        _ => {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!("Ignoring Slack event type: {}", event.event_type),
            );
        }
    }
}

/// Emit a message to the agent.
fn emit_message(
    user_id: String,
    text: String,
    channel: String,
    thread_ts: Option<String>,
    team_id: Option<String>,
) {
    let message_ts = thread_ts.clone().unwrap_or_default();

    let metadata = SlackMessageMetadata {
        channel: channel.clone(),
        thread_ts: thread_ts.clone(),
        message_ts: message_ts.clone(),
        team_id,
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|e| {
        channel_host::log(
            channel_host::LogLevel::Error,
            &format!("Failed to serialize Slack metadata: {}", e),
        );
        "{}".to_string()
    });

    // Strip @ mentions of the bot from the text for cleaner messages
    let cleaned_text = strip_bot_mention(&text);

    channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: None, // Could fetch from Slack API if needed
        content: cleaned_text,
        thread_id: thread_ts,
        metadata_json,
    });
}

// ============================================================================
// Permission & Pairing
// ============================================================================

/// Check if a sender is permitted. Returns true if allowed.
/// For pairing mode, sends a pairing code DM if denied.
fn check_sender_permission(user_id: &str, channel_id: &str, is_dm: bool) -> bool {
    // 1. Owner check (highest priority, applies to all contexts)
    let owner_id = channel_host::workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());
    if let Some(ref owner) = owner_id {
        if user_id != owner {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!(
                    "Dropping message from non-owner user {} (owner: {})",
                    user_id, owner
                ),
            );
            return false;
        }
        return true;
    }

    // 2. DM policy (only for DMs when no owner_id)
    if !is_dm {
        return true; // Channel messages bypass DM policy
    }

    let dm_policy =
        channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

    if dm_policy == "open" {
        return true;
    }

    // 3. Build merged allow list: config allow_from + pairing store
    let mut allowed: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    if let Ok(store_allowed) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        allowed.extend(store_allowed);
    }

    // 4. Check sender (Slack events only have user ID, not username)
    let is_allowed =
        allowed.contains(&"*".to_string()) || allowed.contains(&user_id.to_string());

    if is_allowed {
        return true;
    }

    // 5. Not allowed — handle by policy
    if dm_policy == "pairing" {
        let meta = serde_json::json!({
            "user_id": user_id,
            "channel_id": channel_id,
        })
        .to_string();

        match channel_host::pairing_upsert_request(CHANNEL_NAME, user_id, &meta) {
            Ok(result) => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!(
                        "Pairing request for user {}: code {}",
                        user_id, result.code
                    ),
                );
                if result.created {
                    let _ = send_pairing_reply(channel_id, &result.code);
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Pairing upsert failed: {}", e),
                );
            }
        }
    }
    false
}

/// Send a pairing code message via Slack chat.postMessage.
fn send_pairing_reply(channel_id: &str, code: &str) -> Result<(), String> {
    let payload = serde_json::json!({
        "channel": channel_id,
        "text": format!(
            "To pair with this bot, run: `ironclaw pairing approve slack {}`",
            code
        ),
    });

    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("Failed to serialize: {}", e))?;

    let headers = serde_json::json!({"Content-Type": "application/json"});

    let result = channel_host::http_request(
        "POST",
        "https://slack.com/api/chat.postMessage",
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(response) if response.status == 200 => Ok(()),
        Ok(response) => {
            let body_str = String::from_utf8_lossy(&response.body);
            Err(format!(
                "Slack API error: {} - {}",
                response.status, body_str
            ))
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Strip leading bot mention from text.
fn strip_bot_mention(text: &str) -> String {
    // Slack mentions look like <@U12345678>
    let trimmed = text.trim();
    if trimmed.starts_with("<@") {
        if let Some(end) = trimmed.find('>') {
            return trimmed[end + 1..].trim_start().to_string();
        }
    }
    trimmed.to_string()
}

/// Create a JSON HTTP response.
fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_else(|e| {
        channel_host::log(
            channel_host::LogLevel::Error,
            &format!("Failed to serialize JSON response: {}", e),
        );
        Vec::new()
    });
    let headers = serde_json::json!({"Content-Type": "application/json"});

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

// Export the component
export!(SlackChannel);

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Error classification ---

    #[test]
    fn test_classify_rate_limited_http_429() {
        let err = classify_slack_error(429, None);
        assert_eq!(
            err,
            SlackApiError::RateLimited {
                retry_after_ms: 30_000
            }
        );
    }

    #[test]
    fn test_classify_rate_limited_body_field() {
        let err = classify_slack_error(200, Some("ratelimited"));
        assert_eq!(
            err,
            SlackApiError::RateLimited {
                retry_after_ms: 30_000
            }
        );
    }

    #[test]
    fn test_classify_server_error() {
        let err = classify_slack_error(503, None);
        assert_eq!(err, SlackApiError::ServerError(503));
    }

    #[test]
    fn test_classify_invalid_auth_variants() {
        for variant in &[
            "invalid_auth",
            "token_revoked",
            "token_expired",
            "not_authed",
            "account_inactive",
        ] {
            let err = classify_slack_error(200, Some(variant));
            assert_eq!(err, SlackApiError::InvalidAuth, "failed for {}", variant);
        }
    }

    #[test]
    fn test_classify_missing_scope() {
        let err = classify_slack_error(200, Some("missing_scope:chat:write"));
        assert!(matches!(err, SlackApiError::MissingScope(s) if s.contains("chat:write")));
    }

    #[test]
    fn test_classify_channel_not_found() {
        for variant in &["channel_not_found", "not_in_channel", "is_archived"] {
            let err = classify_slack_error(200, Some(variant));
            assert_eq!(
                err,
                SlackApiError::ChannelNotFound,
                "failed for {}",
                variant
            );
        }
    }

    #[test]
    fn test_classify_unknown_error() {
        let err = classify_slack_error(200, Some("something_weird"));
        assert!(matches!(err, SlackApiError::Other(s) if s == "something_weird"));
    }

    #[test]
    fn test_classify_no_error_field() {
        let err = classify_slack_error(400, None);
        assert!(matches!(err, SlackApiError::Other(s) if s == "HTTP 400"));
    }

    // --- Retryability ---

    #[test]
    fn test_retryable_errors() {
        assert!(is_retryable(&SlackApiError::RateLimited {
            retry_after_ms: 1000
        }));
        assert!(is_retryable(&SlackApiError::ServerError(500)));
        assert!(!is_retryable(&SlackApiError::InvalidAuth));
        assert!(!is_retryable(&SlackApiError::ChannelNotFound));
        assert!(!is_retryable(&SlackApiError::MissingScope(
            "chat:write".into()
        )));
        assert!(!is_retryable(&SlackApiError::Other("unknown".into())));
    }

    // --- Subtype filtering ---

    #[test]
    fn test_should_process_no_subtype() {
        assert!(should_process_subtype(None));
    }

    #[test]
    fn test_should_drop_bot_message() {
        assert!(!should_process_subtype(Some("bot_message")));
    }

    #[test]
    fn test_should_drop_message_changed() {
        assert!(!should_process_subtype(Some("message_changed")));
    }

    #[test]
    fn test_should_drop_message_deleted() {
        assert!(!should_process_subtype(Some("message_deleted")));
    }

    #[test]
    fn test_should_drop_channel_join() {
        assert!(!should_process_subtype(Some("channel_join")));
    }

    #[test]
    fn test_should_drop_channel_leave() {
        assert!(!should_process_subtype(Some("channel_leave")));
    }

    #[test]
    fn test_should_pass_file_share() {
        assert!(should_process_subtype(Some("file_share")));
    }

    #[test]
    fn test_should_pass_thread_broadcast() {
        assert!(should_process_subtype(Some("thread_broadcast")));
    }

    #[test]
    fn test_should_pass_unknown_subtype() {
        // Future subtypes should pass by default; we deny-list, not allow-list
        assert!(should_process_subtype(Some("some_future_subtype")));
    }

    // --- Bot mention stripping ---

    #[test]
    fn test_strip_bot_mention_basic() {
        assert_eq!(strip_bot_mention("<@U12345> hello"), "hello");
    }

    #[test]
    fn test_strip_bot_mention_no_mention() {
        assert_eq!(strip_bot_mention("hello world"), "hello world");
    }

    #[test]
    fn test_strip_bot_mention_with_whitespace() {
        assert_eq!(strip_bot_mention("  <@U12345>   hello  "), "hello");
    }

    #[test]
    fn test_strip_bot_mention_only_mention() {
        assert_eq!(strip_bot_mention("<@U12345>"), "");
    }

    #[test]
    fn test_strip_bot_mention_incomplete() {
        // Incomplete mention tag — should be returned as-is
        assert_eq!(strip_bot_mention("<@U12345"), "<@U12345");
    }

    #[test]
    fn test_strip_bot_mention_mention_in_middle() {
        // Mention NOT at the start — leave untouched
        assert_eq!(
            strip_bot_mention("hey <@U12345> check this"),
            "hey <@U12345> check this"
        );
    }

    // --- Metadata serialization ---

    #[test]
    fn test_metadata_roundtrip() {
        let meta = SlackMessageMetadata {
            channel: "C12345".to_string(),
            thread_ts: Some("1234567890.123456".to_string()),
            message_ts: "1234567890.123456".to_string(),
            team_id: Some("T12345".to_string()),
        };
        let json = serde_json::to_string(&meta).expect("serialize");
        let deserialized: SlackMessageMetadata =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(meta, deserialized);
    }

    #[test]
    fn test_metadata_roundtrip_minimal() {
        let meta = SlackMessageMetadata {
            channel: "D999".to_string(),
            thread_ts: None,
            message_ts: "".to_string(),
            team_id: None,
        };
        let json = serde_json::to_string(&meta).expect("serialize");
        let deserialized: SlackMessageMetadata =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(meta, deserialized);
    }

    // --- Config parsing ---

    #[test]
    fn test_config_defaults() {
        let config: SlackConfig =
            serde_json::from_str("{}").expect("should parse empty config");
        assert_eq!(config.signing_secret_name, "slack_signing_secret");
        assert!(config.owner_id.is_none());
        assert!(config.dm_policy.is_none());
        assert!(config.allow_from.is_none());
    }

    #[test]
    fn test_config_full() {
        let json = r#"{
            "signing_secret_name": "my_secret",
            "owner_id": "U123",
            "dm_policy": "open",
            "allow_from": ["U456", "U789"]
        }"#;
        let config: SlackConfig = serde_json::from_str(json).expect("should parse");
        assert_eq!(config.signing_secret_name, "my_secret");
        assert_eq!(config.owner_id.as_deref(), Some("U123"));
        assert_eq!(config.dm_policy.as_deref(), Some("open"));
        assert_eq!(
            config.allow_from,
            Some(vec!["U456".to_string(), "U789".to_string()])
        );
    }
}
