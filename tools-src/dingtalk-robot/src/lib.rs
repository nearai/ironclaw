//! DingTalk Robot Webhook WASM Tool for IronClaw.
//!
//! Sends messages to DingTalk group chats via custom robot webhook (钉钉自定义机器人).
//! Supports text, markdown, and action card message types.
//!
//! # Authentication
//!
//! Store your DingTalk robot webhook token:
//! `ironclaw secret set dingtalk_webhook_token <token>`
//!
//! Get a token from your DingTalk group robot settings.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const WEBHOOK_URL: &str = "https://oapi.dingtalk.com/robot/send";
const MAX_RETRIES: u32 = 3;

struct DingTalkRobotTool;

impl exports::near::agent::tool::Guest for DingTalkRobotTool {
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
        "Send messages to DingTalk groups via custom robot webhook (钉钉自定义机器人). \
         Supports text, markdown, and action card messages. Authentication is handled via \
         the 'dingtalk_webhook_token' secret injected by the host as a query parameter."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
struct Params {
    action: String,
    content: Option<String>,
    title: Option<String>,
    text: Option<String>,
    #[serde(rename = "singleTitle")]
    single_title: Option<String>,
    #[serde(rename = "singleURL")]
    single_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WebhookResponse {
    errcode: Option<i32>,
    errmsg: Option<String>,
}

fn execute_inner(params: &str) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params).map_err(|e| format!("参数解析失败: {e}"))?;

    if !near::agent::host::secret_exists("dingtalk_webhook_token") {
        return Err(
            "未找到钉钉机器人 webhook token。请使用 ironclaw secret set dingtalk_webhook_token <token> 设置。\
             在钉钉群设置中添加自定义机器人获取 Webhook URL 中的 access_token。"
                .into(),
        );
    }

    match params.action.as_str() {
        "send_text" => {
            let content = params
                .content
                .ok_or_else(|| "send_text 操作需要 content 参数".to_string())?;
            send_text(&content)
        }
        "send_markdown" => {
            let title = params
                .title
                .ok_or_else(|| "send_markdown 操作需要 title 参数".to_string())?;
            let text = params
                .text
                .ok_or_else(|| "send_markdown 操作需要 text 参数".to_string())?;
            send_markdown(&title, &text)
        }
        "send_action_card" => {
            let title = params
                .title
                .ok_or_else(|| "send_action_card 操作需要 title 参数".to_string())?;
            let text = params
                .text
                .ok_or_else(|| "send_action_card 操作需要 text 参数".to_string())?;
            let single_title = params
                .single_title
                .ok_or_else(|| "send_action_card 操作需要 singleTitle 参数".to_string())?;
            let single_url = params
                .single_url
                .ok_or_else(|| "send_action_card 操作需要 singleURL 参数".to_string())?;
            send_action_card(&title, &text, &single_title, &single_url)
        }
        other => Err(format!(
            "未知操作: '{other}'。支持的操作: send_text, send_markdown, send_action_card"
        )),
    }
}

