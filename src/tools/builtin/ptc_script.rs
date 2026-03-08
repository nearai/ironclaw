//! PTC script tool for running multi-step Python programs that call tools.
//!
//! Wraps user-provided Python code in a preamble that imports the IronClaw
//! SDK (`ironclaw_tools`), then executes it via `python3 -c`. The script
//! runs in the same environment as the worker container and can call any
//! registered tool through the SDK's `call_tool()` function.

use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::context::JobContext;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};

/// Maximum output size before truncation (64KB).
const MAX_OUTPUT_SIZE: usize = 64 * 1024;

/// Default script timeout.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Maximum allowed timeout.
const MAX_TIMEOUT_SECS: u64 = 300;

/// Environment variables safe to forward to the Python subprocess.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "PWD",
    "TMPDIR",
    "TMP",
    "TEMP",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
];

/// PTC environment variables required by the ironclaw_tools SDK.
const PTC_ENV_VARS: &[&str] = &[
    "IRONCLAW_ORCHESTRATOR_URL",
    "IRONCLAW_JOB_ID",
    "IRONCLAW_WORKER_TOKEN",
];

/// Python preamble injected before the user's script.
const PREAMBLE: &str = r#"
import json, sys, os

# Import IronClaw SDK
from ironclaw_tools import call_tool, shell, read_file, write_file, http_get

# Structured output collector
_ptc_outputs = {}

def ptc_output(key, value):
    """Register a named output value for structured results."""
    _ptc_outputs[key] = value

try:
"#;

/// Python postamble appended after the user's script.
const POSTAMBLE: &str = r#"
except Exception as _ptc_err:
    print(f"SCRIPT_ERROR: {type(_ptc_err).__name__}: {_ptc_err}", file=sys.stderr)
    sys.exit(1)

# Print structured outputs if any were registered
if _ptc_outputs:
    print("\n__PTC_OUTPUTS__")
    print(json.dumps(_ptc_outputs))
"#;

pub struct PtcScriptTool;

impl Default for PtcScriptTool {
    fn default() -> Self {
        Self
    }
}

impl PtcScriptTool {
    pub fn new() -> Self {
        Self
    }

    /// Build the full Python program from user script + preamble/postamble.
    fn build_program(script: &str) -> String {
        let mut program =
            String::with_capacity(PREAMBLE.len() + script.len() + POSTAMBLE.len() + 256);
        program.push_str(PREAMBLE);

        // Indent user script into the try: block
        for line in script.lines() {
            program.push_str("    ");
            program.push_str(line);
            program.push('\n');
        }

        program.push_str(POSTAMBLE);
        program
    }

    /// Truncate output to MAX_OUTPUT_SIZE with a truncation notice.
    fn truncate_output(output: &str) -> String {
        if output.len() <= MAX_OUTPUT_SIZE {
            output.to_string()
        } else {
            format!(
                "{}\n\n[Output truncated at {} bytes]",
                &output[..MAX_OUTPUT_SIZE],
                MAX_OUTPUT_SIZE
            )
        }
    }
}

#[async_trait]
impl Tool for PtcScriptTool {
    fn name(&self) -> &str {
        "ptc_script"
    }

