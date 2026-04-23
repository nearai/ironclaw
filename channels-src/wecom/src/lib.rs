//! WeCom channel for IronClaw.
//!
//! Current shape:
//! - bot websocket is the primary session path for inbound text/events and text replies
//! - self-built app callback remains available for inbound webhook delivery
//! - Agent API remains the fallback path for proactive send and attachment send

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use aes::cipher::{block_padding::NoPadding, block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use aes::Aes256;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use cbc::Decryptor;
use md5::Md5;
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use subtle::ConstantTimeEq;
use xmlparser::{ElementEnd, Token, Tokenizer};

use exports::near::agent::channel::{
    AgentResponse, Attachment, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage, InboundAttachment};

const CHANNEL_NAME: &str = "wecom";
const OWNER_ID_PATH: &str = "owner_id";
const DM_POLICY_PATH: &str = "dm_policy";
const ALLOW_FROM_PATH: &str = "allow_from";
const API_BASE_PATH: &str = "api_base";
const CORP_ID_PATH: &str = "corp_id";
const CORP_SECRET_PATH: &str = "corp_secret";
const AGENT_ID_PATH: &str = "agent_id";
const CALLBACK_TOKEN_PATH: &str = "callback_token";
const CALLBACK_AES_KEY_PATH: &str = "callback_encoding_aes_key";
const TOKEN_PATH: &str = "tenant_access_token";
const TOKEN_EXPIRY_PATH: &str = "token_expiry";
const RECENT_MSG_IDS_PATH: &str = "recent_msg_ids";
const WEBSOCKET_EVENT_QUEUE_PATH: &str = "state/gateway_event_queue_processing";
const WEBSOCKET_MEDIA_STATE_PATH: &str = "state/websocket_media_sends";

const WECOM_WS_REPLY_CMD: &str = "aibot_respond_msg";
const WECOM_WS_WELCOME_CMD: &str = "aibot_respond_welcome_msg";
const WECOM_WS_SEND_MSG_CMD: &str = "aibot_send_msg";
const WECOM_WS_UPLOAD_MEDIA_INIT_CMD: &str = "aibot_upload_media_init";
const WECOM_WS_UPLOAD_MEDIA_CHUNK_CMD: &str = "aibot_upload_media_chunk";
const WECOM_WS_UPLOAD_MEDIA_FINISH_CMD: &str = "aibot_upload_media_finish";

const TEXT_CHUNK_LIMIT_BYTES: usize = 1800;
const STREAM_CHUNK_LIMIT_BYTES: usize = 20_000;
const WEBSOCKET_MEDIA_CHUNK_SIZE: usize = 512 * 1024;
const MAX_WEBSOCKET_MEDIA_CHUNKS: usize = 100;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const MAX_OUTBOUND_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_WEBSOCKET_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_OUTBOUND_VOICE_BYTES: usize = 2 * 1024 * 1024;
const MAX_OUTBOUND_VIDEO_BYTES: usize = 10 * 1024 * 1024;
const MAX_RECENT_MSG_IDS: usize = 256;

type Aes256CbcDec = Decryptor<Aes256>;

#[derive(Debug, Deserialize)]
struct WecomConfig {
    corp_id: Option<String>,
    corp_secret: Option<String>,
    agent_id: Option<String>,
    callback_token: Option<String>,
    callback_encoding_aes_key: Option<String>,
    api_base: Option<String>,
    owner_id: Option<String>,
    dm_policy: Option<String>,
    allow_from: Option<Vec<String>>,
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

#[derive(Debug)]
struct ParsedCallbackMessage {
    msg_id: String,
    sender_id: String,
    text: Option<String>,
    media_id: Option<String>,
    media_kind: Option<InboundMediaKind>,
    voice_recognition: Option<String>,
}

#[derive(Debug)]
struct ParsedCallbackEvent {
    event_id: String,
    sender_id: Option<String>,
    event_type: String,
    event_key: Option<String>,
    change_type: Option<String>,
}

#[derive(Debug)]
enum ParsedCallbackPayload {
    Message(ParsedCallbackMessage),
    Event(ParsedCallbackEvent),
}

#[derive(Debug, Clone)]
enum PairingReplyRoute {
    AgentApi {
        to_user: String,
    },
    Websocket {
        req_id: String,
        reply_cmd: String,
        chat_type: Option<String>,
    },
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
enum InboundMediaKind {
    Image,
    Voice,
    File,
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

    fn max_bytes(self) -> usize {
        match self {
            Self::Image => MAX_OUTBOUND_IMAGE_BYTES,
            Self::Voice => MAX_OUTBOUND_VOICE_BYTES,
            Self::Video => MAX_OUTBOUND_VIDEO_BYTES,
            Self::File => MAX_ATTACHMENT_BYTES,
        }
    }

    fn websocket_max_bytes(self) -> usize {
        match self {
            Self::Image => MAX_WEBSOCKET_IMAGE_BYTES,
            Self::Voice => MAX_OUTBOUND_VOICE_BYTES,
            Self::Video => MAX_OUTBOUND_VIDEO_BYTES,
            Self::File => MAX_ATTACHMENT_BYTES,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WecomTokenResponse {
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    errmsg: String,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WecomSendResponse {
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    errmsg: String,
}

#[derive(Debug, Deserialize)]
struct WecomUploadMediaResponse {
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    errmsg: String,
    #[serde(default)]
    media_id: Option<String>,
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

#[derive(Debug, Serialize, Deserialize)]
struct PendingWebsocketMediaSend {
    id: String,
    batch_id: String,
    chat_id: String,
    media_type: String,
    filename: String,
    data_base64: String,
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

struct WebsocketMediaStartResult {
    started: usize,
    errors: Vec<String>,
}

fn default_api_base() -> String {
    "https://qyapi.weixin.qq.com".to_string()
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

fn workspace_read_required(path: &str, label: &str) -> Result<String, String> {
    channel_host::workspace_read(path)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{label} not configured"))
}

fn parse_query(req: &IncomingHttpRequest, key: &str) -> Option<String> {
    let query: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&req.query_json).unwrap_or_default();
    query.get(key)?.as_str().map(ToOwned::to_owned)
}

type XmlFields = BTreeMap<String, String>;

fn valid_wecom_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn insert_xml_field(fields: &mut XmlFields, name: String, value: String) -> Result<(), String> {
    if fields.insert(name.clone(), value).is_some() {
        return Err(format!("duplicate XML field '{name}'"));
    }
    Ok(())
}

fn parse_wecom_xml_fields(xml: &str) -> Result<XmlFields, String> {
    let mut fields = XmlFields::new();
    let mut stack: Vec<String> = Vec::new();
    let mut current_field: Option<(String, String)> = None;
    let mut root_seen = false;

    for token in Tokenizer::from(xml) {
        match token.map_err(|e| format!("invalid XML payload: {e}"))? {
            Token::Declaration { .. }
            | Token::Comment { .. }
            | Token::ProcessingInstruction { .. } => {}
            Token::DtdStart { .. }
            | Token::EmptyDtd { .. }
            | Token::EntityDeclaration { .. }
            | Token::DtdEnd { .. } => {
                return Err("XML DTD/entity declarations are not supported".to_string());
            }
            Token::ElementStart { local, .. } => {
                let name = local.as_str();
                if !valid_wecom_xml_name(name) {
                    return Err(format!("invalid XML field name '{name}'"));
                }

                match stack.len() {
                    0 => {
                        if root_seen {
                            return Err("XML payload has multiple root elements".to_string());
                        }
                        if name != "xml" {
                            return Err(format!("unexpected XML root '{name}'"));
                        }
                        root_seen = true;
                        stack.push(name.to_string());
                    }
                    1 => {
                        if current_field.is_some() {
                            return Err(
                                "XML parser entered a new field before closing the previous field"
                                    .to_string(),
                            );
                        }
                        current_field = Some((name.to_string(), String::new()));
                        stack.push(name.to_string());
                    }
                    _ => {
                        return Err(format!("nested XML element '{name}' is not supported"));
                    }
                }
            }
            Token::Attribute { .. } => {}
            Token::ElementEnd { end, .. } => match end {
                ElementEnd::Open => {}
                ElementEnd::Empty => {
                    let Some(name) = stack.pop() else {
                        return Err("unexpected empty XML element".to_string());
                    };
                    if stack.is_empty() {
                        continue;
                    }

                    let Some((field_name, value)) = current_field.take() else {
                        return Err(format!("unexpected empty XML field '{name}'"));
                    };
                    if field_name != name {
                        return Err(format!(
                            "XML field mismatch: started '{field_name}', ended '{name}'"
                        ));
                    }
                    insert_xml_field(&mut fields, field_name, value)?;
                }
                ElementEnd::Close(_, local) => {
                    let close_name = local.as_str();
                    let Some(open_name) = stack.pop() else {
                        return Err(format!("unexpected XML closing tag '{close_name}'"));
                    };
                    if open_name != close_name {
                        return Err(format!(
                            "XML tag mismatch: started '{open_name}', ended '{close_name}'"
                        ));
                    }

                    if stack.len() == 1 {
                        let Some((field_name, value)) = current_field.take() else {
                            return Err(format!("unexpected XML field close '{close_name}'"));
                        };
                        if field_name != close_name {
                            return Err(format!(
                                "XML field mismatch: started '{field_name}', ended '{close_name}'"
                            ));
                        }
                        insert_xml_field(&mut fields, field_name, value)?;
                    }
                }
            },
            Token::Text { text } | Token::Cdata { text, .. } => {
                if let Some((_, value)) = current_field.as_mut() {
                    value.push_str(text.as_str());
                } else if !text.as_str().trim().is_empty() {
                    return Err("unexpected text outside XML fields".to_string());
                }
            }
        }
    }

    if !stack.is_empty() {
        return Err("XML payload ended before all tags were closed".to_string());
    }
    if !root_seen {
        return Err("XML payload is missing root element".to_string());
    }

    Ok(fields)
}

fn xml_field(fields: &XmlFields, tag: &str) -> Option<String> {
    fields.get(tag).cloned()
}

fn verify_callback_signature(
    token: &str,
    timestamp: &str,
    nonce: &str,
    encrypted: &str,
    signature: &str,
) -> bool {
    let mut parts = [
        token.to_string(),
        timestamp.to_string(),
        nonce.to_string(),
        encrypted.to_string(),
    ];
    parts.sort();

    let mut hasher = Sha1::new();
    hasher.update(parts.join("").as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    bool::from(digest.as_bytes().ct_eq(signature.as_bytes()))
}

fn decrypt_callback_message(
    encoding_aes_key: &str,
    encrypted: &str,
) -> Result<(String, String), String> {
    let key = BASE64_STANDARD
        .decode(format!("{encoding_aes_key}="))
        .map_err(|e| format!("Failed to decode EncodingAESKey: {e}"))?;
    if key.len() != 32 {
        return Err(format!("Unexpected AES key length: {}", key.len()));
    }

    let ciphertext = BASE64_STANDARD
        .decode(encrypted)
        .map_err(|e| format!("Failed to decode encrypted payload: {e}"))?;
    let iv = &key[..16];

    let mut buf = ciphertext;
    let plaintext = Aes256CbcDec::new_from_slices(&key, iv)
        .map_err(|e| format!("Failed to initialize callback decryptor: {e}"))?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| format!("Failed to decrypt callback payload: {e}"))?;

    if plaintext.len() < 20 {
        return Err("Decrypted callback payload too short".to_string());
    }

    let content = &plaintext[16..];
    if content.len() < 4 {
        return Err("Callback payload missing message length".to_string());
    }
    let msg_len = u32::from_be_bytes([content[0], content[1], content[2], content[3]]) as usize;
    if content.len() < 4 + msg_len {
        return Err(format!(
            "Callback payload shorter than declared message length ({msg_len})"
        ));
    }

    let xml = String::from_utf8(content[4..4 + msg_len].to_vec())
        .map_err(|e| format!("Decrypted callback XML is not UTF-8: {e}"))?;
    let corp_id = String::from_utf8(content[4 + msg_len..].to_vec())
        .map_err(|e| format!("Decrypted callback corp id is not UTF-8: {e}"))?;

    Ok((xml, corp_id))
}

fn parse_callback_message_fields_with_type(
    fields: &XmlFields,
    msg_type: &str,
) -> Option<ParsedCallbackMessage> {
    let msg_id =
        xml_field(fields, "MsgId").unwrap_or_else(|| channel_host::now_millis().to_string());
    let sender_id = xml_field(fields, "FromUserName")?;

    let mut text = None;
    let mut media_id = None;
    let mut media_kind = None;
    let mut voice_recognition = None;

    match msg_type {
        "text" => {
            text = xml_field(fields, "Content").or(Some(String::new()));
        }
        "image" => {
            media_id = xml_field(fields, "MediaId");
            media_kind = Some(InboundMediaKind::Image);
        }
        "voice" => {
            media_id = xml_field(fields, "MediaId");
            media_kind = Some(InboundMediaKind::Voice);
            voice_recognition = xml_field(fields, "Recognition");
            text = voice_recognition.clone();
        }
        "file" | "video" => {
            media_id = xml_field(fields, "MediaId");
            media_kind = Some(InboundMediaKind::File);
        }
        "location" => {
            text = Some(format_location_message(fields));
        }
        "link" => {
            text = Some(format_link_message(fields));
        }
        _ => return None,
    }

    Some(ParsedCallbackMessage {
        msg_id,
        sender_id,
        text,
        media_id,
        media_kind,
        voice_recognition,
    })
}

fn format_location_message(fields: &XmlFields) -> String {
    let label = xml_field(fields, "Label");
    let poiname = xml_field(fields, "Poiname");
    let location_x = xml_field(fields, "Location_X");
    let location_y = xml_field(fields, "Location_Y");
    let scale = xml_field(fields, "Scale");

    let mut lines = Vec::new();
    if let Some(label) = label.as_deref().filter(|value| !value.is_empty()) {
        lines.push(format!("Shared location: {label}"));
    } else if let Some(poiname) = poiname.as_deref().filter(|value| !value.is_empty()) {
        lines.push(format!("Shared location: {poiname}"));
    } else {
        lines.push("Shared location".to_string());
    }

    if let (Some(location_x), Some(location_y)) = (
        location_x.as_deref().filter(|value| !value.is_empty()),
        location_y.as_deref().filter(|value| !value.is_empty()),
    ) {
        lines.push(format!("Coordinates: {location_x}, {location_y}"));
    }
    if let Some(scale) = scale.as_deref().filter(|value| !value.is_empty()) {
        lines.push(format!("Scale: {scale}"));
    }
    if let Some(poiname) = poiname.as_deref().filter(|value| !value.is_empty()) {
        if label.as_deref() != Some(poiname) {
            lines.push(format!("POI: {poiname}"));
        }
    }

    lines.join("\n")
}

fn format_link_message(fields: &XmlFields) -> String {
    let title = xml_field(fields, "Title");
    let description = xml_field(fields, "Description");
    let url = xml_field(fields, "Url");

    let mut lines = Vec::new();
    if let Some(title) = title.as_deref().filter(|value| !value.is_empty()) {
        lines.push(format!("Shared link: {title}"));
    } else {
        lines.push("Shared link".to_string());
    }
    if let Some(description) = description.as_deref().filter(|value| !value.is_empty()) {
        lines.push(description.to_string());
    }
    if let Some(url) = url.as_deref().filter(|value| !value.is_empty()) {
        lines.push(url.to_string());
    }

    lines.join("\n")
}

fn build_callback_event_id(
    event_type: &str,
    sender_id: Option<&str>,
    create_time: Option<&str>,
    event_key: Option<&str>,
    change_type: Option<&str>,
) -> String {
    let mut parts = vec!["event".to_string(), event_type.to_string()];
    if let Some(change_type) = change_type.filter(|value| !value.is_empty()) {
        parts.push(change_type.to_string());
    }
    if let Some(event_key) = event_key.filter(|value| !value.is_empty()) {
        parts.push(event_key.to_string());
    }
    if let Some(sender_id) = sender_id.filter(|value| !value.is_empty()) {
        parts.push(sender_id.to_string());
    }
    if let Some(create_time) = create_time.filter(|value| !value.is_empty()) {
        parts.push(create_time.to_string());
    }
    parts.join(":")
}

fn parse_callback_event_fields(fields: &XmlFields) -> Option<ParsedCallbackEvent> {
    let event_type = xml_field(fields, "Event")?;
    let sender_id = xml_field(fields, "FromUserName");
    let create_time = xml_field(fields, "CreateTime");
    let event_key = xml_field(fields, "EventKey");
    let change_type = xml_field(fields, "ChangeType");
    let explicit_id = xml_field(fields, "MsgId").filter(|value| !value.is_empty());
    let event_id = explicit_id.unwrap_or_else(|| {
        build_callback_event_id(
            &event_type,
            sender_id.as_deref(),
            create_time.as_deref(),
            event_key.as_deref(),
            change_type.as_deref(),
        )
    });

    Some(ParsedCallbackEvent {
        event_id,
        sender_id,
        event_type,
        event_key,
        change_type,
    })
}

fn try_parse_callback_payload_xml(xml: &str) -> Result<Option<ParsedCallbackPayload>, String> {
    let fields = parse_wecom_xml_fields(xml)?;
    let Some(msg_type) = xml_field(&fields, "MsgType") else {
        return Ok(None);
    };
    if msg_type == "event" {
        return Ok(parse_callback_event_fields(&fields).map(ParsedCallbackPayload::Event));
    }

    Ok(parse_callback_message_fields_with_type(&fields, &msg_type)
        .map(ParsedCallbackPayload::Message))
}

#[cfg(test)]
fn parse_callback_payload_xml(xml: &str) -> Option<ParsedCallbackPayload> {
    try_parse_callback_payload_xml(xml).ok().flatten()
}

#[cfg(test)]
fn parse_callback_message_xml(xml: &str) -> Option<ParsedCallbackMessage> {
    match parse_callback_payload_xml(xml)? {
        ParsedCallbackPayload::Message(parsed) => Some(parsed),
        ParsedCallbackPayload::Event(_) => None,
    }
}

#[cfg(test)]
fn parse_callback_event_xml(xml: &str) -> Option<ParsedCallbackEvent> {
    match parse_callback_payload_xml(xml)? {
        ParsedCallbackPayload::Event(parsed) => Some(parsed),
        ParsedCallbackPayload::Message(_) => None,
    }
}

fn load_allow_from() -> Vec<String> {
    let mut allowed: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if let Ok(stored) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        allowed.extend(stored);
    }
    allowed
}

fn send_pairing_reply(route: &PairingReplyRoute, code: &str) -> Result<(), String> {
    let content = pairing_reply_text(route, code);
    match route {
        PairingReplyRoute::AgentApi { to_user } => send_text_message(to_user, &content),
        PairingReplyRoute::Websocket {
            req_id, reply_cmd, ..
        } => send_websocket_stream_reply(req_id, reply_cmd, &content),
    }
}

fn pairing_route_is_group(route: &PairingReplyRoute) -> bool {
    match route {
        PairingReplyRoute::AgentApi { .. } => false,
        PairingReplyRoute::Websocket { chat_type, .. } => {
            normalize_chat_type(chat_type.as_deref()).as_deref() == Some("group")
        }
    }
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
    pairing_reply: Option<PairingReplyRoute>,
) -> Result<bool, String> {
    let owner_id = channel_host::workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());
    if owner_id.as_deref() == Some(sender_id) {
        return Ok(true);
    }

    let dm_policy =
        channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());
    if dm_policy == "open" {
        return Ok(true);
    }

    let allowed = load_allow_from();
    if allowed
        .iter()
        .any(|entry| entry == "*" || entry == sender_id)
    {
        return Ok(true);
    }

    if dm_policy == "pairing" {
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

fn get_valid_access_token(api_base: &str) -> Result<String, String> {
    if let Some(token) = channel_host::workspace_read(TOKEN_PATH).filter(|s| !s.is_empty()) {
        if let Some(expiry_str) = channel_host::workspace_read(TOKEN_EXPIRY_PATH) {
            if let Ok(expiry) = expiry_str.parse::<u64>() {
                let now = channel_host::now_millis();
                if now + 60_000 < expiry {
                    return Ok(token);
                }
            }
        }
    }

    obtain_access_token(api_base)
}

fn obtain_access_token(api_base: &str) -> Result<String, String> {
    let corp_id = workspace_read_required(CORP_ID_PATH, "corp_id")?;
    let corp_secret = workspace_read_required(CORP_SECRET_PATH, "corp_secret")?;
    let url = format!(
        "{}/cgi-bin/gettoken?corpid={}&corpsecret={}",
        api_base.trim_end_matches('/'),
        corp_id,
        corp_secret
    );

    let response = channel_host::http_request("GET", &url, "{}", None, Some(10_000))
        .map_err(|e| format!("WeCom gettoken request failed: {e}"))?;
    if response.status != 200 {
        return Err(format!(
            "WeCom gettoken returned {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }

    let parsed: WecomTokenResponse = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse WeCom token response: {e}"))?;
    if parsed.errcode != 0 {
        return Err(format!(
            "WeCom gettoken error {}: {}",
            parsed.errcode, parsed.errmsg
        ));
    }

    let token = parsed
        .access_token
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "WeCom token response missing access_token".to_string())?;
    let expires_in = parsed.expires_in.unwrap_or(7200);
    let expiry = channel_host::now_millis().saturating_add(expires_in.saturating_mul(1000));

    let _ = channel_host::workspace_write(TOKEN_PATH, &token);
    let _ = channel_host::workspace_write(TOKEN_EXPIRY_PATH, &expiry.to_string());

    Ok(token)
}

fn send_text_message(to_user: &str, content: &str) -> Result<(), String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH).unwrap_or_else(default_api_base);
    let agent_id = workspace_read_required(AGENT_ID_PATH, "agent_id")?;
    let agent_id_num = agent_id
        .parse::<u64>()
        .map_err(|e| format!("agent_id must be numeric: {e}"))?;
    let access_token = get_valid_access_token(&api_base)?;

    for chunk in chunk_text(content, TEXT_CHUNK_LIMIT_BYTES) {
        let url = format!(
            "{}/cgi-bin/message/send?access_token={}",
            api_base.trim_end_matches('/'),
            access_token
        );
        let body = serde_json::json!({
            "touser": to_user,
            "msgtype": "text",
            "agentid": agent_id_num,
            "text": { "content": chunk },
        });
        let body_json = body.to_string();
        let response = channel_host::http_request(
            "POST",
            &url,
            &serde_json::json!({"Content-Type": "application/json"}).to_string(),
            Some(body_json.as_bytes()),
            Some(15_000),
        )
        .map_err(|e| format!("WeCom message/send request failed: {e}"))?;
        if response.status != 200 {
            return Err(format!(
                "WeCom message/send returned {}: {}",
                response.status,
                String::from_utf8_lossy(&response.body)
            ));
        }
        let parsed: WecomSendResponse = serde_json::from_slice(&response.body)
            .map_err(|e| format!("Failed to parse WeCom send response: {e}"))?;
        if parsed.errcode != 0 {
            return Err(format!(
                "WeCom message/send error {}: {}",
                parsed.errcode, parsed.errmsg
            ));
        }
    }

    Ok(())
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

fn classify_outbound_media(att: &Attachment) -> OutboundMediaKind {
    let preferred = preferred_outbound_media_kind(att);
    if att.data.len() > preferred.max_bytes() {
        OutboundMediaKind::File
    } else {
        preferred
    }
}

fn validate_outbound_media_size(
    media_kind: OutboundMediaKind,
    size_bytes: usize,
) -> Result<(), String> {
    if size_bytes > media_kind.max_bytes() {
        return Err(format!(
            "WeCom {} attachment exceeds {} bytes",
            media_kind.as_api_type(),
            media_kind.max_bytes()
        ));
    }
    Ok(())
}

fn multipart_quoted_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\r' | '\n' => escaped.push('_'),
            ch if ch.is_control() => escaped.push('_'),
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn is_mime_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '!' | '#' | '$' | '&' | '-' | '^' | '_' | '.' | '+')
}

fn safe_multipart_content_type(content_type: &str) -> String {
    let content_type = content_type.trim();
    let Some((kind, subtype)) = content_type.split_once('/') else {
        return "application/octet-stream".to_string();
    };
    if kind.is_empty()
        || subtype.is_empty()
        || !kind.chars().all(is_mime_token_char)
        || !subtype.chars().all(is_mime_token_char)
    {
        return "application/octet-stream".to_string();
    }
    content_type.to_string()
}

fn build_upload_media_multipart_body(att: &Attachment, boundary: &str) -> Vec<u8> {
    let filename = if att.filename.trim().is_empty() {
        "attachment.bin"
    } else {
        att.filename.as_str()
    };
    let filename = multipart_quoted_string(filename);
    let content_type = safe_multipart_content_type(base_mime_type(&att.mime_type));
    let header = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"media\"; filename=\"{}\"; filelength={}\r\nContent-Type: {}\r\n\r\n",
        filename,
        att.data.len(),
        content_type
    );
    let footer = format!("\r\n--{boundary}--\r\n");
    let mut body = Vec::with_capacity(header.len() + att.data.len() + footer.len());
    body.extend_from_slice(header.as_bytes());
    body.extend_from_slice(&att.data);
    body.extend_from_slice(footer.as_bytes());
    body
}

fn upload_media(att: &Attachment, media_kind: OutboundMediaKind) -> Result<String, String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH).unwrap_or_else(default_api_base);
    let access_token = get_valid_access_token(&api_base)?;
    let url = format!(
        "{}/cgi-bin/media/upload?access_token={}&type={}",
        api_base.trim_end_matches('/'),
        access_token,
        media_kind.as_api_type()
    );

    let boundary = format!("----ironclaw-wecom-{}", channel_host::now_millis());
    let body = build_upload_media_multipart_body(att, &boundary);

    let response = channel_host::http_request(
        "POST",
        &url,
        &serde_json::json!({
            "Content-Type": format!("multipart/form-data; boundary={boundary}"),
            "Content-Length": body.len().to_string(),
        })
        .to_string(),
        Some(&body),
        Some(30_000),
    )
    .map_err(|e| format!("WeCom media/upload request failed: {e}"))?;
    if response.status != 200 {
        return Err(format!(
            "WeCom media/upload returned {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }

    let parsed: WecomUploadMediaResponse = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse WeCom media/upload response: {e}"))?;
    if parsed.errcode != 0 {
        return Err(format!(
            "WeCom media/upload error {}: {}",
            parsed.errcode, parsed.errmsg
        ));
    }

    parsed
        .media_id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "WeCom media/upload response missing media_id".to_string())
}

fn send_media_message(to_user: &str, att: &Attachment) -> Result<(), String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH).unwrap_or_else(default_api_base);
    let access_token = get_valid_access_token(&api_base)?;
    let agent_id = workspace_read_required(AGENT_ID_PATH, "agent_id")?;
    let agent_id_num = agent_id
        .parse::<u64>()
        .map_err(|e| format!("agent_id must be numeric: {e}"))?;
    let preferred_kind = preferred_outbound_media_kind(att);
    let media_kind = classify_outbound_media(att);
    if preferred_kind != media_kind {
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "WeCom attachment '{}' exceeded {} message limits; sending as file instead",
                att.filename,
                preferred_kind.as_api_type()
            ),
        );
    }
    validate_outbound_media_size(media_kind, att.data.len()).map_err(|_| {
        format!(
            "WeCom {} attachment '{}' exceeds {} bytes",
            media_kind.as_api_type(),
            att.filename,
            media_kind.max_bytes()
        )
    })?;
    let media_id = upload_media(att, media_kind)?;

    let url = format!(
        "{}/cgi-bin/message/send?access_token={}",
        api_base.trim_end_matches('/'),
        access_token
    );
    let media_type = media_kind.as_api_type();
    let body = serde_json::json!({
        "touser": to_user,
        "msgtype": media_type,
        "agentid": agent_id_num,
        media_type: { "media_id": media_id },
    });
    let body_json = body.to_string();
    let response = channel_host::http_request(
        "POST",
        &url,
        &serde_json::json!({"Content-Type": "application/json"}).to_string(),
        Some(body_json.as_bytes()),
        Some(15_000),
    )
    .map_err(|e| format!("WeCom send media request failed: {e}"))?;
    if response.status != 200 {
        return Err(format!(
            "WeCom send media returned {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    let parsed: WecomSendResponse = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse WeCom media send response: {e}"))?;
    if parsed.errcode != 0 {
        return Err(format!(
            "WeCom send media error {}: {}",
            parsed.errcode, parsed.errmsg
        ));
    }

    Ok(())
}

fn download_inbound_media(
    media_id: &str,
    media_kind: InboundMediaKind,
) -> Result<InboundAttachment, String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH).unwrap_or_else(default_api_base);
    let access_token = get_valid_access_token(&api_base)?;
    let url = format!(
        "{}/cgi-bin/media/get?access_token={}&media_id={}",
        api_base.trim_end_matches('/'),
        access_token,
        media_id
    );

    let response = channel_host::http_request("GET", &url, "{}", None, Some(30_000))
        .map_err(|e| format!("WeCom media/get request failed: {e}"))?;
    if response.status != 200 {
        return Err(format!(
            "WeCom media/get returned {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    if response.body.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "WeCom media {} exceeds {} bytes",
            media_id, MAX_ATTACHMENT_BYTES
        ));
    }

    let headers: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&response.headers_json).unwrap_or_default();
    let content_type = headers
        .get("content-type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(match media_kind {
            InboundMediaKind::Image => "image/jpeg",
            InboundMediaKind::Voice => "audio/amr",
            InboundMediaKind::File => "application/octet-stream",
        })
        .to_string();
    let filename = headers
        .get("content-disposition")
        .and_then(serde_json::Value::as_str)
        .and_then(extract_filename_from_content_disposition)
        .or_else(|| Some(default_filename_for_media(media_id, media_kind)))
        .unwrap_or_else(|| media_id.to_string());

    channel_host::store_attachment_data(media_id, &response.body)
        .map_err(|e| format!("Failed to store inbound media data: {e}"))?;

    Ok(InboundAttachment {
        id: media_id.to_string(),
        mime_type: content_type,
        filename: Some(filename),
        size_bytes: Some(response.body.len() as u64),
        source_url: None,
        storage_key: None,
        extracted_text: None,
        extras_json: "{}".to_string(),
    })
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

