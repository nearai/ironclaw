//! Generic thread-level plan mode.
//!
//! Plan mode is a read-first execution state for any task type, not just
//! coding. When enabled, the agent may inspect, search, summarize, and plan,
//! but it must not take state-changing or externally visible actions until the
//! user explicitly exits plan mode.

use crate::tools::{RiskLevel, Tool};

const ALWAYS_ALLOWED_TOOLS: &[&str] = &[
    "echo",
    "time",
    "json",
    "plan_update",
    "plan_artifact_save",
    "tool_info",
    "tool_list",
    "tool_search",
    "extension_info",
    "skill_list",
    "skill_search",
    "read_file",
    "list_dir",
    "memory_search",
    "memory_read",
    "memory_tree",
    "list_jobs",
    "job_status",
    "job_events",
    "routine_list",
    "routine_history",
    "image_analyze",
];

/// Instruction injected into the conversational prompt when a thread is in
/// plan mode.
pub(crate) const PLAN_MODE_PROMPT: &str = "PLAN MODE IS ACTIVE.\n\
You are in read-first planning mode for this thread.\n\
- You may inspect files, search memory, list tools, use low-risk read-only shell commands, and gather information.\n\
- When you have a concrete plan, save it with `plan_artifact_save` using a title, full markdown, and 2-4 suggested next actions.\n\
- You must not make changes, create jobs/routines, send messages, install tools, or perform other side-effectful actions.\n\
- If execution is needed, explain the next step and ask the user to leave plan mode with /plan exit or /plan-mode exit.";

/// Return a user-facing rejection string when a tool is blocked by plan mode.
pub(crate) fn blocked_tool_message(tool_name: &str, reason: &str) -> String {
    format!(
        "Plan mode is active, so tool '{}' is blocked: {}. \
Use /plan exit or /plan-mode exit before executing changes.",
        tool_name, reason
    )
}

/// Check whether a tool invocation is allowed while plan mode is active.
///
/// Returns `Ok(())` when the tool may run, or a short denial reason when it is
/// blocked.
pub(crate) fn check_tool_allowed(
    tool_name: &str,
    params: &serde_json::Value,
    tool: &dyn Tool,
) -> Result<(), &'static str> {
    if ALWAYS_ALLOWED_TOOLS.contains(&tool_name) {
        return Ok(());
    }

    match tool_name {
        "http" => {
            let method = params
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("GET");
            let save_to = params.get("save_to").and_then(|v| v.as_str()).unwrap_or("");

            if !method.eq_ignore_ascii_case("GET") {
                return Err("only read-only GET requests are allowed");
            }
            if !save_to.trim().is_empty() {
                return Err("downloading to disk is not allowed");
            }
            if matches!(
                tool.requires_approval(params),
                crate::tools::ApprovalRequirement::Never
            ) {
                Ok(())
            } else {
                Err("only unauthenticated GET requests are allowed")
            }
        }
        "shell" => {
            let Some(command) = params.get("command").and_then(|v| v.as_str()) else {
                return Err("missing command");
            };
            let has_forbidden_syntax = ["&&", "||", ";", "|", ">", "<", "$(", "`", "\n", "\r"]
                .iter()
                .any(|needle| command.contains(needle));

            if has_forbidden_syntax {
                return Err("only simple read-only inspection commands are allowed");
            }

            if tool.risk_level_for(params) == RiskLevel::Low {
                Ok(())
            } else {
                Err("only low-risk read-only shell commands are allowed")
            }
        }
        _ => Err("it can change state or produce external side effects"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::builtin::{EchoTool, HttpTool, ReadFileTool, ShellTool, WriteFileTool};

    #[test]
    fn allows_simple_read_only_tools() {
        let read = ReadFileTool::new();
        assert!(check_tool_allowed("read_file", &serde_json::json!({}), &read).is_ok());

        let echo = EchoTool;
        assert!(check_tool_allowed("echo", &serde_json::json!({}), &echo).is_ok());
    }

    #[test]
    fn blocks_write_tools() {
        let write = WriteFileTool::new();
        assert!(check_tool_allowed("write_file", &serde_json::json!({}), &write).is_err());
    }

    #[test]
    fn allows_get_http_but_blocks_post() {
        let http = HttpTool::new();
        assert!(check_tool_allowed("http", &serde_json::json!({ "method": "GET" }), &http).is_ok());
        assert!(
            check_tool_allowed("http", &serde_json::json!({ "method": "POST" }), &http).is_err()
        );
        assert!(
            check_tool_allowed(
                "http",
                &serde_json::json!({
                    "method": "GET",
                    "save_to": "/tmp/download.bin"
                }),
                &http
            )
            .is_err()
        );
    }

    #[test]
    fn allows_only_simple_low_risk_shell_commands() {
        let shell = ShellTool::new();
        assert!(
            check_tool_allowed(
                "shell",
                &serde_json::json!({ "command": "ls -la src" }),
                &shell
            )
            .is_ok()
        );
        assert!(
            check_tool_allowed(
                "shell",
                &serde_json::json!({ "command": "ls -la > out.txt" }),
                &shell
            )
            .is_err()
        );
        assert!(
            check_tool_allowed(
                "shell",
                &serde_json::json!({ "command": "cargo build" }),
                &shell
            )
            .is_err()
        );
    }
}
