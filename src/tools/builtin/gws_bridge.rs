//! GWS Bridge tool.
//!
//! An optional fallback pathway in IronClaw to address Google OAuth blockage
//! in IC-native Google WASM flows. This tool wraps a local `gws` binary explicitly.
//!
//! It allows executing only read-only preflight operations on Gmail, Calendar, and Drive,
//! and is strictly opt-in via environment variables.
//!
//! # Execution
//!
//! Uses `tokio::process::Command` explicitly without shell interpolation for safety.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use regex::Regex;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

/// Maximum output size before truncation (64KB).
const MAX_OUTPUT_SIZE: usize = 64 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// An optional fallback pathway to a local `gws` binary.
#[derive(Debug, Default)]
pub struct GwsBridgeTool;

impl GwsBridgeTool {
    pub fn new() -> Self {
        Self
    }
}

fn strip_wrapping_quotes(s: &str) -> &str {
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        let first = bytes[0] as char;
        let last = bytes[s.len() - 1] as char;
        if (first == '\'' && last == '\'') || (first == '"' && last == '"') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Normalize common model-produced quoting artifacts for `--params`.
///
/// Models often emit shell-quoted forms like:
/// - `--params='{"userId":"me"}'`
/// - `--params` + `'"{...}"'`
fn normalize_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0usize;
    while i < args.len() {
        let arg = &args[i];
        if let Some(raw) = arg.strip_prefix("--params=") {
            let cleaned = strip_wrapping_quotes(raw);
            out.push(format!("--params={}", cleaned));
            i += 1;
            continue;
        }

        if arg == "--params" && i + 1 < args.len() {
            let cleaned = strip_wrapping_quotes(&args[i + 1]).to_string();
            out.push(arg.clone());
            out.push(cleaned);
            i += 2;
            continue;
        }

        out.push(arg.clone());
        i += 1;
    }
    out
}

fn prepend(head: &[&str], tail: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(head.len() + tail.len());
    out.extend(head.iter().map(|s| s.to_string()));
    out.extend_from_slice(tail);
    out
}

fn parse_params_json(args: &[String]) -> Option<serde_json::Value> {
    for i in 0..args.len() {
        if let Some(raw) = args[i].strip_prefix("--params=") {
            return serde_json::from_str::<serde_json::Value>(raw).ok();
        }
        if args[i] == "--params" && i + 1 < args.len() {
            return serde_json::from_str::<serde_json::Value>(&args[i + 1]).ok();
        }
    }
    None
}

fn parse_args_from_params(params: &serde_json::Value) -> Result<Vec<String>, ToolError> {
    if let Some(args_val) = params.get("args") {
        return serde_json::from_value(args_val.clone()).map_err(|e| {
            ToolError::InvalidParameters(format!("'args' must be an array of strings: {}", e))
        });
    }

    let Some(obj) = params.as_object() else {
        return Err(ToolError::InvalidParameters(
            "Expected object params with either 'args' or compatibility shape \
             {service, resource, method}"
                .to_string(),
        ));
    };

    // Compatibility shape:
    // { "service":"calendar","resource":"events","method":"list","params":{...} }
    let service = obj.get("service").and_then(|v| v.as_str());
    let resource = obj.get("resource").and_then(|v| v.as_str());
    let sub_resource = obj
        .get("sub_resource")
        .or_else(|| obj.get("subResource"))
        .and_then(|v| v.as_str());
    let method = obj.get("method").and_then(|v| v.as_str());

    if let (Some(service), Some(resource), Some(method)) = (service, resource, method) {
        let mut args = vec![service.to_string(), resource.to_string()];
        if let Some(sr) = sub_resource {
            args.push(sr.to_string());
        }
        args.push(method.to_string());
        if let Some(p) = obj.get("params")
            && p.is_object()
        {
            let ptxt = serde_json::to_string(p).map_err(|e| {
                ToolError::InvalidParameters(format!("Failed to serialize 'params': {}", e))
            })?;
            args.push(format!("--params={}", ptxt));
        }
        if let Some(j) = obj.get("json")
            && (j.is_object() || j.is_array())
        {
            let jtxt = serde_json::to_string(j).map_err(|e| {
                ToolError::InvalidParameters(format!("Failed to serialize 'json': {}", e))
            })?;
            args.push(format!("--json={}", jtxt));
        }
        return Ok(args);
    }

    Err(ToolError::InvalidParameters(format!(
        "Unrecognized params shape for gws_bridge. Provide 'args' or compatibility \
         shape with service/resource/method. Keys: {:?}",
        obj.keys().collect::<Vec<_>>()
    )))
}