fn default_filename_for_media(media_id: &str, media_kind: InboundMediaKind) -> String {
    let ext = match media_kind {
        InboundMediaKind::Image => "jpg",
        InboundMediaKind::Voice => "amr",
        InboundMediaKind::File => "bin",
    };
    format!("{media_id}.{ext}")
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
        channel_host::websocket_send_text(&payload)
            .map_err(|e| format!("Failed to send WeCom websocket reply: {e}"))?;
    }
    Ok(())
}

fn send_websocket_stream_reply(req_id: &str, reply_cmd: &str, content: &str) -> Result<(), String> {
    send_websocket_response(req_id, reply_cmd, content)
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

fn websocket_control_req_id(cmd: &str, seed: &str) -> String {
    let now = channel_host::now_millis();
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

fn load_pending_websocket_media_state() -> PendingWebsocketMediaState {
    let Some(raw) = channel_host::workspace_read(WEBSOCKET_MEDIA_STATE_PATH) else {
        return PendingWebsocketMediaState::default();
    };
    if raw.trim().is_empty() {
        return PendingWebsocketMediaState::default();
    }
    match serde_json::from_str(&raw) {
        Ok(state) => state,
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
    channel_host::workspace_write(WEBSOCKET_MEDIA_STATE_PATH, &json)
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
    let data = BASE64_STANDARD
        .decode(&send.data_base64)
        .map_err(|e| format!("Failed to decode pending WeCom media data: {e}"))?;
    build_websocket_command_payload(
        WECOM_WS_UPLOAD_MEDIA_INIT_CMD,
        &send.init_req_id,
        serde_json::json!({
            "type": send.media_type,
            "filename": send.filename,
            "total_size": send.total_size,
            "total_chunks": send.total_chunks,
            "md5": websocket_media_md5_hex(&data),
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
    let init_req_id = websocket_control_req_id(WECOM_WS_UPLOAD_MEDIA_INIT_CMD, &id);
    let send = PendingWebsocketMediaSend {
        id,
        batch_id: batch_id.to_string(),
        chat_id: chat_id.to_string(),
        media_type: media_kind.as_api_type().to_string(),
        filename: if attachment.filename.trim().is_empty() {
            "attachment.bin".to_string()
        } else {
            attachment.filename.clone()
        },
        data_base64: BASE64_STANDARD.encode(&attachment.data),
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
    let data = BASE64_STANDARD
        .decode(&send.data_base64)
        .map_err(|e| format!("Failed to decode pending WeCom media data: {e}"))?;
    let start = send.next_chunk_index * WEBSOCKET_MEDIA_CHUNK_SIZE;
    let end = (start + WEBSOCKET_MEDIA_CHUNK_SIZE).min(data.len());
    let chunk = data
        .get(start..end)
        .ok_or_else(|| "WeCom websocket media chunk range is invalid".to_string())?;
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
            "base64_data": BASE64_STANDARD.encode(chunk),
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

fn send_next_websocket_media_chunk(send: &mut PendingWebsocketMediaSend) -> Result<(), String> {
    let req_id = websocket_control_req_id(
        WECOM_WS_UPLOAD_MEDIA_CHUNK_CMD,
        &format!("{}:{}", send.id, send.next_chunk_index),
    );
    send.chunk_req_id = Some(req_id);
    let payload = build_websocket_media_chunk_payload(send)?;
    channel_host::websocket_send_text(&payload)
        .map_err(|e| format!("Failed to send WeCom websocket media chunk: {e}"))
}

fn send_websocket_media_finish(send: &mut PendingWebsocketMediaSend) -> Result<(), String> {
    let req_id = websocket_control_req_id(WECOM_WS_UPLOAD_MEDIA_FINISH_CMD, &send.id);
    send.finish_req_id = Some(req_id);
    let payload = build_websocket_media_finish_payload(send)?;
    channel_host::websocket_send_text(&payload)
        .map_err(|e| format!("Failed to send WeCom websocket media finish: {e}"))
}

fn send_websocket_active_media(send: &mut PendingWebsocketMediaSend) -> Result<(), String> {
    let req_id = websocket_control_req_id(WECOM_WS_SEND_MSG_CMD, &send.id);
    send.send_req_id = Some(req_id);
    let payload = build_websocket_active_media_payload(send)?;
    channel_host::websocket_send_text(&payload)
        .map_err(|e| format!("Failed to send WeCom websocket media message: {e}"))
}

fn send_websocket_active_markdown(chat_id: &str, content: &str) -> Result<(), String> {
    let content = content.trim();
    if content.is_empty() {
        return Ok(());
    }
    let payload = build_websocket_active_markdown_payload(chat_id, content)?;
    channel_host::websocket_send_text(&payload)
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
    let mut errors = Vec::new();
    for (index, attachment) in attachments.iter().enumerate() {
        match build_pending_websocket_media_send(&batch_id, &chat_id, attachment, index) {
            Ok((send, payload)) => match channel_host::websocket_send_text(&payload) {
                Ok(()) => sends.push(send),
                Err(error) => errors.push(format!(
                    "Failed to send WeCom websocket media init for '{}': {error}",
                    attachment.filename
                )),
            },
            Err(error) => errors.push(error),
        }
    }

    let started = sends.len();
    if started > 0 {
        let mut state = load_pending_websocket_media_state();
        state.batches.push(PendingWebsocketMediaBatch {
            id: batch_id,
            chat_id,
            response_req_id: response_req_id.to_string(),
            response_cmd: response_cmd.to_string(),
            final_text: content.trim().to_string(),
            remaining_media: started,
            sent_media: 0,
            failed_media: errors.len(),
            errors: errors.clone(),
        });
        state.sends.extend(sends);
        if let Err(error) = persist_pending_websocket_media_state(&state) {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!("Failed to persist WeCom websocket media state: {error}"),
            );
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
    channel_host::log(
        channel_host::LogLevel::Warn,
        &format!("WeCom websocket media send failed: {error}"),
    );
    complete_websocket_media_batch(state, &send.batch_id, Err(error));
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
            batch.errors.push(error);
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
) -> Result<Option<PendingWebsocketMediaSend>, String> {
    let req_id = ack.headers.req_id.as_str();

    if send.init_req_id == req_id {
        let upload_id = json_body_string(&ack.body, "upload_id")
            .ok_or_else(|| "WeCom websocket upload init ack missing upload_id".to_string())?;
        send.upload_id = Some(upload_id);
        send.next_chunk_index = 0;
        send_next_websocket_media_chunk(&mut send)?;
        return Ok(Some(send));
    }

    if send.chunk_req_id.as_deref() == Some(req_id) {
        send.next_chunk_index += 1;
        if send.next_chunk_index < send.total_chunks {
            send_next_websocket_media_chunk(&mut send)?;
        } else {
            send_websocket_media_finish(&mut send)?;
        }
        return Ok(Some(send));
    }

    if send.finish_req_id.as_deref() == Some(req_id) {
        let media_id = json_body_string(&ack.body, "media_id")
            .ok_or_else(|| "WeCom websocket upload finish ack missing media_id".to_string())?;
        send.media_id = Some(media_id);
        send_websocket_active_media(&mut send)?;
        return Ok(Some(send));
    }

    if send.send_req_id.as_deref() == Some(req_id) {
        return Ok(None);
    }

    Ok(Some(send))
}

fn parse_websocket_ack_frame(frame: &str) -> Option<WecomWsAckFrame> {
    let value: JsonValue = serde_json::from_str(frame).ok()?;
    value.get("errcode")?.as_i64()?;
    serde_json::from_value(value).ok()
}

fn handle_websocket_ack_frame(ack: WecomWsAckFrame) {
    let mut state = load_pending_websocket_media_state();
    let Some(pos) = state
        .sends
        .iter()
        .position(|send| pending_websocket_media_matches_req(send, &ack.headers.req_id))
    else {
        return;
    };

    let send = state.sends.remove(pos);
    let batch_id = send.batch_id.clone();
    if ack.errcode != 0 {
        let error = format!(
            "req_id={} errcode={} errmsg={}",
            ack.headers.req_id, ack.errcode, ack.errmsg
        );
        fail_pending_websocket_media(&mut state, &send, error);
    } else {
        match advance_pending_websocket_media(send, &ack) {
            Ok(Some(next)) => state.sends.push(next),
            Ok(None) => {
                complete_websocket_media_batch(&mut state, &batch_id, Ok(()));
            }
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to advance WeCom websocket media send: {error}"),
                );
                complete_websocket_media_batch(&mut state, &batch_id, Err(error));
            }
        }
    }

    if let Err(error) = persist_pending_websocket_media_state(&state) {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!("Failed to persist WeCom websocket media state: {error}"),
        );
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
        extras_json: serde_json::json!({
            "aeskey": content.aeskey,
            "wecom_ws_msgtype": msg_type,
        })
        .to_string(),
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

fn handle_websocket_message_frame(frame: WecomWsFrame<WecomWsMessageBody>) {
    let body = frame.body;
    if !should_process_message_id(&body.msgid) {
        return;
    }

    let sender_id = body.from.userid;
    match is_sender_allowed(
        &sender_id,
        Some(PairingReplyRoute::Websocket {
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

    channel_host::emit_message(&EmittedMessage {
        user_id: sender_id,
        user_name: None,
        content,
        thread_id: Some(conversation_scope),
        metadata_json,
        attachments,
    });
}

fn handle_websocket_event_frame(frame: WecomWsFrame<WecomWsEventBody>) {
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
        Some(PairingReplyRoute::Websocket {
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
                    Ok(parsed) => handle_websocket_message_frame(parsed),
                    Err(error) => channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Failed to parse WeCom websocket message frame: {error}"),
                    ),
                }
            }
            Some("aibot_event_callback") => {
                match serde_json::from_str::<WecomWsFrame<WecomWsEventBody>>(&frame) {
                    Ok(parsed) => handle_websocket_event_frame(parsed),
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
) -> Result<(bool, String), String> {
    let mut ids: Vec<String> = existing_json
        .filter(|s| !s.trim().is_empty())
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| format!("Failed to parse recent WeCom message ids: {e}"))?
        .unwrap_or_default();

    if ids.iter().any(|existing| existing == msg_id) {
        let json = serde_json::to_string(&ids)
            .map_err(|e| format!("Failed to serialize recent WeCom message ids: {e}"))?;
        return Ok((false, json));
    }

    ids.push(msg_id.to_string());
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

fn handle_callback_message(parsed: ParsedCallbackMessage) {
    if !should_process_message_id(&parsed.msg_id) {
        return;
    }

    let sender_id = parsed.sender_id.clone();
    let conversation_scope = wecom_conversation_scope(&sender_id, None, Some("private"));

    match is_sender_allowed(
        &sender_id,
        Some(PairingReplyRoute::AgentApi {
            to_user: sender_id.clone(),
        }),
    ) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("WeCom sender authorization failed: {error}"),
            );
            return;
        }
    }

    let mut attachments = Vec::new();
    if let (Some(media_id), Some(media_kind)) = (parsed.media_id.as_deref(), parsed.media_kind) {
        match download_inbound_media(media_id, media_kind) {
            Ok(mut attachment) => {
                if let Some(text) = parsed.voice_recognition.clone() {
                    attachment.extracted_text = Some(text);
                }
                attachments.push(attachment);
            }
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to download WeCom inbound media: {error}"),
                );
            }
        }
    }

    let metadata = WecomMessageMetadata {
        to_user: sender_id.clone(),
        target: Some(sender_id.clone()),
        chat_id: None,
        chat_type: Some("private".to_string()),
        source_msg_id: Some(parsed.msg_id),
        ws_req_id: None,
        ws_chat_id: None,
        ws_chat_type: None,
        ws_reply_cmd: None,
    };
    let metadata_json = match serde_json::to_string(&metadata) {
        Ok(json) => json,
        Err(error) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to serialize WeCom metadata: {error}"),
            );
            return;
        }
    };

    channel_host::emit_message(&EmittedMessage {
        user_id: sender_id,
        user_name: None,
        content: parsed.text.unwrap_or_default(),
        thread_id: Some(conversation_scope),
        metadata_json,
        attachments,
    });
}

fn handle_callback_event(event: ParsedCallbackEvent) {
    if !should_process_message_id(&event.event_id) {
        return;
    }

    let sender = event.sender_id.as_deref().unwrap_or("<unknown>");
    let mut details = vec![format!("type={}", event.event_type)];
    if let Some(change_type) = event.change_type.as_deref() {
        details.push(format!("change_type={change_type}"));
    }
    if let Some(event_key) = event.event_key.as_deref() {
        details.push(format!("event_key={event_key}"));
    }

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "Ignoring WeCom callback event from {} ({})",
            sender,
            details.join(", ")
        ),
    );
}

struct WecomChannel;

export!(WecomChannel);

impl Guest for WecomChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: WecomConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse WeCom config: {e}"))?;

        let api_base = config
            .api_base
            .unwrap_or_else(default_api_base)
            .trim_end_matches('/')
            .to_string();
        let _ = channel_host::workspace_write(API_BASE_PATH, &api_base);

        if let Some(value) = config.corp_id {
            let _ = channel_host::workspace_write(CORP_ID_PATH, &value);
        }
        if let Some(value) = config.corp_secret {
            let _ = channel_host::workspace_write(CORP_SECRET_PATH, &value);
        }
        if let Some(value) = config.agent_id {
            let _ = channel_host::workspace_write(AGENT_ID_PATH, &value);
        }
        if let Some(value) = config.callback_token {
            let _ = channel_host::workspace_write(CALLBACK_TOKEN_PATH, &value);
        }
        if let Some(value) = config.callback_encoding_aes_key {
            let _ = channel_host::workspace_write(CALLBACK_AES_KEY_PATH, &value);
        }
        let _ =
            channel_host::workspace_write(OWNER_ID_PATH, config.owner_id.as_deref().unwrap_or(""));
        let _ = channel_host::workspace_write(
            DM_POLICY_PATH,
            config.dm_policy.as_deref().unwrap_or("pairing"),
        );
        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        if channel_host::workspace_read(CORP_ID_PATH).is_some()
            && channel_host::workspace_read(CORP_SECRET_PATH).is_some()
        {
            if let Err(error) = obtain_access_token(&api_base) {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Initial WeCom token fetch failed (will retry later): {error}"),
                );
            }
        }

        let callback_enabled = channel_host::workspace_read(CALLBACK_TOKEN_PATH)
            .filter(|value| !value.trim().is_empty())
            .is_some()
            && channel_host::workspace_read(CALLBACK_AES_KEY_PATH)
                .filter(|value| !value.trim().is_empty())
                .is_some();

        Ok(ChannelConfig {
            display_name: "WeCom".to_string(),
            http_endpoints: if callback_enabled {
                vec![HttpEndpointConfig {
                    path: "/webhook/wecom".to_string(),
                    methods: vec!["GET".to_string(), "POST".to_string()],
                    require_secret: false,
                }]
            } else {
                Vec::new()
            },
            poll: None,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        let callback_token = match workspace_read_required(CALLBACK_TOKEN_PATH, "callback_token") {
            Ok(value) => value,
            Err(error) => return text_response(500, &error),
        };
        let callback_aes_key =
            match workspace_read_required(CALLBACK_AES_KEY_PATH, "callback_encoding_aes_key") {
                Ok(value) => value,
                Err(error) => return text_response(500, &error),
            };
        let configured_corp_id = channel_host::workspace_read(CORP_ID_PATH).unwrap_or_default();

        let signature = parse_query(&req, "msg_signature").unwrap_or_default();
        let timestamp = parse_query(&req, "timestamp").unwrap_or_default();
        let nonce = parse_query(&req, "nonce").unwrap_or_default();

        if req.method == "GET" {
            let echostr = parse_query(&req, "echostr").unwrap_or_default();
            if echostr.is_empty() {
                return text_response(400, "missing echostr");
            }
            if !verify_callback_signature(&callback_token, &timestamp, &nonce, &echostr, &signature)
            {
                return text_response(403, "forbidden");
            }
            let (echo, corp_id) = match decrypt_callback_message(&callback_aes_key, &echostr) {
                Ok(value) => value,
                Err(error) => return text_response(400, &error),
            };
            if !configured_corp_id.is_empty() && corp_id != configured_corp_id {
                return text_response(403, "corp_id mismatch");
            }
            return text_response(200, &echo);
        }

        if req.method != "POST" {
            return text_response(405, "method not allowed");
        }

        let body_str = match std::str::from_utf8(&req.body) {
            Ok(value) => value,
            Err(_) => return text_response(400, "invalid utf-8 body"),
        };
        let outer_fields = match parse_wecom_xml_fields(body_str) {
            Ok(fields) => fields,
            Err(error) => return text_response(400, &error),
        };
        let encrypted = match xml_field(&outer_fields, "Encrypt") {
            Some(value) => value,
            None => return text_response(400, "missing Encrypt field"),
        };

        if !verify_callback_signature(&callback_token, &timestamp, &nonce, &encrypted, &signature) {
            return text_response(403, "forbidden");
        }

        let (inner_xml, corp_id) = match decrypt_callback_message(&callback_aes_key, &encrypted) {
            Ok(value) => value,
            Err(error) => return text_response(400, &error),
        };
        if !configured_corp_id.is_empty() && corp_id != configured_corp_id {
            return text_response(403, "corp_id mismatch");
        }

        let parsed = match try_parse_callback_payload_xml(&inner_xml) {
            Ok(value) => value,
            Err(error) => return text_response(400, &error),
        };

        if let Some(parsed) = parsed {
            match parsed {
                ParsedCallbackPayload::Message(message) => handle_callback_message(message),
                ParsedCallbackPayload::Event(event) => handle_callback_event(event),
            }
        }

        text_response(200, "success")
    }

    fn on_poll() {
        process_websocket_event_queue();
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

        for attachment in &response.attachments {
            send_media_message(&metadata.to_user, attachment)?;
        }

        if !response.content.trim().is_empty() {
            send_text_message(&metadata.to_user, &response.content)?;
        }

        Ok(())
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        for attachment in &response.attachments {
            send_media_message(&user_id, attachment)?;
        }
        if !response.content.trim().is_empty() {
            send_text_message(&user_id, &response.content)?;
        }
        Ok(())
    }

    fn on_status(_update: StatusUpdate) {}

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

    fn encrypt_callback_message_for_test(key_bytes: &[u8; 32], xml: &str, corp_id: &str) -> String {
        let mut payload = vec![b'X'; 16];
        payload.extend_from_slice(&(xml.len() as u32).to_be_bytes());
        payload.extend_from_slice(xml.as_bytes());
        payload.extend_from_slice(corp_id.as_bytes());

        let iv = &key_bytes[..16];
        let mut buf = payload.clone();
        let msg_len = buf.len();
        let block_size = 16;
        let padded_len = ((msg_len / block_size) + 1) * block_size;
        buf.resize(padded_len, 0);

        let ciphertext = Aes256CbcEnc::new_from_slices(key_bytes, iv)
            .expect("encryptor")
            .encrypt_padded_mut::<Pkcs7>(&mut buf, msg_len)
            .expect("encrypt")
            .to_vec();

        BASE64_STANDARD.encode(ciphertext)
    }

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

    #[test]
    fn capabilities_expose_expected_webhook_path_and_bot_first_setup() {
        let caps: serde_json::Value =
            serde_json::from_str(WECOM_CAPABILITIES_JSON).expect("capabilities parse");
        assert_eq!(
            caps["capabilities"]["channel"]["allowed_paths"][0],
            serde_json::Value::String("/webhook/wecom".to_string())
        );
        assert_eq!(
            caps["capabilities"]["channel"]["webhook"]["methods"],
            serde_json::json!(["GET", "POST"])
        );
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
            .any(|entry| entry["name"] == "wecom_corp_id" && entry["optional"] == true));
        assert!(required
            .iter()
            .any(|entry| entry["name"] == "wecom_callback_encoding_aes_key"
                && entry["optional"] == true));
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
        let mut send = PendingWebsocketMediaSend {
            id: "send-1".to_string(),
            batch_id: "batch-1".to_string(),
            chat_id: "ZhangSan".to_string(),
            media_type: "image".to_string(),
            filename: "cat.jpg".to_string(),
            data_base64: BASE64_STANDARD.encode(b"abc"),
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

    #[test]
    fn pairing_reply_hides_code_in_group_chat() {
        let route = PairingReplyRoute::Websocket {
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
        let route = PairingReplyRoute::Websocket {
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
                .expect("extras json")["aeskey"],
            serde_json::json!("aes")
        );
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
    fn parse_text_callback_message_xml_extracts_sender_and_text() {
        let xml = r#"
<xml>
  <ToUserName><![CDATA[ww123]]></ToUserName>
  <FromUserName><![CDATA[zhangsan]]></FromUserName>
  <CreateTime>1710000000</CreateTime>
  <MsgType><![CDATA[text]]></MsgType>
  <Content><![CDATA[hello wecom]]></Content>
  <MsgId>123456789</MsgId>
</xml>
"#;

        let parsed = parse_callback_message_xml(xml).expect("parsed");
        assert_eq!(parsed.sender_id, "zhangsan");
        assert_eq!(parsed.msg_id, "123456789");
        assert_eq!(parsed.text.as_deref(), Some("hello wecom"));
        assert!(parsed.media_id.is_none());
    }

    #[test]
    fn parse_text_callback_message_xml_preserves_markup_inside_cdata() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[zhangsan]]></FromUserName>
  <MsgType><![CDATA[text]]></MsgType>
  <Content><![CDATA[hello <b>wecom</b> & raw text]]></Content>
  <MsgId>markup-1</MsgId>
</xml>
"#;

        let parsed = parse_callback_message_xml(xml).expect("parsed");
        assert_eq!(
            parsed.text.as_deref(),
            Some("hello <b>wecom</b> & raw text")
        );
    }

    #[test]
    fn parse_callback_payload_xml_rejects_duplicate_fields() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[zhangsan]]></FromUserName>
  <MsgType><![CDATA[text]]></MsgType>
  <MsgType><![CDATA[event]]></MsgType>
  <Content><![CDATA[hello]]></Content>
</xml>
"#;

        let error = try_parse_callback_payload_xml(xml).expect_err("duplicate field should fail");
        assert!(error.contains("duplicate XML field"));
    }

    #[test]
    fn parse_callback_payload_xml_rejects_cdata_tag_injection() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[zhangsan]]></FromUserName>
  <MsgType><![CDATA[text]]></MsgType>
  <Content><![CDATA[safe]]></Content><![CDATA[</Content><MsgId>evil</MsgId>]]>
  <MsgId>msg-1</MsgId>
</xml>
"#;

        let error = try_parse_callback_payload_xml(xml).expect_err("malformed XML should fail");
        assert!(error.contains("unexpected text outside XML fields"));
    }

    #[test]
    fn parse_callback_payload_xml_rejects_nested_field_elements() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[zhangsan]]></FromUserName>
  <MsgType><![CDATA[text]]></MsgType>
  <Content><b>hello</b></Content>
  <MsgId>msg-1</MsgId>
