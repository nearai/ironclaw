use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const PROTOCOL_VERSION: &str = "2024-11-05";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OUTPUT_SIZE: usize = 64 * 1024;

const AUTH_STATUS_COMMAND: [&str; 2] = ["auth", "status"];
const GMAIL_READ_COMMANDS: [&[&str]; 2] =
    [&["gmail", "list"], &["gmail", "users", "messages", "list"]];
const CALENDAR_READ_COMMANDS: [&[&str]; 2] = [
    &["calendar", "events", "list"],
    &["calendar", "users", "events", "list"],
];
const DRIVE_READ_COMMANDS: [&[&str]; 2] = [&["drive", "files"], &["drive", "files", "list"]];

static BEARER_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| compile_regex(r"(?i)(bearer\s+)([a-zA-Z0-9_\-\.]{20,})"));
static OAUTH_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| compile_regex(r#"(?i)(token[=\'":\s]+)([a-zA-Z0-9_\-\.]{20,})"#));
static YA29_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| compile_regex(r"(ya29\.[a-zA-Z0-9_\-\.]+)"));
static AKIA_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| compile_regex(r"(?i)(AKIA[0-9A-Z]{16})"));
static SK_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| compile_regex(r"(?i)(sk-[a-zA-Z0-9]{32,})"));

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[derive(Debug, Serialize)]
struct ToolDefinition {
    name: &'static str,
    description: &'static str,
    #[serde(rename = "inputSchema")]
    input_schema: serde_json::Value,
    annotations: ToolAnnotations,
}

#[derive(Debug, Default, Serialize)]
struct ToolAnnotations {
    #[serde(rename = "readOnlyHint")]
    read_only_hint: bool,
}

#[derive(Debug, Serialize)]
struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: &'static str,
    capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    server_info: ServerInfo,
    instructions: &'static str,
}

#[derive(Debug, Serialize)]
struct ServerCapabilities {
    tools: ToolsCapability,
}

#[derive(Debug, Serialize)]
struct ToolsCapability {
    #[serde(rename = "listChanged")]
    list_changed: bool,
}

#[derive(Debug, Serialize)]
struct ServerInfo {
    name: &'static str,
    version: &'static str,
}

#[derive(Debug, Deserialize)]
struct ToolCallArguments {
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ToolCallRequest {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct CallToolResult {
    content: Vec<ContentBlock>,
    #[serde(rename = "isError")]
    is_error: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
}

pub async fn run() -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = stdout;

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let response = handle_line(&line).await;
        if let Some(response) = response {
            let text = serde_json::to_string(&response)?;
            writer.write_all(text.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }

    Ok(())
}

async fn handle_line(line: &str) -> Option<JsonRpcResponse> {
    let parsed: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            return Some(jsonrpc_error(
                serde_json::Value::Null,
                -32700,
                "Parse error".to_string(),
            ));
        }
    };

    let request: JsonRpcRequest = match serde_json::from_value(parsed.clone()) {
        Ok(req) => req,
        Err(_) => {
            return Some(jsonrpc_error(
                parsed.get("id").cloned().unwrap_or(serde_json::Value::Null),
                -32600,
                "Invalid Request".to_string(),
            ));
        }
    };

    handle_request(request).await
}

async fn handle_request(request: JsonRpcRequest) -> Option<JsonRpcResponse> {
    if request.jsonrpc != "2.0" {
        return Some(jsonrpc_error(
            request.id.unwrap_or(serde_json::Value::Null),
            -32600,
            "Invalid Request".to_string(),
        ));
    }

    let id = request.id.unwrap_or(serde_json::Value::Null);

    match request.method.as_str() {
        "initialize" => Some(jsonrpc_ok(
            id,
            serde_json::to_value(InitializeResult {
                protocol_version: PROTOCOL_VERSION,
                capabilities: ServerCapabilities {
                    tools: ToolsCapability {
                        list_changed: false,
                    },
                },
                server_info: ServerInfo {
                    name: "gws-bridge",
                    version: env!("CARGO_PKG_VERSION"),
                },
                instructions: "Standalone fallback bridge around a local gws binary. Enable with GWS_BRIDGE_ENABLED=true and configure GWS_BINARY_PATH if needed.",
            })
            .expect("initialize result serializes"),
        )),
        "notifications/initialized" => None,
        "tools/list" => Some(jsonrpc_ok(
            id,
            serde_json::json!({
                "tools": [tool_definition()]
            }),
        )),
        "tools/call" => {
            let params = request.params.unwrap_or(serde_json::Value::Null);
            match serde_json::from_value::<ToolCallRequest>(params) {
                Ok(tool_call) => Some(call_tool_response(id, tool_call).await),
                Err(e) => Some(tool_error(
                    id,
                    format!("Invalid tool arguments: {}", e),
                )),
            }
        }
        _ => Some(jsonrpc_error(
            id,
            -32601,
            format!("Method not found: {}", request.method),
        )),
    }
}

