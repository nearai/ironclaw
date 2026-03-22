//! Composio WASM Tool for IronClaw.
//!
//! Connects to 250+ third-party apps via Composio's REST API (v3).
//! Provides a single multiplexed tool with actions: list, execute, connect,
//! connected_accounts.
//!
//! # Authentication
//!
//! Store your Composio API key:
//! `ironclaw secret set composio_api_key <key>`
//!
//! Get a key at: https://app.composio.dev/

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const API_BASE: &str = "https://backend.composio.dev/api/v3";
const MAX_RETRIES: u32 = 3;

struct ComposioTool;

impl exports::near::agent::tool::Guest for ComposioTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params, req.context.as_deref()) {
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
        "Connect to 250+ apps (Gmail, GitHub, Slack, Notion, etc.) via Composio. \
         Actions: \"list\" (browse tools), \"execute\" (run a tool), \
         \"connect\" (OAuth-link an app), \"connected_accounts\" (list linked accounts). \
         Authentication is handled via the 'composio_api_key' secret injected by the host."
            .to_string()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    action: String,
    app: Option<String>,
    tool_slug: Option<String>,
    params: Option<serde_json::Value>,
    connected_account_id: Option<String>,
}

fn execute_inner(params_str: &str, context: Option<&str>) -> Result<String, String> {
    let params: Params =
        serde_json::from_str(params_str).map_err(|e| format!("Invalid parameters: {e}"))?;

    if params.action.is_empty() {
        return Err("'action' must not be empty".into());
    }

    // Best-effort pre-flight: check if the secret is configured in capabilities.
    // This won't catch every case (the host may only check the allowlist), but
    // avoids wasting a rate-limited API call when clearly misconfigured.
    if !near::agent::host::secret_exists("composio_api_key") {
        return Err(
            "Composio API key not configured. Set it with: \
             ironclaw secret set composio_api_key <key>. \
             Get a key at: https://app.composio.dev/"
                .into(),
        );
    }

    let entity_id = extract_entity_id(context);

    match params.action.as_str() {
        "list" => list_tools(params.app.as_deref()),
        "execute" => {
            let tool_slug = params
                .tool_slug
                .as_deref()
                .ok_or("missing 'tool_slug' for execute action")?;
            let action_params = params.params.unwrap_or(serde_json::json!({}));
            execute_action(
                tool_slug,
                &action_params,
                &entity_id,
                params.connected_account_id.as_deref(),
            )
        }
        "connect" => {
            let app = params
                .app
                .as_deref()
                .ok_or("missing 'app' for connect action")?;
            connect_app(app, &entity_id)
        }
        "connected_accounts" => list_accounts(params.app.as_deref(), &entity_id),
        other => Err(format!(
            "unknown action \"{other}\", expected: list, execute, connect, connected_accounts"
        )),
    }
}

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

fn api_get(path: &str, query: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let url = build_url(path, query);

    let headers = serde_json::json!({
        "Accept": "application/json",
        "User-Agent": "IronClaw-Composio-Tool/0.1"
    });

    let response = get_with_retry(&url, &headers.to_string())?;
    parse_json_body(&response.body)
}

fn api_post(path: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    let url = build_url(path, &[]);

    let headers = serde_json::json!({
        "Accept": "application/json",
        "Content-Type": "application/json",
        "User-Agent": "IronClaw-Composio-Tool/0.1"
    });

    let body_bytes = serde_json::to_vec(body).map_err(|e| format!("JSON serialize error: {e}"))?;

    // POST is not idempotent — no retry to avoid duplicate side effects.
    let resp = near::agent::host::http_request("POST", &url, &headers.to_string(), Some(&body_bytes), None)
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if resp.status >= 200 && resp.status < 300 {
        return parse_json_body(&resp.body);
    }

    // Surface helpful message on auth failure
    if resp.status == 401 || resp.status == 403 {
        return Err(
            "Composio API authentication failed. Ensure your API key is set: \
             ironclaw secret set composio_api_key <key>. \
             Get a key at: https://app.composio.dev/"
                .into(),
        );
    }

    let truncated_bytes = if resp.body.len() > 512 { &resp.body[..512] } else { &resp.body };
    let truncated = String::from_utf8_lossy(truncated_bytes);
    Err(format!("Composio API error (HTTP {}): {truncated}", resp.status))
}