</xml>
"#;

        let error = try_parse_callback_payload_xml(xml).expect_err("nested XML should fail");
        assert!(error.contains("nested XML element"));
    }

    #[test]
    fn parse_voice_callback_message_xml_uses_recognition_text() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[lisi]]></FromUserName>
  <MsgType><![CDATA[voice]]></MsgType>
  <MediaId><![CDATA[media_voice_1]]></MediaId>
  <Recognition><![CDATA[voice transcript]]></Recognition>
  <MsgId>voice-1</MsgId>
</xml>
"#;

        let parsed = parse_callback_message_xml(xml).expect("parsed");
        assert_eq!(parsed.media_id.as_deref(), Some("media_voice_1"));
        assert_eq!(parsed.media_kind, Some(InboundMediaKind::Voice));
        assert_eq!(parsed.text.as_deref(), Some("voice transcript"));
    }

    #[test]
    fn parse_video_callback_message_xml_maps_to_file_kind() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[wangwu]]></FromUserName>
  <MsgType><![CDATA[video]]></MsgType>
  <MediaId><![CDATA[media_video_1]]></MediaId>
  <MsgId>video-1</MsgId>
</xml>
"#;

        let parsed = parse_callback_message_xml(xml).expect("parsed");
        assert_eq!(parsed.media_kind, Some(InboundMediaKind::File));
        assert_eq!(parsed.media_id.as_deref(), Some("media_video_1"));
    }

    #[test]
    fn parse_location_callback_message_xml_formats_text_content() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[wangwu]]></FromUserName>
  <MsgType><![CDATA[location]]></MsgType>
  <Location_X>31.2304</Location_X>
  <Location_Y>121.4737</Location_Y>
  <Scale>15</Scale>
  <Label><![CDATA[Shanghai Tower]]></Label>
  <Poiname><![CDATA[Lujiazui]]></Poiname>
  <MsgId>location-1</MsgId>
