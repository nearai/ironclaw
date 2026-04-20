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
use std::collections::HashMap;

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage, InboundAttachment};

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

    /// File attachments shared in the message.
    #[serde(default)]
    files: Option<Vec<SlackFile>>,
}

/// Slack file attachment.
#[derive(Debug, Deserialize)]
struct SlackFile {
    /// File ID.
    id: String,
    /// MIME type.
    mimetype: Option<String>,
    /// Original filename.
    name: Option<String>,
    /// File size in bytes.
    size: Option<u64>,
    /// URL to download the file (requires auth).
    url_private: Option<String>,
}

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
struct ActiveSlackThreadKey {
    team_id: Option<String>,
    channel: String,
    thread_ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveSlackThread {
    #[serde(flatten)]
    key: ActiveSlackThreadKey,
    #[serde(default)]
    last_seen_ms: u64,
}

/// Slack API response for chat.postMessage.
#[derive(Debug, Deserialize)]
struct SlackPostMessageResponse {
    ok: bool,
    error: Option<String>,
    ts: Option<String>,
}

/// Workspace path for persisting owner_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_id";
/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";
/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";
/// Workspace path for thread timestamps the bot has already joined.
const ACTIVE_THREADS_PATH: &str = "state/active_threads";
/// Threads expire after 24h of inactivity so the participation cache stays bounded.
const ACTIVE_THREAD_TTL_MS: u64 = 24 * 60 * 60 * 1000;
/// Hard cap on remembered threads per workspace.
const ACTIVE_THREAD_MAX_ENTRIES: usize = 256;
/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "slack";

#[cfg(not(test))]
fn host_workspace_read(path: &str) -> Option<String> {
    channel_host::workspace_read(path)
}

#[cfg(test)]
fn host_workspace_read(path: &str) -> Option<String> {
    test_host::workspace_read(path)
}

#[cfg(not(test))]
fn host_workspace_write(path: &str, content: &str) -> Result<(), String> {
    channel_host::workspace_write(path, content)
}

#[cfg(test)]
fn host_workspace_write(path: &str, content: &str) -> Result<(), String> {
    test_host::workspace_write(path, content)
}

#[cfg(not(test))]
fn host_emit_message(message: &EmittedMessage) {
    channel_host::emit_message(message);
}

#[cfg(test)]
fn host_emit_message(message: &EmittedMessage) {
    test_host::emit_message(message);
}

#[cfg(not(test))]
fn host_now_millis() -> u64 {
    channel_host::now_millis()
}

#[cfg(test)]
fn host_now_millis() -> u64 {
    test_host::now_millis()
}

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
            let _ = host_workspace_write(OWNER_ID_PATH, owner_id);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: user {}", owner_id),
            );
        } else {
            let _ = host_workspace_write(OWNER_ID_PATH, "");
        }

        // Persist dm_policy and allow_from for DM pairing
        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing");
        let _ = host_workspace_write(DM_POLICY_PATH, dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = host_workspace_write(ALLOW_FROM_PATH, &allow_from_json);

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
                    handle_slack_event(event, event_wrapper.team_id, event_wrapper.event_id);
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
        let metadata: SlackMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        let thread_ts = response.thread_id.clone().or(metadata.thread_ts.clone());
        let ts = post_slack_message(
            &metadata.channel,
            &response.content,
            thread_ts.as_deref(),
        )?;

        if let Some(thread_ts) = thread_ts {
            if let Err(e) = remember_active_slack_thread(
                metadata.team_id.as_deref(),
                &metadata.channel,
                &thread_ts,
            ) {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to track active thread: {}", e),
                );
            }
        }

        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Posted message to Slack channel {}: ts={}",
                metadata.channel,
                ts.unwrap_or_default()
            ),
        );

        Ok(())
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        let target = resolve_broadcast_target(&user_id);
        if target.is_empty() {
            return Err(
                "broadcast failed: no target specified. Pass a Slack channel ID (C0...) \
                 or user ID (U0...) as the target."
                    .to_string(),
            );
        }

        if !looks_like_slack_id(target) {
            return Err(format!(
                "Broadcast target '{}' is not a valid Slack ID (expected C/U/D/G/W prefix). \
                 Use a channel ID (C0...) or user ID (U0...), not a channel name.",
                target
            ));
        }

        let ts = post_slack_message(target, &response.content, response.thread_id.as_deref())?;

        // Track the thread so replies to this broadcast are recognized as
        // active threads. Use the explicit thread_id if provided, otherwise
        // fall back to the message timestamp returned by Slack (which becomes
        // the thread root if someone replies to this message).
        if let Some(thread_ts) = response.thread_id.as_deref().or(ts.as_deref()) {
            if let Err(e) = track_active_thread(target, thread_ts) {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to track active thread: {}", e),
                );
            }
        }

        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Broadcast message to Slack target {}: ts={}",
                target,
                ts.unwrap_or_default()
            ),
        );

        Ok(())
    }

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Slack channel shutting down");
    }
}

