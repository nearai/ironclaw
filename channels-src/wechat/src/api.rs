use base64::Engine as _;
use rand::RngCore;

use crate::near::agent::channel_host;
use crate::types::{
    BaseInfo, GetConfigRequest, GetConfigResponse, GetUpdatesRequest, GetUpdatesResponse,
    GetUploadUrlRequest, GetUploadUrlResponse, MessageItem, OutboundWechatMessage,
    SendMessageRequest, SendTypingRequest, SendTypingResponse, TextItem, WechatConfig,
    MESSAGE_ITEM_TEXT, MESSAGE_STATE_FINISH, MESSAGE_TYPE_BOT,
};

pub fn base_info() -> BaseInfo {
    BaseInfo {
        channel_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn ensure_trailing_slash(base_url: &str) -> String {
    if base_url.ends_with('/') {
        base_url.to_string()
    } else {
        format!("{base_url}/")
    }
}

fn random_wechat_uin() -> String {
    let mut bytes = [0_u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes)
}

fn request_headers(body: &[u8]) -> String {
    serde_json::json!({
        "Content-Type": "application/json",
        "AuthorizationType": "ilink_bot_token",
        "Authorization": "Bearer {WECHAT_BOT_TOKEN}",
        "Content-Length": body.len().to_string(),
        "X-WECHAT-UIN": random_wechat_uin(),
    })
    .to_string()
}

pub fn get_updates(
    config: &WechatConfig,
    get_updates_buf: &str,
) -> Result<GetUpdatesResponse, String> {
    get_updates_with_timeout(config, get_updates_buf, config.long_poll_timeout_ms)
}

pub fn get_updates_with_timeout(
    config: &WechatConfig,
    get_updates_buf: &str,
    timeout_ms: u32,
) -> Result<GetUpdatesResponse, String> {
    let body = serde_json::to_vec(&GetUpdatesRequest {
        get_updates_buf: get_updates_buf.to_string(),
        base_info: base_info(),
    })
    .map_err(|e| format!("Failed to encode getUpdates request: {e}"))?;
    let headers = request_headers(&body);
    let url = format!(
        "{}ilink/bot/getupdates",
        ensure_trailing_slash(&config.base_url)
    );
    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "WeChat getUpdates request: cursor_len={} timeout_ms={}",
            get_updates_buf.len(),
            timeout_ms
        ),
    );
    let response =
        channel_host::http_request("POST", &url, &headers, Some(&body), Some(timeout_ms))
            .map_err(|e| format!("getUpdates request failed: {e}"))?;

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "WeChat getUpdates response: status={} bytes={}",
            response.status,
            response.body.len()
        ),
    );

    if response.status != 200 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!("getUpdates returned {}: {}", response.status, body));
    }

    let parsed: GetUpdatesResponse = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse getUpdates response: {e}"))?;
    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "WeChat getUpdates parsed: ret={:?} errcode={:?} msg_count={} next_cursor_len={}",
            parsed.ret,
            parsed.errcode,
            parsed.msgs.len(),
            parsed.get_updates_buf.as_deref().unwrap_or_default().len()
        ),
    );
    Ok(parsed)
}

pub fn send_text_message(
    config: &WechatConfig,
    to_user_id: &str,
    text: &str,
    context_token: Option<&str>,
) -> Result<(), String> {
    let message = SendMessageRequest {
        msg: OutboundWechatMessage {
            from_user_id: String::new(),
            to_user_id: to_user_id.to_string(),
            client_id: format!("wechat-{}", channel_host::now_millis()),
            message_type: MESSAGE_TYPE_BOT,
            message_state: MESSAGE_STATE_FINISH,
            item_list: vec![MessageItem {
                r#type: Some(MESSAGE_ITEM_TEXT),
                text_item: Some(TextItem {
                    text: text.to_string(),
                }),
                image_item: None,
                voice_item: None,
                file_item: None,
                video_item: None,
            }],
            context_token: context_token.map(str::to_string),
        },
        base_info: base_info(),
    };

    send_message_request(config, &message)
}

