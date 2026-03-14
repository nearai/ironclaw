//! Tool for opening local files in the user's GUI editor/application.
//!
//! This is intended for user requests like "open this file in TextEdit".

use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use tokio::fs;
use tokio::process::Command;

use crate::context::JobContext;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput, require_str,
};

/// Open a local file in the default application (or a specific app on macOS).
#[derive(Debug, Default)]
pub struct OpenFileTool;

const MAX_PREVIEW_BYTES: usize = 16 * 1024;

impl OpenFileTool {
    pub fn new() -> Self {
        Self
    }
}

fn build_open_program_and_args(path: &PathBuf, app: Option<&str>) -> (String, Vec<String>) {
    #[cfg(target_os = "macos")]
    {
        let mut args = Vec::new();
        if let Some(app_name) = app
            && !app_name.trim().is_empty()
        {
            args.push("-a".to_string());
            args.push(app_name.trim().to_string());
        }
        args.push(path.display().to_string());
        ("open".to_string(), args)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        ("xdg-open".to_string(), vec![path.display().to_string()])
    }
}

#[async_trait]
impl Tool for OpenFileTool {
    fn name(&self) -> &str {
        "open_file"
    }

    fn description(&self) -> &str {
        "Open a LOCAL FILESYSTEM path in the user's default GUI app. \
         On macOS, optionally set app='TextEdit' (or another app name). \
         Use this when the user asks to open a file in an editor."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative local filesystem path to open"
                },
                "app": {
                    "type": "string",
                    "description": "Optional app name (macOS), e.g. 'TextEdit' or 'Visual Studio Code'"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let path_str = require_str(&params, "path")?;
        let app = params.get("app").and_then(|v| v.as_str());

        let raw = PathBuf::from(path_str);
        let path = if raw.is_absolute() {
            raw
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(raw)
        };

        if !path.exists() {
            return Err(ToolError::InvalidParameters(format!(
                "Path does not exist: {}",
                path.display()
            )));
        }

        let (program, args) = build_open_program_and_args(&path, app);
        let status = Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status()
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to launch {}: {}", program, e))
            })?;

        if !status.success() {
            return Err(ToolError::ExecutionFailed(format!(
                "Failed to open path '{}' (exit code: {})",
                path.display(),
                status.code().unwrap_or(-1)
            )));
        }

        // Also return a text preview so follow-up prompts like "summarise that file"
        // have local context without requiring another tool-routing hop.
        let mut content_preview: Option<String> = None;
        let mut preview_truncated = false;
        let mut preview_note = None;
        if let Ok(metadata) = fs::metadata(&path).await
            && metadata.is_file()
            && metadata.len() > 0
        {
            match fs::read(&path).await {
                Ok(bytes) => {
                    let take = bytes.len().min(MAX_PREVIEW_BYTES);
                    let clipped = &bytes[..take];
                    match String::from_utf8(clipped.to_vec()) {
                        Ok(text) => {
                            content_preview = Some(text);
                            preview_truncated = bytes.len() > MAX_PREVIEW_BYTES;
                        }
                        Err(_) => {
                            preview_note =
                                Some("File is not UTF-8 text; preview omitted.".to_string());
                        }
                    }
                }
                Err(_) => {
                    preview_note = Some("Could not read file preview.".to_string());
                }
            }
        }

        Ok(ToolOutput::success(
            serde_json::json!({
                "opened": true,
                "path": path.display().to_string(),
                "app": app.unwrap_or("default"),
                "content_preview": content_preview,
                "preview_truncated": preview_truncated,
                "preview_note": preview_note,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_name_are_stable() {
        let tool = OpenFileTool::new();
        assert_eq!(tool.name(), "open_file"); // safety: test-only assertion
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object()); // safety: test-only assertion
        let has_required_path = schema["required"] // safety: test-only assertion
            .as_array()
            .unwrap() // safety: test-only assertion
            .contains(&"path".into());
        assert!(has_required_path); // safety: test-only assertion
    }

    #[test]
    fn build_open_command_includes_app_on_macos() {
        let path = PathBuf::from("/tmp/test.txt");
        let (_program, args) = build_open_program_and_args(&path, Some("TextEdit"));
        #[cfg(target_os = "macos")]
        {
            assert_eq!(args, vec!["-a", "TextEdit", "/tmp/test.txt"]); // safety: test-only assertion
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(args, vec!["/tmp/test.txt"]); // safety: test-only assertion
        }
    }
}
