//! WeCom channel for IronClaw.
//!
//! Current shape:
//! - bot websocket is the session path for inbound text/events and replies
//! - bot upload/send commands handle generated media without app credentials
//! - HTTP callback and proactive REST send support are intentionally out of scope

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
use aes::Aes256;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use cbc::Decryptor;
use md5::Md5;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use sha1::{Digest, Sha1};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashMap;

use exports::near::agent::channel::{
    AgentResponse, Attachment, ChannelConfig, Guest, IncomingHttpRequest, OutgoingHttpResponse,
    PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage, InboundAttachment};

const CHANNEL_NAME: &str = "wecom";
const OWNER_ID_PATH: &str = "owner_id";
const DM_POLICY_PATH: &str = "dm_policy";
const ALLOW_FROM_PATH: &str = "allow_from";
const RECENT_MSG_IDS_PATH: &str = "recent_msg_ids";
const WEBSOCKET_EVENT_QUEUE_PATH: &str = "state/gateway_event_queue_processing";
const WEBSOCKET_MEDIA_STATE_PATH: &str = "state/websocket_media_sends";
const WEBSOCKET_MEDIA_CHUNK_BLOBS_PREFIX: &str = "state/websocket_media_chunks";
const PENDING_INBOUND_PATH: &str = "state/pending_inbound_bundles";
const INBOUND_MERGE_WINDOW_MS_PATH: &str = "state/inbound_merge_window_ms";
const PENDING_ATTACHMENT_BLOBS_PREFIX: &str = "state/pending_attachment_blobs";

const WECOM_WS_REPLY_CMD: &str = "aibot_respond_msg";
const WECOM_WS_WELCOME_CMD: &str = "aibot_respond_welcome_msg";
const WECOM_WS_SEND_MSG_CMD: &str = "aibot_send_msg";
const WECOM_WS_UPLOAD_MEDIA_INIT_CMD: &str = "aibot_upload_media_init";
const WECOM_WS_UPLOAD_MEDIA_CHUNK_CMD: &str = "aibot_upload_media_chunk";
const WECOM_WS_UPLOAD_MEDIA_FINISH_CMD: &str = "aibot_upload_media_finish";

const STREAM_CHUNK_LIMIT_BYTES: usize = 20_000;
const STATUS_MESSAGE_MAX_CHARS: usize = 1200;
const WEBSOCKET_MEDIA_CHUNK_SIZE: usize = 512 * 1024;
const MAX_WEBSOCKET_MEDIA_CHUNKS: usize = 100;
const MAX_WEBSOCKET_MEDIA_ATTACHMENTS_PER_RESPONSE: usize = 4;
const MAX_WEBSOCKET_MEDIA_TOTAL_BYTES_PER_RESPONSE: usize = 20 * 1024 * 1024;
const MAX_WEBSOCKET_MEDIA_BATCH_ERRORS: usize = 10;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const MAX_WEBSOCKET_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_WEBSOCKET_VOICE_BYTES: usize = 2 * 1024 * 1024;
const MAX_WEBSOCKET_VIDEO_BYTES: usize = 10 * 1024 * 1024;
const MAX_RECENT_MSG_IDS: usize = 256;
const MAX_RECENT_MSG_ID_AGE_MS: u64 = 24 * 60 * 60 * 1000;
const WEBSOCKET_MEDIA_SEND_TTL_MS: u64 = 10 * 60 * 1000;
const DEFAULT_INBOUND_MERGE_WINDOW_MS: u64 = 5_000;
const MAX_INBOUND_MERGE_WINDOW_MS: u64 = 60_000;
const WECOM_POLL_INTERVAL_MS: u32 = 1_000;

type Aes256CbcDec = Decryptor<Aes256>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DmPolicy {
    #[default]
    Pairing,
    Open,
    Allowlist,
}