</xml>
"#;

        let parsed = parse_callback_message_xml(xml).expect("parsed");
        assert_eq!(parsed.msg_id, "location-1");
        assert_eq!(
            parsed.text.as_deref(),
            Some(
                "Shared location: Shanghai Tower\nCoordinates: 31.2304, 121.4737\nScale: 15\nPOI: Lujiazui"
            )
        );
        assert!(parsed.media_id.is_none());
    }

    #[test]
    fn parse_link_callback_message_xml_formats_text_content() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[zhaoliu]]></FromUserName>
  <MsgType><![CDATA[link]]></MsgType>
  <Title><![CDATA[IronClaw Docs]]></Title>
  <Description><![CDATA[Setup guide]]></Description>
  <Url><![CDATA[https://example.com/docs]]></Url>
  <MsgId>link-1</MsgId>
</xml>
"#;

        let parsed = parse_callback_message_xml(xml).expect("parsed");
        assert_eq!(parsed.msg_id, "link-1");
        assert_eq!(
            parsed.text.as_deref(),
            Some("Shared link: IronClaw Docs\nSetup guide\nhttps://example.com/docs")
        );
        assert!(parsed.media_id.is_none());
    }

    #[test]
    fn parse_event_callback_message_xml_is_ignored() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[wangwu]]></FromUserName>
  <MsgType><![CDATA[event]]></MsgType>
  <Event><![CDATA[enter_agent]]></Event>