async fn call_tool_response(id: serde_json::Value, request: ToolCallRequest) -> JsonRpcResponse {
    if request.name != "gws_bridge" {
        return tool_error(id, format!("Unknown tool: {}", request.name));
    }

    let args: ToolCallArguments = match serde_json::from_value(request.arguments) {
        Ok(args) => args,
        Err(e) => {
            return tool_error(id, format!("Invalid tool arguments: {}", e));
        }
    };

    if !bridge_enabled_from_env(std::env::var("GWS_BRIDGE_ENABLED").ok().as_deref()) {
        return tool_error(
            id,
            "gws_bridge is disabled. Set GWS_BRIDGE_ENABLED=true to enable it.".to_string(),
        );
    }

    if let Err(reason) = check_allowlist(&args.args) {
        return tool_error(id, format!("Command blocked by allowlist: {}", reason));
    }

    let bin_path = std::env::var("GWS_BINARY_PATH").unwrap_or_else(|_| "gws".to_string());
    if bin_path.is_empty() {
        return tool_error(
            id,
            "GWS_BINARY_PATH is empty. Set it to a valid path or leave it unset to use gws from PATH.".to_string(),
        );
    }

    let mut command = Command::new(&bin_path);
    command
        .args(&args.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return tool_error(id, format!("Failed to spawn {}: {}", bin_path, e));
        }
    };

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let result = tokio::time::timeout(DEFAULT_TIMEOUT, async {
        let stdout_fut = async {
            if let Some(mut out) = stdout_handle {
                let mut buf = Vec::new();
                let _ = (&mut out)
                    .take(MAX_OUTPUT_SIZE as u64)
                    .read_to_end(&mut buf)
                    .await;
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::new()
            }
        };

        let stderr_fut = async {
            if let Some(mut err) = stderr_handle {
                let mut buf = Vec::new();
                let _ = (&mut err)
                    .take(MAX_OUTPUT_SIZE as u64)
                    .read_to_end(&mut buf)
                    .await;
                String::from_utf8_lossy(&buf).to_string()
            } else {
                String::new()
            }
        };

        let (stdout, stderr, wait_result) = tokio::join!(stdout_fut, stderr_fut, child.wait());
        let status = wait_result.map_err(|e| format!("Wait error: {}", e))?;
        Ok::<_, String>((stdout, stderr, status.code().unwrap_or(-1)))
    })
    .await;

    match result {
        Ok(Ok((stdout, stderr, code))) => {
            let mut combined = if stderr.is_empty() {
                stdout
            } else if stdout.is_empty() {
                stderr
            } else {
                format!("{}\n\n--- stderr ---\n{}", stdout, stderr)
            };

            if combined.len() > MAX_OUTPUT_SIZE {
                let half = MAX_OUTPUT_SIZE / 2;
                let head_end = floor_char_boundary(&combined, half);
                let tail_start =
                    floor_char_boundary(&combined, combined.len().saturating_sub(half));
                combined = format!(
                    "{}\n\n... [truncated {} bytes] ...\n\n{}",
                    &combined[..head_end],
                    combined.len() - MAX_OUTPUT_SIZE,
                    &combined[tail_start..]
                );
            }

            let redacted = redact_secrets(&combined);
            jsonrpc_ok(
                id,
                serde_json::json!({
                    "content": [
                        {
                            "type": "text",
                            "text": redacted
                        }
                    ],
                    "isError": code != 0,
                    "exit_code": code,
                    "success": code == 0
                }),
            )
        }
        Ok(Err(e)) => tool_error(id, format!("Execution error: {}", e)),
        Err(_) => {
            let _ = child.kill().await;
            tool_error(id, format!("Timed out after {:?}", DEFAULT_TIMEOUT))
        }
    }
}

fn tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "gws_bridge",
        description: "Optional fallback pathway wrapping a local gws binary to interact with Google Workspace. Only read-only operations are permitted.",
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments to pass to the gws binary (for example [\"gmail\", \"users\", \"messages\", \"list\"])."
                }
            },
            "required": ["args"]
        }),
        annotations: ToolAnnotations {
            read_only_hint: true,
        },
    }
}

fn jsonrpc_ok(id: serde_json::Value, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn jsonrpc_error(id: serde_json::Value, code: i32, message: String) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError { code, message }),
    }
}