impl DmPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pairing => "pairing",
            Self::Open => "open",
            Self::Allowlist => "allowlist",
        }
    }

    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "open" => Self::Open,
            "allowlist" => Self::Allowlist,
            _ => Self::Pairing,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WecomConfig {
    owner_id: Option<String>,
    dm_policy: Option<DmPolicy>,
    allow_from: Option<Vec<String>>,
    inbound_merge_window_ms: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WecomMessageMetadata {
    to_user: String,
    target: Option<String>,
    chat_id: Option<String>,
    chat_type: Option<String>,
    source_msg_id: Option<String>,
    ws_req_id: Option<String>,
    ws_chat_id: Option<String>,
    ws_chat_type: Option<String>,
    ws_reply_cmd: Option<String>,
}

#[derive(Debug, Clone)]
struct PairingReplyRoute {
    req_id: String,
    reply_cmd: String,
    chat_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WecomWsFrame<T> {
    headers: WecomWsHeaders,
    body: T,
}

#[derive(Debug, Deserialize)]
struct WecomWsHeaders {
    req_id: String,
}

#[derive(Debug, Deserialize)]
struct WecomWsAckFrame {
    headers: WecomWsHeaders,
    errcode: i64,
    #[serde(default)]
    errmsg: String,
    #[serde(default)]
    body: JsonValue,
}

#[derive(Debug, Deserialize)]
struct WecomWsSender {
    userid: String,
}

#[derive(Debug, Deserialize)]
struct WecomWsTextContent {
    content: String,
}

#[derive(Debug, Deserialize)]
struct WecomWsBinaryContent {
    url: String,
    #[serde(default)]
    #[cfg_attr(test, allow(dead_code))]
    aeskey: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WecomWsMixedItem {
    #[serde(default, alias = "msgtype", alias = "type", alias = "itemtype")]
    item_type: Option<String>,
    #[serde(default)]
    text: Option<WecomWsTextContent>,
    #[serde(default)]
    image: Option<WecomWsBinaryContent>,
    #[serde(default)]
    file: Option<WecomWsBinaryContent>,
    #[serde(default)]
    video: Option<WecomWsBinaryContent>,
}

#[derive(Debug, Deserialize)]
struct WecomWsMixedContent {
    #[serde(default, alias = "msgItem", alias = "items")]
    msg_item: Vec<WecomWsMixedItem>,
}

#[derive(Debug, Deserialize)]
struct WecomWsQuoteContent {
    #[serde(default, alias = "msgtype", alias = "type")]
    msg_type: Option<String>,
    #[serde(default)]
    text: Option<WecomWsTextContent>,
    #[serde(default)]
    voice: Option<WecomWsTextContent>,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WecomWsMessageBody {
    msgid: String,
    #[serde(default)]
    chatid: Option<String>,
    #[serde(default)]
    chattype: Option<String>,
    from: WecomWsSender,
    msgtype: String,
    #[serde(default)]
    text: Option<WecomWsTextContent>,
    #[serde(default)]
    voice: Option<WecomWsTextContent>,
    #[serde(default)]
    image: Option<WecomWsBinaryContent>,
    #[serde(default)]
    file: Option<WecomWsBinaryContent>,
    #[serde(default)]
    video: Option<WecomWsBinaryContent>,
    #[serde(default)]
    mixed: Option<WecomWsMixedContent>,
    #[serde(default)]
    quote: Option<WecomWsQuoteContent>,
}

#[derive(Debug, Deserialize)]
struct WecomWsEventBody {
    msgid: String,
    #[serde(default)]
    chatid: Option<String>,
    #[serde(default)]
    chattype: Option<String>,
    from: WecomWsSender,
    event: WecomWsEvent,
}

#[derive(Debug, Deserialize)]
struct WecomWsEvent {
    eventtype: String,
    #[serde(flatten)]
    extra: JsonMap<String, JsonValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutboundMediaKind {
    Image,
    Voice,
    Video,
    File,
}

impl OutboundMediaKind {
    fn as_api_type(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Voice => "voice",
            Self::Video => "video",
            Self::File => "file",
        }
    }

    fn websocket_max_bytes(self) -> usize {
        match self {
            Self::Image => MAX_WEBSOCKET_IMAGE_BYTES,
            Self::Voice => MAX_WEBSOCKET_VOICE_BYTES,
            Self::Video => MAX_WEBSOCKET_VIDEO_BYTES,
            Self::File => MAX_ATTACHMENT_BYTES,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct PendingWebsocketMediaState {
    #[serde(default)]
    sends: Vec<PendingWebsocketMediaSend>,
    #[serde(default)]
    batches: Vec<PendingWebsocketMediaBatch>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingWebsocketMediaBatch {
    id: String,
    chat_id: String,
    #[serde(default)]
    created_at_ms: u64,
    #[serde(default)]
    response_req_id: String,
    #[serde(default)]
    response_cmd: String,
    final_text: String,
    remaining_media: usize,
    sent_media: usize,
    failed_media: usize,
    #[serde(default)]
    errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingWebsocketMediaSend {
    id: String,
    batch_id: String,
    chat_id: String,
    #[serde(default)]
    created_at_ms: u64,
    media_type: String,
    filename: String,
    #[serde(default)]
    md5_hex: String,
    #[serde(default)]
    chunk_blob_paths: Vec<String>,
    total_size: usize,
    total_chunks: usize,
    next_chunk_index: usize,
    init_req_id: String,
    #[serde(default)]
    chunk_req_id: Option<String>,
    #[serde(default)]
    finish_req_id: Option<String>,
    #[serde(default)]
    send_req_id: Option<String>,
    #[serde(default)]
    upload_id: Option<String>,
    #[serde(default)]
    media_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RecentMessageIdEntry {
    id: String,
    seen_at_ms: u64,
}

struct PendingWebsocketMediaOutbound {
    send_id: String,
    payload: String,
    complete_after_send: bool,
}

enum PendingWebsocketMediaAdvance {
    Send(PendingWebsocketMediaOutbound, PendingWebsocketMediaSend),
    Complete,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct StoredInboundAttachment {
    id: String,
    mime_type: String,
    filename: Option<String>,
    size_bytes: Option<u64>,
    extracted_text: Option<String>,
}

impl From<InboundAttachment> for StoredInboundAttachment {
    fn from(value: InboundAttachment) -> Self {
        Self {
            id: value.id,
            mime_type: value.mime_type,
            filename: value.filename,
            size_bytes: value.size_bytes,
            extracted_text: value.extracted_text,
        }
    }
}

impl From<StoredInboundAttachment> for InboundAttachment {
    fn from(value: StoredInboundAttachment) -> Self {
        Self {
            id: value.id,
            mime_type: value.mime_type,
            filename: value.filename,
            size_bytes: value.size_bytes,
            source_url: None,
            storage_key: None,
            extracted_text: value.extracted_text,
            extras_json: "{}".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct PendingInboundBundle {
    user_id: String,
    user_name: Option<String>,
    thread_id: String,
    metadata_json: String,
    content: String,
    attachments: Vec<StoredInboundAttachment>,
    flush_at_ms: u64,
}

struct WebsocketMediaStartResult {
    started: usize,
    errors: Vec<String>,
}

fn inbound_merge_window_ms() -> u64 {
    channel_host::workspace_read(INBOUND_MERGE_WINDOW_MS_PATH)
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_INBOUND_MERGE_WINDOW_MS)
        .min(MAX_INBOUND_MERGE_WINDOW_MS)
}

fn merge_inbound_text(existing: &str, incoming: &str) -> String {
    let existing = existing.trim();
    let incoming = incoming.trim();
    match (existing.is_empty(), incoming.is_empty()) {
        (true, true) => String::new(),
        (true, false) => incoming.to_string(),
        (false, true) => existing.to_string(),
        (false, false) => format!("{existing}\n\n{incoming}"),
    }
}

fn next_inbound_flush_deadline(now_ms: u64, merge_window_ms: u64) -> u64 {
    now_ms.saturating_add(merge_window_ms)
}

#[cfg(test)]
thread_local! {
    static TEST_WORKSPACE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static TEST_WEBSOCKET_OUTBOUND: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static TEST_WEBSOCKET_SEND_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

#[cfg(test)]
fn test_reset_websocket_state() {
    TEST_WORKSPACE.with(|workspace| workspace.borrow_mut().clear());
    TEST_WEBSOCKET_OUTBOUND.with(|outbound| outbound.borrow_mut().clear());
    TEST_WEBSOCKET_SEND_ERROR.with(|error| *error.borrow_mut() = None);
}

fn read_wecom_workspace(path: &str) -> Option<String> {
    #[cfg(test)]
    {
        TEST_WORKSPACE.with(|workspace| workspace.borrow().get(path).cloned())
    }
    #[cfg(not(test))]
    {
        channel_host::workspace_read(path)
    }
}

fn write_wecom_workspace(path: &str, content: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        TEST_WORKSPACE.with(|workspace| {
            if content.is_empty() {
                workspace.borrow_mut().remove(path);
            } else {
                workspace
                    .borrow_mut()
                    .insert(path.to_string(), content.to_string());
            }
        });
        Ok(())
    }
    #[cfg(not(test))]
    {
        channel_host::workspace_write(path, content)
    }
}

fn send_websocket_text(payload: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        if let Some(error) = TEST_WEBSOCKET_SEND_ERROR.with(|error| error.borrow_mut().take()) {
            return Err(error);
        }
        TEST_WEBSOCKET_OUTBOUND.with(|outbound| outbound.borrow_mut().push(payload.to_string()));
        Ok(())
    }
    #[cfg(not(test))]
    {
        channel_host::websocket_send_text(payload)
    }
}

fn log_wecom(level: channel_host::LogLevel, message: &str) {
    #[cfg(test)]
    {
        let _ = (level, message);
    }
    #[cfg(not(test))]
    {
        channel_host::log(level, message);
    }
}

fn pending_attachment_blob_path(attachment_id: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(attachment_id.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("{PENDING_ATTACHMENT_BLOBS_PREFIX}/{digest}.b64")
}

fn persist_pending_attachment_blob(attachment_id: &str, data: &[u8]) -> Result<(), String> {
    if data.is_empty() {
        return Ok(());
    }
    let encoded = BASE64_STANDARD.encode(data);
    channel_host::workspace_write(&pending_attachment_blob_path(attachment_id), &encoded)
        .map_err(|e| format!("Failed to persist pending WeCom attachment blob: {e}"))
}

fn load_pending_attachment_blob(attachment_id: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(raw) = channel_host::workspace_read(&pending_attachment_blob_path(attachment_id))
    else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    BASE64_STANDARD
        .decode(trimmed)
        .map(Some)
        .map_err(|e| format!("Failed to decode pending WeCom attachment blob: {e}"))
}

fn clear_pending_attachment_blob(attachment_id: &str) {
    let _ = channel_host::workspace_write(&pending_attachment_blob_path(attachment_id), "");
}

fn load_pending_inbound_bundles() -> HashMap<String, PendingInboundBundle> {
    let Some(raw) = channel_host::workspace_read(PENDING_INBOUND_PATH) else {
        return HashMap::new();
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return HashMap::new();
    }
    match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(error) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to parse WeCom pending inbound bundles: {error}"),
            );
            HashMap::new()
        }
    }
}

fn persist_pending_inbound_bundles(
    pending: &HashMap<String, PendingInboundBundle>,
) -> Result<(), String> {
    let serialized = serde_json::to_string(pending)
        .map_err(|e| format!("Failed to serialize WeCom pending inbound bundles: {e}"))?;
    channel_host::workspace_write(PENDING_INBOUND_PATH, &serialized)
        .map_err(|e| format!("Failed to persist WeCom pending inbound bundles: {e}"))
}

fn take_due_pending_inbound_bundles(
    pending: &mut HashMap<String, PendingInboundBundle>,
    now_ms: u64,
) -> Vec<PendingInboundBundle> {
    let due_keys: Vec<String> = pending
        .iter()
        .filter_map(|(key, bundle)| (bundle.flush_at_ms <= now_ms).then_some(key.clone()))
        .collect();
    due_keys
        .into_iter()
        .filter_map(|key| pending.remove(&key))
        .collect()
}

fn process_pending_inbound_bundle(
    pending: &mut HashMap<String, PendingInboundBundle>,
    key: &str,
    mut bundle: PendingInboundBundle,
    now_ms: u64,
    merge_window_ms: u64,
) -> Vec<PendingInboundBundle> {
    let bundle_has_text = !bundle.content.trim().is_empty();
    let bundle_has_attachments = !bundle.attachments.is_empty();

    if let Some(mut existing) = pending.remove(key) {
        existing.content = merge_inbound_text(&existing.content, &bundle.content);
        existing.attachments.extend(bundle.attachments);
        existing.user_id = bundle.user_id;
        existing.user_name = bundle.user_name;
        existing.thread_id = bundle.thread_id;
        existing.metadata_json = bundle.metadata_json;

        let has_text = !existing.content.trim().is_empty();
        let has_attachments = !existing.attachments.is_empty();
        if has_text || merge_window_ms == 0 {
            return vec![existing];
        }
        if has_attachments {
            existing.flush_at_ms = next_inbound_flush_deadline(now_ms, merge_window_ms);
            pending.insert(key.to_string(), existing);
        }
        return Vec::new();
    }

    if bundle_has_attachments && !bundle_has_text && merge_window_ms > 0 {
        bundle.flush_at_ms = next_inbound_flush_deadline(now_ms, merge_window_ms);
        pending.insert(key.to_string(), bundle);
        Vec::new()
    } else {
        vec![bundle]
    }
}

fn emit_pending_inbound_bundle(bundle: PendingInboundBundle) {
    let mut attachments: Vec<InboundAttachment> =
        bundle.attachments.into_iter().map(Into::into).collect();
    let attachment_ids: Vec<String> = attachments.iter().map(|att| att.id.clone()).collect();
    let mut hydrated = Vec::with_capacity(attachments.len());
    for mut attachment in attachments {
        if let Err(error) = rehydrate_pending_inbound_attachment_data(&mut attachment) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!(
                    "Dropping WeCom attachment '{}' because inline data is unavailable: {}",
                    attachment.id, error
                ),
            );
            continue;
        }
        hydrated.push(attachment);
    }
    attachments = hydrated;

    if bundle.content.trim().is_empty() && attachments.is_empty() {
        channel_host::log(
            channel_host::LogLevel::Warn,
            "Skipping buffered WeCom message because both content and attachments are empty after rehydrate",
        );
        for attachment_id in attachment_ids {
            clear_pending_attachment_blob(&attachment_id);
        }
        return;
    }

    channel_host::emit_message(&EmittedMessage {
        user_id: bundle.user_id,
        user_name: bundle.user_name,
        content: bundle.content,
        thread_id: Some(bundle.thread_id),
        metadata_json: bundle.metadata_json,
        attachments,
    });
    for attachment_id in attachment_ids {
        clear_pending_attachment_blob(&attachment_id);
    }
}

fn emit_or_buffer_incoming_user_message(
    user_id: String,
    user_name: Option<String>,
    content: String,
    thread_id: String,
    metadata_json: String,
    attachments: Vec<InboundAttachment>,
) {
    let now_ms = channel_host::now_millis();
    let mut pending = load_pending_inbound_bundles();
    let mut emitted = take_due_pending_inbound_bundles(&mut pending, now_ms);
    let key = thread_id.clone();
    emitted.extend(process_pending_inbound_bundle(
        &mut pending,
        &key,
        PendingInboundBundle {
            user_id,
            user_name,
            content,
            thread_id,
            metadata_json,
            attachments: attachments.into_iter().map(Into::into).collect(),
            flush_at_ms: 0,
        },
        now_ms,
        inbound_merge_window_ms(),
    ));

    if let Err(error) = persist_pending_inbound_bundles(&pending) {
        channel_host::log(channel_host::LogLevel::Warn, &error);
    }

    for bundle in emitted {
        emit_pending_inbound_bundle(bundle);
    }
}

fn flush_due_pending_inbound_bundles() {
    let now_ms = channel_host::now_millis();
    let mut pending = load_pending_inbound_bundles();
    let due = take_due_pending_inbound_bundles(&mut pending, now_ms);
    if due.is_empty() {
        return;
    }

    if let Err(error) = persist_pending_inbound_bundles(&pending) {
        channel_host::log(channel_host::LogLevel::Warn, &error);
    }

    for bundle in due {
        emit_pending_inbound_bundle(bundle);
    }
}

fn text_response(status: u16, body: &str) -> OutgoingHttpResponse {
    OutgoingHttpResponse {
        status,
        headers_json: serde_json::json!({
            "Content-Type": "text/plain; charset=utf-8",
        })
        .to_string(),
        body: body.as_bytes().to_vec(),
    }
}

fn load_allow_from() -> Vec<String> {
    let mut allowed = Vec::new();
    // silent-ok: missing static allowlist means "no statically allowed senders";
    // pairing_read_allow_from below still provides DB-backed approvals.
    if let Some(raw) = channel_host::workspace_read(ALLOW_FROM_PATH) {
        match serde_json::from_str::<Vec<String>>(&raw) {
            Ok(values) => allowed = values,
            Err(error) => channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to parse WeCom allow_from list: {error}"),
            ),
        }
    }
    if let Ok(stored) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        allowed.extend(stored);
    }
    allowed
}

fn load_dm_policy() -> DmPolicy {
    channel_host::workspace_read(DM_POLICY_PATH)
        .map(|value| DmPolicy::parse(&value))
        .unwrap_or_default()
}

struct SenderAccessSnapshot {
    dm_policy: DmPolicy,
    allow_from: Vec<String>,
}

impl SenderAccessSnapshot {
    fn load() -> Self {
        Self {
            dm_policy: load_dm_policy(),
            allow_from: load_allow_from(),
        }
    }
}

fn send_pairing_reply(route: &PairingReplyRoute, code: &str) -> Result<(), String> {
    let content = pairing_reply_text(route, code);
    send_websocket_stream_reply(&route.req_id, &route.reply_cmd, &content)
}

fn pairing_route_is_group(route: &PairingReplyRoute) -> bool {
    chat_type_is_group(route.chat_type.as_deref())
}

fn chat_type_is_group(chat_type: Option<&str>) -> bool {
    normalize_chat_type(chat_type).as_deref() == Some("group")
}

fn chat_type_is_private(chat_type: Option<&str>) -> bool {
    normalize_chat_type(chat_type).as_deref() == Some("private")
}

fn dm_policy_allows_sender_without_allowlist(dm_policy: DmPolicy, chat_type: Option<&str>) -> bool {
    dm_policy == DmPolicy::Open && chat_type_is_private(chat_type)
}

fn should_send_pairing_reply(route: &PairingReplyRoute, created: bool) -> bool {
    if pairing_route_is_group(route) {
        // Group chats only receive the generic "please DM" notice once per
        // pairing request to avoid leaking or spamming operational details.
        created
    } else {
        // In private chats, always send the code so users can still get it
        // even if the request was originally created in a group context.
        true
    }
}

fn pairing_reply_text(route: &PairingReplyRoute, code: &str) -> String {
    if pairing_route_is_group(route) {
        "This WeCom channel requires approval before chatting. For security, please DM the bot to get your pairing code.".to_string()
    } else {
        format!(
            "This WeCom channel requires approval before chatting. Pairing code: {}",
            code
        )
    }
}

fn is_sender_allowed(
    sender_id: &str,
    chat_type: Option<&str>,
    access: &SenderAccessSnapshot,
    pairing_reply: Option<PairingReplyRoute>,
) -> Result<bool, String> {
    let owner_id = channel_host::workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());
    if owner_id.as_deref() == Some(sender_id) {
        return Ok(true);
    }

    if dm_policy_allows_sender_without_allowlist(access.dm_policy, chat_type) {
        return Ok(true);
    }

    if access
        .allow_from
        .iter()
        .any(|entry| entry == "*" || entry == sender_id)
    {
        return Ok(true);
    }

    if access.dm_policy == DmPolicy::Pairing {
        let meta = serde_json::json!({ "user_id": sender_id }).to_string();
        let result = channel_host::pairing_upsert_request(CHANNEL_NAME, sender_id, &meta)?;
        if let Some(reply_route) = pairing_reply {
            if should_send_pairing_reply(&reply_route, result.created) {
                let _ = send_pairing_reply(&reply_route, &result.code);
            }
        }
    }

    Ok(false)
}

fn base_mime_type(mime_type: &str) -> &str {
    mime_type.split(';').next().unwrap_or("").trim()
}

fn lowercase_filename_extension(filename: &str) -> Option<String> {
    let (_, ext) = filename.rsplit_once('.')?;
    let ext = ext.trim();
    if ext.is_empty() {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

fn preferred_outbound_media_kind(att: &Attachment) -> OutboundMediaKind {
    let mime = base_mime_type(&att.mime_type).to_ascii_lowercase();
    let ext = lowercase_filename_extension(&att.filename);

    if matches!(mime.as_str(), "image/jpeg" | "image/png")
        || matches!(ext.as_deref(), Some("jpg" | "jpeg" | "png"))
    {
        OutboundMediaKind::Image
    } else if matches!(mime.as_str(), "audio/amr" | "audio/x-amr") || ext.as_deref() == Some("amr")
    {
        OutboundMediaKind::Voice
    } else if mime == "video/mp4" || ext.as_deref() == Some("mp4") {
        OutboundMediaKind::Video
    } else {
        OutboundMediaKind::File
    }
}

fn rehydrate_pending_inbound_attachment_data(
    attachment: &mut InboundAttachment,
) -> Result<(), String> {
    if let Some(data) = load_pending_attachment_blob(&attachment.id)? {
        channel_host::store_attachment_data(&attachment.id, &data)
            .map_err(|e| format!("Failed to restore pending WeCom attachment data: {e}"))?;
        if attachment.size_bytes.is_none() {
            attachment.size_bytes = Some(data.len() as u64);
        }
        return Ok(());
    }

    Err("Pending WeCom websocket attachment blob is missing; cannot restore inline data".to_string())
}

fn extract_filename_from_content_disposition(header: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let idx = lower.find("filename=")?;
    let raw = header[idx + "filename=".len()..].trim();
    Some(
        raw.trim_matches('"')
            .trim_matches('\'')
            .split(';')
            .next()
            .unwrap_or(raw)
            .to_string(),
    )
}

fn websocket_default_mime_and_extension(msg_type: &str) -> (&'static str, &'static str) {
    match msg_type {
        "image" => ("image/jpeg", "jpg"),
        "video" => ("video/mp4", "mp4"),
        _ => ("application/octet-stream", "bin"),
    }
}

#[cfg_attr(test, allow(dead_code))]
fn header_value_case_insensitive<'a>(
    headers: &'a JsonMap<String, JsonValue>,
    name: &str,
) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_str())
}

fn decode_base64_with_padding(value: &str) -> Result<Vec<u8>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Base64 value is empty".to_string());
    }

    let mut padded = trimmed.to_string();
    let missing_padding = padded.len() % 4;
    if missing_padding != 0 {
        padded.push_str(&"=".repeat(4 - missing_padding));
    }

    BASE64_STANDARD
        .decode(padded)
        .map_err(|e| format!("Failed to decode base64 value: {e}"))
}

fn remove_wecom_pkcs7_padding(data: &[u8]) -> Result<&[u8], String> {
    let Some(last) = data.last().copied() else {
        return Err("Decrypted payload is empty".to_string());
    };
    let pad_len = last as usize;
    if pad_len == 0 || pad_len > 32 || pad_len > data.len() {
        return Err(format!("Invalid WeCom PKCS#7 padding length: {pad_len}"));
    }
    if !data[data.len() - pad_len..]
        .iter()
        .all(|byte| *byte as usize == pad_len)
    {
        return Err("Invalid WeCom PKCS#7 padding bytes".to_string());
    }
    Ok(&data[..data.len() - pad_len])
}

fn decrypt_websocket_media_payload(ciphertext: &[u8], aes_key: &str) -> Result<Vec<u8>, String> {
    if ciphertext.is_empty() {
        return Err("Encrypted websocket media payload is empty".to_string());
    }
    if !ciphertext.len().is_multiple_of(16) {
        return Err(format!(
            "Encrypted websocket media length {} is not a multiple of AES block size",
            ciphertext.len()
        ));
    }

    let key = decode_base64_with_padding(aes_key)?;
    if key.len() != 32 {
        return Err(format!(
            "Unexpected websocket media AES key length: {}",
            key.len()
        ));
    }

    let iv = &key[..16];
    let mut buf = ciphertext.to_vec();
    let decrypted = Aes256CbcDec::new_from_slices(&key, iv)
        .map_err(|e| format!("Failed to initialize websocket media decryptor: {e}"))?
        .decrypt_padded_mut::<NoPadding>(&mut buf)
        .map_err(|e| format!("Failed to decrypt websocket media payload: {e}"))?;
    let unpadded = remove_wecom_pkcs7_padding(decrypted)?;
    Ok(unpadded.to_vec())
}

#[cfg_attr(test, allow(dead_code))]
fn hydrate_websocket_binary_attachment_data(
    attachment: &mut InboundAttachment,
    msg_type: &str,
    aes_key: Option<&str>,
) -> Result<(), String> {
    let source_url = attachment
        .source_url
        .as_deref()
        .ok_or_else(|| "Websocket attachment source_url is missing".to_string())?;

    let response = channel_host::http_request("GET", source_url, "{}", None, Some(30_000))
        .map_err(|e| format!("Failed to download websocket attachment: {e}"))?;
    if response.status != 200 {
        return Err(format!(
            "Websocket attachment download returned {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    if response.body.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "Websocket attachment exceeds {} bytes",
            MAX_ATTACHMENT_BYTES
        ));
    }

    let headers: JsonMap<String, JsonValue> =
        serde_json::from_str(&response.headers_json).unwrap_or_default();
    let body = response.body;
    let data = match aes_key {
        Some(raw_key) if raw_key.trim().is_empty() => {
            return Err("Websocket attachment aeskey is empty".to_string());
        }
        Some(raw_key) => {
            decrypt_websocket_media_payload(&body, raw_key.trim()).map_err(|decrypt_error| {
                format!("Failed to decrypt websocket attachment payload: {decrypt_error}")
            })?
        }
        None => body,
    };
    if data.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "Decrypted websocket attachment exceeds {} bytes",
            MAX_ATTACHMENT_BYTES
        ));
    }

    channel_host::store_attachment_data(&attachment.id, &data)
        .map_err(|e| format!("Failed to store websocket attachment data: {e}"))?;
    persist_pending_attachment_blob(&attachment.id, &data)?;

    let (fallback_mime, _) = websocket_default_mime_and_extension(msg_type);
    let mut mime_type = header_value_case_insensitive(&headers, "content-type")
        .map(base_mime_type)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| fallback_mime.to_string());
    let mime_lower = mime_type.to_ascii_lowercase();
    if msg_type == "image" && !mime_lower.starts_with("image/") {
        mime_type = "image/jpeg".to_string();
    } else if msg_type == "video" && !mime_lower.starts_with("video/") {
        mime_type = "video/mp4".to_string();
    }

    if let Some(filename) = header_value_case_insensitive(&headers, "content-disposition")
        .and_then(extract_filename_from_content_disposition)
        .filter(|value| !value.trim().is_empty())
    {
        attachment.filename = Some(filename);
    }
    attachment.mime_type = mime_type;
    attachment.size_bytes = Some(data.len() as u64);

    Ok(())
}