    fn description(&self) -> &str {
        "Execute a Python script that can call IronClaw tools programmatically. \
         The script has access to call_tool(), shell(), read_file(), write_file(), \
         and http_get() from the ironclaw_tools SDK. Use ptc_output(key, value) \
         to return structured results."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "script": {
                    "type": "string",
                    "description": "Python script to execute. Has access to call_tool(), shell(), read_file(), write_file(), http_get(), and ptc_output()."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default 120, max 300).",
                    "default": 120,
                    "minimum": 1,
                    "maximum": 300
                }
            },
            "required": ["script"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let script = require_str(&params, "script")?;
        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);
        let timeout = Duration::from_secs(timeout_secs);

        let program = Self::build_program(script);

        // Build the subprocess command
        let mut command = Command::new("python3");
        command.args(["-c", &program]);

        // Scrub environment -- only forward safe vars + PTC vars + extra_env
        command.env_clear();
        for var in SAFE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                command.env(var, val);
            }
        }
        for var in PTC_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                command.env(var, val);
            }
        }
        // Forward extra_env from JobContext (credentials fetched by worker runtime)
        for (k, v) in ctx.extra_env.iter() {
            command.env(k, v);
        }

        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Spawn and drain stdout/stderr concurrently
        let mut child = command
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn python3: {}", e)))?;

        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        let result = tokio::time::timeout(timeout, async {
            let stdout_fut = async {
                if let Some(mut out) = stdout_handle {
                    let mut buf = Vec::new();
                    (&mut out)
                        .take(MAX_OUTPUT_SIZE as u64)
                        .read_to_end(&mut buf)
                        .await
                        .ok();
                    tokio::io::copy(&mut out, &mut tokio::io::sink()).await.ok();
                    String::from_utf8_lossy(&buf).to_string()
                } else {
                    String::new()
                }
            };

            let stderr_fut = async {
                if let Some(mut err) = stderr_handle {
                    let mut buf = Vec::new();
                    (&mut err)
                        .take(MAX_OUTPUT_SIZE as u64)
                        .read_to_end(&mut buf)
                        .await
                        .ok();
                    tokio::io::copy(&mut err, &mut tokio::io::sink()).await.ok();
                    String::from_utf8_lossy(&buf).to_string()
                } else {
                    String::new()
                }
            };

            let (stdout, stderr, wait_result) = tokio::join!(stdout_fut, stderr_fut, child.wait());
            let status = wait_result?;
            Ok::<_, std::io::Error>((stdout, stderr, status.code().unwrap_or(-1)))
        })
        .await;

        let duration = start.elapsed();

        match result {
            Ok(Ok((stdout, stderr, exit_code))) => {
                if exit_code != 0 {
                    let error_msg = if stderr.is_empty() {
                        format!("Script exited with code {}", exit_code)
                    } else {
                        format!(
                            "Script exited with code {}:\n{}",
                            exit_code,
                            Self::truncate_output(&stderr)
                        )
                    };
                    return Err(ToolError::ExecutionFailed(error_msg));
                }

                // Combine output
                let output = if stderr.is_empty() {
                    stdout
                } else {
                    format!("{}\n\n--- stderr ---\n{}", stdout, stderr)
                };

                Ok(ToolOutput::text(Self::truncate_output(&output), duration))
            }
            Ok(Err(e)) => Err(ToolError::ExecutionFailed(format!(
                "Script execution failed: {}",
                e
            ))),
            Err(_) => {
                let _ = child.kill().await;
                Err(ToolError::Timeout(timeout))
            }
        }
    }

    fn requires_sanitization(&self) -> bool {
        true
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Container
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(MAX_TIMEOUT_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_program_indents_script() {
        let script = "x = 1\nprint(x)";
        let program = PtcScriptTool::build_program(script);
        assert!(program.contains("    x = 1\n"));
        assert!(program.contains("    print(x)\n"));
        assert!(program.contains("from ironclaw_tools import"));
        assert!(program.contains("def ptc_output("));
    }

    #[test]
    fn test_build_program_empty_script() {
        let program = PtcScriptTool::build_program("");
        // Empty script should still have preamble + postamble
        assert!(program.contains("try:"));
        assert!(program.contains("except Exception"));
    }

    #[test]
    fn test_truncate_output() {
        let short = "hello";
        assert_eq!(PtcScriptTool::truncate_output(short), "hello");

        let long = "x".repeat(MAX_OUTPUT_SIZE + 100);
        let truncated = PtcScriptTool::truncate_output(&long);
        assert!(truncated.len() < long.len());
        assert!(truncated.contains("[Output truncated"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = PtcScriptTool::new();
        assert_eq!(tool.name(), "ptc_script");
        assert_eq!(tool.domain(), ToolDomain::Container);
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always
        );
        assert!(tool.requires_sanitization());
    }
}