/// Canonicalize common shorthand emitted by models into valid `gws` syntax.
///
/// `gws` requires: `<service> <resource> [sub-resource] <method>`.
/// Models frequently emit short forms like `list` or `messages list`.
fn canonicalize_args(args: &[String]) -> Vec<String> {
    if args.is_empty() {
        return Vec::new();
    }

    match args[0].as_str() {
        // Ambiguous bare `list` can occur; disambiguate by params.
        "list" => {
            if let Some(params) = parse_params_json(args)
                && let Some(obj) = params.as_object()
            {
                let has_calendar_keys = obj.contains_key("timeMin")
                    || obj.contains_key("timeMax")
                    || obj.contains_key("singleEvents")
                    || obj.contains_key("orderBy")
                    || obj.get("calendarId").and_then(|v| v.as_str()).is_some();
                if has_calendar_keys {
                    return prepend(&["calendar", "events", "list"], &args[1..]);
                }
                let looks_like_spam = obj
                    .get("q")
                    .and_then(|v| v.as_str())
                    .map(|q| q.to_lowercase().contains("in:spam"))
                    .unwrap_or(false)
                    || obj.get("labelIds").is_some();
                if looks_like_spam {
                    return prepend(&["gmail", "users", "messages", "list"], &args[1..]);
                }
            }
            // Unknown `list` should fail allowlist so the model is forced
            // to provide an explicit service/resource/method.
            args.to_vec()
        }
        "messages" => {
            if args.len() >= 2 && matches!(args[1].as_str(), "list" | "get") {
                let mut out = vec![
                    "gmail".to_string(),
                    "users".to_string(),
                    "messages".to_string(),
                    args[1].clone(),
                ];
                out.extend_from_slice(&args[2..]);
                out
            } else {
                prepend(&["gmail", "users", "messages", "list"], &args[1..])
            }
        }
        "threads" => {
            if args.len() >= 2 && matches!(args[1].as_str(), "list" | "get") {
                let mut out = vec![
                    "gmail".to_string(),
                    "users".to_string(),
                    "threads".to_string(),
                    args[1].clone(),
                ];
                out.extend_from_slice(&args[2..]);
                out
            } else {
                prepend(&["gmail", "users", "threads", "list"], &args[1..])
            }
        }
        // Accept `gmail list` shorthand.
        "gmail" => {
            if args.len() >= 2 && args[1] == "list" {
                let mut out = vec![
                    "gmail".to_string(),
                    "users".to_string(),
                    "messages".to_string(),
                    "list".to_string(),
                ];
                out.extend_from_slice(&args[2..]);
                out
            } else {
                args.to_vec()
            }
        }
        // Accept `calendar list` shorthand.
        "calendar" => {
            if args.len() >= 2 && args[1] == "list" {
                let mut out = vec![
                    "calendar".to_string(),
                    "events".to_string(),
                    "list".to_string(),
                ];
                out.extend_from_slice(&args[2..]);
                out
            } else {
                args.to_vec()
            }
        }
        // Accept `drive list` shorthand.
        "drive" => {
            if args.len() >= 2 && args[1] == "list" {
                let mut out = vec!["drive".to_string(), "files".to_string(), "list".to_string()];
                out.extend_from_slice(&args[2..]);
                out
            } else {
                args.to_vec()
            }
        }
        _ => args.to_vec(),
    }
}

fn normalize_gmail_list_params(args: &[String]) -> Vec<String> {
    if args.len() < 4
        || args[0] != "gmail"
        || args[1] != "users"
        || args[2] != "messages"
        || args[3] != "list"
    {
        return args.to_vec();
    }

    let mut out = args.to_vec();

    let rewrite_params = |raw: &str| -> Option<String> {
        let mut parsed = serde_json::from_str::<serde_json::Value>(raw).ok()?;
        let obj = parsed.as_object_mut()?;
        if let Some(label_ids) = obj.get("labelIds").cloned()
            && let Some(arr) = label_ids.as_array()
        {
            let joined = arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(",");
            if !joined.is_empty() {
                obj.insert("labelIds".to_string(), serde_json::Value::String(joined));
                return serde_json::to_string(&parsed).ok();
            }
        }
        None
    };

    for i in 0..out.len() {
        if let Some(raw) = out[i].strip_prefix("--params=") {
            if let Some(rewritten) = rewrite_params(raw) {
                out[i] = format!("--params={}", rewritten);
            }
            return out;
        }
        if out[i] == "--params" && i + 1 < out.len() {
            if let Some(rewritten) = rewrite_params(&out[i + 1]) {
                out[i + 1] = rewritten;
            }
            return out;
        }
    }

    out
}