</xml>
"#;

        assert!(parse_callback_message_xml(xml).is_none());
    }

    #[test]
    fn parse_event_callback_xml_extracts_enter_agent_fields() {
        let xml = r#"
<xml>
  <ToUserName><![CDATA[ww123]]></ToUserName>
  <FromUserName><![CDATA[zhangsan]]></FromUserName>
  <CreateTime>1710000001</CreateTime>
  <MsgType><![CDATA[event]]></MsgType>
  <Event><![CDATA[enter_agent]]></Event>
  <AgentID>1000002</AgentID>
</xml>
"#;

        let parsed = parse_callback_event_xml(xml).expect("parsed");
        assert_eq!(parsed.sender_id.as_deref(), Some("zhangsan"));
        assert_eq!(parsed.event_type, "enter_agent");
        assert_eq!(parsed.event_key, None);
        assert_eq!(parsed.change_type, None);
        assert_eq!(parsed.event_id, "event:enter_agent:zhangsan:1710000001");
    }

    #[test]
    fn parse_event_callback_xml_uses_event_key_and_change_type_in_dedupe_id() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[zhangsan]]></FromUserName>
  <CreateTime>1710000002</CreateTime>
  <MsgType><![CDATA[event]]></MsgType>
  <Event><![CDATA[change_contact]]></Event>
  <ChangeType><![CDATA[update_user]]></ChangeType>
  <EventKey><![CDATA[userid42]]></EventKey>