/// Extract attachments from Slack file objects.
fn extract_slack_attachments(files: &Option<Vec<SlackFile>>) -> Vec<InboundAttachment> {
    let Some(files) = files else {
        return Vec::new();
    };
    files
        .iter()
        .map(|f| InboundAttachment {
            id: f.id.clone(),
            mime_type: f
                .mimetype
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            filename: f.name.clone(),
            size_bytes: f.size,
            source_url: f.url_private.clone(),
            storage_key: None,
            extracted_text: None,
            extras_json: String::new(),
        })
        .collect()
}

/// Download a file from Slack using the url_private endpoint.
///
/// Slack file downloads require Bearer auth with the bot token, which is
/// injected by the host credential system via `channel_host::http_request`.
fn download_slack_file(url: &str) -> Result<Vec<u8>, String> {
    let headers = serde_json::json!({});

    let result = channel_host::http_request("GET", url, &headers.to_string(), None, None);

    let response = result.map_err(|e| format!("Slack file download failed: {}", e))?;

    if response.status != 200 {
        let body_str = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Slack file download returned {}: {}",
            response.status, body_str
        ));
    }

    Ok(response.body)
}

/// Download file bytes and store them via the host for processing.
///
/// Downloads all file types (images, documents, etc.) so the host-side
/// middleware can process them (vision pipeline for images, text extraction
/// for documents, transcription for audio, etc.).
/// Maximum file size to download (20 MB). Files larger than this are skipped
/// to avoid excessive memory use and slow downloads in the WASM runtime.
const MAX_DOWNLOAD_SIZE_BYTES: u64 = 20 * 1024 * 1024;

fn download_and_store_slack_files(attachments: &[InboundAttachment]) {
    for att in attachments {
        let Some(ref url) = att.source_url else {
            continue;
        };

        // Skip files that exceed the size limit
        if let Some(size) = att.size_bytes {
            if size > MAX_DOWNLOAD_SIZE_BYTES {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "Skipping Slack file download: {} bytes exceeds {} MB limit (id={})",
                        size,
                        MAX_DOWNLOAD_SIZE_BYTES / (1024 * 1024),
                        att.id
                    ),
                );
                continue;
            }
        }

        match download_slack_file(url) {
            Ok(bytes) => {
                // Post-download size guard: metadata size_bytes is optional,
                // so a file with no size info could bypass the pre-download check.
                if bytes.len() as u64 > MAX_DOWNLOAD_SIZE_BYTES {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!(
                            "Discarding Slack file after download: {} bytes exceeds {} MB limit (id={})",
                            bytes.len(),
                            MAX_DOWNLOAD_SIZE_BYTES / (1024 * 1024),
                            att.id
                        ),
                    );
                    continue;
                }

                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!(
                        "Downloaded Slack file: {} bytes, mime={}",
                        bytes.len(),
                        att.mime_type
                    ),
                );
                if let Err(e) = channel_host::store_attachment_data(&att.id, &bytes) {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to store Slack file data: {}", e),
                    );
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to download Slack file: {}", e),
                );
            }
        }
    }
}

fn prepare_inbound_attachments(files: &Option<Vec<SlackFile>>) -> Vec<InboundAttachment> {
    let attachments = extract_slack_attachments(files);
    download_and_store_slack_files(&attachments);
    attachments
}

