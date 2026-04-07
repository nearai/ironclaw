//! WeCom self-built app callback channel for IronClaw.
//!
//! This initial MVP uses WeCom's HTTP callback + Agent API path:
//! - inbound messages arrive through `/webhook/wecom`
//! - outbound replies use `cgi-bin/message/send`
//! - media uploads use `cgi-bin/media/upload`
//!
//! The official OpenClaw WeCom plugin primarily uses the bot websocket path.
//! IronClaw's current WASM websocket runtime is still Discord-shaped, so this
//! channel starts with the callback path while we evaluate a more generic WS
//! runtime later.

wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use aes::Aes256;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use cbc::Decryptor;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use subtle::ConstantTimeEq;

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

const TEXT_CHUNK_LIMIT_BYTES: usize = 1800;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const MAX_OUTBOUND_IMAGE_BYTES: usize = 2 * 1024 * 1024;
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
    source_msg_id: Option<String>,
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

fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let cdata_open = format!("<{tag}><![CDATA[");
    let cdata_close = format!("]]></{tag}>");
    if let Some(start) = xml.find(&cdata_open) {
        let value_start = start + cdata_open.len();
        if let Some(end_rel) = xml[value_start..].find(&cdata_close) {
            return Some(xml[value_start..value_start + end_rel].to_string());
        }
    }

    let plain_open = format!("<{tag}>");
    let plain_close = format!("</{tag}>");
    if let Some(start) = xml.find(&plain_open) {
        let value_start = start + plain_open.len();
        if let Some(end_rel) = xml[value_start..].find(&plain_close) {
            return Some(xml[value_start..value_start + end_rel].to_string());
        }
    }

    None
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