pub fn send_message_request(
    config: &WechatConfig,
    message: &SendMessageRequest,
) -> Result<(), String> {
    let body = serde_json::to_vec(message)
        .map_err(|e| format!("Failed to encode sendMessage request: {e}"))?;
    let headers = request_headers(&body);
    let url = format!(
        "{}ilink/bot/sendmessage",
        ensure_trailing_slash(&config.base_url)
    );

    let response = channel_host::http_request("POST", &url, &headers, Some(&body), Some(15_000))
        .map_err(|e| format!("sendMessage request failed: {e}"))?;

    if response.status != 200 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "sendMessage returned {}: {}",
            response.status, body
        ));
    }

    Ok(())
}

pub fn get_upload_url(
    config: &WechatConfig,
    request: &GetUploadUrlRequest,
) -> Result<GetUploadUrlResponse, String> {
    let body = serde_json::to_vec(request)
        .map_err(|e| format!("Failed to encode getUploadUrl request: {e}"))?;
    let headers = request_headers(&body);
    let url = format!(
        "{}ilink/bot/getuploadurl",
        ensure_trailing_slash(&config.base_url)
    );

    let response = channel_host::http_request("POST", &url, &headers, Some(&body), Some(15_000))
        .map_err(|e| format!("getUploadUrl request failed: {e}"))?;

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "WeChat getUploadUrl response: status={} bytes={} media_type={} has_thumb_fields={}",
            response.status,
            response.body.len(),
            request.media_type,
            request.thumb_rawsize.is_some()
        ),
    );

    if response.status != 200 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "getUploadUrl returned {}: {}",
            response.status, body
        ));
    }

    let parsed: GetUploadUrlResponse = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse getUploadUrl response: {e}"))?;
    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "WeChat getUploadUrl parsed: has_upload_param={} has_thumb_upload_param={}",
            parsed
                .upload_param
                .as_deref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
            parsed
                .thumb_upload_param
                .as_deref()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
        ),
    );
    Ok(parsed)
}

pub fn get_config(
    config: &WechatConfig,
    ilink_user_id: &str,
    context_token: Option<&str>,
) -> Result<GetConfigResponse, String> {
    let body = serde_json::to_vec(&GetConfigRequest {
        ilink_user_id: ilink_user_id.to_string(),
        context_token: context_token.map(str::to_string),
        base_info: base_info(),
    })
    .map_err(|e| format!("Failed to encode getConfig request: {e}"))?;
    let headers = request_headers(&body);
    let url = format!(
        "{}ilink/bot/getconfig",
        ensure_trailing_slash(&config.base_url)
    );

    let response = channel_host::http_request("POST", &url, &headers, Some(&body), Some(10_000))
        .map_err(|e| format!("getConfig request failed: {e}"))?;

    if response.status != 200 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!("getConfig returned {}: {}", response.status, body));
    }

    serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse getConfig response: {e}"))
}

pub fn send_typing(
    config: &WechatConfig,
    ilink_user_id: &str,
    typing_ticket: &str,
    status: i32,
) -> Result<(), String> {
    let body = serde_json::to_vec(&SendTypingRequest {
        ilink_user_id: ilink_user_id.to_string(),
        typing_ticket: typing_ticket.to_string(),
        status,
        base_info: base_info(),
    })
    .map_err(|e| format!("Failed to encode sendTyping request: {e}"))?;
    let headers = request_headers(&body);
    let url = format!(
        "{}ilink/bot/sendtyping",
        ensure_trailing_slash(&config.base_url)
    );

    let response = channel_host::http_request("POST", &url, &headers, Some(&body), Some(10_000))
        .map_err(|e| format!("sendTyping request failed: {e}"))?;

    if response.status != 200 {
        let body = String::from_utf8_lossy(&response.body);
        return Err(format!("sendTyping returned {}: {}", response.status, body));
    }

    let parsed: SendTypingResponse = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse sendTyping response: {e}"))?;

    if !matches!(parsed.ret, Some(0)) {
        let errmsg = parsed
            .errmsg
            .as_deref()
            .unwrap_or("unknown WeChat sendTyping error");
        return Err(format!(
            "sendTyping returned ret={} errmsg={errmsg}",
            parsed.ret.unwrap_or(-1)
        ));
    }

    Ok(())
}