fn chunk_text(text: &str, limit_bytes: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in trimmed.chars() {
        let ch_len = ch.len_utf8();
        if !current.is_empty() && current.len() + ch_len > limit_bytes {
            chunks.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn websocket_stream_id(req_id: &str) -> String {
    format!("stream-{req_id}")
}

fn build_websocket_stream_reply_payload(
    req_id: &str,
    reply_cmd: &str,
    content: &str,
    finish: bool,
) -> Result<String, String> {
    let payload = serde_json::json!({
        "cmd": reply_cmd,
        "headers": {
            "req_id": req_id,
        },
        "body": {
            "msgtype": "stream",
            "stream": {
                "id": websocket_stream_id(req_id),
                "content": content,
                "finish": finish,
            },
        }
    });
    serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize WeCom websocket reply: {e}"))
}

fn build_websocket_text_stream_reply_payloads(
    req_id: &str,
    reply_cmd: &str,
    content: &str,
    finish_last_chunk: bool,
) -> Result<Vec<String>, String> {
    let mut payloads = Vec::new();
    let chunks = chunk_text(content, STREAM_CHUNK_LIMIT_BYTES);
    for (index, chunk) in chunks.iter().enumerate() {
        let finish = finish_last_chunk && index + 1 == chunks.len();
        let payload = build_websocket_stream_reply_payload(req_id, reply_cmd, chunk, finish)?;
        payloads.push(payload);
    }
    Ok(payloads)
}

fn websocket_media_md5_hex(data: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn send_websocket_response(req_id: &str, reply_cmd: &str, content: &str) -> Result<(), String> {
    for payload in build_websocket_text_stream_reply_payloads(req_id, reply_cmd, content, true)? {
        send_websocket_text(&payload)
            .map_err(|e| format!("Failed to send WeCom websocket reply: {e}"))?;
    }
    Ok(())
}

fn send_websocket_stream_reply(req_id: &str, reply_cmd: &str, content: &str) -> Result<(), String> {
    send_websocket_response(req_id, reply_cmd, content)
}

fn truncate_status_message(message: &str, max_chars: usize) -> String {
    let mut chars = message.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    if chars.clone().count() < max_chars {
        return message.to_string();
    }

    let keep = max_chars.saturating_sub(3).max(1);
    let mut out = String::new();
    out.push(first);
    out.extend(chars.take(keep.saturating_sub(1)));
    out.push_str("...");
    out
}

fn status_message_for_user(message: &str) -> Option<String> {
    let message = message.trim();
    if message.is_empty() {
        None
    } else {
        Some(truncate_status_message(message, STATUS_MESSAGE_MAX_CHARS))
    }
}

fn classify_status_update(update: &StatusUpdate) -> Option<String> {
    match update.status {
        StatusType::Thinking => None,
        StatusType::Done => None,
        StatusType::Interrupted => status_message_for_user(&update.message)
            .or_else(|| Some("Request interrupted. Please try again.".to_string())),
        // Tool-level telemetry is too noisy in chat UX.
        StatusType::ToolStarted | StatusType::ToolCompleted | StatusType::ToolResult => None,
        StatusType::Status => {
            let message = update.message.trim();
            if message.eq_ignore_ascii_case("Done")
                || message.eq_ignore_ascii_case("Interrupted")
                || message.eq_ignore_ascii_case("Awaiting approval")
                || message.eq_ignore_ascii_case("Rejected")
            {
                None
            } else {
                status_message_for_user(message)
            }
        }
        StatusType::ApprovalNeeded
        | StatusType::JobStarted
        | StatusType::AuthRequired
        | StatusType::AuthCompleted => status_message_for_user(&update.message),
    }
}

fn send_status_notification(metadata: &WecomMessageMetadata, content: &str) -> Result<(), String> {
    if let Some(req_id) = metadata.ws_req_id.as_deref() {
        let reply_cmd = metadata
            .ws_reply_cmd
            .as_deref()
            .unwrap_or(WECOM_WS_REPLY_CMD);
        return send_websocket_stream_reply(req_id, reply_cmd, content);
    }

    Err("WeCom Bot status update missing websocket route metadata".to_string())
}

fn status_update_is_sensitive(update: &StatusUpdate) -> bool {
    matches!(
        &update.status,
        StatusType::ApprovalNeeded | StatusType::AuthRequired | StatusType::AuthCompleted
    )
}

fn metadata_is_group_chat(metadata: &WecomMessageMetadata) -> bool {
    chat_type_is_group(
        metadata
            .ws_chat_type
            .as_deref()
            .or(metadata.chat_type.as_deref()),
    )
}

fn safe_group_status_content(update: &StatusUpdate, content: String) -> String {
    if status_update_is_sensitive(update) {
        "This action needs a private authorization step. Please DM the bot to continue."
            .to_string()
    } else {
        content
    }
}

fn build_websocket_command_payload(
    cmd: &str,
    req_id: &str,
    body: JsonValue,
) -> Result<String, String> {
    serde_json::to_string(&serde_json::json!({
        "cmd": cmd,
        "headers": {
            "req_id": req_id,
        },
        "body": body,
    }))
        .map_err(|e| format!("Failed to serialize WeCom websocket command: {e}"))
}

fn websocket_req_id_now_millis() -> u64 {
    #[cfg(test)]
    {
        1
    }
    #[cfg(not(test))]
    {
        channel_host::now_millis()
    }
}

fn websocket_control_req_id(cmd: &str, seed: &str) -> String {
    let now = websocket_req_id_now_millis();
    let mut hasher = Sha1::new();
    hasher.update(cmd.as_bytes());
    hasher.update(seed.as_bytes());
    hasher.update(now.to_string().as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("{cmd}_{now}_{}", &digest[..8])
}

fn websocket_media_batch_id(metadata: &WecomMessageMetadata) -> String {
    let seed = metadata
        .source_msg_id
        .as_deref()
        .or(metadata.ws_req_id.as_deref())
        .unwrap_or("response");
    websocket_control_req_id("ironclaw_wecom_media_batch", seed)
}

fn websocket_media_send_id(batch_id: &str, index: usize, attachment: &Attachment) -> String {
    let seed = format!(
        "{batch_id}:{index}:{}:{}:{}",
        attachment.filename,
        attachment.mime_type,
        attachment.data.len()
    );
    websocket_control_req_id("ironclaw_wecom_media", &seed)
}

fn websocket_media_chunk_blob_path(send_id: &str, chunk_index: usize) -> String {
    let mut hasher = Sha1::new();
    hasher.update(send_id.as_bytes());
    hasher.update(b":");
    hasher.update(chunk_index.to_string().as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    format!("{WEBSOCKET_MEDIA_CHUNK_BLOBS_PREFIX}/{digest}.b64")
}

fn persist_websocket_media_chunks(send_id: &str, data: &[u8]) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    for (index, chunk) in data.chunks(WEBSOCKET_MEDIA_CHUNK_SIZE).enumerate() {
        let path = websocket_media_chunk_blob_path(send_id, index);
        if let Err(error) = write_wecom_workspace(&path, &BASE64_STANDARD.encode(chunk)) {
            cleanup_websocket_media_chunk_paths(&paths);
            return Err(format!(
                "Failed to persist WeCom websocket media chunk: {error}"
            ));
        }
        paths.push(path);
    }
    Ok(paths)
}

fn cleanup_websocket_media_chunk_paths(paths: &[String]) {
    for path in paths {
        if let Err(error) = write_wecom_workspace(path, "") {
            log_wecom(
                channel_host::LogLevel::Warn,
                &format!("Failed to cleanup WeCom websocket media chunk blob: {error}"),
            );
        }
    }
}

fn cleanup_websocket_media_chunks(send: &PendingWebsocketMediaSend) {
    cleanup_websocket_media_chunk_paths(&send.chunk_blob_paths);
}

fn read_websocket_media_chunk_base64(
    send: &PendingWebsocketMediaSend,
    chunk_index: usize,
) -> Result<String, String> {
    let path = send
        .chunk_blob_paths
        .get(chunk_index)
        .ok_or_else(|| format!("WeCom websocket media chunk {chunk_index} path is missing"))?;
    let raw = read_wecom_workspace(path)
        .ok_or_else(|| format!("WeCom websocket media chunk {chunk_index} blob is missing"))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("WeCom websocket media chunk {chunk_index} blob is empty"));
    }
    Ok(trimmed.to_string())
}

fn prune_stale_pending_websocket_media_state(
    state: &mut PendingWebsocketMediaState,
    now_ms: u64,
) -> bool {
    let mut removed = false;
    let mut retained_sends = Vec::with_capacity(state.sends.len());
    for send in state.sends.drain(..) {
        let age_ms = now_ms.saturating_sub(send.created_at_ms);
        if send.created_at_ms == 0 || age_ms > WEBSOCKET_MEDIA_SEND_TTL_MS {
            cleanup_websocket_media_chunks(&send);
            removed = true;
        } else {
            retained_sends.push(send);
        }
    }
    state.sends = retained_sends;

    let active_batch_ids: std::collections::HashSet<&str> =
        state.sends.iter().map(|send| send.batch_id.as_str()).collect();
    let before_batches = state.batches.len();
    state.batches.retain(|batch| {
        let age_ms = now_ms.saturating_sub(batch.created_at_ms);
        active_batch_ids.contains(batch.id.as_str())
            || (batch.created_at_ms > 0 && age_ms <= WEBSOCKET_MEDIA_SEND_TTL_MS)
    });
    removed || before_batches != state.batches.len()
}

fn load_pending_websocket_media_state() -> PendingWebsocketMediaState {
    let Some(raw) = read_wecom_workspace(WEBSOCKET_MEDIA_STATE_PATH) else {
        return PendingWebsocketMediaState::default();
    };
    if raw.trim().is_empty() {
        return PendingWebsocketMediaState::default();
    }
    match serde_json::from_str(&raw) {
        Ok(mut state) => {
            if prune_stale_pending_websocket_media_state(&mut state, websocket_req_id_now_millis())
            {
                if let Err(error) = persist_pending_websocket_media_state(&state) {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Failed to persist pruned WeCom websocket media state: {error}"),
                    );
                }
            }
            state
        }
        Err(error) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to parse pending WeCom websocket media state: {error}"),
            );
            PendingWebsocketMediaState::default()
        }
    }
}

fn persist_pending_websocket_media_state(state: &PendingWebsocketMediaState) -> Result<(), String> {
    let json = serde_json::to_string(state)
        .map_err(|e| format!("Failed to serialize pending WeCom websocket media state: {e}"))?;
    write_wecom_workspace(WEBSOCKET_MEDIA_STATE_PATH, &json)
}

fn websocket_media_target(metadata: &WecomMessageMetadata) -> Option<String> {
    metadata
        .ws_chat_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let to_user = metadata.to_user.trim();
            (!to_user.is_empty()).then_some(to_user)
        })
        .map(str::to_string)
}

fn classify_websocket_media(att: &Attachment) -> OutboundMediaKind {
    let preferred = preferred_outbound_media_kind(att);
    if att.data.len() > preferred.websocket_max_bytes() {
        OutboundMediaKind::File
    } else {
        preferred
    }
}

fn validate_websocket_media_size(
    media_kind: OutboundMediaKind,
    size_bytes: usize,
) -> Result<(), String> {
    if size_bytes > media_kind.websocket_max_bytes() {
        return Err(format!(
            "WeCom websocket {} attachment exceeds {} bytes",
            media_kind.as_api_type(),
            media_kind.websocket_max_bytes()
        ));
    }
    let total_chunks = size_bytes.div_ceil(WEBSOCKET_MEDIA_CHUNK_SIZE).max(1);
    if total_chunks > MAX_WEBSOCKET_MEDIA_CHUNKS {
        return Err(format!(
            "WeCom websocket attachment requires {total_chunks} chunks; maximum is {MAX_WEBSOCKET_MEDIA_CHUNKS}"
        ));
    }
    Ok(())
}

fn build_websocket_media_init_payload(send: &PendingWebsocketMediaSend) -> Result<String, String> {
    if send.md5_hex.trim().is_empty() {
        return Err("WeCom websocket media md5 is missing".to_string());
    }
    build_websocket_command_payload(
        WECOM_WS_UPLOAD_MEDIA_INIT_CMD,
        &send.init_req_id,
        serde_json::json!({
            "type": send.media_type,
            "filename": send.filename,
            "total_size": send.total_size,
            "total_chunks": send.total_chunks,
            "md5": send.md5_hex,
        }),
    )
}

fn build_pending_websocket_media_send(
    batch_id: &str,
    chat_id: &str,
    attachment: &Attachment,
    index: usize,
) -> Result<(PendingWebsocketMediaSend, String), String> {
    if attachment.data.is_empty() {
        return Err(format!(
            "WeCom websocket attachment '{}' has no data",
            attachment.filename
        ));
    }

    let media_kind = classify_websocket_media(attachment);
    validate_websocket_media_size(media_kind, attachment.data.len())?;
    let total_chunks = attachment
        .data
        .len()
        .div_ceil(WEBSOCKET_MEDIA_CHUNK_SIZE)
        .max(1);
    let id = websocket_media_send_id(batch_id, index, attachment);
    let chunk_blob_paths = persist_websocket_media_chunks(&id, &attachment.data)?;
    let init_req_id = websocket_control_req_id(WECOM_WS_UPLOAD_MEDIA_INIT_CMD, &id);
    let send = PendingWebsocketMediaSend {
        id,
        batch_id: batch_id.to_string(),
        chat_id: chat_id.to_string(),
        created_at_ms: websocket_req_id_now_millis(),
        media_type: media_kind.as_api_type().to_string(),
        filename: if attachment.filename.trim().is_empty() {
            "attachment.bin".to_string()
        } else {
            attachment.filename.clone()
        },
        md5_hex: websocket_media_md5_hex(&attachment.data),
        chunk_blob_paths,
        total_size: attachment.data.len(),
        total_chunks,
        next_chunk_index: 0,
        init_req_id,
        chunk_req_id: None,
        finish_req_id: None,
        send_req_id: None,
        upload_id: None,
        media_id: None,
    };
    let payload = build_websocket_media_init_payload(&send)?;
    Ok((send, payload))
}