/// GET with retry on transient errors (429, 5xx). Safe to retry since GET is idempotent.
fn get_with_retry(
    url: &str,
    headers: &str,
) -> Result<near::agent::host::HttpResponse, String> {
    let mut attempt = 0;
    loop {
        attempt += 1;

        let resp = near::agent::host::http_request("GET", url, headers, None, None)
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        if resp.status >= 200 && resp.status < 300 {
            return Ok(resp);
        }

        // Surface helpful message on auth failure
        if resp.status == 401 || resp.status == 403 {
            return Err(
                "Composio API authentication failed. Ensure your API key is set: \
                 ironclaw secret set composio_api_key <key>. \
                 Get a key at: https://app.composio.dev/"
                    .into(),
            );
        }

        if attempt < MAX_RETRIES && (resp.status == 429 || resp.status >= 500) {
            near::agent::host::log(
                near::agent::host::LogLevel::Warn,
                &format!(
                    "Composio API error {} (attempt {}/{}). Retrying...",
                    resp.status, attempt, MAX_RETRIES
                ),
            );
            continue;
        }

        // Truncate at byte level before UTF-8 conversion to avoid
        // panicking on multibyte character boundaries.
        let truncated_bytes = if resp.body.len() > 512 {
            &resp.body[..512]
        } else {
            &resp.body
        };
        let truncated = String::from_utf8_lossy(truncated_bytes);
        return Err(format!("Composio API error (HTTP {}): {truncated}", resp.status));
    }
}

/// Parse a JSON response body directly from bytes (avoids extra allocation).
fn parse_json_body(body: &[u8]) -> Result<serde_json::Value, String> {
    serde_json::from_slice(body).map_err(|e| format!("invalid JSON: {e}"))
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn list_tools(app: Option<&str>) -> Result<String, String> {
    let query: Vec<(&str, &str)> = match app {
        Some(a) => vec![("toolkit_slug", a)],
        None => vec![],
    };
    let result = api_get("/tools", &query)?;
    serde_json::to_string(&result).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn execute_action(
    tool_slug: &str,
    params: &serde_json::Value,
    entity_id: &str,
    connected_account_id: Option<&str>,
) -> Result<String, String> {
    // Auto-resolve connected account if not provided
    let account_id = match connected_account_id {
        Some(id) => id.to_string(),
        None => resolve_account(tool_slug, entity_id)?,
    };

    let body = serde_json::json!({
        "connected_account_id": account_id,
        "entity_id": entity_id,
        "input": params,
    });
    let result = api_post(&format!("/tools/execute/{}", url_encode(tool_slug)), &body)?;
    serde_json::to_string(&result).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn connect_app(app: &str, entity_id: &str) -> Result<String, String> {
    // Resolve auth config for this app
    let configs = api_get("/auth_configs", &[("toolkit_slug", app)])?;
    let auth_config_id = configs
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("id"))
        .and_then(|id| id.as_str())
        .ok_or_else(|| {
            format!("no auth config found for {app} — configure it at app.composio.dev")
        })?;

    let body = serde_json::json!({
        "auth_config_id": auth_config_id,
        "user_id": entity_id,
    });
    let result = api_post("/connected_accounts/link", &body)?;
    serde_json::to_string(&result).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn list_accounts(app: Option<&str>, entity_id: &str) -> Result<String, String> {
    let mut query = vec![("user_id", entity_id)];
    if let Some(a) = app {
        query.push(("toolkit_slug", a));
    }
    let result = api_get("/connected_accounts", &query)?;
    serde_json::to_string(&result).map_err(|e| format!("Failed to serialize output: {e}"))
}

/// Look up the toolkit/app slug for a tool via the Composio API.
///
/// Querying the API is more reliable than parsing the tool slug string,
/// which breaks for multi-word app names (e.g., `GOOGLE_DRIVE_UPLOAD`
/// would incorrectly resolve to `"google"` instead of `"google_drive"`).
fn lookup_app_for_tool(tool_slug: &str) -> Result<String, String> {
    let tools = api_get("/tools", &[("search", tool_slug)])?;
    tools
        .as_array()
        .and_then(|arr| {
            arr.iter().find(|t| {
                t.get("slug")
                    .and_then(|s| s.as_str())
                    .map(|s| s.eq_ignore_ascii_case(tool_slug))
                    .unwrap_or(false)
            })
        })
        .and_then(|t| t.get("toolkit_slug").or_else(|| t.get("appName")))
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| {
            format!("could not determine app for tool \"{tool_slug}\" — verify the slug is correct")
        })
}

/// Auto-resolve connected account for a tool slug.
fn resolve_account(tool_slug: &str, entity_id: &str) -> Result<String, String> {
    let app = lookup_app_for_tool(tool_slug)?;

    let accounts = api_get("/connected_accounts", &[("user_id", entity_id), ("toolkit_slug", &app)])?;

    accounts
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .filter(|a| a.get("status").and_then(|s| s.as_str()) == Some("ACTIVE"))
                .max_by_key(|a| {
                    a.get("updatedAt")
                        .and_then(|u| u.as_str())
                        .unwrap_or("")
                        .to_string()
                })
        })
        .and_then(|a| a.get("id"))
        .and_then(|id| id.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!("no connected account for {app} — use composio with action=\"connect\" first")
        })
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

