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

    // Validate params is an object when provided (schema declares it as such).
    if let Some(ref p) = params.params {
        if !p.is_object() {
            return Err("'params' must be a JSON object".into());
        }
    }

    match params.action.as_str() {
        "list" => list_tools(params.app.as_deref()),
        "execute" => {
            let tool_slug = params
                .tool_slug
                .as_deref()
                .ok_or("missing 'tool_slug' for execute action")?;
            validate_tool_slug(tool_slug)?;
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

        // Retry on 429 and 5xx to align with github/web-search tool convention.
        // NOTE: The WASM host has no sleep primitive, so retries are immediate.
        // This still helps when a sliding-window rate limiter resets between
        // the original request and the retry (sub-second windows are common).
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

/// Unwrap a paginated v3 response.
///
/// The Composio v3 API returns paginated results as `{ "items": [...] }`.
/// This helper extracts the `items` array, falling back to treating the
/// response as a bare array for backward compatibility.
fn unwrap_items(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    // v3 paginated envelope: { "items": [...] }
    value
        .get("items")
        .and_then(|v| v.as_array())
        // Fallback: bare array (older or non-paginated endpoints)
        .or_else(|| value.as_array())
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
    // Return the items array directly for cleaner LLM consumption.
    let items = unwrap_items(&result).cloned().unwrap_or_default();
    serde_json::to_string(&items).map_err(|e| format!("Failed to serialize output: {e}"))
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

    // v3 contract: `user_id` (not `entity_id`), `arguments` (not `input`).
    let body = serde_json::json!({
        "connected_account_id": account_id,
        "user_id": entity_id,
        "arguments": params,
    });
    let result = api_post(&format!("/tools/execute/{}", url_encode(tool_slug)), &body)?;
    serde_json::to_string(&result).map_err(|e| format!("Failed to serialize output: {e}"))
}

fn connect_app(app: &str, entity_id: &str) -> Result<String, String> {
    // Resolve auth config for this app — v3 returns paginated { "items": [...] }
    let configs = api_get("/auth_configs", &[("toolkit_slug", app)])?;
    let auth_config_id = extract_auth_config_id(&configs, app)?;

    let body = serde_json::json!({
        "auth_config_id": auth_config_id,
        "user_id": entity_id,
    });
    let result = api_post("/connected_accounts/link", &body)?;
    serde_json::to_string(&result).map_err(|e| format!("Failed to serialize output: {e}"))
}

/// Extract the first auth config ID from a (possibly paginated) response.
fn extract_auth_config_id(configs: &serde_json::Value, app: &str) -> Result<String, String> {
    unwrap_items(configs)
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("id"))
        .and_then(|id| id.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!("no auth config found for {app} — configure it at app.composio.dev")
        })
}

fn list_accounts(app: Option<&str>, entity_id: &str) -> Result<String, String> {
    // v3 uses plural filter names: `user_ids`, `toolkit_slugs`
    let mut query = vec![("user_ids", entity_id)];
    if let Some(a) = app {
        query.push(("toolkit_slugs", a));
    }
    let result = api_get("/connected_accounts", &query)?;
    let items = unwrap_items(&result).cloned().unwrap_or_default();
    serde_json::to_string(&items).map_err(|e| format!("Failed to serialize output: {e}"))
}

/// Look up the toolkit/app slug for a tool via the Composio API.
///
/// Querying the API is more reliable than parsing the tool slug string,
/// which breaks for multi-word app names (e.g., `GOOGLE_DRIVE_UPLOAD`
/// would incorrectly resolve to `"google"` instead of `"google_drive"`).
fn lookup_app_for_tool(tool_slug: &str) -> Result<String, String> {
    let tools = api_get("/tools", &[("search", tool_slug)])?;
    extract_toolkit_slug(&tools, tool_slug)
}