fn build_websocket_media_chunk_payload(send: &PendingWebsocketMediaSend) -> Result<String, String> {
    let upload_id = send
        .upload_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "WeCom websocket media upload_id missing before chunk".to_string())?;
    let chunk_base64 = read_websocket_media_chunk_base64(send, send.next_chunk_index)?;
    let req_id = send
        .chunk_req_id
        .as_deref()
        .ok_or_else(|| "WeCom websocket media chunk req_id missing".to_string())?;
    build_websocket_command_payload(
        WECOM_WS_UPLOAD_MEDIA_CHUNK_CMD,
        req_id,
        serde_json::json!({
            "upload_id": upload_id,
            "chunk_index": send.next_chunk_index,
            "base64_data": chunk_base64,
        }),
    )
}

fn build_websocket_media_finish_payload(
    send: &PendingWebsocketMediaSend,
) -> Result<String, String> {
    let upload_id = send
        .upload_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "WeCom websocket media upload_id missing before finish".to_string())?;
    let req_id = send
        .finish_req_id
        .as_deref()
        .ok_or_else(|| "WeCom websocket media finish req_id missing".to_string())?;
    build_websocket_command_payload(
        WECOM_WS_UPLOAD_MEDIA_FINISH_CMD,
        req_id,
        serde_json::json!({ "upload_id": upload_id }),
    )
}

fn build_websocket_active_media_payload(
    send: &PendingWebsocketMediaSend,
) -> Result<String, String> {
    let media_id = send
        .media_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "WeCom websocket media_id missing before send".to_string())?;
    let req_id = send
        .send_req_id
        .as_deref()
        .ok_or_else(|| "WeCom websocket media send req_id missing".to_string())?;
    let media_type = send.media_type.as_str();
    build_websocket_command_payload(
        WECOM_WS_SEND_MSG_CMD,
        req_id,
        serde_json::json!({
            "chatid": send.chat_id,
            "msgtype": media_type,
            media_type: { "media_id": media_id },
        }),
    )
}

fn build_websocket_active_markdown_payload(chat_id: &str, content: &str) -> Result<String, String> {
    let req_id = websocket_control_req_id(WECOM_WS_SEND_MSG_CMD, content);
    build_websocket_command_payload(
        WECOM_WS_SEND_MSG_CMD,
        &req_id,
        serde_json::json!({
            "chatid": chat_id,
            "msgtype": "markdown",
            "markdown": {
                "content": content,
            },
        }),
    )
}

fn prepare_next_websocket_media_chunk(send: &mut PendingWebsocketMediaSend) -> Result<String, String> {
    let req_id = websocket_control_req_id(
        WECOM_WS_UPLOAD_MEDIA_CHUNK_CMD,
        &format!("{}:{}", send.id, send.next_chunk_index),
    );
    send.chunk_req_id = Some(req_id);
    build_websocket_media_chunk_payload(send)
}

fn prepare_websocket_media_finish(send: &mut PendingWebsocketMediaSend) -> Result<String, String> {
    let req_id = websocket_control_req_id(WECOM_WS_UPLOAD_MEDIA_FINISH_CMD, &send.id);
    send.finish_req_id = Some(req_id);
    build_websocket_media_finish_payload(send)
}

fn prepare_websocket_active_media(send: &mut PendingWebsocketMediaSend) -> Result<String, String> {
    let req_id = websocket_control_req_id(WECOM_WS_SEND_MSG_CMD, &send.id);
    send.send_req_id = Some(req_id);
    build_websocket_active_media_payload(send)
}

fn send_websocket_active_markdown(chat_id: &str, content: &str) -> Result<(), String> {
    let content = content.trim();
    if content.is_empty() {
        return Ok(());
    }
    let payload = build_websocket_active_markdown_payload(chat_id, content)?;
    send_websocket_text(&payload)
        .map_err(|e| format!("Failed to send WeCom websocket markdown message: {e}"))
}

fn start_websocket_media_batch(
    metadata: &WecomMessageMetadata,
    response_req_id: &str,
    response_cmd: &str,
    content: &str,
    attachments: &[Attachment],
) -> WebsocketMediaStartResult {
    let Some(chat_id) = websocket_media_target(metadata) else {
        return WebsocketMediaStartResult {
            started: 0,
            errors: vec!["WeCom websocket media send requires chat_id or user_id".to_string()],
        };
    };

    let batch_id = websocket_media_batch_id(metadata);
    let mut sends = Vec::new();
    let mut outbound_payloads = Vec::new();
    let mut errors = Vec::new();
    let mut selected_media_count = 0usize;
    let mut selected_media_bytes = 0usize;
    for (index, attachment) in attachments.iter().enumerate() {
        if selected_media_count >= MAX_WEBSOCKET_MEDIA_ATTACHMENTS_PER_RESPONSE {
            push_websocket_media_error(
                &mut errors,
                format!(
                    "WeCom websocket media response exceeds {MAX_WEBSOCKET_MEDIA_ATTACHMENTS_PER_RESPONSE} attachment(s); remaining attachments were skipped"
                ),
            );
            continue;
        }
        if selected_media_bytes.saturating_add(attachment.data.len())
            > MAX_WEBSOCKET_MEDIA_TOTAL_BYTES_PER_RESPONSE
        {
            push_websocket_media_error(
                &mut errors,
                format!(
                    "WeCom websocket media response exceeds {MAX_WEBSOCKET_MEDIA_TOTAL_BYTES_PER_RESPONSE} total bytes; attachment '{}' was skipped",
                    attachment.filename
                ),
            );
            continue;
        }
        match build_pending_websocket_media_send(&batch_id, &chat_id, attachment, index) {
            Ok((send, payload)) => {
                selected_media_count += 1;
                selected_media_bytes += attachment.data.len();
                outbound_payloads.push(PendingWebsocketMediaOutbound {
                    send_id: send.id.clone(),
                    payload,
                    complete_after_send: false,
                });
                sends.push(send);
            }
            Err(error) => push_websocket_media_error(&mut errors, error),
        }
    }

    let started = sends.len();
    if started > 0 {
        let mut state = load_pending_websocket_media_state();
        state.batches.push(PendingWebsocketMediaBatch {
            id: batch_id,
            chat_id,
            created_at_ms: websocket_req_id_now_millis(),
            response_req_id: response_req_id.to_string(),
            response_cmd: response_cmd.to_string(),
            final_text: content.trim().to_string(),
            remaining_media: started,
            sent_media: 0,
            failed_media: errors.len(),
            errors: capped_websocket_media_errors(errors.clone()),
        });
        state.sends.extend(sends.iter().cloned());
        if let Err(error) = persist_pending_websocket_media_state(&state) {
            for send in &sends {
                cleanup_websocket_media_chunks(send);
            }
            push_websocket_media_error(
                &mut errors,
                format!("Failed to persist WeCom websocket media state: {error}"),
            );
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to persist WeCom websocket media state: {error}"),
            );
            return WebsocketMediaStartResult { started: 0, errors };
        }

        for outbound in outbound_payloads {
            if let Err(error) = send_websocket_text(&outbound.payload) {
                let error = format!("Failed to send WeCom websocket media init: {error}");
                fail_persisted_websocket_media_send(&outbound.send_id, error.clone());
                errors.push(error);
            }
        }
    }

    WebsocketMediaStartResult { started, errors }
}

fn append_websocket_media_errors_to_text(content: &str, errors: &[String]) -> String {
    let first_error = errors
        .first()
        .map(String::as_str)
        .unwrap_or("unknown error");
    let trimmed = content.trim();
    if trimmed.is_empty() {
        format!("附件已生成，但发送失败：{first_error}")
    } else {
        format!("{trimmed}\n\n附件已生成，但发送失败：{first_error}")
    }
}

fn pending_websocket_media_matches_req(send: &PendingWebsocketMediaSend, req_id: &str) -> bool {
    send.init_req_id == req_id
        || send.chunk_req_id.as_deref() == Some(req_id)
        || send.finish_req_id.as_deref() == Some(req_id)
        || send.send_req_id.as_deref() == Some(req_id)
}

fn json_body_string(body: &JsonValue, key: &str) -> Option<String> {
    body.get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn fail_pending_websocket_media(
    state: &mut PendingWebsocketMediaState,
    send: &PendingWebsocketMediaSend,
    error: String,
) {
    log_wecom(
        channel_host::LogLevel::Warn,
        &format!("WeCom websocket media send failed: {error}"),
    );
    cleanup_websocket_media_chunks(send);
    complete_websocket_media_batch(state, &send.batch_id, Err(error));
}

fn fail_persisted_websocket_media_send(send_id: &str, error: String) {
    let mut state = load_pending_websocket_media_state();
    let Some(pos) = state.sends.iter().position(|send| send.id == send_id) else {
        log_wecom(
            channel_host::LogLevel::Warn,
            &format!("WeCom websocket media send failed after state was already cleared: {error}"),
        );
        return;
    };

    let send = state.sends.remove(pos);
    fail_pending_websocket_media(&mut state, &send, error);
    if let Err(error) = persist_pending_websocket_media_state(&state) {
        log_wecom(
            channel_host::LogLevel::Warn,
            &format!("Failed to persist WeCom websocket media failure state: {error}"),
        );
    }
}

fn complete_persisted_websocket_media_send(send_id: &str) {
    let mut state = load_pending_websocket_media_state();
    let Some(pos) = state.sends.iter().position(|send| send.id == send_id) else {
        log_wecom(
            channel_host::LogLevel::Warn,
            &format!(
                "WeCom websocket media send completed after state was already cleared: {send_id}"
            ),
        );
        return;
    };

    let send = state.sends.remove(pos);
    cleanup_websocket_media_chunks(&send);
    complete_websocket_media_batch(&mut state, &send.batch_id, Ok(()));
    if let Err(error) = persist_pending_websocket_media_state(&state) {
        log_wecom(
            channel_host::LogLevel::Warn,
            &format!("Failed to persist completed WeCom websocket media state: {error}"),
        );
    }
}

fn capped_websocket_media_errors(errors: Vec<String>) -> Vec<String> {
    if errors.len() <= MAX_WEBSOCKET_MEDIA_BATCH_ERRORS {
        return errors;
    }
    let omitted = errors.len() - MAX_WEBSOCKET_MEDIA_BATCH_ERRORS;
    let mut capped: Vec<String> = errors
        .into_iter()
        .take(MAX_WEBSOCKET_MEDIA_BATCH_ERRORS)
        .collect();
    capped.push(format!(
        "{omitted} additional WeCom websocket media error(s) omitted"
    ));
    capped
}

fn push_websocket_media_error(errors: &mut Vec<String>, error: String) {
    if errors.len() < MAX_WEBSOCKET_MEDIA_BATCH_ERRORS {
        errors.push(error);
    } else if errors.len() == MAX_WEBSOCKET_MEDIA_BATCH_ERRORS {
        errors.push(format!(
            "Additional WeCom websocket media errors omitted after first {MAX_WEBSOCKET_MEDIA_BATCH_ERRORS}"
        ));
    }
}

fn complete_websocket_media_batch(
    state: &mut PendingWebsocketMediaState,
    batch_id: &str,
    result: Result<(), String>,
) {
    let Some(pos) = state.batches.iter().position(|batch| batch.id == batch_id) else {
        return;
    };

    let batch = &mut state.batches[pos];
    if batch.remaining_media > 0 {
        batch.remaining_media -= 1;
    }
    match result {
        Ok(()) => batch.sent_media += 1,
        Err(error) => {
            batch.failed_media += 1;
            push_websocket_media_error(&mut batch.errors, error);
        }
    }

    if batch.remaining_media > 0 {
        return;
    }

    let batch = state.batches.remove(pos);
    let final_text = if batch.failed_media > 0 {
        append_websocket_media_errors_to_text(&batch.final_text, &batch.errors)
    } else {
        batch.final_text
    };

    let final_text = final_text.trim();
    if !final_text.is_empty() {
        let send_result = if batch.response_req_id.trim().is_empty() {
            send_websocket_active_markdown(&batch.chat_id, final_text)
        } else {
            let response_cmd = if batch.response_cmd.trim().is_empty() {
                WECOM_WS_REPLY_CMD
            } else {
                batch.response_cmd.as_str()
            };
            send_websocket_stream_reply(&batch.response_req_id, response_cmd, final_text)
        };

        if let Err(error) = send_result {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to send WeCom websocket final text after media: {error}"),
            );
        }
    }
}

fn advance_pending_websocket_media(
    mut send: PendingWebsocketMediaSend,
    ack: &WecomWsAckFrame,
) -> Result<PendingWebsocketMediaAdvance, String> {
    let req_id = ack.headers.req_id.as_str();

    if send.init_req_id == req_id {
        let upload_id = json_body_string(&ack.body, "upload_id")
            .ok_or_else(|| "WeCom websocket upload init ack missing upload_id".to_string())?;
        send.upload_id = Some(upload_id);
        send.next_chunk_index = 0;
        let payload = prepare_next_websocket_media_chunk(&mut send)?;
        let outbound = PendingWebsocketMediaOutbound {
            send_id: send.id.clone(),
            payload,
            complete_after_send: false,
        };
        return Ok(PendingWebsocketMediaAdvance::Send(outbound, send));
    }

    if send.chunk_req_id.as_deref() == Some(req_id) {
        send.next_chunk_index += 1;
        let payload = if send.next_chunk_index < send.total_chunks {
            prepare_next_websocket_media_chunk(&mut send)?
        } else {
            prepare_websocket_media_finish(&mut send)?
        };
        let outbound = PendingWebsocketMediaOutbound {
            send_id: send.id.clone(),
            payload,
            complete_after_send: false,
        };
        return Ok(PendingWebsocketMediaAdvance::Send(outbound, send));
    }

    if send.finish_req_id.as_deref() == Some(req_id) {
        let media_id = json_body_string(&ack.body, "media_id")
            .ok_or_else(|| "WeCom websocket upload finish ack missing media_id".to_string())?;
        send.media_id = Some(media_id);
        let payload = prepare_websocket_active_media(&mut send)?;
        let outbound = PendingWebsocketMediaOutbound {
            send_id: send.id.clone(),
            payload,
            complete_after_send: true,
        };
        return Ok(PendingWebsocketMediaAdvance::Send(outbound, send));
    }

    if send.send_req_id.as_deref() == Some(req_id) {
        return Ok(PendingWebsocketMediaAdvance::Complete);
    }

    Err("WeCom websocket ack matched send but no media phase advanced".to_string())
}

