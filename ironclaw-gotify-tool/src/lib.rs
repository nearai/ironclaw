//! IronClaw WASM tool: Gotify push notifications
//!
//! Uses IronClaw's host-provided http-request function.
//! Secrets (GOTIFY_APP_TOKEN) are injected by the host into
//! HTTP headers at the host boundary — never exposed to WASM.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "wit/tool.wit",
});

use serde::{Deserialize, Serialize};

use exports::near::agent::tool;

// ── Types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SendInput {
    #[serde(default = "default_title")]
    title: String,
    #[serde(default = "default_message")]
    message: String,
    #[serde(default = "default_priority")]
    priority: i32,
}

fn default_title() -> String {
    "Kageho".to_string()
}

fn default_message() -> String {
    "Notification from Kageho".to_string()
}

fn default_priority() -> i32 {
    9
}

#[derive(Serialize)]
struct GotifyMessage {
    title: String,
    message: String,
    priority: i32,
}

// ── Tool implementation ─────────────────────────────────────────

struct GotifyTool;

export!(GotifyTool);

impl tool::Guest for GotifyTool {
    fn execute(req: tool::Request) -> tool::Response {
        let result = dispatch(&req.params);
        match result {
            Ok(output) => tool::Response {
                output: Some(output),
                error: None,
            },
            Err(e) => tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        r#"{
  "type": "object",
  "properties": {
    "title": {
      "type": "string",
      "description": "Notification title. Defaults to 'Kageho'."
    },
    "message": {
      "type": "string",
      "description": "Notification body text. Supports markdown."
    },
    "priority": {
      "type": "integer",
      "description": "Priority: 1-3=low, 5-7=medium, 8-10=high. Default 3.",
      "default": 3
    }
  },
  "required": ["message"]
}"#
        .to_string()
    }

fn description() -> String {
        "Send a push notification via Gotify. Parameters (JSON object): message (string, REQUIRED), title (string, default: Kageho), priority (integer: 1-3=low, 5-7=medium, 8-10=high, default: 3). Example: {\"message\": \"hello\", \"priority\": 5}".to_string()
    }
}

// ── Logic ───────────────────────────────────────────────────────

fn dispatch(params_json: &str) -> Result<String, String> {
    let params: SendInput = match serde_json::from_str(params_json) {
        Ok(p) => p,
        Err(_) => {
            let trimmed = params_json.trim().trim_matches('"');
            let msg = if !trimmed.is_empty() && trimmed != "{}" {
                trimmed.to_string()
            } else {
                default_message()
            };
            SendInput {
                title: default_title(),
                message: msg,
                priority: default_priority(),
            }
        }
    };    

    if !near::agent::host::secret_exists("gotify_app_token") {
        return Err("Secret 'gotify_app_token' not configured.".into());
    }

    let msg = GotifyMessage {
        title: params.title,
        message: params.message,
        priority: params.priority,
    };

    let body = serde_json::to_string(&msg).map_err(|e| format!("JSON error: {e}"))?;

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!("Sending Gotify notification: {}", msg.title),
    );

    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    let url = "https://gotify.darkc.sobe.world/message";

    let response = near::agent::host::http_request(
        "POST",
        url,
        &headers.to_string(),
        Some(body.as_bytes()),
        Some(10000),
    )
    .map_err(|e| format!("HTTP failed: {e}"))?;

    let status = response.status;
    let resp_body = String::from_utf8_lossy(&response.body).to_string();

    if status >= 200 && status < 300 {
        near::agent::host::log(
            near::agent::host::LogLevel::Info,
            &format!("Gotify notification sent (HTTP {status})"),
        );
        Ok(format!(
            "{{\"success\":true,\"message\":\"Notification sent (HTTP {status})\"}}"
        ))
    } else {
        Err(format!("Gotify returned HTTP {status}: {resp_body}"))
    }
}
