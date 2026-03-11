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

/// Helper to parse arguments properly, separating commands and args
fn check_allowlist(args: &[String]) -> Result<(), &'static str> {
    if args.is_empty() {
        return Err("No command provided");
    }

    let cmd = args[0].as_str();

    match cmd {
        "auth" => {
            if args.len() > 1 && args[1] == "status" {
                Ok(())
            } else {
                Err("Only 'auth status' is permitted for auth commands")
            }
        }
        "gmail" | "calendar" | "drive" => {
            // Check for mutating operations
            let mutating_patterns = [
                "create", "update", "delete", "send", "trash", "modify", "insert", "patch", "move",
                "copy", "upload", "clear", "import", "export",
            ];

            for arg in &args[1..] {
                let lower = arg.to_lowercase();
                for pat in mutating_patterns {
                    if lower.contains(pat) {
                        return Err(
                            "Mutating operations (create/update/delete/send/etc) are blocked in phase 1",
                        );
                    }
                }
            }
            Ok(())
        }
        _ => Err(
            "Command not in the strict phase 1 allowlist (only auth status, gmail read, calendar read, drive read allowed)",
        ),
    }
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

    let mut result = input.to_string();
    result = BEARER_RE.replace_all(&result, "${1}[REDACTED]").to_string();
    result = OAUTH_RE.replace_all(&result, "${1}[REDACTED]").to_string();
    result = YA29_RE
        .replace_all(&result, "[REDACTED_OAUTH_TOKEN]")
        .to_string();
    result = AKIA_RE
        .replace_all(&result, "[REDACTED_AWS_KEY]")
        .to_string();
    result = SK_RE
        .replace_all(&result, "[REDACTED_SECRET_KEY]")
        .to_string();

    result
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
         and Drive are permitted."
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
                    "description": "Arguments to pass to the gws binary (e.g., [\"gmail\", \"list\"])"
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
        let args_val = params.get("args").ok_or_else(|| {
            ToolError::InvalidParameters("Missing 'args' array parameter".to_string())
        })?;

        let args: Vec<String> = serde_json::from_value(args_val.clone()).map_err(|e| {
            ToolError::InvalidParameters(format!("'args' must be an array of strings: {}", e))
        })?;

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
        let mut command = Command::new(&bin_path);
        command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                let mut msg = format!("Failed to spawn {}: {}", bin_path, e);
                if e.kind() == std::io::ErrorKind::NotFound {
                    msg.push_str("\nMake sure the binary is installed. If it's not in your PATH, you can configure it via the GWS_BINARY_PATH environment variable (e.g., GWS_BINARY_PATH=/Users/username/.cargo/bin/gws).");
                }
                return Err(ToolError::ExecutionFailed(msg));
            }
        };

        // 6. Capture output with bounded size
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
                let redacted = redact_secrets(&combined);

                let output_json = serde_json::json!({
                    "output": redacted,
                    "exit_code": code,
                    "success": code == 0,
                });

                Ok(ToolOutput::success(output_json, start.elapsed()))
            }
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

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Since we already block mutating commands in execute/check_allowlist,
        // what remains is safe to auto-approve.
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
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
        assert!(check_allowlist(&["gmail".to_string(), "list".to_string()]).is_ok());
        assert!(
            check_allowlist(&[
                "calendar".to_string(),
                "events".to_string(),
                "list".to_string()
            ])
            .is_ok()
        );
        assert!(check_allowlist(&["drive".to_string(), "files".to_string()]).is_ok());
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

    #[tokio::test]
    async fn test_gws_bridge_disabled_by_default() {
        let tool = GwsBridgeTool::new();
        let ctx = JobContext::default();
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