fn parse_websocket_ack_frame(frame: &str) -> Option<WecomWsAckFrame> {
    let value: JsonValue = serde_json::from_str(frame).ok()?;
    value.get("errcode")?.as_i64()?;
    serde_json::from_value(value).ok()
}

enum WebsocketAckApplyResult {
    Unknown,
    Applied {
        outbound: Option<PendingWebsocketMediaOutbound>,
    },
}

fn apply_websocket_ack_to_media_state(
    state: &mut PendingWebsocketMediaState,
    ack: &WecomWsAckFrame,
) -> WebsocketAckApplyResult {
    let Some(pos) = state
        .sends
        .iter()
        .position(|send| pending_websocket_media_matches_req(send, &ack.headers.req_id))
    else {
        return WebsocketAckApplyResult::Unknown;
    };

    let send = state.sends.remove(pos);
    let batch_id = send.batch_id.clone();
    let mut outbound = None;
    if ack.errcode != 0 {
        let error = format!(
            "req_id={} errcode={} errmsg={}",
            ack.headers.req_id, ack.errcode, ack.errmsg
        );
        fail_pending_websocket_media(state, &send, error);
    } else {
        let send_for_failure = send.clone();
        match advance_pending_websocket_media(send, ack) {
            Ok(PendingWebsocketMediaAdvance::Send(next_outbound, next)) => {
                outbound = Some(next_outbound);
                state.sends.push(next);
            }
            Ok(PendingWebsocketMediaAdvance::Complete) => {
                cleanup_websocket_media_chunks(&send_for_failure);
                complete_websocket_media_batch(state, &batch_id, Ok(()));
            }
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to advance WeCom websocket media send: {error}"),
                );
                cleanup_websocket_media_chunks(&send_for_failure);
                complete_websocket_media_batch(state, &batch_id, Err(error));
            }
        }
    }

    WebsocketAckApplyResult::Applied { outbound }
}

fn handle_websocket_ack_frame(ack: WecomWsAckFrame) {
    let mut state = load_pending_websocket_media_state();
    let outbound = match apply_websocket_ack_to_media_state(&mut state, &ack) {
        WebsocketAckApplyResult::Unknown => return,
        WebsocketAckApplyResult::Applied { outbound } => outbound,
    };

    if let Err(error) = persist_pending_websocket_media_state(&state) {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!("Failed to persist WeCom websocket media state: {error}"),
        );
        return;
    }

    if let Some(outbound) = outbound {
        if let Err(error) = send_websocket_text(&outbound.payload) {
            fail_persisted_websocket_media_send(
                &outbound.send_id,
                format!("Failed to send WeCom websocket media command: {error}"),
            );
        } else if outbound.complete_after_send {
            // WeCom does not reliably deliver an ACK for the final
            // aibot_send_msg media command. Once the command has been queued
            // successfully, complete the batch so final_text is sent.
            complete_persisted_websocket_media_send(&outbound.send_id);
        }
    }
}

fn websocket_fallback_target(sender_id: &str, chat_type: Option<&str>) -> String {
    if chat_type == Some("group") {
        String::new()
    } else {
        sender_id.to_string()
    }
}

fn normalize_chat_type(chat_type: Option<&str>) -> Option<String> {
    let kind = chat_type.map(str::trim).filter(|value| !value.is_empty())?;
    Some(match kind {
        "single" => "private".to_string(),
        other => other.to_string(),
    })
}

fn wecom_conversation_scope(
    sender_id: &str,
    chat_id: Option<&str>,
    chat_type: Option<&str>,
) -> String {
    let normalized_chat_type = normalize_chat_type(chat_type);
    if normalized_chat_type.as_deref() == Some("group") {
        if let Some(group_chat_id) = chat_id.map(str::trim).filter(|value| !value.is_empty()) {
            return format!("wecom:group:{group_chat_id}");
        }
    }
    format!("wecom:dm:{sender_id}")
}

fn websocket_metadata_json(
    sender_id: &str,
    msg_id: &str,
    req_id: &str,
    chat_id: Option<&str>,
    chat_type: Option<&str>,
    reply_cmd: &str,
) -> Result<String, String> {
    let normalized_chat_type = normalize_chat_type(chat_type);
    let normalized_chat_id = chat_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let to_user = websocket_fallback_target(sender_id, chat_type);
    let target = normalized_chat_id
        .clone()
        .or_else(|| (!to_user.is_empty()).then_some(to_user.clone()));

    serde_json::to_string(&WecomMessageMetadata {
        to_user,
        target,
        chat_id: normalized_chat_id,
        chat_type: normalized_chat_type,
        source_msg_id: Some(msg_id.to_string()),
        ws_req_id: Some(req_id.to_string()),
        ws_chat_id: chat_id.map(str::to_string),
        ws_chat_type: chat_type.map(str::to_string),
        ws_reply_cmd: Some(reply_cmd.to_string()),
    })
    .map_err(|e| format!("Failed to serialize WeCom websocket metadata: {e}"))
}

fn websocket_attachment_from_binary(
    msg_id: &str,
    msg_type: &str,
    content: &WecomWsBinaryContent,
) -> InboundAttachment {
    let (mime_type, extension) = websocket_default_mime_and_extension(msg_type);
    let attachment = InboundAttachment {
        id: format!("{msg_id}:{msg_type}"),
        mime_type: mime_type.to_string(),
        filename: Some(format!("{msg_id}.{extension}")),
        size_bytes: None,
        source_url: Some(content.url.clone()),
        storage_key: None,
        extracted_text: None,
        extras_json: serde_json::json!({ "wecom_ws_msgtype": msg_type }).to_string(),
    };

    #[cfg(not(test))]
    let attachment = {
        let mut hydrated = attachment;
        if let Err(error) = hydrate_websocket_binary_attachment_data(
            &mut hydrated,
            msg_type,
            content.aeskey.as_deref(),
        ) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!(
                    "Failed to hydrate WeCom websocket {} attachment '{}': {}",
                    msg_type, hydrated.id, error
                ),
            );
        }
        hydrated
    };

    attachment
}

fn websocket_quote_context(quote: Option<&WecomWsQuoteContent>) -> Option<String> {
    let quote = quote?;
    let quoted_text = quote
        .text
        .as_ref()
        .map(|text| text.content.trim())
        .filter(|text| !text.is_empty())
        .or_else(|| {
            quote
                .voice
                .as_ref()
                .map(|voice| voice.content.trim())
                .filter(|text| !text.is_empty())
        })
        .or_else(|| {
            quote
                .content
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
        })?;

    let quote_kind = quote
        .msg_type
        .as_deref()
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or("message");
    Some(format!("Quoted {quote_kind}: {quoted_text}"))
}

fn with_websocket_quote_context(content: String, quote: Option<&WecomWsQuoteContent>) -> String {
    match websocket_quote_context(quote) {
        Some(quoted) if content.trim().is_empty() => quoted,
        Some(quoted) => format!("{quoted}\n\n{content}"),
        None => content,
    }
}

fn websocket_event_summary(event: &WecomWsEvent) -> Option<String> {
    match event.eventtype.as_str() {
        "enter_chat" => Some("User entered the WeCom bot chat.".to_string()),
        "template_card_event" => {
            let event_key = event
                .extra
                .get("event_key")
                .or_else(|| event.extra.get("eventKey"))
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            Some(match event_key {
                Some(key) => format!("User clicked a WeCom template card action: {key}"),
                None => "User clicked a WeCom template card action.".to_string(),
            })
        }
        "feedback_event" => {
            let score = event
                .extra
                .get("score")
                .or_else(|| event.extra.get("rating"))
                .and_then(JsonValue::as_i64);
            Some(match score {
                Some(score) => format!("User submitted WeCom feedback with score {score}."),
                None => "User submitted WeCom feedback.".to_string(),
            })
        }
        _ => None,
    }
}

fn infer_mixed_item_type(item: &WecomWsMixedItem) -> &str {
    item.item_type
        .as_deref()
        .filter(|kind| !kind.trim().is_empty())
        .unwrap_or_else(|| {
            if item.text.is_some() {
                "text"
            } else if item.image.is_some() {
                "image"
            } else if item.file.is_some() {
                "file"
            } else if item.video.is_some() {
                "video"
            } else {
                "unknown"
            }
        })
}

fn websocket_mixed_content_parts(
    msg_id: &str,
    mixed: &WecomWsMixedContent,
) -> (String, Vec<InboundAttachment>) {
    let mut text_parts = Vec::new();
    let mut attachments = Vec::new();

    for (index, item) in mixed.msg_item.iter().enumerate() {
        match infer_mixed_item_type(item) {
            "text" => {
                if let Some(text) = item
                    .text
                    .as_ref()
                    .map(|text| text.content.trim())
                    .filter(|text| !text.is_empty())
                {
                    text_parts.push(text.to_string());
                }
            }
            "image" => {
                if let Some(image) = item.image.as_ref() {
                    attachments.push(websocket_attachment_from_binary(
                        &format!("{msg_id}:{index}"),
                        "image",
                        image,
                    ));
                }
            }
            "file" => {
                if let Some(file) = item.file.as_ref() {
                    attachments.push(websocket_attachment_from_binary(
                        &format!("{msg_id}:{index}"),
                        "file",
                        file,
                    ));
                }
            }
            "video" => {
                if let Some(video) = item.video.as_ref() {
                    attachments.push(websocket_attachment_from_binary(
                        &format!("{msg_id}:{index}"),
                        "video",
                        video,
                    ));
                }
            }
            _ => {}
        }
    }

    (text_parts.join("\n"), attachments)
}

fn handle_websocket_message_frame(
    frame: WecomWsFrame<WecomWsMessageBody>,
    access: &SenderAccessSnapshot,
) {
    let body = frame.body;
    if !should_process_message_id(&body.msgid) {
        return;
    }

    let sender_id = body.from.userid;
    match is_sender_allowed(
        &sender_id,
        body.chattype.as_deref(),
        access,
        Some(PairingReplyRoute {
            req_id: frame.headers.req_id.clone(),
            reply_cmd: WECOM_WS_REPLY_CMD.to_string(),
            chat_type: body.chattype.clone(),
        }),
    ) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("WeCom websocket sender authorization failed: {error}"),
            );
            return;
        }
    }

    let mut attachments = Vec::new();
    let content = match body.msgtype.as_str() {
        "text" => body.text.map(|text| text.content).unwrap_or_default(),
        "voice" => body.voice.map(|voice| voice.content).unwrap_or_default(),
        "markdown" => body.text.map(|text| text.content).unwrap_or_default(),
        "image" => {
            if let Some(image) = body.image.as_ref() {
                attachments.push(websocket_attachment_from_binary(
                    &body.msgid,
                    "image",
                    image,
                ));
            }
            String::new()
        }
        "file" => {
            if let Some(file) = body.file.as_ref() {
                attachments.push(websocket_attachment_from_binary(&body.msgid, "file", file));
            }
            String::new()
        }
        "video" => {
            if let Some(video) = body.video.as_ref() {
                attachments.push(websocket_attachment_from_binary(
                    &body.msgid,
                    "video",
                    video,
                ));
            }
            String::new()
        }
        "mixed" => {
            if let Some(mixed) = body.mixed.as_ref() {
                let (mixed_text, mixed_attachments) =
                    websocket_mixed_content_parts(&body.msgid, mixed);
                attachments.extend(mixed_attachments);
                mixed_text
            } else {
                String::new()
            }
        }
        other => {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Ignoring unsupported WeCom websocket message type: {other}"),
            );
            return;
        }
    };
    let content = with_websocket_quote_context(content, body.quote.as_ref());

    let metadata_json = match websocket_metadata_json(
        &sender_id,
        &body.msgid,
        &frame.headers.req_id,
        body.chatid.as_deref(),
        body.chattype.as_deref(),
        WECOM_WS_REPLY_CMD,
    ) {
        Ok(json) => json,
        Err(error) => {
            channel_host::log(channel_host::LogLevel::Error, &error);
            return;
        }
    };
    let conversation_scope =
        wecom_conversation_scope(&sender_id, body.chatid.as_deref(), body.chattype.as_deref());

    emit_or_buffer_incoming_user_message(
        sender_id,
        None,
        content,
        conversation_scope,
        metadata_json,
        attachments,
    );
}

fn handle_websocket_event_frame(
    frame: WecomWsFrame<WecomWsEventBody>,
    access: &SenderAccessSnapshot,
) {
    let body = frame.body;
    if !should_process_message_id(&body.msgid) {
        return;
    }

    let Some(content) = websocket_event_summary(&body.event) else {
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Ignoring WeCom websocket event type: {}",
                body.event.eventtype
            ),
        );
        return;
    };

    let reply_cmd = if body.event.eventtype == "enter_chat" {
        WECOM_WS_WELCOME_CMD
    } else {
        WECOM_WS_REPLY_CMD
    };

    let sender_id = body.from.userid;
    match is_sender_allowed(
        &sender_id,
        body.chattype.as_deref(),
        access,
        Some(PairingReplyRoute {
            req_id: frame.headers.req_id.clone(),
            reply_cmd: reply_cmd.to_string(),
            chat_type: body.chattype.clone(),
        }),
    ) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("WeCom websocket sender authorization failed: {error}"),
            );
            return;
        }
    }

    let metadata_json = match websocket_metadata_json(
        &sender_id,
        &body.msgid,
        &frame.headers.req_id,
        body.chatid.as_deref(),
        body.chattype.as_deref(),
        reply_cmd,
    ) {
        Ok(json) => json,
        Err(error) => {
            channel_host::log(channel_host::LogLevel::Error, &error);
            return;
        }
    };
    let conversation_scope =
        wecom_conversation_scope(&sender_id, body.chatid.as_deref(), body.chattype.as_deref());

    channel_host::emit_message(&EmittedMessage {
        user_id: sender_id,
        user_name: None,
        content,
        thread_id: Some(conversation_scope),
        metadata_json,
        attachments: Vec::new(),
    });
}

fn process_websocket_event_queue() {
    let queue_json = channel_host::workspace_read(WEBSOCKET_EVENT_QUEUE_PATH).unwrap_or_default();
    if queue_json.trim().is_empty() || queue_json.trim() == "[]" {
        return;
    }

    let frames: Vec<String> = match serde_json::from_str(&queue_json) {
        Ok(value) => value,
        Err(error) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to deserialize WeCom websocket queue: {error}"),
            );
            let _ = channel_host::workspace_write(WEBSOCKET_EVENT_QUEUE_PATH, "[]");
            return;
        }
    };

    if let Err(error) = channel_host::workspace_write(WEBSOCKET_EVENT_QUEUE_PATH, "[]") {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!("Failed to clear WeCom websocket queue: {error}"),
        );
    }

    let access = SenderAccessSnapshot::load();

    for frame in frames {
        let cmd = serde_json::from_str::<serde_json::Value>(&frame)
            .ok()
            .and_then(|value| {
                value
                    .get("cmd")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            });

        match cmd.as_deref() {
            Some("aibot_msg_callback") => {
                match serde_json::from_str::<WecomWsFrame<WecomWsMessageBody>>(&frame) {
                    Ok(parsed) => handle_websocket_message_frame(parsed, &access),
                    Err(error) => channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Failed to parse WeCom websocket message frame: {error}"),
                    ),
                }
            }
            Some("aibot_event_callback") => {
                match serde_json::from_str::<WecomWsFrame<WecomWsEventBody>>(&frame) {
                    Ok(parsed) => handle_websocket_event_frame(parsed, &access),
                    Err(error) => channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Failed to parse WeCom websocket event frame: {error}"),
                    ),
                }
            }
            Some(other) => {
                if let Some(ack) = parse_websocket_ack_frame(&frame) {
                    handle_websocket_ack_frame(ack);
                } else {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("Ignoring WeCom websocket control frame: {other}"),
                    );
                }
            }
            None => {
                if let Some(ack) = parse_websocket_ack_frame(&frame) {
                    handle_websocket_ack_frame(ack);
                }
            }
        }
    }
}