/// Handle a Slack event and emit message if applicable.
fn handle_slack_event(event: SlackEvent, team_id: Option<String>, _event_id: Option<String>) {
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
                let attachments = prepare_inbound_attachments(&event.files);
                emit_message(
                    user,
                    text,
                    channel,
                    event.thread_ts.or(Some(ts)),
                    team_id,
                    attachments,
                );
            }
        }

        // Direct message or thread follow-up to the bot
        "message" => {
            // Skip messages from bots (including ourselves)
            if event.bot_id.is_some() || event.subtype.is_some() {
                return;
            }

            if let (Some(user), Some(channel), Some(text), Some(ts)) = (
                event.user,
                event.channel.clone(),
                event.text,
                event.ts.clone(),
            ) {
                let is_dm = channel.starts_with('D');
                let is_active_thread = event.thread_ts.as_deref().is_some_and(|thread_ts| {
                    is_active_slack_thread(team_id.as_deref(), &channel, thread_ts)
                });

                // DMs are always processed. For channel threads, once the bot
                // has already replied in a thread we intentionally allow
                // follow-ups from that thread without re-running DM pairing or
                // allow_from checks. This matches Slack's app_mention behavior:
                // the thread stays as visible as the surrounding channel.
                if is_dm || is_active_thread {
                    if !check_sender_permission(&user, &channel, is_dm) {
                        return;
                    }
                    let attachments = prepare_inbound_attachments(&event.files);
                    emit_message(
                        user,
                        text,
                        channel,
                        event.thread_ts.or(Some(ts)),
                        team_id,
                        attachments,
                    );
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
    attachments: Vec<InboundAttachment>,
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

    host_emit_message(&EmittedMessage {
        user_id,
        user_name: None, // Could fetch from Slack API if needed
        content: cleaned_text,
        thread_id: thread_ts,
        metadata_json,
        attachments,
    });
}

fn active_slack_thread_key(
    team_id: Option<&str>,
    channel: &str,
    thread_ts: &str,
) -> ActiveSlackThreadKey {
    ActiveSlackThreadKey {
        team_id: team_id.map(str::to_string),
        channel: channel.to_string(),
        thread_ts: thread_ts.to_string(),
    }
}

fn active_slack_thread_entry(
    team_id: Option<&str>,
    channel: &str,
    thread_ts: &str,
    last_seen_ms: u64,
) -> ActiveSlackThread {
    ActiveSlackThread {
        key: active_slack_thread_key(team_id, channel, thread_ts),
        last_seen_ms,
    }
}

fn parse_active_slack_threads(
    raw: Option<&str>,
    now_ms: u64,
) -> HashMap<ActiveSlackThreadKey, u64> {
    raw.and_then(|value| serde_json::from_str::<Vec<ActiveSlackThread>>(value).ok())
        .map(|threads| {
            threads
                .into_iter()
                .map(|thread| {
                    (
                        thread.key,
                        if thread.last_seen_ms == 0 {
                            now_ms
                        } else {
                            thread.last_seen_ms
                        },
                    )
                })
                .collect()
        })
        .or_else(|| {
            raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
                .map(|legacy| {
                    legacy
                        .into_iter()
                        .map(|thread_ts| (active_slack_thread_key(None, "", &thread_ts), now_ms))
                        .collect()
                })
        })
        .unwrap_or_default()
}

fn serialize_active_slack_threads(threads: &HashMap<ActiveSlackThreadKey, u64>) -> String {
    let mut sorted: Vec<_> = threads
        .iter()
        .map(|(key, last_seen_ms)| {
            active_slack_thread_entry(
                key.team_id.as_deref(),
                &key.channel,
                &key.thread_ts,
                *last_seen_ms,
            )
        })
        .collect();
    sorted.sort_unstable_by(|left, right| {
        left.key
            .team_id
            .cmp(&right.key.team_id)
            .then(left.key.channel.cmp(&right.key.channel))
            .then(left.key.thread_ts.cmp(&right.key.thread_ts))
    });
    serde_json::to_string(&sorted).unwrap_or_else(|_| "[]".to_string())
}

fn prune_active_slack_threads(threads: &mut HashMap<ActiveSlackThreadKey, u64>, now_ms: u64) {
    let cutoff = now_ms.saturating_sub(ACTIVE_THREAD_TTL_MS);
    threads.retain(|_, last_seen_ms| *last_seen_ms >= cutoff);

    if threads.len() <= ACTIVE_THREAD_MAX_ENTRIES {
        return;
    }

    let mut entries: Vec<_> = threads
        .iter()
        .map(|(key, last_seen_ms)| (key.clone(), *last_seen_ms))
        .collect();
    entries.sort_unstable_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then(left.0.team_id.cmp(&right.0.team_id))
            .then(left.0.channel.cmp(&right.0.channel))
            .then(left.0.thread_ts.cmp(&right.0.thread_ts))
    });
    entries.truncate(ACTIVE_THREAD_MAX_ENTRIES);
    *threads = entries.into_iter().collect();
}