fn normalize_calendar_list_params(args: &[String]) -> Vec<String> {
    if args.len() < 3 || args[0] != "calendar" || args[1] != "events" || args[2] != "list" {
        return args.to_vec();
    }

    let mut out = args.to_vec();
    let start = Utc::now();
    let end = start + ChronoDuration::days(7);

    let ensure_window = |raw: &str| -> Option<String> {
        let mut parsed = serde_json::from_str::<serde_json::Value>(raw).ok()?;
        let obj = parsed.as_object_mut()?;
        obj.entry("calendarId".to_string())
            .or_insert_with(|| serde_json::Value::String("primary".to_string()));
        obj.entry("singleEvents".to_string())
            .or_insert(serde_json::Value::Bool(true));
        obj.entry("orderBy".to_string())
            .or_insert_with(|| serde_json::Value::String("startTime".to_string()));
        obj.entry("maxResults".to_string())
            .or_insert_with(|| serde_json::Value::Number(50.into()));
        obj.entry("timeMin".to_string())
            .or_insert_with(|| serde_json::Value::String(start.to_rfc3339()));
        obj.entry("timeMax".to_string())
            .or_insert_with(|| serde_json::Value::String(end.to_rfc3339()));
        serde_json::to_string(&parsed).ok()
    };

    for i in 0..out.len() {
        if let Some(raw) = out[i].strip_prefix("--params=") {
            if let Some(rewritten) = ensure_window(raw) {
                out[i] = format!("--params={}", rewritten);
            }
            return out;
        }
        if out[i] == "--params" && i + 1 < out.len() {
            if let Some(rewritten) = ensure_window(&out[i + 1]) {
                out[i + 1] = rewritten;
            }
            return out;
        }
    }

    out.push(format!(
        "--params={{\"calendarId\":\"primary\",\"singleEvents\":true,\"orderBy\":\"startTime\",\"maxResults\":50,\"timeMin\":\"{}\",\"timeMax\":\"{}\"}}",
        start.to_rfc3339(),
        end.to_rfc3339()
    ));
    out
}

/// Helper to parse arguments properly, separating commands and args
fn check_allowlist(args: &[String]) -> Result<(), &'static str> {
    if args.is_empty() {
        return Err("No command provided");
    }

    match args[0].as_str() {
        "auth" => {
            if args.len() == 2 && args[1] == "status" {
                return Ok(());
            }
            Err("Only 'auth status' is permitted for auth commands")
        }
        "gmail" => {
            if args.len() < 4 || args[1] != "users" {
                return Err(
                    "gmail commands must use canonical form: gmail users <resource> <method>",
                );
            }
            let resource = args[2].as_str();
            let method = args[3].as_str();
            let ok = matches!(resource, "messages" | "threads" | "labels")
                && matches!(method, "list" | "get");
            if ok {
                validate_extra_flags(args, 4)
            } else {
                Err(
                    "Allowed gmail commands are read-only: gmail users {messages|threads|labels} {list|get}",
                )
            }
        }
        "calendar" => {
            if args.len() < 3 {
                return Err(
                    "calendar commands must use canonical form: calendar <resource> <method>",
                );
            }
            let resource = args[1].as_str();
            let method = args[2].as_str();
            let ok = matches!(resource, "events" | "calendars") && matches!(method, "list" | "get");
            if ok {
                validate_extra_flags(args, 3)
            } else {
                Err(
                    "Allowed calendar commands are read-only: calendar {events|calendars} {list|get}",
                )
            }
        }
        "drive" => {
            if args.len() < 3 {
                return Err("drive commands must use canonical form: drive <resource> <method>");
            }
            let resource = args[1].as_str();
            let method = args[2].as_str();
            let ok = matches!(resource, "files" | "drives") && matches!(method, "list" | "get");
            if ok {
                validate_extra_flags(args, 3)
            } else {
                Err("Allowed drive commands are read-only: drive {files|drives} {list|get}")
            }
        }
        _ => Err(
            "Command not in strict allowlist (only auth status and read-only gmail/calendar/drive commands allowed)",
        ),
    }
}