fn tool_error(id: serde_json::Value, message: String) -> JsonRpcResponse {
    jsonrpc_ok(
        id,
        serde_json::to_value(CallToolResult {
            content: vec![ContentBlock::Text { text: message }],
            is_error: true,
        })
        .expect("tool error result serializes"),
    )
}

fn compile_regex(pattern: &str) -> Option<Regex> {
    Regex::new(pattern).ok()
}

fn bridge_enabled_from_env(value: Option<&str>) -> bool {
    matches!(
        value.unwrap_or_default().to_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

fn check_allowlist(args: &[String]) -> Result<(), &'static str> {
    if args.is_empty() {
        return Err("No command provided");
    }

    match args[0].as_str() {
        "auth" => {
            if matches_exact_command(args, &AUTH_STATUS_COMMAND) {
                Ok(())
            } else {
                Err("Only 'auth status' is permitted for auth commands")
            }
        }
        "gmail" => {
            if matches_exact_any_command(args, &GMAIL_READ_COMMANDS) {
                Ok(())
            } else {
                Err("Only explicit read-only Gmail tuples are permitted in phase 1")
            }
        }
        "calendar" => {
            if matches_exact_any_command(args, &CALENDAR_READ_COMMANDS) {
                Ok(())
            } else {
                Err("Only explicit read-only Calendar tuples are permitted in phase 1")
            }
        }
        "drive" => {
            if matches_exact_any_command(args, &DRIVE_READ_COMMANDS) {
                Ok(())
            } else {
                Err("Only explicit read-only Drive tuples are permitted in phase 1")
            }
        }
        _ => Err(
            "Command not in the strict phase 1 allowlist (only auth status, gmail read, calendar read, drive read allowed)",
        ),
    }
}

fn matches_exact_command(args: &[String], allowed: &[&str]) -> bool {
    args.len() == allowed.len()
        && args
            .iter()
            .zip(allowed.iter())
            .all(|(arg, allowed)| arg == allowed)
}

fn matches_exact_any_command(args: &[String], allowed: &[&[&str]]) -> bool {
    allowed
        .iter()
        .any(|allowed| matches_exact_command(args, allowed))
}

fn redact_secrets(input: &str) -> String {
    let mut result = input.to_string();
    if let Some(re) = BEARER_RE.as_ref() {
        result = re.replace_all(&result, "${1}[REDACTED]").to_string();
    }
    if let Some(re) = OAUTH_RE.as_ref() {
        result = re.replace_all(&result, "${1}[REDACTED]").to_string();
    }
    if let Some(re) = YA29_RE.as_ref() {
        result = re
            .replace_all(&result, "[REDACTED_OAUTH_TOKEN]")
            .to_string();
    }
    if let Some(re) = AKIA_RE.as_ref() {
        result = re.replace_all(&result, "[REDACTED_AWS_KEY]").to_string();
    }
    if let Some(re) = SK_RE.as_ref() {
        result = re.replace_all(&result, "[REDACTED_SECRET_KEY]").to_string();
    }
    result
}

fn floor_char_boundary(s: &str, idx: usize) -> usize {
    let mut idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_accepts_read_only_tuples() {
        assert!(check_allowlist(&["auth".into(), "status".into()]).is_ok());
        assert!(check_allowlist(&["gmail".into(), "list".into()]).is_ok());
        assert!(check_allowlist(&[
            "gmail".into(),
            "users".into(),
            "messages".into(),
            "list".into()
        ])
        .is_ok());
        assert!(check_allowlist(&["calendar".into(), "events".into(), "list".into()]).is_ok());
        assert!(check_allowlist(&["drive".into(), "files".into(), "list".into()]).is_ok());
    }

    #[test]
    fn allowlist_blocks_mutating_commands() {
        assert!(check_allowlist(&["gmail".into(), "send".into()]).is_err());
        assert!(check_allowlist(&["calendar".into(), "delete".into()]).is_err());
        assert!(check_allowlist(&["drive".into(), "upload".into()]).is_err());
    }

    #[test]
    fn redact_secrets_masks_known_formats() {
        let redacted = redact_secrets(
            "Bearer abcdefghijklmnopqrstuvwxyz123456\nya29.abcd1234\nAKIA1234567890ABCDEF\nsk-abcdefghijklmnopqrstuvwxyz1234567890",
        );
        assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyz123456"));
        assert!(!redacted.contains("ya29.abcd1234"));
        assert!(!redacted.contains("AKIA1234567890ABCDEF"));
        assert!(!redacted.contains("sk-abcdefghijklmnopqrstuvwxyz1234567890"));
    }

    #[test]
    fn floor_char_boundary_handles_mid_codepoint_indices() {
        let s = "héllo";
        let idx = floor_char_boundary(s, 2);
        assert!(s.is_char_boundary(idx));
    }
}
