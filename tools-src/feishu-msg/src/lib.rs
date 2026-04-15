//! Feishu/Lark Messaging WASM Tool for IronClaw.
//!
//! Send and list Feishu/Lark messages (飞书消息).
//! Supports sending text messages, interactive cards, and listing chat messages.
//!
//! # Authentication
//!
//! Store your Feishu tenant_access_token:
//! `ironclaw secret set feishu_access_token <token>`
//!
//! Get a token at: https://open.feishu.cn/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const BASE_URL: &str = "https://open.feishu.cn";
const MAX_RETRIES: u32 = 3;

struct FeishuMsgTool;

impl exports::near::agent::tool::Guest for FeishuMsgTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Send and list Feishu/Lark messages (飞书消息). \
         Send text messages, interactive cards, and list chat messages. \
         Authentication is handled via the 'feishu_access_token' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    chat_id: Option<String>,
    text: Option<String>,
    card_content: Option<String>,
    page_size: Option<u32>,
    page_token: Option<String>,
}

// --- Feishu IM API response types ---

#[derive(Debug, Deserialize)]
struct FeishuResponse<T> {
    code: i32,
    msg: Option<String>,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct SendMessageData {
    message_id: Option<String>,
    create_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListMessagesData {
    has_more: Option<bool>,
    page_token: Option<String>,
    items: Option<Vec<MessageItem>>,
}

#[derive(Debug, Deserialize)]
struct MessageItem {
    message_id: Option<String>,
    msg_type: Option<String>,
    create_time: Option<String>,
    sender: Option<SenderInfo>,
    body: Option<MessageBody>,
}

#[derive(Debug, Deserialize)]
struct SenderInfo {
    sender_id: Option<SenderId>,
    sender_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SenderId {
    open_id: Option<String>,
    user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageBody {
    content: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    if !near::agent::host::secret_exists("feishu_access_token") {
        return Err(
            "Feishu access token not found in secret store. Set it with: \
             ironclaw secret set feishu_access_token <token>. \
             Get a token at: https://open.feishu.cn/"
                .into(),
        );
    }

    match params.action.as_str() {
        "send_message" => send_message(&params),
        "send_card" => send_card(&params),
        "list_messages" => list_messages(&params),
        _ => Err(format!(
            "Unknown action '{}'. Expected: send_message, send_card, list_messages",
            params.action
        )),
    }
}

fn send_message(params: &Params) -> Result<String, String> {
    let chat_id = params
        .chat_id
        .as_deref()
        .ok_or("'chat_id' is required for send_message")?;
    let text = params
        .text
        .as_deref()
        .ok_or("'text' is required for send_message")?;

    if chat_id.is_empty() {
        return Err("'chat_id' must not be empty".into());
    }
    if text.is_empty() {
        return Err("'text' must not be empty".into());
    }

    let text_content = serde_json::json!({"text": text});
    let url = format!(
        "{BASE_URL}/open-apis/im/v1/messages?receive_id_type=chat_id"
    );
    let body = serde_json::json!({
        "receive_id": chat_id,
        "msg_type": "text",
        "content": text_content.to_string(),
    });

    let resp_body = feishu_request("POST", &url, Some(&body))?;
    let resp: FeishuResponse<SendMessageData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data;
    let output = serde_json::json!({
        "action": "send_message",
        "message_id": data.as_ref().and_then(|d| d.message_id.as_deref()),
        "create_time": data.as_ref().and_then(|d| d.create_time.as_deref()),
        "status": "sent",
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn send_card(params: &Params) -> Result<String, String> {
    let chat_id = params
        .chat_id
        .as_deref()
        .ok_or("'chat_id' is required for send_card")?;
    let card_content = params
        .card_content
        .as_deref()
        .ok_or("'card_content' is required for send_card")?;

    if chat_id.is_empty() {
        return Err("'chat_id' must not be empty".into());
    }
    if card_content.is_empty() {
        return Err("'card_content' must not be empty".into());
    }

    // Validate card_content is valid JSON.
    serde_json::from_str::<serde_json::Value>(card_content)
        .map_err(|e| format!("'card_content' must be valid JSON: {e}"))?;

    let url = format!(
        "{BASE_URL}/open-apis/im/v1/messages?receive_id_type=chat_id"
    );
    let body = serde_json::json!({
        "receive_id": chat_id,
        "msg_type": "interactive",
        "content": card_content,
    });

    let resp_body = feishu_request("POST", &url, Some(&body))?;
    let resp: FeishuResponse<SendMessageData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data;
    let output = serde_json::json!({
        "action": "send_card",
        "message_id": data.as_ref().and_then(|d| d.message_id.as_deref()),
        "create_time": data.as_ref().and_then(|d| d.create_time.as_deref()),
        "status": "sent",
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn list_messages(params: &Params) -> Result<String, String> {
    let chat_id = params
        .chat_id
        .as_deref()
        .ok_or("'chat_id' is required for list_messages")?;

    if chat_id.is_empty() {
        return Err("'chat_id' must not be empty".into());
    }

    let page_size = params.page_size.unwrap_or(20).clamp(1, 50);
    let mut url = format!(
        "{BASE_URL}/open-apis/im/v1/messages?container_id_type=chat&container_id={chat_id}&page_size={page_size}"
    );
    if let Some(ref page_token) = params.page_token {
        if !page_token.is_empty() {
            url.push_str(&format!("&page_token={page_token}"));
        }
    }

    let resp_body = feishu_request("GET", &url, None)?;
    let resp: FeishuResponse<ListMessagesData> =
        serde_json::from_str(&resp_body).map_err(|e| format!("Failed to parse response: {e}"))?;

    if resp.code != 0 {
        let msg = resp.msg.unwrap_or_default();
        return Err(format!("Feishu API error (code {}): {}", resp.code, msg));
    }

    let data = resp.data.unwrap_or(ListMessagesData {
        has_more: Some(false),
        page_token: None,
        items: Some(vec![]),
    });

    let messages: Vec<serde_json::Value> = data
        .items
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            let message_id = m.message_id?;
            let mut entry = serde_json::json!({"message_id": message_id});
            if let Some(msg_type) = m.msg_type {
                entry["msg_type"] = serde_json::json!(msg_type);
            }
            if let Some(create_time) = m.create_time {
                entry["create_time"] = serde_json::json!(create_time);
            }
            if let Some(sender) = m.sender {
                if let Some(sender_type) = sender.sender_type {
                    entry["sender_type"] = serde_json::json!(sender_type);
                }
                if let Some(sender_id) = sender.sender_id {
                    if let Some(open_id) = sender_id.open_id {
                        entry["sender_open_id"] = serde_json::json!(open_id);
                    }
                }
            }
            if let Some(body) = m.body {
                if let Some(content) = body.content {
                    entry["content"] = serde_json::json!(content);
                }
            }
            Some(entry)
        })
        .collect();

    let output = serde_json::json!({
        "action": "list_messages",
        "chat_id": chat_id,
        "has_more": data.has_more.unwrap_or(false),
        "page_token": data.page_token,
        "result_count": messages.len(),
        "messages": messages,
    });

    serde_json::to_string(&output).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn feishu_request(
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
) -> Result<String, String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-FeishuMsg-Tool/0.1"
    });

    let mut attempt = 0;
    loop {
        attempt += 1;

        let body_bytes = body.map(|b| b.to_string().into_bytes());
        let resp = near::agent::host::http_request(
            method,
            url,
            &headers.to_string(),
            body_bytes.as_deref(),
            None,
        )
        .map_err(|e| format!("HTTP request failed: {e}"))?;

        if resp.status >= 200 && resp.status < 300 {
            return String::from_utf8(resp.body)
                .map_err(|e| format!("Invalid UTF-8 response: {e}"));
        }

        if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!(
                    "Feishu API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        let body_str = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Feishu API error (HTTP {}): {}",
            resp.status, body_str
        ));
    }
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "The action to perform: 'send_message' (发送文本消息), 'send_card' (发送卡片消息), 'list_messages' (列出聊天消息)",
            "enum": ["send_message", "send_card", "list_messages"]
        },
        "chat_id": {
            "type": "string",
            "description": "Chat/group ID (required for all actions)"
        },
        "text": {
            "type": "string",
            "description": "Text message content (required for send_message)"
        },
        "card_content": {
            "type": "string",
            "description": "Interactive card JSON content (required for send_card)"
        },
        "page_size": {
            "type": "integer",
            "description": "Number of messages to return (1-50, default 20, for list_messages)",
            "minimum": 1,
            "maximum": 50,
            "default": 20
        },
        "page_token": {
            "type": "string",
            "description": "Pagination token for next page (optional for list_messages)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(FeishuMsgTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_send_message_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "message_id": "om_abc123",
                "create_time": "1700000000"
            }
        }"#;
        let resp: FeishuResponse<SendMessageData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.message_id.as_deref(), Some("om_abc123"));
        assert_eq!(data.create_time.as_deref(), Some("1700000000"));
    }

    #[test]
    fn test_parse_list_messages_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "has_more": true,
                "page_token": "next_page_token",
                "items": [
                    {
                        "message_id": "om_msg001",
                        "msg_type": "text",
                        "create_time": "1700000100",
                        "sender": {
                            "sender_id": {
                                "open_id": "ou_sender001",
                                "user_id": "uid001"
                            },
                            "sender_type": "user"
                        },
                        "body": {
                            "content": "{\"text\":\"你好\"}"
                        }
                    }
                ]
            }
        }"#;
        let resp: FeishuResponse<ListMessagesData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert_eq!(data.has_more, Some(true));
        assert_eq!(data.page_token.as_deref(), Some("next_page_token"));
        let items = data.items.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].message_id.as_deref(), Some("om_msg001"));
        assert_eq!(items[0].msg_type.as_deref(), Some("text"));
        let sender = items[0].sender.as_ref().unwrap();
        assert_eq!(sender.sender_type.as_deref(), Some("user"));
        assert_eq!(
            sender.sender_id.as_ref().and_then(|s| s.open_id.as_deref()),
            Some("ou_sender001")
        );
    }

    #[test]
    fn test_parse_empty_messages_response() {
        let json = r#"{
            "code": 0,
            "msg": "success",
            "data": {
                "has_more": false,
                "items": []
            }
        }"#;
        let resp: FeishuResponse<ListMessagesData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let data = resp.data.unwrap();
        assert!(data.items.unwrap().is_empty());
        assert_eq!(data.has_more, Some(false));
    }

    #[test]
    fn test_parse_error_response() {
        let json = r#"{"code": 99991663, "msg": "token invalid", "data": null}"#;
        let resp: FeishuResponse<SendMessageData> = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 99991663);
        assert_eq!(resp.msg.as_deref(), Some("token invalid"));
        assert!(resp.data.is_none());
    }
}