fn load_active_slack_threads_from_workspace() -> HashMap<ActiveSlackThreadKey, u64> {
    let raw = host_workspace_read(ACTIVE_THREADS_PATH);
    let now_ms = host_now_millis();
    let mut threads = parse_active_slack_threads(raw.as_deref(), now_ms);
    prune_active_slack_threads(&mut threads, now_ms);

    let serialized = serialize_active_slack_threads(&threads);
    let should_persist = raw.as_deref().is_some_and(|existing| existing != serialized)
        || (raw.is_none() && !threads.is_empty());
    if should_persist {
        let _ = host_workspace_write(ACTIVE_THREADS_PATH, &serialized);
    }

    threads
}

fn active_slack_thread_is_known(
    raw: Option<&str>,
    team_id: Option<&str>,
    channel: &str,
    thread_ts: &str,
    now_ms: u64,
) -> bool {
    let mut threads = parse_active_slack_threads(raw, now_ms);
    prune_active_slack_threads(&mut threads, now_ms);
    threads.contains_key(&active_slack_thread_key(team_id, channel, thread_ts))
        || threads.contains_key(&active_slack_thread_key(None, channel, thread_ts))
        || threads.contains_key(&active_slack_thread_key(None, "", thread_ts))
}

fn is_active_slack_thread(team_id: Option<&str>, channel: &str, thread_ts: &str) -> bool {
    let threads = load_active_slack_threads_from_workspace();
    threads.contains_key(&active_slack_thread_key(team_id, channel, thread_ts))
        || threads.contains_key(&active_slack_thread_key(None, channel, thread_ts))
        || threads.contains_key(&active_slack_thread_key(None, "", thread_ts))
}

fn track_active_thread(channel: &str, thread_ts: &str) -> Result<(), String> {
    remember_active_slack_thread(None, channel, thread_ts)
}

fn remember_active_slack_thread(
    team_id: Option<&str>,
    channel: &str,
    thread_ts: &str,
) -> Result<(), String> {
    if channel.starts_with('D') {
        return Ok(());
    }

    let now_ms = host_now_millis();
    let mut threads = load_active_slack_threads_from_workspace();
    let key = active_slack_thread_key(team_id, channel, thread_ts);
    threads.insert(key, now_ms);
    threads.remove(&active_slack_thread_key(None, "", thread_ts));
    prune_active_slack_threads(&mut threads, now_ms);

    host_workspace_write(ACTIVE_THREADS_PATH, &serialize_active_slack_threads(&threads))
}

type ActiveThreads = HashMap<String, u64>;

fn active_thread_key(channel: &str, thread_ts: &str) -> String {
    format!("{channel}/{thread_ts}")
}

fn is_thread_marker_fresh(last_seen_millis: u64, now_millis: u64) -> bool {
    now_millis.saturating_sub(last_seen_millis) <= ACTIVE_THREAD_TTL_MS
}

fn prune_active_threads(active_threads: &mut ActiveThreads, now_millis: u64) -> bool {
    let mut changed = false;
    active_threads.retain(|_, last_seen_millis| {
        let keep = is_thread_marker_fresh(*last_seen_millis, now_millis);
        if !keep {
            changed = true;
        }
        keep
    });

    if active_threads.len() > ACTIVE_THREAD_MAX_ENTRIES {
        let mut oldest_first: Vec<_> = active_threads
            .iter()
            .map(|(key, last_seen_millis)| (key.clone(), *last_seen_millis))
            .collect();
        oldest_first.sort_by_key(|(_, last_seen_millis)| *last_seen_millis);

        for (key, _) in oldest_first
            .into_iter()
            .take(active_threads.len() - ACTIVE_THREAD_MAX_ENTRIES)
        {
            active_threads.remove(&key);
            changed = true;
        }
    }

    changed
}

// ============================================================================
// Permission & Pairing
// ============================================================================