fn parse_callback_message_xml_with_type(
    xml: &str,
    msg_type: &str,
) -> Option<ParsedCallbackMessage> {
    let msg_id =
        extract_xml_value(xml, "MsgId").unwrap_or_else(|| channel_host::now_millis().to_string());
    let sender_id = extract_xml_value(xml, "FromUserName")?;

    let mut text = None;
    let mut media_id = None;
    let mut media_kind = None;
    let mut voice_recognition = None;

    match msg_type {
        "text" => {
            text = extract_xml_value(xml, "Content").or(Some(String::new()));
        }
        "image" => {
            media_id = extract_xml_value(xml, "MediaId");
            media_kind = Some(InboundMediaKind::Image);
        }
        "voice" => {
            media_id = extract_xml_value(xml, "MediaId");
            media_kind = Some(InboundMediaKind::Voice);
            voice_recognition = extract_xml_value(xml, "Recognition");
            text = voice_recognition.clone();
        }
        "file" | "video" => {
            media_id = extract_xml_value(xml, "MediaId");
            media_kind = Some(InboundMediaKind::File);
        }
        "location" => {
            text = Some(format_location_message(xml));
        }
        "link" => {
            text = Some(format_link_message(xml));
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

fn format_location_message(xml: &str) -> String {
    let label = extract_xml_value(xml, "Label");
    let poiname = extract_xml_value(xml, "Poiname");
    let location_x = extract_xml_value(xml, "Location_X");
    let location_y = extract_xml_value(xml, "Location_Y");
    let scale = extract_xml_value(xml, "Scale");

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

fn format_link_message(xml: &str) -> String {
    let title = extract_xml_value(xml, "Title");
    let description = extract_xml_value(xml, "Description");
    let url = extract_xml_value(xml, "Url");

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

fn parse_callback_event_xml(xml: &str) -> Option<ParsedCallbackEvent> {
    let event_type = extract_xml_value(xml, "Event")?;
    let sender_id = extract_xml_value(xml, "FromUserName");
    let create_time = extract_xml_value(xml, "CreateTime");
    let event_key = extract_xml_value(xml, "EventKey");
    let change_type = extract_xml_value(xml, "ChangeType");
    let explicit_id = extract_xml_value(xml, "MsgId").filter(|value| !value.is_empty());
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

fn parse_callback_payload_xml(xml: &str) -> Option<ParsedCallbackPayload> {
    let msg_type = extract_xml_value(xml, "MsgType")?;
    if msg_type == "event" {
        return parse_callback_event_xml(xml).map(ParsedCallbackPayload::Event);
    }

    parse_callback_message_xml_with_type(xml, &msg_type).map(ParsedCallbackPayload::Message)
}

#[cfg(test)]
fn parse_callback_message_xml(xml: &str) -> Option<ParsedCallbackMessage> {
    match parse_callback_payload_xml(xml)? {
        ParsedCallbackPayload::Message(parsed) => Some(parsed),
        ParsedCallbackPayload::Event(_) => None,
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

fn is_sender_allowed(sender_id: &str) -> Result<bool, String> {
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
        if result.created {
            let _ = send_text_message(
                sender_id,
                &format!(
                    "This WeCom channel requires approval before chatting. Pairing code: {}",
                    result.code
                ),
            );
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

fn upload_media(att: &Attachment, media_kind: OutboundMediaKind) -> Result<String, String> {
    let api_base = channel_host::workspace_read(API_BASE_PATH).unwrap_or_else(default_api_base);
    let access_token = get_valid_access_token(&api_base)?;
    let url = format!(
        "{}/cgi-bin/media/upload?access_token={}&type={}",
        api_base.trim_end_matches('/'),
        access_token,
        media_kind.as_api_type()
    );
    let content_type = base_mime_type(&att.mime_type);

    let boundary = format!("----ironclaw-wecom-{}", channel_host::now_millis());
    let header = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"media\"; filename=\"{}\"; filelength={}\r\nContent-Type: {}\r\n\r\n",
        att.filename,
        att.data.len(),
        if content_type.is_empty() {
            "application/octet-stream"
        } else {
            content_type
        }
    );
    let footer = format!("\r\n--{boundary}--\r\n");
    let mut body = Vec::with_capacity(header.len() + att.data.len() + footer.len());
    body.extend_from_slice(header.as_bytes());
    body.extend_from_slice(&att.data);
    body.extend_from_slice(footer.as_bytes());

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

    match is_sender_allowed(&sender_id) {
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
        source_msg_id: Some(parsed.msg_id),
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
        thread_id: None,
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

        Ok(ChannelConfig {
            display_name: "WeCom".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/wecom".to_string(),
                methods: vec!["GET".to_string(), "POST".to_string()],
                require_secret: false,
            }],
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
        let encrypted = match extract_xml_value(body_str, "Encrypt") {
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

        if let Some(parsed) = parse_callback_payload_xml(&inner_xml) {
            match parsed {
                ParsedCallbackPayload::Message(message) => handle_callback_message(message),
                ParsedCallbackPayload::Event(event) => handle_callback_event(event),
            }
        }

        text_response(200, "success")
    }

    fn on_poll() {}

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: WecomMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse WeCom response metadata: {e}"))?;

        if !response.content.trim().is_empty() {
            send_text_message(&metadata.to_user, &response.content)?;
        }

        for attachment in &response.attachments {
            send_media_message(&metadata.to_user, attachment)?;
        }

        Ok(())
    }

    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        if !response.content.trim().is_empty() {
            send_text_message(&user_id, &response.content)?;
        }
        for attachment in &response.attachments {
            send_media_message(&user_id, attachment)?;
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

    #[test]
    fn capabilities_expose_expected_webhook_path_and_required_secrets() {
        let caps: serde_json::Value =
            serde_json::from_str(WECOM_CAPABILITIES_JSON).expect("capabilities parse");
        assert_eq!(
            caps["capabilities"]["channel"]["allowed_paths"][0],
            serde_json::Value::String("/webhook/wecom".to_string())
        );
        let required = caps["setup"]["required_secrets"]
            .as_array()
            .expect("required secrets array");
        assert!(required
            .iter()
            .any(|entry| entry["name"] == "wecom_corp_id"));
        assert!(required
            .iter()
            .any(|entry| entry["name"] == "wecom_callback_encoding_aes_key"));
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