fn validate_extra_flags(args: &[String], start_idx: usize) -> Result<(), &'static str> {
    let mut i = start_idx;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--params" || arg == "--json" {
            // Split form: requires a following JSON payload token.
            if i + 1 >= args.len() {
                return Err("Flag is missing required value");
            }
            i += 2;
            continue;
        }
        if arg.starts_with("--params=") || arg.starts_with("--json=") {
            i += 1;
            continue;
        }
        return Err("Only --params/--json flags are permitted");
    }
    Ok(())
}

/// Apply basic regex redaction to hide common secret formats from outputs.
fn redact_secrets(input: &str) -> String {
    use std::sync::LazyLock;
    static BEARER_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(bearer\s+)([a-zA-Z0-9_\-\.]{20,})").unwrap());
    static OAUTH_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)(token[=\'":\s]+)([a-zA-Z0-9_\-\.]{20,})"#).unwrap());
    static YA29_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(ya29\.[a-zA-Z0-9_\-\.]+)").unwrap());
    static AKIA_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(AKIA[0-9A-Z]{16})").unwrap());
    static SK_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(sk-[a-zA-Z0-9]{32,})").unwrap());

    let result = BEARER_RE.replace_all(input, "${1}[REDACTED]");
    let result = OAUTH_RE.replace_all(&result, "${1}[REDACTED]");
    let result = YA29_RE.replace_all(&result, "[REDACTED_OAUTH_TOKEN]");
    let result = AKIA_RE.replace_all(&result, "[REDACTED_AWS_KEY]");
    let result = SK_RE.replace_all(&result, "[REDACTED_SECRET_KEY]");
    result.into_owned()
}

fn extract_message_count(args: &[String], parsed: &serde_json::Value) -> Option<usize> {
    // Only annotate Gmail list calls, where result count is a common user-facing need.
    if args.len() < 4
        || args[0] != "gmail"
        || args[1] != "users"
        || args[2] != "messages"
        || args[3] != "list"
    {
        return None;
    }

    if let Some(arr) = parsed.get("messages").and_then(|m| m.as_array()) {
        return Some(arr.len());
    }

    parsed
        .get("resultSizeEstimate")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
}

fn should_retry_spam_zero(args: &[String], message_count: Option<usize>) -> bool {
    if message_count != Some(0) {
        return false;
    }
    if args.len() < 4
        || args[0] != "gmail"
        || args[1] != "users"
        || args[2] != "messages"
        || args[3] != "list"
    {
        return false;
    }
    let Some(params) = parse_params_json(args) else {
        return false;
    };
    let Some(obj) = params.as_object() else {
        return false;
    };

    let q_is_spam = obj
        .get("q")
        .and_then(|v| v.as_str())
        .is_some_and(|q| q.to_lowercase().contains("in:spam"));
    let labels_include_spam = match obj.get("labelIds") {
        Some(serde_json::Value::String(s)) => s.to_lowercase().contains("spam"),
        Some(serde_json::Value::Array(a)) => a.iter().any(|v| {
            v.as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case("spam") || s.eq_ignore_ascii_case("SPAM"))
        }),
        _ => false,
    };

    q_is_spam || labels_include_spam
}

fn build_spam_retry_args(args: &[String]) -> Vec<String> {
    let user_id = parse_params_json(args)
        .and_then(|v| {
            v.get("userId")
                .and_then(|u| u.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| "me".to_string());

    vec![
        "gmail".to_string(),
        "users".to_string(),
        "messages".to_string(),
        "list".to_string(),
        format!(
            "--params={{\"userId\":\"{}\",\"q\":\"in:spam\",\"maxResults\":50}}",
            user_id
        ),
    ]
}

async fn run_gws_command(bin_path: &str, args: &[String]) -> Result<(String, i32), ToolError> {
    let mut command = Command::new(bin_path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| {
        let mut msg = format!("Failed to spawn {}: {}", bin_path, e);
        if e.kind() == std::io::ErrorKind::NotFound {
            msg.push_str("\nMake sure the binary is installed. If it's not in your PATH, you can configure it via the GWS_BINARY_PATH environment variable (e.g., GWS_BINARY_PATH=/Users/username/.cargo/bin/gws).");
        }
        ToolError::ExecutionFailed(msg)
    })?;

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
        let combined = if stderr.is_empty() {
            stdout
        } else if stdout.is_empty() {
            stderr
        } else {
            format!("{}\n\n--- stderr ---\n{}", stdout, stderr)
        };
        Ok::<_, String>((combined, status.code().unwrap_or(-1)))
    })
    .await;

    match result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(ToolError::ExecutionFailed(format!(
            "Execution error: {}",
            e
        ))),
        Err(_) => {
            let _ = child.kill().await;
            Err(ToolError::Timeout(DEFAULT_TIMEOUT))
        }
    }
}