fn build_url(path: &str, query: &[(&str, &str)]) -> String {
    let mut url = format!("{API_BASE}{path}");
    if !query.is_empty() {
        url.push('?');
        for (i, (k, v)) in query.iter().enumerate() {
            if i > 0 {
                url.push('&');
            }
            url.push_str(&url_encode(k));
            url.push('=');
            url.push_str(&url_encode(v));
        }
    }
    url
}

/// Percent-encode a string for safe use in URL query parameters.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

const SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "action": {
            "type": "string",
            "enum": ["list", "execute", "connect", "connected_accounts"],
            "description": "Action to perform"
        },
        "app": {
            "type": "string",
            "description": "App/toolkit slug (e.g., \"gmail\", \"github\", \"notion\")"
        },
        "tool_slug": {
            "type": "string",
            "description": "Tool action slug for execute (e.g., \"GMAIL_SEND_EMAIL\")"
        },
        "params": {
            "description": "Parameters for the tool action (JSON object)"
        },
        "connected_account_id": {
            "type": "string",
            "description": "Specific connected account ID (auto-resolved if omitted)"
        }
    },
    "required": ["action"],
    "additionalProperties": false
}"#;

/// Extract an entity identifier from context JSON.
///
/// Checks `entity_id`, then `user_id` (from JobContext), then `requester_id`,
/// falling back to "default" if none are present.
fn extract_entity_id(context: Option<&str>) -> String {
    context
        .and_then(|ctx| serde_json::from_str::<serde_json::Value>(ctx).ok())
        .and_then(|v| {
            v.get("entity_id")
                .or_else(|| v.get("user_id"))
                .or_else(|| v.get("requester_id"))
                .and_then(|e| e.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| "default".to_string())
}

export!(ComposioTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_encode() {
        assert_eq!(url_encode("hello world"), "hello%20world");
        assert_eq!(url_encode("foo&bar=baz"), "foo%26bar%3Dbaz");
        assert_eq!(url_encode("simple"), "simple");
    }

    #[test]
    fn test_url_encode_multibyte() {
        assert_eq!(url_encode("café"), "caf%C3%A9");
    }

    #[test]
    fn test_build_url_no_query() {
        let url = build_url("/tools", &[]);
        assert_eq!(url, format!("{API_BASE}/tools"));
    }

    #[test]
    fn test_build_url_with_query() {
        let url = build_url("/tools", &[("toolkit_slug", "gmail"), ("search", "send")]);
        assert!(url.starts_with(&format!("{API_BASE}/tools?")));
        assert!(url.contains("toolkit_slug=gmail"));
        assert!(url.contains("search=send"));
    }

    #[test]
    fn test_build_url_encodes_special_chars() {
        let url = build_url("/tools", &[("q", "my app+1")]);
        assert!(url.contains("q=my%20app%2B1"));
    }

    #[test]
    fn test_extract_entity_id_from_entity_id() {
        let ctx = r#"{"entity_id": "tenant-42", "user_id": "user-1"}"#;
        assert_eq!(extract_entity_id(Some(ctx)), "tenant-42");
    }

    #[test]
    fn test_extract_entity_id_falls_back_to_user_id() {
        let ctx = r#"{"user_id": "user-1", "requester_id": "req-1"}"#;
        assert_eq!(extract_entity_id(Some(ctx)), "user-1");
    }

    #[test]
    fn test_extract_entity_id_falls_back_to_requester_id() {
        let ctx = r#"{"requester_id": "req-1"}"#;
        assert_eq!(extract_entity_id(Some(ctx)), "req-1");
    }

    #[test]
    fn test_extract_entity_id_defaults_when_none() {
        assert_eq!(extract_entity_id(None), "default");
    }

    #[test]
    fn test_extract_entity_id_defaults_on_empty_context() {
        assert_eq!(extract_entity_id(Some("{}")), "default");
    }

    #[test]
    fn test_extract_entity_id_defaults_on_malformed_json() {
        assert_eq!(extract_entity_id(Some("not json")), "default");
    }
}