</xml>
"#;

        let parsed = parse_callback_event_xml(xml).expect("parsed");
        assert_eq!(parsed.event_type, "change_contact");
        assert_eq!(parsed.change_type.as_deref(), Some("update_user"));
        assert_eq!(parsed.event_key.as_deref(), Some("userid42"));
        assert_eq!(
            parsed.event_id,
            "event:change_contact:update_user:userid42:zhangsan:1710000002"
        );
    }

    #[test]
    fn parse_callback_payload_xml_routes_event_payloads() {
        let xml = r#"
<xml>
  <FromUserName><![CDATA[lisi]]></FromUserName>
  <CreateTime>1710000003</CreateTime>
  <MsgType><![CDATA[event]]></MsgType>
  <Event><![CDATA[click]]></Event>
  <EventKey><![CDATA[menu.help]]></EventKey>
</xml>
"#;

        let parsed = parse_callback_payload_xml(xml).expect("parsed");
        match parsed {
            ParsedCallbackPayload::Event(event) => {
                assert_eq!(event.sender_id.as_deref(), Some("lisi"));
                assert_eq!(event.event_type, "click");
                assert_eq!(event.event_key.as_deref(), Some("menu.help"));
            }
            ParsedCallbackPayload::Message(_) => panic!("expected event payload"),
        }
    }

    #[test]
    fn update_recent_message_ids_rejects_duplicates() {
        let existing = r#"["msg-1","msg-2"]"#;
        let (is_new, json) =
            update_recent_message_ids(Some(existing), "msg-2", 8).expect("dedupe update");

        assert!(!is_new);
        let ids: Vec<String> = serde_json::from_str(&json).expect("ids parse");
        assert_eq!(ids, vec!["msg-1".to_string(), "msg-2".to_string()]);
    }

    #[test]
    fn update_recent_message_ids_trims_oldest_entries() {
        let existing = r#"["msg-1","msg-2","msg-3"]"#;
        let (is_new, json) =
            update_recent_message_ids(Some(existing), "msg-4", 3).expect("dedupe update");

        assert!(is_new);
        let ids: Vec<String> = serde_json::from_str(&json).expect("ids parse");
        assert_eq!(
            ids,
            vec![
                "msg-2".to_string(),
                "msg-3".to_string(),
                "msg-4".to_string()
            ]
        );
    }

    #[test]
    fn decrypt_callback_message_round_trips_xml_and_corp_id() {
        let key_bytes = [7u8; 32];
        let encoding_aes_key = BASE64_STANDARD.encode(key_bytes);
        let encoding_aes_key = encoding_aes_key.trim_end_matches('=').to_string();
        let xml = "<xml><MsgType><![CDATA[text]]></MsgType></xml>";
        let corp_id = "ww123456";

        let encrypted = encrypt_callback_message_for_test(&key_bytes, xml, corp_id);
        let (decrypted_xml, decrypted_corp_id) =
            decrypt_callback_message(&encoding_aes_key, &encrypted).expect("decrypt");

        assert_eq!(decrypted_xml, xml);
        assert_eq!(decrypted_corp_id, corp_id);
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

    #[test]
    fn verify_callback_signature_matches_wecom_sorting() {
        let token = "test-token";
        let timestamp = "1711111111";
        let nonce = "nonce-123";
        let encrypted = "ciphertext";
        let mut parts = [
            token.to_string(),
            timestamp.to_string(),
            nonce.to_string(),
            encrypted.to_string(),
        ];
        parts.sort();

        let mut hasher = Sha1::new();
        hasher.update(parts.join("").as_bytes());
        let signature = format!("{:x}", hasher.finalize());

        assert!(verify_callback_signature(
            token, timestamp, nonce, encrypted, &signature
        ));
        assert!(!verify_callback_signature(
            token, timestamp, nonce, encrypted, "deadbeef"
        ));
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
    fn multipart_quoted_string_escapes_header_breaking_filename_chars() {
        assert_eq!(
            multipart_quoted_string("a\"b\\c\r\nx.png"),
            "a\\\"b\\\\c__x.png"
        );
    }

    #[test]
    fn upload_media_multipart_body_sanitizes_headers() {
        let att = Attachment {
            filename: "a\"b\\c\r\nX-Bad: y.png".to_string(),
            mime_type: "image/png\r\nX-Bad: y".to_string(),
            data: b"abc".to_vec(),
        };

        let body = build_upload_media_multipart_body(&att, "test-boundary");
        let body = String::from_utf8_lossy(&body);

        assert!(body.contains(
            "Content-Disposition: form-data; name=\"media\"; filename=\"a\\\"b\\\\c__X-Bad: y.png\"; filelength=3"
        ));
        assert!(body.contains("Content-Type: application/octet-stream\r\n\r\nabc"));
        assert!(!body.contains("filename=\"a\"b\\c\r\nX-Bad: y.png\""));
        assert!(!body.contains("Content-Type: image/png\r\nX-Bad: y"));
    }

    #[test]
    fn classify_outbound_media_maps_supported_wecom_types() {
        assert_eq!(
            classify_outbound_media(&make_outbound_attachment("photo.png", "image/png", 128)),
            OutboundMediaKind::Image
        );
        assert_eq!(
            classify_outbound_media(&make_outbound_attachment("voice.amr", "audio/amr", 128)),
            OutboundMediaKind::Voice
        );
        assert_eq!(
            classify_outbound_media(&make_outbound_attachment("clip.mp4", "video/mp4", 128)),
            OutboundMediaKind::Video
        );
        assert_eq!(
            classify_outbound_media(&make_outbound_attachment(
                "report.pdf",
                "application/pdf",
                128
            )),
            OutboundMediaKind::File
        );
    }

    #[test]
    fn classify_outbound_media_uses_filename_extension_when_mime_is_generic() {
        assert_eq!(
            classify_outbound_media(&make_outbound_attachment(
                "screenshot.jpeg",
                "application/octet-stream",
                128
            )),
            OutboundMediaKind::Image
        );
        assert_eq!(
            classify_outbound_media(&make_outbound_attachment(
                "recording.amr",
                "application/octet-stream",
                128
            )),
            OutboundMediaKind::Voice
        );
    }

    #[test]
    fn classify_outbound_media_falls_back_to_file_when_specific_media_is_too_large() {
        assert_eq!(
            classify_outbound_media(&make_outbound_attachment(
                "photo.png",
                "image/png",
                MAX_OUTBOUND_IMAGE_BYTES + 1
            )),
            OutboundMediaKind::File
        );
        assert_eq!(
            classify_outbound_media(&make_outbound_attachment(
                "clip.mp4",
                "video/mp4",
                MAX_OUTBOUND_VIDEO_BYTES + 1
            )),
            OutboundMediaKind::File
        );
    }

    #[test]
    fn validate_outbound_media_size_rejects_oversized_files() {
        assert!(
            validate_outbound_media_size(OutboundMediaKind::File, MAX_ATTACHMENT_BYTES).is_ok()
        );
        assert!(
            validate_outbound_media_size(OutboundMediaKind::File, MAX_ATTACHMENT_BYTES + 1)
                .is_err()
        );
    }
}
