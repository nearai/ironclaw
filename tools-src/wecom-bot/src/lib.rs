//! WeCom Group Robot Webhook WASM Tool for IronClaw.
//!
//! Sends messages to WeCom (企业微信) group chats via robot webhook.
//! Supports text and markdown message types with @mention capabilities.
//!
//! # Authentication
//!
//! Store your WeCom robot webhook key:
//! `ironclaw secret set wecom_webhook_key <key>`
//!
//! Get a key from your WeCom group robot settings.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const WEBHOOK_URL: &str = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send";
const MAX_RETRIES: u32 = 3;

struct WeComBotTool;

impl exports::near::agent::tool::Guest for WeComBotTool {
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
        "Send messages to WeCom groups via robot webhook (企业微信群机器人). Supports text \
         and markdown messages with @mention capabilities. Authentication is handled via \
         the 'wecom_webhook_key' secret injected by the host as a query parameter."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    content: Option<String>,
    #[serde(rename = "mentionedList")]
    mentioned_list: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct WeComResponse {
    errcode: Option<i32>,
    errmsg: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("参数解析失败: {e}"))?;

    if !near::agent::host::secret_exists("wecom_webhook_key") {
        return Err(
            "未找到企业微信机器人 webhook key。请使用 ironclaw secret set wecom_webhook_key <key> 设置。\
             在企业微信群中添加机器人获取 Webhook URL 中的 key。"
                .into(),
        );
    }

    match params.action.as_str() {
        "send_text" => {
            let content = params
                .content
                .ok_or_else(|| "send_text 操作需要 content 参数".to_string())?;
            send_text(&content, params.mentioned_list.as_deref())
        }
        "send_markdown" => {
            let content = params
                .content
                .ok_or_else(|| "send_markdown 操作需要 content 参数".to_string())?;
            send_markdown(&content)
        }
        other => Err(format!(
            "未知操作: '{other}'。支持的操作: send_text, send_markdown"
        )),
    }
}

fn send_text(content: &str, mentioned_list: Option<&[String]>) -> Result<String, String> {
    let mut text_obj = serde_json::json!({
        "content": content,
    });
    if let Some(mentions) = mentioned_list {
        text_obj["mentioned_list"] = serde_json::json!(mentions);
    }
    let body = serde_json::json!({
        "msgtype": "text",
        "text": text_obj,
    });
    send_webhook(&body)?;
    let output = serde_json::json!({
        "action": "send_text",
        "success": true,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn send_markdown(content: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "msgtype": "markdown",
        "markdown": {
            "content": content,
        },
    });
    send_webhook(&body)?;
    let output = serde_json::json!({
        "action": "send_markdown",
        "success": true,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn send_webhook(body: &serde_json::Value) -> Result<(), String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-WeComBot-Tool/0.1"
    });

    let body_bytes = body.to_string().into_bytes();

    let response = {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let resp = near::agent::host::http_request(
                "POST",
                WEBHOOK_URL,
                &headers.to_string(),
                Some(&body_bytes),
                None,
            )
            .map_err(|e| format!("HTTP 请求失败: {e}"))?;

            if resp.status >= 200 && resp.status < 300 {
                break resp;
            }

            if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
                near::agent::host::log(
                    near::agent::host::LogLevel::Warn,
                    &format!(
                        "WeCom Webhook 错误 {} (尝试 {}/{}), 重试中...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body_str = String::from_utf8_lossy(&resp.body);
            return Err(format!(
                "WeCom Webhook 错误 (HTTP {}): {}",
                resp.status, body_str
            ));
        }
    };

    let body_str =
        String::from_utf8(response.body).map_err(|e| format!("响应编码错误: {e}"))?;
    let wecom_resp: WeComResponse =
        serde_json::from_str(&body_str).map_err(|e| format!("响应解析失败: {e}"))?;

    if let Some(code) = wecom_resp.errcode {
        if code != 0 {
            let msg = wecom_resp.errmsg.unwrap_or_default();
            return Err(format!("企业微信机器人错误 (errcode {}): {}", code, msg));
        }
    }

    Ok(())
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "操作类型: send_text (发送文本), send_markdown (发送 Markdown)",
            "enum": ["send_text", "send_markdown"]
        },
        "content": {
            "type": "string",
            "description": "消息内容。send_text: 纯文本; send_markdown: Markdown 格式文本"
        },
        "mentionedList": {
            "type": "array",
            "items": { "type": "string" },
            "description": "@提醒的用户列表 (用户 userId)，使用 '@all' 提醒所有人 (仅 send_text)"
        }
    },
    "required": ["action", "content"],
    "additionalProperties": false
}"#;

export!(WeComBotTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_wecom_success_response() {
        let json = r#"{"errcode": 0, "errmsg": "ok"}"#;
        let resp: WeComResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.errcode, Some(0));
        assert_eq!(resp.errmsg.as_deref(), Some("ok"));
    }

    #[test]
    fn test_parse_wecom_error_response() {
        let json = r#"{"errcode": 93000, "errmsg": "invalid webhook url, hint: [xxx]"}"#;
        let resp: WeComResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.errcode, Some(93000));
        assert!(resp.errmsg.as_ref().map_or(false, |m| m.contains("invalid webhook")));
    }

    #[test]
    fn test_parse_wecom_empty_response() {
        let json = r#"{}"#;
        let resp: WeComResponse = serde_json::from_str(json).unwrap();
        assert!(resp.errcode.is_none());
        assert!(resp.errmsg.is_none());
    }

    #[test]
    fn test_text_message_body() {
        let body = serde_json::json!({
            "msgtype": "text",
            "text": {
                "content": "测试消息",
                "mentioned_list": ["user1", "@all"],
            },
        });
        let s = body.to_string();
        assert!(s.contains("测试消息"));
        assert!(s.contains("\"msgtype\":\"text\""));
        assert!(s.contains("@all"));
    }

    #[test]
    fn test_markdown_message_body() {
        let body = serde_json::json!({
            "msgtype": "markdown",
            "markdown": {
                "content": "## 标题\n> 引用文本\n- 列表项",
            },
        });
        let s = body.to_string();
        assert!(s.contains("\"msgtype\":\"markdown\""));
        assert!(s.contains("标题"));
    }

    #[test]
    fn test_text_without_mentions() {
        let body = serde_json::json!({
            "msgtype": "text",
            "text": { "content": "无@消息" },
        });
        let s = body.to_string();
        assert!(!s.contains("mentioned_list"));
    }
}