#[async_trait]
impl Tool for GwsBridgeTool {
    fn name(&self) -> &str {
        "gws_bridge"
    }

    fn description(&self) -> &str {
        "Optional fallback pathway wrapping a local 'gws' binary to interact with Google Workspace. \
         Note: IC-native Google WASM tools are primary/default. This tool must be explicitly enabled \
         via GWS_BRIDGE_ENABLED environment variable. Only read-only operations on Gmail, Calendar, \
         and Drive are permitted. Preferred args examples: [\"gmail\",\"users\",\"messages\",\"list\",\"--params={\\\"userId\\\":\\\"me\\\"}\"] \
         or [\"calendar\",\"events\",\"list\",\"--params={\\\"calendarId\\\":\\\"primary\\\"}\"]."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "args": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Arguments to pass to the gws binary in canonical form: <service> <resource> [sub-resource] <method> [flags]"
                }
            },
            "required": ["args"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // 1. Check if tool is enabled at runtime
        let enabled = std::env::var("GWS_BRIDGE_ENABLED")
            .unwrap_or_default()
            .to_lowercase();
        if !["true", "1", "yes", "on"].contains(&enabled.as_str()) {
            return Err(ToolError::ExecutionFailed(
                "gws_bridge is disabled. It is an optional fallback and must be explicitly enabled \
                 by setting GWS_BRIDGE_ENABLED=true in the environment.".to_string(),
            ));
        }

        // 2. Parse arguments
        let args = parse_args_from_params(&params)?;
        let args = normalize_calendar_list_params(&normalize_gmail_list_params(
            &canonicalize_args(&normalize_args(&args)),
        ));

        // 3. Strict allowlist validation
        if let Err(reason) = check_allowlist(&args) {
            return Err(ToolError::NotAuthorized(format!(
                "Command blocked by allowlist: {}",
                reason
            )));
        }

        // 4. Determine binary path
        let bin_path = std::env::var("GWS_BINARY_PATH").unwrap_or_else(|_| "gws".to_string());
        if bin_path.is_empty() {
            // Unlikely to be empty if derived from unwrap_or_else, but just in case
            return Err(ToolError::ExecutionFailed(
                "GWS_BINARY_PATH is set but empty. Please set it to a valid path or leave it unset to use 'gws' from PATH. \
                 Example: GWS_BINARY_PATH=/Users/username/.cargo/bin/gws".to_string(),
            ));
        }

        // 5. Execute command directly (no shell interpolation)
        match run_gws_command(&bin_path, &args).await {
            Ok((mut combined, code)) => {
                // Truncate if somehow larger than limit (safety)
                if combined.len() > MAX_OUTPUT_SIZE {
                    let half = MAX_OUTPUT_SIZE / 2;
                    let head_end = crate::util::floor_char_boundary(&combined, half);
                    let tail_start =
                        crate::util::floor_char_boundary(&combined, combined.len() - half);
                    combined = format!(
                        "{}\n\n... [truncated {} bytes] ...\n\n{}",
                        &combined[..head_end],
                        combined.len() - MAX_OUTPUT_SIZE,
                        &combined[tail_start..]
                    );
                }

                // Apply redaction
                let mut redacted = redact_secrets(&combined);

                let mut parsed_output = serde_json::from_str::<serde_json::Value>(&redacted).ok();
                let mut message_count = parsed_output
                    .as_ref()
                    .and_then(|p| extract_message_count(&args, p));

                // Retry for known false-zero spam list cases.
                if should_retry_spam_zero(&args, message_count) {
                    let retry_args = build_spam_retry_args(&args);
                    if let Ok((retry_out, retry_code)) =
                        run_gws_command(&bin_path, &retry_args).await
                        && retry_code == 0
                    {
                        let retry_redacted = redact_secrets(&retry_out);
                        let retry_parsed =
                            serde_json::from_str::<serde_json::Value>(&retry_redacted).ok();
                        let retry_count = retry_parsed
                            .as_ref()
                            .and_then(|p| extract_message_count(&retry_args, p));
                        if retry_count.unwrap_or(0) > 0 {
                            redacted = retry_redacted;
                            parsed_output = retry_parsed;
                            message_count = retry_count;
                        }
                    }
                }

                let output_json = serde_json::json!({
                    "output": redacted,
                    "exit_code": code,
                    "success": code == 0,
                    "parsed_output": parsed_output,
                    "message_count": message_count,
                });

                Ok(ToolOutput::success(output_json, start.elapsed()))
            }
            Err(e) => Err(e),
        }
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Host process execution should always require explicit user approval.
        ApprovalRequirement::Always
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn requires_sanitization(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_auth_status() {
        assert!(check_allowlist(&["auth".to_string(), "status".to_string()]).is_ok());
        assert!(check_allowlist(&["auth".to_string(), "login".to_string()]).is_err());
    }

    #[test]
    fn test_allowlist_read_only() {
        assert!(
            check_allowlist(&[
                "gmail".to_string(),
                "users".to_string(),
                "messages".to_string(),
                "list".to_string()
            ])
            .is_ok()
        );
        assert!(
            check_allowlist(&[
                "calendar".to_string(),
                "events".to_string(),
                "list".to_string()
            ])
            .is_ok()
        );
        assert!(
            check_allowlist(&["drive".to_string(), "files".to_string(), "list".to_string()])
                .is_ok()
        );
    }

    #[test]
    fn test_allowlist_blocks_mutating() {
        assert!(check_allowlist(&["gmail".to_string(), "send".to_string()]).is_err());
        assert!(check_allowlist(&["calendar".to_string(), "create".to_string()]).is_err());
        assert!(check_allowlist(&["drive".to_string(), "upload".to_string()]).is_err());
        assert!(check_allowlist(&["drive".to_string(), "trash".to_string()]).is_err());
        assert!(check_allowlist(&["gmail".to_string(), "modify".to_string()]).is_err());
        assert!(check_allowlist(&["calendar".to_string(), "delete".to_string()]).is_err());
    }

    #[test]
    fn test_allowlist_blocks_unknown() {
        assert!(check_allowlist(&["unknown_command".to_string()]).is_err());
        assert!(check_allowlist(&[]).is_err());
    }

    #[test]
    fn test_allowlist_blocks_dangerous_flags() {
        assert!(
            check_allowlist(&[
                "gmail".to_string(),
                "users".to_string(),
                "messages".to_string(),
                "list".to_string(),
                "--config=/tmp/evil".to_string()
            ])
            .is_err()
        );
    }

    #[test]
    fn test_allowlist_accepts_supported_flags() {
        assert!(
            check_allowlist(&[
                "calendar".to_string(),
                "events".to_string(),
                "list".to_string(),
                "--params={\"calendarId\":\"primary\"}".to_string()
            ])
            .is_ok()
        );
        assert!(
            check_allowlist(&[
                "drive".to_string(),
                "files".to_string(),
                "list".to_string(),
                "--json".to_string(),
                "{}".to_string()
            ])
            .is_ok()
        );
    }

    #[test]
    fn test_normalize_args_params_equals_wrapped_quotes() {
        let input = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "list".to_string(),
            "--params='{\"userId\":\"me\",\"q\":\"in:spam\"}'".to_string(),
        ];
        let out = normalize_args(&input);
        assert_eq!(
            out[4],
            "--params={\"userId\":\"me\",\"q\":\"in:spam\"}".to_string()
        );
    }

    #[test]
    fn test_normalize_args_params_separate_wrapped_quotes() {
        let input = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "get".to_string(),
            "--params".to_string(),
            "'{\"userId\":\"me\",\"id\":\"123\"}'".to_string(),
        ];
        let out = normalize_args(&input);
        assert_eq!(out[4], "--params".to_string());
        assert_eq!(out[5], "{\"userId\":\"me\",\"id\":\"123\"}".to_string());
    }

    #[test]
    fn test_canonicalize_args_bare_list_to_gmail_messages_list() {
        let input = vec![
            "list".to_string(),
            "--params={\"userId\":\"me\",\"q\":\"in:spam\"}".to_string(),
        ];
        let out = canonicalize_args(&input);
        assert_eq!(
            out,
            vec![
                "gmail".to_string(),
                "users".to_string(),
                "messages".to_string(),
                "list".to_string(),
                "--params={\"userId\":\"me\",\"q\":\"in:spam\"}".to_string()
            ]
        );
    }

    #[test]
    fn test_canonicalize_args_messages_list_to_gmail_users_messages_list() {
        let input = vec![
            "messages".to_string(),
            "list".to_string(),
            "--params={\"userId\":\"me\"}".to_string(),
        ];
        let out = canonicalize_args(&input);
        assert_eq!(
            out,
            vec![
                "gmail".to_string(),
                "users".to_string(),
                "messages".to_string(),
                "list".to_string(),
                "--params={\"userId\":\"me\"}".to_string()
            ]
        );
    }

    #[test]
    fn test_canonicalize_args_gmail_list_to_users_messages_list() {
        let input = vec![
            "gmail".to_string(),
            "list".to_string(),
            "--params={\"userId\":\"me\"}".to_string(),
        ];
        let out = canonicalize_args(&input);
        assert_eq!(
            out,
            vec![
                "gmail".to_string(),
                "users".to_string(),
                "messages".to_string(),
                "list".to_string(),
                "--params={\"userId\":\"me\"}".to_string()
            ]
        );
    }

    #[test]
    fn test_redact_secrets() {
        let text = "Output: Bearer abcdefghijklmnopqrstuvwxyz123456\nOther: ya29.abcdefg1234567890\nKey: AKIA1234567890ABCDEF\nSk: sk-abcdefghijklmnopqrstuvwxyz1234567890";
        let redacted = redact_secrets(text);

        assert!(redacted.contains("Bearer [REDACTED]"));
        assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyz123456"));

        assert!(redacted.contains("[REDACTED_OAUTH_TOKEN]"));
        assert!(!redacted.contains("ya29.abcdefg1234567890"));

        assert!(redacted.contains("[REDACTED_AWS_KEY]"));
        assert!(!redacted.contains("AKIA1234567890ABCDEF"));

        assert!(redacted.contains("[REDACTED_SECRET_KEY]"));
        assert!(!redacted.contains("sk-abcdefghijklmnopqrstuvwxyz1234567890"));
    }

    #[test]
    fn test_extract_message_count_from_messages_array() {
        let args = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "list".to_string(),
        ];
        let parsed = serde_json::json!({
            "messages": [{"id":"1"}, {"id":"2"}, {"id":"3"}],
            "resultSizeEstimate": 99
        });
        assert_eq!(extract_message_count(&args, &parsed), Some(3));
    }

    #[test]
    fn test_extract_message_count_from_estimate_when_no_messages() {
        let args = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "list".to_string(),
        ];
        let parsed = serde_json::json!({
            "resultSizeEstimate": 4
        });
        assert_eq!(extract_message_count(&args, &parsed), Some(4));
    }

    #[test]
    fn test_extract_message_count_none_for_non_gmail_list() {
        let args = vec![
            "calendar".to_string(),
            "events".to_string(),
            "list".to_string(),
        ];
        let parsed = serde_json::json!({
            "items": [1,2,3]
        });
        assert_eq!(extract_message_count(&args, &parsed), None);
    }

    #[test]
    fn test_normalize_gmail_list_params_label_ids_array_equals_form() {
        let input = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "list".to_string(),
            "--params={\"userId\":\"me\",\"labelIds\":[\"SPAM\"],\"maxResults\":10}".to_string(),
        ];
        let out = normalize_gmail_list_params(&input);
        assert_eq!(
            out[4],
            "--params={\"labelIds\":\"SPAM\",\"maxResults\":10,\"userId\":\"me\"}".to_string()
        );
    }

    #[test]
    fn test_normalize_gmail_list_params_label_ids_array_split_form() {
        let input = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "list".to_string(),
            "--params".to_string(),
            "{\"userId\":\"me\",\"labelIds\":[\"SPAM\",\"INBOX\"]}".to_string(),
        ];
        let out = normalize_gmail_list_params(&input);
        assert_eq!(out[4], "--params".to_string());
        assert_eq!(
            out[5],
            "{\"labelIds\":\"SPAM,INBOX\",\"userId\":\"me\"}".to_string()
        );
    }

    #[test]
    fn test_should_retry_spam_zero_when_label_ids_array_spam() {
        let args = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "list".to_string(),
            "--params={\"userId\":\"me\",\"labelIds\":[\"SPAM\"],\"maxResults\":10}".to_string(),
        ];
        assert!(should_retry_spam_zero(&args, Some(0)));
    }

    #[test]
    fn test_should_retry_spam_zero_when_params_missing() {
        let args = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "list".to_string(),
        ];
        assert!(!should_retry_spam_zero(&args, Some(0)));
    }

    #[test]
    fn test_should_not_retry_spam_when_non_zero() {
        let args = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "list".to_string(),
            "--params={\"userId\":\"me\",\"q\":\"in:spam\"}".to_string(),
        ];
        assert!(!should_retry_spam_zero(&args, Some(3)));
    }

    #[test]
    fn test_should_retry_spam_zero_even_without_spam_markers() {
        let args = vec![
            "gmail".to_string(),
            "users".to_string(),
            "messages".to_string(),
            "list".to_string(),
            "--params={\"userId\":\"me\",\"maxResults\":50}".to_string(),
        ];
        assert!(!should_retry_spam_zero(&args, Some(0)));
    }

    #[test]
    fn test_parse_params_json_equals_form() {
        let args = vec![
            "calendar".to_string(),
            "events".to_string(),
            "list".to_string(),
            "--params={\"calendarId\":\"primary\"}".to_string(),
        ];
        let parsed = parse_params_json(&args).expect("params parsed");
        assert_eq!(parsed["calendarId"], "primary");
    }

    #[test]
    fn test_normalize_calendar_list_params_adds_default_window_when_missing() {
        let input = vec![
            "calendar".to_string(),
            "events".to_string(),
            "list".to_string(),
        ];
        let out = normalize_calendar_list_params(&input);
        assert_eq!(out[0], "calendar");
        assert_eq!(out[1], "events");
        assert_eq!(out[2], "list");
        assert!(out.iter().any(|a| a.starts_with("--params=")));
        let params = out
            .iter()
            .find_map(|a| a.strip_prefix("--params="))
            .expect("params present");
        let parsed: serde_json::Value = serde_json::from_str(params).expect("valid json");
        assert_eq!(parsed["calendarId"], "primary");
        assert_eq!(parsed["singleEvents"], true);
        assert_eq!(parsed["orderBy"], "startTime");
        assert!(parsed.get("timeMin").is_some());
        assert!(parsed.get("timeMax").is_some());
    }

    #[test]
    fn test_normalize_calendar_list_params_preserves_existing_and_fills_missing() {
        let input = vec![
            "calendar".to_string(),
            "events".to_string(),
            "list".to_string(),
            "--params={\"calendarId\":\"work\",\"maxResults\":10}".to_string(),
        ];
        let out = normalize_calendar_list_params(&input);
        let params = out[3]
            .strip_prefix("--params=")
            .expect("params form retained");
        let parsed: serde_json::Value = serde_json::from_str(params).expect("valid json");
        assert_eq!(parsed["calendarId"], "work");
        assert_eq!(parsed["maxResults"], 10);
        assert_eq!(parsed["singleEvents"], true);
        assert_eq!(parsed["orderBy"], "startTime");
        assert!(parsed.get("timeMin").is_some());
        assert!(parsed.get("timeMax").is_some());
    }

    #[test]
    fn test_canonicalize_bare_list_to_calendar_when_calendar_params_present() {
        let input = vec![
            "list".to_string(),
            "--params={\"calendarId\":\"primary\",\"timeMin\":\"2026-03-11T00:00:00Z\"}"
                .to_string(),
        ];
        let out = canonicalize_args(&input);
        assert_eq!(
            out,
            vec![
                "calendar".to_string(),
                "events".to_string(),
                "list".to_string(),
                "--params={\"calendarId\":\"primary\",\"timeMin\":\"2026-03-11T00:00:00Z\"}"
                    .to_string()
            ]
        );
    }

    #[test]
    fn test_canonicalize_bare_list_kept_when_untyped() {
        let input = vec!["list".to_string()];
        let out = canonicalize_args(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn test_parse_args_from_params_compat_shape() {
        let params = serde_json::json!({
            "service": "calendar",
            "resource": "events",
            "method": "list",
            "params": { "calendarId": "primary" }
        });
        let out = parse_args_from_params(&params).expect("parsed");
        assert_eq!(out[0], "calendar");
        assert_eq!(out[1], "events");
        assert_eq!(out[2], "list");
        assert!(out[3].starts_with("--params="));
    }

    #[test]
    fn test_parse_args_from_params_unrecognized_shape_is_error() {
        let params = serde_json::json!({});
        let out = parse_args_from_params(&params);
        assert!(out.is_err());
    }

    #[tokio::test]
    async fn test_gws_bridge_disabled_by_default() {
        let tool = GwsBridgeTool::new();
        let ctx = JobContext::default();
        // SAFETY: This test mutates process environment in a single-threaded
        // section to validate runtime opt-in behavior.
        unsafe { std::env::remove_var("GWS_BRIDGE_ENABLED") };

        let result = tool
            .execute(serde_json::json!({"args": ["auth", "status"]}), &ctx)
            .await;
        assert!(result.is_err());
        if let Err(ToolError::ExecutionFailed(msg)) = result {
            assert!(msg.contains("disabled"));
        } else {
            panic!("Expected ExecutionFailed");
        }
    }
}