fn update_recent_message_ids(
    existing_json: Option<&str>,
    msg_id: &str,
    max_ids: usize,
    now_ms: u64,
    ttl_ms: u64,
) -> Result<(bool, String), String> {
    let mut ids: Vec<RecentMessageIdEntry> = match existing_json.filter(|s| !s.trim().is_empty()) {
        Some(raw) => serde_json::from_str::<Vec<RecentMessageIdEntry>>(raw).or_else(|_| {
            serde_json::from_str::<Vec<String>>(raw).map(|legacy| {
                legacy
                    .into_iter()
                    .map(|id| RecentMessageIdEntry {
                        id,
                        seen_at_ms: now_ms,
                    })
                    .collect()
            })
        })
        .map_err(|e| format!("Failed to parse recent WeCom message ids: {e}"))?,
        None => Vec::new(),
    };

    ids.retain(|entry| now_ms.saturating_sub(entry.seen_at_ms) <= ttl_ms);

    if ids.iter().any(|existing| existing.id == msg_id) {
        let json = serde_json::to_string(&ids)
            .map_err(|e| format!("Failed to serialize recent WeCom message ids: {e}"))?;
        return Ok((false, json));
    }

    ids.push(RecentMessageIdEntry {
        id: msg_id.to_string(),
        seen_at_ms: now_ms,
    });
    if ids.len() > max_ids {
        let to_drop = ids.len() - max_ids;
        ids.drain(0..to_drop);
    }

    let json = serde_json::to_string(&ids)
        .map_err(|e| format!("Failed to serialize recent WeCom message ids: {e}"))?;
    Ok((true, json))
}

fn should_process_message_id(msg_id: &str) -> bool {
    match update_recent_message_ids(
        channel_host::workspace_read(RECENT_MSG_IDS_PATH).as_deref(),
        msg_id,
        MAX_RECENT_MSG_IDS,
        channel_host::now_millis(),
        MAX_RECENT_MSG_ID_AGE_MS,
    ) {
        Ok((true, json)) => {
            if let Err(error) = channel_host::workspace_write(RECENT_MSG_IDS_PATH, &json) {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to persist WeCom dedupe state: {error}"),
                );
            }
            true
        }
        Ok((false, _)) => false,
        Err(error) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to update WeCom dedupe state: {error}"),
            );
            true
        }
    }
}

struct WecomChannel;

export!(WecomChannel);