/// Check if a sender is permitted. Returns true if allowed.
/// For pairing mode, sends a pairing code DM if denied.
fn check_sender_permission(user_id: &str, channel_id: &str, is_dm: bool) -> bool {
    // 1. Owner check (highest priority, applies to all contexts)
    let owner_id = host_workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());
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

    let dm_policy = host_workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

    if dm_policy == "open" {
        return true;
    }

    // 3. Build merged allow list: config allow_from + pairing store
    let mut allowed: Vec<String> = host_workspace_read(ALLOW_FROM_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    if let Ok(store_allowed) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        allowed.extend(store_allowed);
    }

    // 4. Check sender (Slack events only have user ID, not username)
    let is_allowed = allowed.contains(&"*".to_string()) || allowed.contains(&user_id.to_string());

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
                    &format!("Pairing request for user {}: code {}", user_id, result.code),
                );
                let _ = send_pairing_reply(channel_id, &result.code);
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
            "Enter this code in IronClaw to pair your slack account: `{}`. CLI fallback: `ironclaw pairing approve slack {}`",
            code, code
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

/// Post a message via Slack `chat.postMessage` and return the message timestamp.
///
/// The bot token is injected by the host credential system — this function
/// only sets `Content-Type`. Used by both `on_respond` and `on_broadcast`.
fn post_slack_message(
    channel: &str,
    text: &str,
    thread_ts: Option<&str>,
) -> Result<Option<String>, String> {
    let payload = build_broadcast_payload(channel, text, thread_ts);
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| format!("Failed to serialize payload: {}", e))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    let result = channel_host::http_request(
        "POST",
        "https://slack.com/api/chat.postMessage",
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(http_response) => {
            if http_response.status != 200 {
                return Err(format!(
                    "Slack API returned status {}",
                    http_response.status
                ));
            }

            let slack_response: SlackPostMessageResponse =
                serde_json::from_slice(&http_response.body)
                    .map_err(|e| format!("Failed to parse Slack response: {}", e))?;

            if !slack_response.ok {
                return Err(format!(
                    "Slack API error: {}",
                    slack_response
                        .error
                        .unwrap_or_else(|| "unknown".to_string())
                ));
            }

            Ok(slack_response.ts)
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

/// Normalize a broadcast target by stripping a leading `#` if present.
///
/// The message tool passes the target as `user_id` (e.g. `#C0123ABC`,
/// `C0123ABC`, or `U0123ABC`). The Slack API expects a channel ID (C0...)
/// or user ID (U0...), not a channel name.
fn resolve_broadcast_target(raw: &str) -> &str {
    raw.strip_prefix('#').unwrap_or(raw)
}

/// Check if a string looks like a Slack ID (starts with C, U, D, G, or W followed by alphanumeric).
fn looks_like_slack_id(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some('C' | 'U' | 'D' | 'G' | 'W') => {
            chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

/// Build the JSON payload for a Slack `chat.postMessage` broadcast.
fn build_broadcast_payload(
    target: &str,
    content: &str,
    thread_ts: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "channel": target,
        "text": content,
    });
    if let Some(ts) = thread_ts {
        payload["thread_ts"] = serde_json::Value::String(ts.to_string());
    }
    payload
}

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

#[cfg(test)]
mod test_host {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RecordedMessage {
        pub user_id: String,
        pub content: String,
        pub thread_id: Option<String>,
        pub metadata_json: String,
    }

    #[derive(Default)]
    struct TestHostState {
        workspace: HashMap<String, String>,
        emitted_messages: Vec<RecordedMessage>,
        now_millis: u64,
    }

    std::thread_local! {
        static STATE: RefCell<TestHostState> = RefCell::new(TestHostState::default());
    }

    pub fn reset() {
        STATE.with(|state| *state.borrow_mut() = TestHostState::default());
    }

    pub fn set_now_millis(now_millis: u64) {
        STATE.with(|state| state.borrow_mut().now_millis = now_millis);
    }

    pub fn now_millis() -> u64 {
        STATE.with(|state| state.borrow().now_millis)
    }

    pub fn workspace_read(path: &str) -> Option<String> {
        STATE.with(|state| state.borrow().workspace.get(path).cloned())
    }

    pub fn workspace_write(path: &str, content: &str) -> Result<(), String> {
        STATE.with(|state| {
            state
                .borrow_mut()
                .workspace
                .insert(path.to_string(), content.to_string());
        });
        Ok(())
    }

    pub fn set_workspace(path: &str, content: &str) {
        let _ = workspace_write(path, content);
    }

    pub fn emit_message(message: &EmittedMessage) {
        STATE.with(|state| {
            state.borrow_mut().emitted_messages.push(RecordedMessage {
                user_id: message.user_id.clone(),
                content: message.content.clone(),
                thread_id: message.thread_id.clone(),
                metadata_json: message.metadata_json.clone(),
            });
        });
    }

    pub fn take_emitted_messages() -> Vec<RecordedMessage> {
        STATE.with(|state| std::mem::take(&mut state.borrow_mut().emitted_messages))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_thread_message_event(thread_ts: &str) -> SlackEvent {
        SlackEvent {
            event_type: "message".to_string(),
            user: Some("U123".to_string()),
            channel: Some("C123".to_string()),
            text: Some("follow up".to_string()),
            thread_ts: Some(thread_ts.to_string()),
            ts: Some("1710000000.000002".to_string()),
            bot_id: None,
            subtype: None,
            files: None,
        }
    }

    #[test]
    fn test_extract_slack_attachments_with_files() {
        let files = Some(vec![
            SlackFile {
                id: "F123".to_string(),
                mimetype: Some("image/png".to_string()),
                name: Some("screenshot.png".to_string()),
                size: Some(50000),
                url_private: Some("https://files.slack.com/F123".to_string()),
            },
            SlackFile {
                id: "F456".to_string(),
                mimetype: Some("application/pdf".to_string()),
                name: Some("doc.pdf".to_string()),
                size: Some(120000),
                url_private: None,
            },
        ]);

        let attachments = extract_slack_attachments(&files);
        assert_eq!(attachments.len(), 2);

        assert_eq!(attachments[0].id, "F123");
        assert_eq!(attachments[0].mime_type, "image/png");
        assert_eq!(attachments[0].filename, Some("screenshot.png".to_string()));
        assert_eq!(attachments[0].size_bytes, Some(50000));
        assert_eq!(
            attachments[0].source_url,
            Some("https://files.slack.com/F123".to_string())
        );

        assert_eq!(attachments[1].id, "F456");
        assert_eq!(attachments[1].mime_type, "application/pdf");
        assert!(attachments[1].source_url.is_none());
    }

    #[test]
    fn test_extract_slack_attachments_none() {
        let attachments = extract_slack_attachments(&None);
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_extract_slack_attachments_empty() {
        let attachments = extract_slack_attachments(&Some(vec![]));
        assert!(attachments.is_empty());
    }

    #[test]
    fn test_extract_slack_attachments_missing_mime() {
        let files = Some(vec![SlackFile {
            id: "F789".to_string(),
            mimetype: None,
            name: Some("unknown".to_string()),
            size: None,
            url_private: None,
        }]);

        let attachments = extract_slack_attachments(&files);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].mime_type, "application/octet-stream");
    }

    #[test]
    fn test_parse_slack_event_with_files() {
        let json = r#"{
            "type": "message",
            "user": "U123",
            "channel": "D456",
            "text": "Check this file",
            "ts": "1234567890.000001",
            "files": [
                {
                    "id": "F001",
                    "mimetype": "image/jpeg",
                    "name": "photo.jpg",
                    "size": 30000,
                    "url_private": "https://files.slack.com/F001"
                }
            ]
        }"#;

        let event: SlackEvent = serde_json::from_str(json).unwrap();
        assert!(event.files.is_some());
        let files = event.files.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].id, "F001");
    }

    #[test]
    fn test_parse_slack_event_without_files() {
        let json = r#"{
            "type": "message",
            "user": "U123",
            "channel": "D456",
            "text": "Just text",
            "ts": "1234567890.000001"
        }"#;

        let event: SlackEvent = serde_json::from_str(json).unwrap();
        assert!(event.files.is_none());
    }

    #[test]
    fn test_max_download_size_constant() {
        assert_eq!(MAX_DOWNLOAD_SIZE_BYTES, 20 * 1024 * 1024);
    }

    #[test]
    fn test_active_slack_threads_round_trip() {
        let now_ms = 1_710_000_000_000_u64;
        let raw = format!(
            r#"[{{"team_id":null,"channel":"G2","thread_ts":"678.90","last_seen_ms":{now_ms}}},{{"team_id":"T1","channel":"C1","thread_ts":"123.45","last_seen_ms":{now_ms}}}]"#
        );
        let threads = parse_active_slack_threads(Some(&raw), now_ms);
        assert_eq!(
            threads.get(&active_slack_thread_key(Some("T1"), "C1", "123.45")),
            Some(&now_ms)
        );
        assert_eq!(
            threads.get(&active_slack_thread_key(None, "G2", "678.90")),
            Some(&now_ms)
        );
        assert!(active_slack_thread_is_known(
            Some(&raw),
            Some("T1"),
            "C1",
            "123.45",
            now_ms,
        ));
        assert!(!active_slack_thread_is_known(
            Some(&raw),
            Some("T1"),
            "C2",
            "123.45",
            now_ms,
        ));
        assert_eq!(serialize_active_slack_threads(&threads), raw);
    }

    #[test]
    fn test_active_slack_threads_accept_legacy_timestamps() {
        let now_ms = 1_710_000_000_000_u64;
        let raw = r#"["123.45","678.90"]"#;
        let threads = parse_active_slack_threads(Some(raw), now_ms);
        assert_eq!(
            threads.get(&active_slack_thread_key(None, "", "123.45")),
            Some(&now_ms)
        );
        assert!(active_slack_thread_is_known(
            Some(raw),
            Some("T1"),
            "C1",
            "123.45",
            now_ms,
        ));
        assert!(!active_slack_thread_is_known(
            Some(raw),
            Some("T1"),
            "C1",
            "999.99",
            now_ms,
        ));
    }

    #[test]
    fn test_active_slack_threads_prune_expired_and_cap_entries() {
        let now_ms = ACTIVE_THREAD_TTL_MS + 10_000;
        let mut threads = HashMap::new();
        threads.insert(active_slack_thread_key(Some("T1"), "C1", "expired"), 1);
        for idx in 0..(ACTIVE_THREAD_MAX_ENTRIES + 10) {
            threads.insert(
                active_slack_thread_key(Some("T1"), "C1", &format!("live-{idx}")),
                now_ms.saturating_add(idx as u64),
            );
        }

        prune_active_slack_threads(&mut threads, now_ms);
        assert_eq!(threads.len(), ACTIVE_THREAD_MAX_ENTRIES);
        assert!(!threads.contains_key(&active_slack_thread_key(Some("T1"), "C1", "expired")));
        assert!(!threads.contains_key(&active_slack_thread_key(Some("T1"), "C1", "live-0")));
    }

    #[test]
    fn test_active_slack_threads_ignore_invalid_json() {
        assert!(parse_active_slack_threads(Some("not-json"), 123).is_empty());
        assert!(parse_active_slack_threads(None, 123).is_empty());
    }

    #[test]
    fn test_handle_slack_event_emits_for_known_active_thread() {
        test_host::reset();
        test_host::set_now_millis(1_710_000_000_000_u64);

        let threads = HashMap::from([(
            active_slack_thread_key(Some("T1"), "C123", "1710000000.000001"),
            1_710_000_000_000_u64,
        )]);
        test_host::set_workspace(ACTIVE_THREADS_PATH, &serialize_active_slack_threads(&threads));

        handle_slack_event(
            sample_thread_message_event("1710000000.000001"),
            Some("T1".to_string()),
            None,
        );

        let emitted = test_host::take_emitted_messages();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].user_id, "U123");
        assert_eq!(emitted[0].content, "follow up");
        assert_eq!(
            emitted[0].thread_id.as_deref(),
            Some("1710000000.000001")
        );
    }

    #[test]
    fn test_handle_slack_event_skips_unknown_active_thread() {
        test_host::reset();
        test_host::set_now_millis(1_710_000_000_000_u64);

        handle_slack_event(
            sample_thread_message_event("1710000000.000001"),
            Some("T1".to_string()),
            None,
        );

        assert!(test_host::take_emitted_messages().is_empty());
    }

    #[test]
    fn test_active_thread_key_scopes_by_channel_and_thread() {
        assert_eq!(
            active_thread_key("C123", "1742486400.000100"),
            "C123/1742486400.000100"
        );
    }

    #[test]
    fn test_prune_active_threads_removes_expired_entries() {
        let now_millis = ACTIVE_THREAD_TTL_MS + 1_000;
        let mut active_threads = ActiveThreads::from([
            (
                "C1/expired".to_string(),
                now_millis - ACTIVE_THREAD_TTL_MS - 1,
            ),
            ("C1/fresh".to_string(), now_millis - ACTIVE_THREAD_TTL_MS),
        ]);

        let changed = prune_active_threads(&mut active_threads, now_millis);

        assert!(changed);
        assert!(!active_threads.contains_key("C1/expired"));
        assert!(active_threads.contains_key("C1/fresh"));
    }

    #[test]
    fn test_prune_active_threads_trims_oldest_entries_when_over_limit() {
        let now_millis = ACTIVE_THREAD_TTL_MS + 1_000;
        let mut active_threads = ActiveThreads::new();

        for i in 0..=ACTIVE_THREAD_MAX_ENTRIES {
            active_threads.insert(format!("C1/{i}"), now_millis + i as u64);
        }

        let changed = prune_active_threads(
            &mut active_threads,
            now_millis + ACTIVE_THREAD_MAX_ENTRIES as u64,
        );

        assert!(changed);
        assert_eq!(active_threads.len(), ACTIVE_THREAD_MAX_ENTRIES);
        assert!(!active_threads.contains_key("C1/0"));
        assert!(active_threads.contains_key(&format!("C1/{ACTIVE_THREAD_MAX_ENTRIES}")));
    }

    #[test]
    fn test_is_thread_marker_fresh_respects_ttl_boundary() {
        let now_millis = ACTIVE_THREAD_TTL_MS + 1_000;
        assert!(is_thread_marker_fresh(
            now_millis - ACTIVE_THREAD_TTL_MS,
            now_millis
        ));
        assert!(!is_thread_marker_fresh(
            now_millis - ACTIVE_THREAD_TTL_MS - 1,
            now_millis
        ));
    }

    #[test]
    fn test_resolve_broadcast_target_strips_hash() {
        assert_eq!(resolve_broadcast_target("#general"), "general");
        assert_eq!(resolve_broadcast_target("#staging-eli5"), "staging-eli5");
    }

    #[test]
    fn test_resolve_broadcast_target_preserves_ids() {
        assert_eq!(resolve_broadcast_target("C0123ABC"), "C0123ABC");
        assert_eq!(resolve_broadcast_target("U0123ABC"), "U0123ABC");
    }

    #[test]
    fn test_resolve_broadcast_target_empty_input() {
        assert_eq!(resolve_broadcast_target(""), "");
        assert_eq!(resolve_broadcast_target("#"), "");
    }

    #[test]
    fn test_build_broadcast_payload_without_thread() {
        let payload = build_broadcast_payload("C0123", "hello world", None);
        assert_eq!(payload["channel"], "C0123");
        assert_eq!(payload["text"], "hello world");
        assert!(payload.get("thread_ts").is_none());
    }

    #[test]
    fn test_build_broadcast_payload_with_thread() {
        let payload = build_broadcast_payload("C0123", "threaded reply", Some("1742486400.000100"));
        assert_eq!(payload["channel"], "C0123");
        assert_eq!(payload["text"], "threaded reply");
        assert_eq!(payload["thread_ts"], "1742486400.000100");
    }

    #[test]
    fn test_looks_like_slack_id_valid() {
        assert!(looks_like_slack_id("C0123ABC"));
        assert!(looks_like_slack_id("U0123ABC"));
        assert!(looks_like_slack_id("D0123ABC"));
        assert!(looks_like_slack_id("G0123ABC"));
        assert!(looks_like_slack_id("W0123ABC"));
    }

    #[test]
    fn test_looks_like_slack_id_invalid() {
        assert!(!looks_like_slack_id("general"));
        assert!(!looks_like_slack_id("staging-eli5"));
        assert!(!looks_like_slack_id(""));
        assert!(!looks_like_slack_id("C")); // too short, no second char
        assert!(!looks_like_slack_id("c0123")); // lowercase
    }

    #[test]
    fn test_resolve_broadcast_target_rejects_names_via_id_check() {
        // After stripping '#', channel names fail the ID check
        let target = resolve_broadcast_target("#general");
        assert!(!looks_like_slack_id(target));

        let target = resolve_broadcast_target("random-channel");
        assert!(!looks_like_slack_id(target));
    }

    #[test]
    fn test_resolve_broadcast_target_accepts_prefixed_ids() {
        // IDs with '#' prefix are accepted after stripping
        let target = resolve_broadcast_target("#C0123ABC");
        assert!(looks_like_slack_id(target));
        assert_eq!(target, "C0123ABC");
    }
}