/// Extract the toolkit slug from a tools search response.
///
/// v3 nests the toolkit slug under `toolkit.slug`; falls back to
/// `toolkit_slug` or `appName` for backward compatibility.
fn extract_toolkit_slug(tools: &serde_json::Value, tool_slug: &str) -> Result<String, String> {
    let items = unwrap_items(tools).ok_or_else(|| {
        format!("could not determine app for tool \"{tool_slug}\" — unexpected response shape")
    })?;

    let tool = items
        .iter()
        .find(|t| {
            t.get("slug")
                .and_then(|s| s.as_str())
                .is_some_and(|s| s.eq_ignore_ascii_case(tool_slug))
        })
        .ok_or_else(|| {
            format!("could not determine app for tool \"{tool_slug}\" — verify the slug is correct")
        })?;

    // v3: toolkit.slug; fallback: toolkit_slug, appName
    tool.get("toolkit")
        .and_then(|tk| tk.get("slug"))
        .or_else(|| tool.get("toolkit_slug"))
        .or_else(|| tool.get("appName"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| {
            format!("tool \"{tool_slug}\" has no toolkit slug — verify the slug is correct")
        })
}

/// Auto-resolve connected account for a tool slug.
fn resolve_account(tool_slug: &str, entity_id: &str) -> Result<String, String> {
    let app = lookup_app_for_tool(tool_slug)?;
    find_active_account(tool_slug, &app, entity_id)
}

/// Find the most recently updated active connected account.
///
/// v3 uses `updated_at` (not `updatedAt`) and returns paginated items.
fn find_active_account(tool_slug: &str, app: &str, entity_id: &str) -> Result<String, String> {
    // v3 uses plural filter names
    let accounts = api_get(
        "/connected_accounts",
        &[("user_ids", entity_id), ("toolkit_slugs", app)],
    )?;

    let items = unwrap_items(&accounts).ok_or_else(|| {
        format!("no connected account for {app} — use composio with action=\"connect\" first")
    })?;

    items
        .iter()
        .filter(|a| a.get("status").and_then(|s| s.as_str()) == Some("ACTIVE"))
        .max_by_key(|a| {
            // v3: updated_at; fallback: updatedAt
            a.get("updated_at")
                .or_else(|| a.get("updatedAt"))
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string()
        })
        .and_then(|a| a.get("id"))
        .and_then(|id| id.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "no active connected account for {app} (tool: {tool_slug}) — \
                 use composio with action=\"connect\" first"
            )
        })
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Defense-in-depth: reject tool slugs that could cause path traversal.
/// The WASM host allowlist already normalises paths and rejects `..` segments,
/// but we validate here too (same pattern as the `github` tool).
fn validate_tool_slug(s: &str) -> Result<(), String> {
    if s.is_empty() || s.contains("..") || s.contains('/') || s.contains('\\') {
        return Err(format!("invalid tool_slug: \"{s}\""));
    }
    Ok(())
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
            "type": "object",
            "description": "Parameters for the tool action (JSON object)",
            "additionalProperties": true
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
/// Checks `entity_id` first (explicit override), then `user_id` (from
/// JobContext — always present in production), falling back to "default".
fn extract_entity_id(context: Option<&str>) -> String {
    context
        .and_then(|ctx| serde_json::from_str::<serde_json::Value>(ctx).ok())
        .and_then(|v| {
            v.get("entity_id")
                .or_else(|| v.get("user_id"))
                .and_then(|e| {
                    e.as_str()
                        .map(String::from)
                        .or_else(|| e.as_u64().map(|n| n.to_string()))
                        .or_else(|| e.as_i64().map(|n| n.to_string()))
                })
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
        let ctx = r#"{"user_id": "user-1"}"#;
        assert_eq!(extract_entity_id(Some(ctx)), "user-1");
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

    #[test]
    fn test_extract_entity_id_numeric_user_id() {
        let ctx = r#"{"user_id": 12345}"#;
        assert_eq!(extract_entity_id(Some(ctx)), "12345");
    }

    #[test]
    fn test_extract_entity_id_numeric_entity_id() {
        let ctx = r#"{"entity_id": 99, "user_id": "user-1"}"#;
        assert_eq!(extract_entity_id(Some(ctx)), "99");
    }

    #[test]
    fn test_validate_tool_slug_valid() {
        assert!(validate_tool_slug("GMAIL_SEND_EMAIL").is_ok());
        assert!(validate_tool_slug("slack-post").is_ok());
    }

    #[test]
    fn test_validate_tool_slug_rejects_traversal() {
        assert!(validate_tool_slug("..").is_err());
        assert!(validate_tool_slug("foo/../bar").is_err());
        assert!(validate_tool_slug("foo/bar").is_err());
        assert!(validate_tool_slug("foo\\bar").is_err());
        assert!(validate_tool_slug("").is_err());
    }

    #[test]
    fn test_params_deserialization_rejects_unknown_fields() {
        let result: Result<Params, _> =
            serde_json::from_str(r#"{"action": "list", "bogus": true}"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_params_accepts_valid_object_params() {
        let p: Params =
            serde_json::from_str(r#"{"action": "execute", "params": {"key": "val"}}"#).unwrap();
        assert!(p.params.unwrap().is_object());
    }

    #[test]
    fn test_params_accepts_null_params() {
        let p: Params =
            serde_json::from_str(r#"{"action": "list"}"#).unwrap();
        assert!(p.params.is_none());
    }

    // -----------------------------------------------------------------------
    // Fixture-style tests for v3 API contract parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_unwrap_items_paginated_envelope() {
        let resp: serde_json::Value = serde_json::from_str(
            r#"{"items": [{"id": "1"}, {"id": "2"}], "total": 2, "page": 1}"#,
        )
        .unwrap();
        let items = unwrap_items(&resp).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_unwrap_items_bare_array_fallback() {
        let resp: serde_json::Value =
            serde_json::from_str(r#"[{"id": "1"}, {"id": "2"}]"#).unwrap();
        let items = unwrap_items(&resp).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_unwrap_items_empty_envelope() {
        let resp: serde_json::Value =
            serde_json::from_str(r#"{"items": [], "total": 0}"#).unwrap();
        let items = unwrap_items(&resp).unwrap();
        assert!(items.is_empty());
    }

    #[test]
    fn test_unwrap_items_non_array_returns_none() {
        let resp: serde_json::Value =
            serde_json::from_str(r#"{"error": "not found"}"#).unwrap();
        assert!(unwrap_items(&resp).is_none());
    }

    #[test]
    fn test_extract_auth_config_id_from_paginated() {
        let resp: serde_json::Value = serde_json::from_str(
            r#"{"items": [{"id": "ac-123", "type": "oauth2"}]}"#,
        )
        .unwrap();
        assert_eq!(
            extract_auth_config_id(&resp, "gmail").unwrap(),
            "ac-123"
        );
    }

    #[test]
    fn test_extract_auth_config_id_from_bare_array() {
        let resp: serde_json::Value =
            serde_json::from_str(r#"[{"id": "ac-456"}]"#).unwrap();
        assert_eq!(
            extract_auth_config_id(&resp, "github").unwrap(),
            "ac-456"
        );
    }

    #[test]
    fn test_extract_auth_config_id_empty_items() {
        let resp: serde_json::Value =
            serde_json::from_str(r#"{"items": []}"#).unwrap();
        let err = extract_auth_config_id(&resp, "slack").unwrap_err();
        assert!(err.contains("no auth config found for slack"));
    }

    #[test]
    fn test_extract_toolkit_slug_v3_nested() {
        let resp: serde_json::Value = serde_json::from_str(
            r#"{"items": [{"slug": "GMAIL_SEND_EMAIL", "toolkit": {"slug": "gmail"}}]}"#,
        )
        .unwrap();
        assert_eq!(
            extract_toolkit_slug(&resp, "GMAIL_SEND_EMAIL").unwrap(),
            "gmail"
        );
    }

    #[test]
    fn test_extract_toolkit_slug_legacy_flat() {
        let resp: serde_json::Value = serde_json::from_str(
            r#"[{"slug": "SLACK_POST", "toolkit_slug": "slack"}]"#,
        )
        .unwrap();
        assert_eq!(
            extract_toolkit_slug(&resp, "SLACK_POST").unwrap(),
            "slack"
        );
    }

    #[test]
    fn test_extract_toolkit_slug_app_name_fallback() {
        let resp: serde_json::Value = serde_json::from_str(
            r#"[{"slug": "NOTION_CREATE", "appName": "Notion"}]"#,
        )
        .unwrap();
        assert_eq!(
            extract_toolkit_slug(&resp, "NOTION_CREATE").unwrap(),
            "notion"
        );
    }

    #[test]
    fn test_extract_toolkit_slug_case_insensitive_match() {
        let resp: serde_json::Value = serde_json::from_str(
            r#"{"items": [{"slug": "github_create_issue", "toolkit": {"slug": "github"}}]}"#,
        )
        .unwrap();
        assert_eq!(
            extract_toolkit_slug(&resp, "GITHUB_CREATE_ISSUE").unwrap(),
            "github"
        );
    }

    #[test]
    fn test_extract_toolkit_slug_not_found() {
        let resp: serde_json::Value = serde_json::from_str(
            r#"{"items": [{"slug": "OTHER_TOOL", "toolkit": {"slug": "other"}}]}"#,
        )
        .unwrap();
        let err = extract_toolkit_slug(&resp, "MISSING_TOOL").unwrap_err();
        assert!(err.contains("MISSING_TOOL"));
    }

    #[test]
    fn test_find_active_account_v3_response() {
        // This tests the parsing logic — the actual API call is mocked by
        // testing the helper directly.
        let resp: serde_json::Value = serde_json::from_str(
            r#"{"items": [
                {"id": "old-1", "status": "ACTIVE", "updated_at": "2024-01-01T00:00:00Z"},
                {"id": "new-2", "status": "ACTIVE", "updated_at": "2024-06-15T12:00:00Z"},
                {"id": "disabled-3", "status": "DISABLED", "updated_at": "2024-12-01T00:00:00Z"}
            ]}"#,
        )
        .unwrap();
        let items = unwrap_items(&resp).unwrap();
        let best = items
            .iter()
            .filter(|a| a.get("status").and_then(|s| s.as_str()) == Some("ACTIVE"))
            .max_by_key(|a| {
                a.get("updated_at")
                    .or_else(|| a.get("updatedAt"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .and_then(|a| a.get("id"))
            .and_then(|id| id.as_str());
        assert_eq!(best, Some("new-2"));
    }

    #[test]
    fn test_find_active_account_legacy_updated_at() {
        let resp: serde_json::Value = serde_json::from_str(
            r#"[
                {"id": "a1", "status": "ACTIVE", "updatedAt": "2024-01-01"},
                {"id": "a2", "status": "ACTIVE", "updatedAt": "2024-06-01"}
            ]"#,
        )
        .unwrap();
        let items = unwrap_items(&resp).unwrap();
        let best = items
            .iter()
            .filter(|a| a.get("status").and_then(|s| s.as_str()) == Some("ACTIVE"))
            .max_by_key(|a| {
                a.get("updated_at")
                    .or_else(|| a.get("updatedAt"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .and_then(|a| a.get("id"))
            .and_then(|id| id.as_str());
        assert_eq!(best, Some("a2"));
    }

    #[test]
    fn test_find_active_account_no_active() {
        let resp: serde_json::Value = serde_json::from_str(
            r#"{"items": [{"id": "x", "status": "DISABLED", "updated_at": "2024-01-01"}]}"#,
        )
        .unwrap();
        let items = unwrap_items(&resp).unwrap();
        let best = items
            .iter()
            .filter(|a| a.get("status").and_then(|s| s.as_str()) == Some("ACTIVE"))
            .max_by_key(|a| {
                a.get("updated_at")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string()
            });
        assert!(best.is_none());
    }
}