impl Guest for WecomChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: WecomConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse WeCom config: {e}"))?;

        let _ =
            channel_host::workspace_write(OWNER_ID_PATH, config.owner_id.as_deref().unwrap_or(""));
        let dm_policy = config.dm_policy.unwrap_or_default();
        let _ = channel_host::workspace_write(DM_POLICY_PATH, dm_policy.as_str());
        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);
        let inbound_merge_window_ms = config
            .inbound_merge_window_ms
            .map(u64::from)
            .unwrap_or(DEFAULT_INBOUND_MERGE_WINDOW_MS)
            .min(MAX_INBOUND_MERGE_WINDOW_MS);
        let _ = channel_host::workspace_write(
            INBOUND_MERGE_WINDOW_MS_PATH,
            &inbound_merge_window_ms.to_string(),
        );

        Ok(ChannelConfig {
            display_name: "WeCom".to_string(),
            http_endpoints: Vec::new(),
            poll: Some(PollConfig {
                interval_ms: WECOM_POLL_INTERVAL_MS,
                enabled: true,
            }),
        })
    }

    fn on_http_request(_req: IncomingHttpRequest) -> OutgoingHttpResponse {
        text_response(404, "WeCom Bot channel does not expose an HTTP callback endpoint")
    }

    fn on_poll() {
        process_websocket_event_queue();
        flush_due_pending_inbound_bundles();
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: WecomMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse WeCom response metadata: {e}"))?;

        if let Some(req_id) = metadata.ws_req_id.as_deref() {
            let reply_cmd = metadata
                .ws_reply_cmd
                .as_deref()
                .unwrap_or(WECOM_WS_REPLY_CMD);

            if !response.attachments.is_empty() {
                let media = start_websocket_media_batch(
                    &metadata,
                    req_id,
                    reply_cmd,
                    &response.content,
                    &response.attachments,
                );
                if media.started > 0 {
                    return Ok(());
                }

                let content =
                    append_websocket_media_errors_to_text(&response.content, &media.errors);
                send_websocket_stream_reply(req_id, reply_cmd, &content)?;
                return Ok(());
            }

            if !response.content.trim().is_empty() {
                send_websocket_stream_reply(req_id, reply_cmd, &response.content)?;
            }
            return Ok(());
        }

        Err("WeCom Bot outbound requires websocket route metadata".to_string())
    }

    fn on_broadcast(_user_id: String, _response: AgentResponse) -> Result<(), String> {
        Err("WeCom Bot broadcast is not supported without an active websocket chat route".to_string())
    }

    fn on_status(update: StatusUpdate) {
        let Some(content) = classify_status_update(&update) else {
            return;
        };
        let metadata: WecomMessageMetadata = match serde_json::from_str(&update.metadata_json) {
            Ok(metadata) => metadata,
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Failed to parse WeCom status metadata: {error}"),
                );
                return;
            }
        };
        let content = if metadata_is_group_chat(&metadata) {
            safe_group_status_content(&update, content)
        } else {
            content
        };
        if let Err(error) = send_status_notification(&metadata, &content) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to send WeCom status notification: {error}"),
            );
        }
    }

    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "WeCom channel shutting down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::block_padding::NoPadding;
    use aes::cipher::BlockEncryptMut;
    use cbc::Encryptor;

    const WECOM_CAPABILITIES_JSON: &str = include_str!("../wecom.capabilities.json");

    type Aes256CbcEnc = Encryptor<Aes256>;

    fn encrypt_websocket_media_for_test(key_bytes: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
        let block_size = 32usize;
        let pad_len = block_size - (plaintext.len() % block_size);
        let mut padded = plaintext.to_vec();
        padded.extend(std::iter::repeat_n(pad_len as u8, pad_len));

        let iv = &key_bytes[..16];
        let msg_len = padded.len();
        Aes256CbcEnc::new_from_slices(key_bytes, iv)
            .expect("encryptor")
            .encrypt_padded_mut::<NoPadding>(&mut padded, msg_len)
            .expect("encrypt")
            .to_vec()
    }

    fn websocket_ack_for_test(req_id: &str, body: JsonValue) -> WecomWsAckFrame {
        WecomWsAckFrame {
            headers: WecomWsHeaders {
                req_id: req_id.to_string(),
            },
            errcode: 0,
            errmsg: String::new(),
            body,
        }
    }

    #[test]
    fn capabilities_are_bot_only() {
        let caps: serde_json::Value =
            serde_json::from_str(WECOM_CAPABILITIES_JSON).expect("capabilities parse");
        let required = caps["setup"]["required_secrets"]
            .as_array()
            .expect("required secrets array");
        assert!(required
            .iter()
            .any(|entry| entry["name"] == "wecom_bot_id" && entry["optional"] == false));
        assert!(required
            .iter()
            .any(|entry| entry["name"] == "wecom_bot_secret" && entry["optional"] == false));
        assert!(required
            .iter()
            .all(|entry| !entry["name"].as_str().unwrap_or("").contains("corp")));
        assert!(caps["setup"]["validation_endpoint"].is_null());
        assert!(caps["capabilities"]["channel"]["webhook"].is_null());
    }

    #[test]
    fn websocket_stream_reply_payloads_chunk_content_and_reuse_req_id() {
        let content = "a".repeat(STREAM_CHUNK_LIMIT_BYTES + 17);
        let payloads = build_websocket_text_stream_reply_payloads(
            "req-123",
            WECOM_WS_REPLY_CMD,
            &content,
            true,
        )
        .expect("payloads");

        assert_eq!(payloads.len(), 2);
        let first: serde_json::Value = serde_json::from_str(&payloads[0]).expect("first payload");
        let second: serde_json::Value = serde_json::from_str(&payloads[1]).expect("second payload");

        assert_eq!(first["cmd"], serde_json::json!(WECOM_WS_REPLY_CMD));
        assert_eq!(first["headers"]["req_id"], serde_json::json!("req-123"));
        assert_eq!(
            first["body"]["stream"]["id"],
            serde_json::json!(websocket_stream_id("req-123"))
        );
        assert_eq!(
            first["body"]["stream"]["content"]
                .as_str()
                .expect("first chunk")
                .len(),
            STREAM_CHUNK_LIMIT_BYTES
        );
        assert_eq!(first["body"]["stream"]["finish"], serde_json::json!(false));
        assert_eq!(
            second["body"]["stream"]["content"]
                .as_str()
                .expect("second chunk")
                .len(),
            17
        );
        assert_eq!(second["body"]["stream"]["finish"], serde_json::json!(true));
    }

    #[test]
    fn websocket_media_upload_payloads_match_aibot_sdk_shape() {
        test_reset_websocket_state();
        let chunk_blob_paths =
            persist_websocket_media_chunks("send-1", b"abc").expect("persist chunk");
        let mut send = PendingWebsocketMediaSend {
            id: "send-1".to_string(),
            batch_id: "batch-1".to_string(),
            chat_id: "ZhangSan".to_string(),
            created_at_ms: 1,
            media_type: "image".to_string(),
            filename: "cat.jpg".to_string(),
            md5_hex: websocket_media_md5_hex(b"abc"),
            chunk_blob_paths,
            total_size: 3,
            total_chunks: 1,
            next_chunk_index: 0,
            init_req_id: "init-1".to_string(),
            chunk_req_id: Some("chunk-1".to_string()),
            finish_req_id: Some("finish-1".to_string()),
            send_req_id: Some("send-req-1".to_string()),
            upload_id: Some("upload-1".to_string()),
            media_id: Some("media-1".to_string()),
        };

        let init: serde_json::Value =
            serde_json::from_str(&build_websocket_media_init_payload(&send).expect("init"))
                .expect("init json");
        assert_eq!(
            init["cmd"],
            serde_json::json!(WECOM_WS_UPLOAD_MEDIA_INIT_CMD)
        );
        assert_eq!(init["body"]["type"], serde_json::json!("image"));
        assert_eq!(init["body"]["filename"], serde_json::json!("cat.jpg"));
        assert_eq!(init["body"]["total_size"], serde_json::json!(3));
        assert_eq!(
            init["body"]["md5"],
            serde_json::json!("900150983cd24fb0d6963f7d28e17f72")
        );

        let chunk: serde_json::Value =
            serde_json::from_str(&build_websocket_media_chunk_payload(&send).expect("chunk"))
                .expect("chunk json");
        assert_eq!(
            chunk["cmd"],
            serde_json::json!(WECOM_WS_UPLOAD_MEDIA_CHUNK_CMD)
        );
        assert_eq!(chunk["body"]["upload_id"], serde_json::json!("upload-1"));
        assert_eq!(chunk["body"]["chunk_index"], serde_json::json!(0));
        assert_eq!(chunk["body"]["base64_data"], serde_json::json!("YWJj"));

        let finish: serde_json::Value =
            serde_json::from_str(&build_websocket_media_finish_payload(&send).expect("finish"))
                .expect("finish json");
        assert_eq!(
            finish["cmd"],
            serde_json::json!(WECOM_WS_UPLOAD_MEDIA_FINISH_CMD)
        );
        assert_eq!(finish["body"]["upload_id"], serde_json::json!("upload-1"));

        send.media_id = Some("media-2".to_string());
        let media: serde_json::Value =
            serde_json::from_str(&build_websocket_active_media_payload(&send).expect("send media"))
                .expect("media json");
        assert_eq!(media["cmd"], serde_json::json!(WECOM_WS_SEND_MSG_CMD));
        assert_eq!(media["body"]["chatid"], serde_json::json!("ZhangSan"));
        assert_eq!(media["body"]["msgtype"], serde_json::json!("image"));
        assert_eq!(
            media["body"]["image"]["media_id"],
            serde_json::json!("media-2")
        );
    }

    #[test]
    fn websocket_media_ack_updates_state_before_returning_next_payload() {
        test_reset_websocket_state();
        let attachment = make_outbound_attachment(
            "cat.jpg",
            "image/jpeg",
            WEBSOCKET_MEDIA_CHUNK_SIZE + 1,
        );
        let (send, _init_payload) =
            build_pending_websocket_media_send("batch-1", "chat-1", &attachment, 0)
                .expect("pending send");
        let init_req_id = send.init_req_id.clone();
        let mut state = PendingWebsocketMediaState {
            sends: vec![send],
            batches: vec![PendingWebsocketMediaBatch {
                id: "batch-1".to_string(),
                chat_id: "chat-1".to_string(),
                created_at_ms: 1,
                response_req_id: "reply-1".to_string(),
                response_cmd: WECOM_WS_REPLY_CMD.to_string(),
                final_text: "done".to_string(),
                remaining_media: 1,
                sent_media: 0,
                failed_media: 0,
                errors: Vec::new(),
            }],
        };

        let first = apply_websocket_ack_to_media_state(
            &mut state,
            &websocket_ack_for_test(&init_req_id, serde_json::json!({"upload_id": "upload-1"})),
        );
        let first_payload = match first {
            WebsocketAckApplyResult::Applied {
                outbound: Some(outbound),
            } => outbound.payload,
            _ => panic!("init ack should prepare first chunk"),
        };

        let first_chunk_req_id = state.sends[0]
            .chunk_req_id
            .clone()
            .expect("chunk req id must be persisted before send");
        assert!(
            first_payload.contains(&first_chunk_req_id),
            "payload should use the req_id already present in state"
        );

        let second = apply_websocket_ack_to_media_state(
            &mut state,
            &websocket_ack_for_test(&first_chunk_req_id, serde_json::json!({})),
        );
        let second_payload = match second {
            WebsocketAckApplyResult::Applied {
                outbound: Some(outbound),
            } => outbound.payload,
            _ => panic!("first chunk ack should prepare second chunk"),
        };

        let second_chunk_req_id = state.sends[0]
            .chunk_req_id
            .as_deref()
            .expect("next chunk req id must be persisted before send");
        assert_ne!(first_chunk_req_id, second_chunk_req_id);
        assert!(
            second_payload.contains(second_chunk_req_id),
            "next payload should use the req_id already present in state"
        );
    }

    #[test]
    fn handle_websocket_ack_frame_persists_state_before_sending_next_command() {
        test_reset_websocket_state();
        let attachment = make_outbound_attachment("cat.jpg", "image/jpeg", 3);
        let (send, _init_payload) =
            build_pending_websocket_media_send("batch-1", "chat-1", &attachment, 0)
                .expect("pending send");
        let init_req_id = send.init_req_id.clone();
        let state = PendingWebsocketMediaState {
            sends: vec![send],
            batches: vec![PendingWebsocketMediaBatch {
                id: "batch-1".to_string(),
                chat_id: "chat-1".to_string(),
                created_at_ms: 1,
                response_req_id: "reply-1".to_string(),
                response_cmd: WECOM_WS_REPLY_CMD.to_string(),
                final_text: "done".to_string(),
                remaining_media: 1,
                sent_media: 0,
                failed_media: 0,
                errors: Vec::new(),
            }],
        };
        persist_pending_websocket_media_state(&state).expect("persist state");

        handle_websocket_ack_frame(websocket_ack_for_test(
            &init_req_id,
            serde_json::json!({"upload_id": "upload-1"}),
        ));

        let outbound = TEST_WEBSOCKET_OUTBOUND.with(|outbound| outbound.borrow().clone());
        assert_eq!(outbound.len(), 1);
        let persisted = load_pending_websocket_media_state();
        let chunk_req_id = persisted.sends[0]
            .chunk_req_id
            .as_deref()
            .expect("chunk req_id persisted");
        assert!(outbound[0].contains(chunk_req_id));
    }

    #[test]
    fn handle_websocket_ack_frame_records_media_failure_when_next_command_send_fails() {
        test_reset_websocket_state();
        let attachment = make_outbound_attachment("cat.jpg", "image/jpeg", 3);
        let (send, _init_payload) =
            build_pending_websocket_media_send("batch-2", "chat-2", &attachment, 0)
                .expect("pending send");
        let chunk_paths = send.chunk_blob_paths.clone();
        let init_req_id = send.init_req_id.clone();
        let state = PendingWebsocketMediaState {
            sends: vec![send],
            batches: vec![PendingWebsocketMediaBatch {
                id: "batch-2".to_string(),
                chat_id: "chat-2".to_string(),
                created_at_ms: 1,
                response_req_id: "reply-2".to_string(),
                response_cmd: WECOM_WS_REPLY_CMD.to_string(),
                final_text: "done".to_string(),
                remaining_media: 1,
                sent_media: 0,
                failed_media: 0,
                errors: Vec::new(),
            }],
        };
        persist_pending_websocket_media_state(&state).expect("persist state");
        TEST_WEBSOCKET_SEND_ERROR.with(|error| {
            *error.borrow_mut() = Some("send failed".to_string());
        });

        handle_websocket_ack_frame(websocket_ack_for_test(
            &init_req_id,
            serde_json::json!({"upload_id": "upload-2"}),
        ));

        let persisted = load_pending_websocket_media_state();
        assert!(persisted.sends.is_empty());
        assert!(persisted.batches.is_empty());
        for path in chunk_paths {
            assert!(
                read_wecom_workspace(&path).is_none(),
                "failed media chunk blob should be cleaned up"
            );
        }
        let outbound = TEST_WEBSOCKET_OUTBOUND.with(|outbound| outbound.borrow().clone());
        assert!(
            outbound
                .iter()
                .any(|payload| payload.contains("附件已生成，但发送失败")),
            "failure should still surface a visible text response"
        );
    }

    #[test]
    fn stale_websocket_media_state_prunes_chunk_blobs() {
        test_reset_websocket_state();
        let attachment = make_outbound_attachment("old.jpg", "image/jpeg", 3);
        let (mut send, _init_payload) =
            build_pending_websocket_media_send("batch-old", "chat-old", &attachment, 0)
                .expect("pending send");
        let chunk_paths = send.chunk_blob_paths.clone();
        send.created_at_ms = 1;
        let mut state = PendingWebsocketMediaState {
            sends: vec![send],
            batches: vec![PendingWebsocketMediaBatch {
                id: "batch-old".to_string(),
                chat_id: "chat-old".to_string(),
                created_at_ms: 1,
                response_req_id: "reply-old".to_string(),
                response_cmd: WECOM_WS_REPLY_CMD.to_string(),
                final_text: "done".to_string(),
                remaining_media: 1,
                sent_media: 0,
                failed_media: 0,
                errors: Vec::new(),
            }],
        };

        assert!(prune_stale_pending_websocket_media_state(
            &mut state,
            WEBSOCKET_MEDIA_SEND_TTL_MS + 2
        ));

        assert!(state.sends.is_empty());
        assert!(state.batches.is_empty());
        for path in chunk_paths {
            assert!(read_wecom_workspace(&path).is_none());
        }
    }

    #[test]
    fn websocket_media_errors_are_capped() {
        let errors = (0..(MAX_WEBSOCKET_MEDIA_BATCH_ERRORS + 5))
            .map(|idx| format!("error-{idx}"))
            .collect::<Vec<_>>();

        let capped = capped_websocket_media_errors(errors);

        assert_eq!(capped.len(), MAX_WEBSOCKET_MEDIA_BATCH_ERRORS + 1);
        assert!(capped.last().unwrap().contains("omitted"));
    }

    #[test]
    fn websocket_media_batch_caps_startup_validation_errors() {
        test_reset_websocket_state();
        let metadata = WecomMessageMetadata {
            to_user: "ZhangSan".to_string(),
            target: Some("chat-1".to_string()),
            chat_id: Some("chat-1".to_string()),
            chat_type: Some("private".to_string()),
            source_msg_id: Some("msg-1".to_string()),
            ws_req_id: Some("req-1".to_string()),
            ws_chat_id: Some("chat-1".to_string()),
            ws_chat_type: Some("private".to_string()),
            ws_reply_cmd: Some(WECOM_WS_REPLY_CMD.to_string()),
        };
        let attachments: Vec<Attachment> = (0..(MAX_WEBSOCKET_MEDIA_BATCH_ERRORS + 5))
            .map(|idx| make_outbound_attachment(&format!("empty-{idx}.png"), "image/png", 0))
            .collect();

        let result = start_websocket_media_batch(
            &metadata,
            "req-1",
            WECOM_WS_REPLY_CMD,
            "caption",
            &attachments,
        );

        assert_eq!(result.started, 0);
        assert_eq!(result.errors.len(), MAX_WEBSOCKET_MEDIA_BATCH_ERRORS + 1);
        assert!(result.errors.last().unwrap().contains("omitted"));
        assert!(load_pending_websocket_media_state().sends.is_empty());
    }

    #[test]
    fn websocket_media_ack_accepts_numeric_created_at() {
        let ack = parse_websocket_ack_frame(
            r#"{"headers":{"req_id":"finish-1"},"errcode":0,"errmsg":"ok","body":{"type":"image","media_id":"media-1","created_at":1776832851}}"#,
        )
        .expect("ack");

        assert_eq!(ack.headers.req_id, "finish-1");
        assert_eq!(ack.errcode, 0);
        assert_eq!(
            json_body_string(&ack.body, "media_id").as_deref(),
            Some("media-1")
        );
    }

    #[test]
    fn websocket_media_target_prefers_chat_id_over_user_id() {
        let metadata = WecomMessageMetadata {
            to_user: "ZhangSan".to_string(),
            target: Some("wr-chat".to_string()),
            chat_id: Some("wr-chat".to_string()),
            chat_type: Some("group".to_string()),
            source_msg_id: Some("msg-1".to_string()),
            ws_req_id: Some("req-1".to_string()),
            ws_chat_id: Some("wr-chat".to_string()),
            ws_chat_type: Some("group".to_string()),
            ws_reply_cmd: Some(WECOM_WS_REPLY_CMD.to_string()),
        };

        assert_eq!(
            websocket_media_target(&metadata).as_deref(),
            Some("wr-chat")
        );
    }

    #[test]
    fn websocket_media_batch_enforces_per_response_limits_before_persisting() {
        test_reset_websocket_state();
        let metadata = WecomMessageMetadata {
            to_user: "ZhangSan".to_string(),
            target: Some("chat-1".to_string()),
            chat_id: Some("chat-1".to_string()),
            chat_type: Some("private".to_string()),
            source_msg_id: Some("msg-1".to_string()),
            ws_req_id: Some("req-1".to_string()),
            ws_chat_id: Some("chat-1".to_string()),
            ws_chat_type: Some("private".to_string()),
            ws_reply_cmd: Some(WECOM_WS_REPLY_CMD.to_string()),
        };
        let attachments: Vec<Attachment> = (0..(MAX_WEBSOCKET_MEDIA_ATTACHMENTS_PER_RESPONSE + 2))
            .map(|idx| make_outbound_attachment(&format!("image-{idx}.png"), "image/png", 1))
            .collect();

        let result = start_websocket_media_batch(
            &metadata,
            "req-1",
            WECOM_WS_REPLY_CMD,
            "caption",
            &attachments,
        );

        assert_eq!(result.started, MAX_WEBSOCKET_MEDIA_ATTACHMENTS_PER_RESPONSE);
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("attachment(s)"))
        );
        let persisted = load_pending_websocket_media_state();
        assert_eq!(
            persisted.sends.len(),
            MAX_WEBSOCKET_MEDIA_ATTACHMENTS_PER_RESPONSE
        );
    }

    #[test]
    fn websocket_metadata_json_marks_group_chats_as_agent_api_ineligible() {
        let json = websocket_metadata_json(
            "zhangsan",
            "msg-1",
            "req-1",
            Some("chat-1"),
            Some("group"),
            WECOM_WS_REPLY_CMD,
        )
        .expect("metadata json");

        let metadata: WecomMessageMetadata = serde_json::from_str(&json).expect("metadata");
        assert_eq!(metadata.to_user, "");
        assert_eq!(metadata.target.as_deref(), Some("chat-1"));
        assert_eq!(metadata.chat_id.as_deref(), Some("chat-1"));
        assert_eq!(metadata.chat_type.as_deref(), Some("group"));
        assert_eq!(metadata.ws_req_id.as_deref(), Some("req-1"));
        assert_eq!(metadata.ws_chat_id.as_deref(), Some("chat-1"));
        assert_eq!(metadata.ws_chat_type.as_deref(), Some("group"));
        assert_eq!(metadata.ws_reply_cmd.as_deref(), Some(WECOM_WS_REPLY_CMD));
    }

    #[test]
    fn classify_status_update_surfaces_failures_and_ignores_noise() {
        let thinking = StatusUpdate {
            status: StatusType::Thinking,
            message: "Thinking".to_string(),
            metadata_json: "{}".to_string(),
        };
        assert!(classify_status_update(&thinking).is_none());

        let tool_started = StatusUpdate {
            status: StatusType::ToolStarted,
            message: "Tool started".to_string(),
            metadata_json: "{}".to_string(),
        };
        assert!(classify_status_update(&tool_started).is_none());

        let status_done = StatusUpdate {
            status: StatusType::Status,
            message: "Done".to_string(),
            metadata_json: "{}".to_string(),
        };
        assert!(classify_status_update(&status_done).is_none());

        let provider_error = StatusUpdate {
            status: StatusType::Status,
            message: "Provider error: upstream 502".to_string(),
            metadata_json: "{}".to_string(),
        };
        assert_eq!(
            classify_status_update(&provider_error).as_deref(),
            Some("Provider error: upstream 502")
        );

        let interrupted = StatusUpdate {
            status: StatusType::Interrupted,
            message: "".to_string(),
            metadata_json: "{}".to_string(),
        };
        assert_eq!(
            classify_status_update(&interrupted).as_deref(),
            Some("Request interrupted. Please try again.")
        );
    }

    #[test]
    fn dm_policy_open_only_bypasses_allowlist_for_private_chats() {
        assert!(dm_policy_allows_sender_without_allowlist(
            DmPolicy::Open,
            Some("single")
        ));
        assert!(dm_policy_allows_sender_without_allowlist(
            DmPolicy::Open,
            Some("private")
        ));
        assert!(!dm_policy_allows_sender_without_allowlist(
            DmPolicy::Open,
            Some("group")
        ));
        assert!(!dm_policy_allows_sender_without_allowlist(
            DmPolicy::Open,
            None
        ));
        assert!(!dm_policy_allows_sender_without_allowlist(
            DmPolicy::Pairing,
            Some("single")
        ));
    }

    #[test]
    fn sensitive_status_uses_generic_group_chat_message() {
        let approval = StatusUpdate {
            status: StatusType::ApprovalNeeded,
            message: "Approve tool call with secret context".to_string(),
            metadata_json: "{}".to_string(),
        };

        let safe = safe_group_status_content(
            &approval,
            classify_status_update(&approval).expect("approval status content"),
        );

        assert!(safe.contains("private authorization"));
        assert!(!safe.contains("secret context"));
    }

    #[test]
    fn parse_realistic_websocket_group_mixed_payload() {
        let raw = r#"{
            "cmd":"aibot_msg_callback",
            "headers":{"req_id":"req-group-1"},
            "body":{
                "msgid":"msg-group-1",
                "chatid":"wr7NnM9z0",
                "chattype":"group",
                "from":{"userid":"ZhangSan"},
                "msgtype":"mixed",
                "mixed":{
                    "msgItem":[
                        {"itemtype":"text","text":{"content":"请看这张图"}},
                        {"itemtype":"image","image":{"url":"https://openws.work.weixin.qq.com/image/1","aeskey":"dGVzdGtleQ=="}}
                    ]
                },
                "quote":{
                    "msgtype":"text",
                    "text":{"content":"上一条消息"}
                }
            }
        }"#;

        let frame: WecomWsFrame<WecomWsMessageBody> =
            serde_json::from_str(raw).expect("frame parse");
        assert_eq!(frame.headers.req_id, "req-group-1");
        assert_eq!(frame.body.msgtype, "mixed");
        assert_eq!(frame.body.chatid.as_deref(), Some("wr7NnM9z0"));
        assert_eq!(frame.body.chattype.as_deref(), Some("group"));
        assert_eq!(frame.body.from.userid, "ZhangSan");

        let (content, attachments) = websocket_mixed_content_parts(
            &frame.body.msgid,
            frame.body.mixed.as_ref().expect("mixed"),
        );
        assert_eq!(content, "请看这张图");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "msg-group-1:1:image");
        assert_eq!(
            attachments[0].source_url.as_deref(),
            Some("https://openws.work.weixin.qq.com/image/1")
        );
        assert_eq!(
            with_websocket_quote_context(content, frame.body.quote.as_ref()),
            "Quoted text: 上一条消息\n\n请看这张图"
        );
        assert_eq!(
            wecom_conversation_scope(
                &frame.body.from.userid,
                frame.body.chatid.as_deref(),
                frame.body.chattype.as_deref()
            ),
            "wecom:group:wr7NnM9z0"
        );
    }

    #[test]
    fn parse_realistic_websocket_event_payload_with_event_key_alias() {
        let raw = r#"{
            "cmd":"aibot_event_callback",
            "headers":{"req_id":"req-event-2"},
            "body":{
                "msgid":"evt-group-1",
                "chatid":"wr7NnM9z0",
                "chattype":"group",
                "from":{"userid":"LiSi"},
                "event":{
                    "eventtype":"template_card_event",
                    "eventKey":"approve_order"
                }
            }
        }"#;

        let frame: WecomWsFrame<WecomWsEventBody> = serde_json::from_str(raw).expect("event parse");
        assert_eq!(frame.headers.req_id, "req-event-2");
        assert_eq!(frame.body.event.eventtype, "template_card_event");
        assert_eq!(
            websocket_event_summary(&frame.body.event).as_deref(),
            Some("User clicked a WeCom template card action: approve_order")
        );
        assert_eq!(
            wecom_conversation_scope(
                &frame.body.from.userid,
                frame.body.chatid.as_deref(),
                frame.body.chattype.as_deref()
            ),
            "wecom:group:wr7NnM9z0"
        );
    }

    #[test]
    fn wecom_conversation_scope_splits_group_and_dm() {
        assert_eq!(
            wecom_conversation_scope("zhangsan", Some("chat-1"), Some("group")),
            "wecom:group:chat-1"
        );
        assert_eq!(
            wecom_conversation_scope("zhangsan", Some("chat-1"), Some("single")),
            "wecom:dm:zhangsan"
        );
        assert_eq!(
            wecom_conversation_scope("zhangsan", None, Some("private")),
            "wecom:dm:zhangsan"
        );
    }

    fn make_pending_bundle(
        thread_id: &str,
        content: &str,
        attachment_count: usize,
    ) -> PendingInboundBundle {
        PendingInboundBundle {
            user_id: "zhangsan".to_string(),
            user_name: None,
            thread_id: thread_id.to_string(),
            metadata_json: r#"{"to_user":"zhangsan"}"#.to_string(),
            content: content.to_string(),
            attachments: (0..attachment_count)
                .map(|idx| StoredInboundAttachment {
                    id: format!("att-{idx}"),
                    mime_type: "image/jpeg".to_string(),
                    filename: Some(format!("att-{idx}.jpg")),
                    size_bytes: Some(3),
                    extracted_text: None,
                })
                .collect(),
            flush_at_ms: 0,
        }
    }

    #[test]
    fn stored_inbound_attachment_strips_transport_sensitive_fields() {
        let inbound = InboundAttachment {
            id: "msg-1:image".to_string(),
            mime_type: "image/jpeg".to_string(),
            filename: Some("pic.jpg".to_string()),
            size_bytes: Some(3),
            source_url: Some("https://openws.work.weixin.qq.com/file".to_string()),
            storage_key: Some("tmp-key".to_string()),
            extracted_text: None,
            extras_json: r#"{"aeskey":"secret","wecom_ws_msgtype":"image"}"#.to_string(),
        };

        let stored = StoredInboundAttachment::from(inbound);
        let stored_json = serde_json::to_string(&stored).expect("stored json");
        assert!(!stored_json.contains("source_url"));
        assert!(!stored_json.contains("storage_key"));
        assert!(!stored_json.contains("aeskey"));

        let restored: InboundAttachment = stored.into();
        assert_eq!(restored.source_url, None);
        assert_eq!(restored.storage_key, None);
        assert_eq!(restored.extras_json, "{}");
    }

    #[test]
    fn pending_attachment_blob_path_uses_hashed_identifier() {
        let path = pending_attachment_blob_path("msg-1:image");
        assert!(path.starts_with("state/pending_attachment_blobs/"));
        assert!(path.ends_with(".b64"));
        assert!(!path.contains("msg-1:image"));
    }

    #[test]
    fn process_pending_inbound_bundle_buffers_attachment_only_message() {
        let mut pending = HashMap::new();
        let emitted = process_pending_inbound_bundle(
            &mut pending,
            "wecom:dm:zhangsan",
            make_pending_bundle("wecom:dm:zhangsan", "", 1),
            100,
            5_000,
        );

        assert!(emitted.is_empty());
        let stored = pending
            .get("wecom:dm:zhangsan")
            .expect("attachment-only message should be buffered");
        assert_eq!(stored.attachments.len(), 1);
        assert_eq!(stored.flush_at_ms, 5_100);
    }

    #[test]
    fn process_pending_inbound_bundle_merges_buffered_attachment_with_follow_up_text() {
        let mut pending = HashMap::new();
        let _ = process_pending_inbound_bundle(
            &mut pending,
            "wecom:dm:zhangsan",
            make_pending_bundle("wecom:dm:zhangsan", "", 1),
            100,
            5_000,
        );

        let emitted = process_pending_inbound_bundle(
            &mut pending,
            "wecom:dm:zhangsan",
            make_pending_bundle("wecom:dm:zhangsan", "请看这张图", 0),
            150,
            5_000,
        );
        assert_eq!(emitted.len(), 1);
        assert!(pending.is_empty());
        assert_eq!(emitted[0].content, "请看这张图");
        assert_eq!(emitted[0].attachments.len(), 1);
    }

    #[test]
    fn take_due_pending_inbound_bundles_only_returns_expired_entries() {
        let mut pending = HashMap::from([
            (
                "wecom:dm:zhangsan".to_string(),
                PendingInboundBundle {
                    flush_at_ms: 200,
                    ..make_pending_bundle("wecom:dm:zhangsan", "", 1)
                },
            ),
            (
                "wecom:dm:lisi".to_string(),
                PendingInboundBundle {
                    flush_at_ms: 400,
                    ..make_pending_bundle("wecom:dm:lisi", "", 1)
                },
            ),
        ]);

        let due = take_due_pending_inbound_bundles(&mut pending, 250);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].thread_id, "wecom:dm:zhangsan");
        assert!(pending.contains_key("wecom:dm:lisi"));
    }

    #[test]
    fn pairing_reply_hides_code_in_group_chat() {
        let route = PairingReplyRoute {
            req_id: "req-1".to_string(),
            reply_cmd: WECOM_WS_REPLY_CMD.to_string(),
            chat_type: Some("group".to_string()),
        };

        assert!(should_send_pairing_reply(&route, true));
        assert!(!should_send_pairing_reply(&route, false));

        let content = pairing_reply_text(&route, "ABCD1234");
        assert!(content.contains("please DM"));
        assert!(!content.contains("ABCD1234"));
    }

    #[test]
    fn pairing_reply_always_sends_code_in_private_chat() {
        let route = PairingReplyRoute {
            req_id: "req-2".to_string(),
            reply_cmd: WECOM_WS_REPLY_CMD.to_string(),
            chat_type: Some("single".to_string()),
        };

        assert!(should_send_pairing_reply(&route, true));
        assert!(should_send_pairing_reply(&route, false));

        let content = pairing_reply_text(&route, "EFGH5678");
        assert!(content.contains("EFGH5678"));
    }

    #[test]
    fn websocket_quote_context_prefixes_user_content() {
        let quote = WecomWsQuoteContent {
            msg_type: Some("text".to_string()),
            text: Some(WecomWsTextContent {
                content: "Earlier message".to_string(),
            }),
            voice: None,
            content: None,
        };

        let content = with_websocket_quote_context("Current message".to_string(), Some(&quote));
        assert_eq!(content, "Quoted text: Earlier message\n\nCurrent message");
    }

    #[test]
    fn websocket_mixed_content_parts_extract_text_and_images() {
        let mixed = WecomWsMixedContent {
            msg_item: vec![
                WecomWsMixedItem {
                    item_type: Some("text".to_string()),
                    text: Some(WecomWsTextContent {
                        content: "First line".to_string(),
                    }),
                    image: None,
                    file: None,
                    video: None,
                },
                WecomWsMixedItem {
                    item_type: Some("image".to_string()),
                    text: None,
                    image: Some(WecomWsBinaryContent {
                        url: "https://example.com/image".to_string(),
                        aeskey: Some("aes".to_string()),
                    }),
                    file: None,
                    video: None,
                },
                WecomWsMixedItem {
                    item_type: None,
                    text: Some(WecomWsTextContent {
                        content: "Second line".to_string(),
                    }),
                    image: None,
                    file: None,
                    video: None,
                },
            ],
        };

        let (content, attachments) = websocket_mixed_content_parts("msg-1", &mixed);
        assert_eq!(content, "First line\nSecond line");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, "msg-1:1:image");
        assert_eq!(
            attachments[0].source_url.as_deref(),
            Some("https://example.com/image")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&attachments[0].extras_json)
                .expect("extras json")["wecom_ws_msgtype"],
            serde_json::json!("image")
        );
        assert!(!attachments[0].extras_json.contains("aes"));
    }

    #[test]
    fn websocket_event_summary_formats_interactive_events() {
        let template_card_event = WecomWsEvent {
            eventtype: "template_card_event".to_string(),
            extra: serde_json::from_value(serde_json::json!({
                "event_key": "approve"
            }))
            .expect("template card extras"),
        };
        assert_eq!(
            websocket_event_summary(&template_card_event).as_deref(),
            Some("User clicked a WeCom template card action: approve")
        );

        let feedback_event = WecomWsEvent {
            eventtype: "feedback_event".to_string(),
            extra: serde_json::from_value(serde_json::json!({
                "score": 5
            }))
            .expect("feedback extras"),
        };
        assert_eq!(
            websocket_event_summary(&feedback_event).as_deref(),
            Some("User submitted WeCom feedback with score 5.")
        );
    }

    #[test]
    fn update_recent_message_ids_rejects_duplicates() {
        let existing = r#"["msg-1","msg-2"]"#;
        let (is_new, json) = update_recent_message_ids(Some(existing), "msg-2", 8, 100, 1_000)
            .expect("dedupe update");

        assert!(!is_new);
        let ids: Vec<RecentMessageIdEntry> = serde_json::from_str(&json).expect("ids parse");
        assert_eq!(
            ids.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(),
            vec!["msg-1", "msg-2"]
        );
    }

    #[test]
    fn update_recent_message_ids_trims_oldest_entries() {
        let existing = r#"["msg-1","msg-2","msg-3"]"#;
        let (is_new, json) = update_recent_message_ids(Some(existing), "msg-4", 3, 100, 1_000)
            .expect("dedupe update");

        assert!(is_new);
        let ids: Vec<RecentMessageIdEntry> = serde_json::from_str(&json).expect("ids parse");
        assert_eq!(
            ids.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(),
            vec!["msg-2", "msg-3", "msg-4"]
        );
    }

    #[test]
    fn update_recent_message_ids_prunes_expired_entries() {
        let existing = serde_json::to_string(&vec![
            RecentMessageIdEntry {
                id: "old".to_string(),
                seen_at_ms: 10,
            },
            RecentMessageIdEntry {
                id: "fresh".to_string(),
                seen_at_ms: 95,
            },
        ])
        .expect("serialize dedupe entries");

        let (is_new, json) = update_recent_message_ids(Some(&existing), "new", 8, 100, 20)
            .expect("dedupe update");

        assert!(is_new);
        let ids: Vec<RecentMessageIdEntry> = serde_json::from_str(&json).expect("ids parse");
        assert_eq!(
            ids.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(),
            vec!["fresh", "new"]
        );
    }

    #[test]
    fn decrypt_websocket_media_payload_round_trips_with_trimmed_key_padding() {
        let key_bytes = [11u8; 32];
        let key_base64 = BASE64_STANDARD.encode(key_bytes);
        let trimmed_key = key_base64.trim_end_matches('=');
        let plaintext = b"wecom websocket media payload";

        let ciphertext = encrypt_websocket_media_for_test(&key_bytes, plaintext);
        let decrypted =
            decrypt_websocket_media_payload(&ciphertext, trimmed_key).expect("decrypt media");

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn remove_wecom_pkcs7_padding_rejects_inconsistent_padding() {
        let data = b"hello\x04\x04\x04\x03";
        let error = remove_wecom_pkcs7_padding(data).expect_err("padding mismatch");
        assert!(error.contains("padding"));
    }

    fn make_outbound_attachment(filename: &str, mime_type: &str, size_bytes: usize) -> Attachment {
        Attachment {
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
            data: vec![0; size_bytes],
        }
    }

    #[test]
    fn base_mime_type_strips_parameters() {
        assert_eq!(base_mime_type("audio/amr; codecs=amr"), "audio/amr");
        assert_eq!(base_mime_type("image/png"), "image/png");
        assert_eq!(base_mime_type(""), "");
    }

    #[test]
    fn classify_websocket_media_maps_supported_wecom_types() {
        assert_eq!(
            classify_websocket_media(&make_outbound_attachment("photo.png", "image/png", 128)),
            OutboundMediaKind::Image
        );
        assert_eq!(
            classify_websocket_media(&make_outbound_attachment("voice.amr", "audio/amr", 128)),
            OutboundMediaKind::Voice
        );
        assert_eq!(
            classify_websocket_media(&make_outbound_attachment("clip.mp4", "video/mp4", 128)),
            OutboundMediaKind::Video
        );
        assert_eq!(
            classify_websocket_media(&make_outbound_attachment(
                "report.pdf",
                "application/pdf",
                128
            )),
            OutboundMediaKind::File
        );
    }

    #[test]
    fn classify_websocket_media_uses_filename_extension_when_mime_is_generic() {
        assert_eq!(
            classify_websocket_media(&make_outbound_attachment(
                "screenshot.jpeg",
                "application/octet-stream",
                128
            )),
            OutboundMediaKind::Image
        );
        assert_eq!(
            classify_websocket_media(&make_outbound_attachment(
                "recording.amr",
                "application/octet-stream",
                128
            )),
            OutboundMediaKind::Voice
        );
    }

    #[test]
    fn classify_websocket_media_falls_back_to_file_when_specific_media_is_too_large() {
        assert_eq!(
            classify_websocket_media(&make_outbound_attachment(
                "photo.png",
                "image/png",
                MAX_WEBSOCKET_IMAGE_BYTES + 1
            )),
            OutboundMediaKind::File
        );
        assert_eq!(
            classify_websocket_media(&make_outbound_attachment(
                "clip.mp4",
                "video/mp4",
                MAX_WEBSOCKET_VIDEO_BYTES + 1
            )),
            OutboundMediaKind::File
        );
    }

    #[test]
    fn validate_websocket_media_size_rejects_oversized_files() {
        assert!(
            validate_websocket_media_size(OutboundMediaKind::File, MAX_ATTACHMENT_BYTES).is_ok()
        );
        assert!(
            validate_websocket_media_size(OutboundMediaKind::File, MAX_ATTACHMENT_BYTES + 1)
                .is_err()
        );
    }
}