fn send_text(content: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "msgtype": "text",
        "text": { "content": content },
    });
    send_webhook(&body)?;
    let output = serde_json::json!({
        "action": "send_text",
        "success": true,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn send_markdown(title: &str, text: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "msgtype": "markdown",
        "markdown": {
            "title": title,
            "text": text,
        },
    });
    send_webhook(&body)?;
    let output = serde_json::json!({
        "action": "send_markdown",
        "success": true,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn send_action_card(
    title: &str,
    text: &str,
    single_title: &str,
    single_url: &str,
) -> Result<String, String> {
    let body = serde_json::json!({
        "msgtype": "actionCard",
        "actionCard": {
            "title": title,
            "text": text,
            "singleTitle": single_title,
            "singleURL": single_url,
        },
    });
    send_webhook(&body)?;
    let output = serde_json::json!({
        "action": "send_action_card",
        "success": true,
    });
    serde_json::to_string(&output).map_err(|e| format!("序列化失败: {e}"))
}

fn send_webhook(body: &serde_json::Value) -> Result<(), String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "IronClaw-DingTalkRobot-Tool/0.1"
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
                        "DingTalk Webhook 错误 {} (尝试 {}/{}), 重试中...",
                        resp.status, attempt, MAX_RETRIES
                    ),
                );
                continue;
            }

            let body_str = String::from_utf8_lossy(&resp.body);
            return Err(format!(
                "DingTalk Webhook 错误 (HTTP {}): {}",
                resp.status, body_str
            ));
        }
    };

    let body_str =
        String::from_utf8(response.body).map_err(|e| format!("响应编码错误: {e}"))?;
    let webhook_resp: WebhookResponse =
        serde_json::from_str(&body_str).map_err(|e| format!("响应解析失败: {e}"))?;

    if let Some(code) = webhook_resp.errcode {
        if code != 0 {
            let msg = webhook_resp.errmsg.unwrap_or_default();
            return Err(format!("钉钉机器人错误 (errcode {}): {}", code, msg));
        }
    }

    Ok(())
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "description": "操作类型: send_text (发送文本), send_markdown (发送 Markdown), send_action_card (发送卡片)",
            "enum": ["send_text", "send_markdown", "send_action_card"]
        },
        "content": {
            "type": "string",
            "description": "文本消息内容 (send_text 必填)"
        },
        "title": {
            "type": "string",
            "description": "消息标题 (send_markdown 和 send_action_card 必填)"
        },
        "text": {
            "type": "string",
            "description": "消息正文，支持 Markdown 格式 (send_markdown 和 send_action_card 必填)"
        },
        "singleTitle": {
            "type": "string",
            "description": "按钮标题 (send_action_card 必填)"
        },
        "singleURL": {
            "type": "string",
            "description": "按钮链接 URL (send_action_card 必填)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

export!(DingTalkRobotTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webhook_success_response() {
        let json = r#"{"errcode": 0, "errmsg": "ok"}"#;
        let resp: WebhookResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.errcode, Some(0));
        assert_eq!(resp.errmsg.as_deref(), Some("ok"));
    }

    #[test]
    fn test_parse_webhook_error_response() {
        let json = r#"{"errcode": 310000, "errmsg": "sign not match"}"#;
        let resp: WebhookResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.errcode, Some(310000));
        assert_eq!(resp.errmsg.as_deref(), Some("sign not match"));
    }

    #[test]
    fn test_parse_webhook_response_missing_fields() {
        let json = r#"{}"#;
        let resp: WebhookResponse = serde_json::from_str(json).unwrap();
        assert!(resp.errcode.is_none());
        assert!(resp.errmsg.is_none());
    }

    #[test]
    fn test_text_message_body() {
        let body = serde_json::json!({
            "msgtype": "text",
            "text": { "content": "测试消息" },
        });
        let s = body.to_string();
        assert!(s.contains("测试消息"));
        assert!(s.contains("\"msgtype\":\"text\""));
    }

    #[test]
    fn test_markdown_message_body() {
        let body = serde_json::json!({
            "msgtype": "markdown",
            "markdown": {
                "title": "标题",
                "text": "## 正文\n- 项目1",
            },
        });
        let s = body.to_string();
        assert!(s.contains("\"msgtype\":\"markdown\""));
        assert!(s.contains("标题"));
    }

    #[test]
    fn test_action_card_message_body() {
        let body = serde_json::json!({
            "msgtype": "actionCard",
            "actionCard": {
                "title": "卡片标题",
                "text": "卡片内容",
                "singleTitle": "查看详情",
                "singleURL": "https://example.com",
            },
        });
        let s = body.to_string();
        assert!(s.contains("\"msgtype\":\"actionCard\""));
        assert!(s.contains("查看详情"));
        assert!(s.contains("https://example.com"));
    }
}
