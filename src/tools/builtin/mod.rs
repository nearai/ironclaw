//! Built-in tools that come with the agent.

use std::path::Path;

use base64::Engine as _;

mod echo;
pub mod extension_tools;
mod file;
pub mod file_edit_guard;
pub mod file_history;
mod glob_tool;
mod grep_tool;
mod http;
mod job;
mod json;
pub mod memory;
mod message;
pub mod path_utils;
mod plan;
mod restart;
pub mod routine;
pub mod secrets_tools;
pub(crate) mod shell;
pub mod skill_tools;
pub mod system;
mod time;
mod tool_info;

pub use echo::EchoTool;
pub use extension_tools::{
    ExtensionInfoTool, ToolActivateTool, ToolAuthTool, ToolInstallTool, ToolListTool,
    ToolPermissionSetTool, ToolRemoveTool, ToolSearchTool, ToolUpgradeTool,
};
pub use file::{ApplyPatchTool, ListDirTool, ReadFileTool, WriteFileTool};
pub use file_edit_guard::{SharedReadFileState, shared_read_file_state};
pub use file_history::{FileHistory, FileUndoTool, SharedFileHistory, shared_file_history};
pub use glob_tool::GlobTool;
pub use grep_tool::GrepTool;
pub use http::{HttpTool, extract_host_from_params};
pub use job::{
    CancelJobTool, CreateJobTool, JobEventsTool, JobPromptTool, JobStatusTool, ListJobsTool,
    PromptQueue, SchedulerSlot,
};
pub use json::JsonTool;
pub use memory::{MemoryReadTool, MemorySearchTool, MemoryTreeTool, MemoryWriteTool};
pub use message::MessageTool;
pub use plan::PlanUpdateTool;
pub use restart::RestartTool;
pub use routine::{
    EventEmitTool, RoutineCreateTool, RoutineDeleteTool, RoutineFireTool, RoutineHistoryTool,
    RoutineListTool, RoutineUpdateTool,
};
pub use secrets_tools::{SecretDeleteTool, SecretListTool};
pub use shell::ShellTool;
pub use skill_tools::{SkillInstallTool, SkillListTool, SkillRemoveTool, SkillSearchTool};
pub use system::{SystemToolsListTool, SystemVersionTool};
pub use time::TimeTool;
pub use tool_info::ToolInfoTool;
mod html_converter;
pub mod image_analyze;
pub mod image_edit;
pub mod image_gen;

pub use html_converter::convert_html_to_markdown;
pub use image_analyze::ImageAnalyzeTool;
pub use image_edit::ImageEditTool;
pub use image_gen::ImageGenerateTool;

/// Detect image media type from file extension via `mime_guess`.
/// Falls back to `image/jpeg` for unrecognized or non-image extensions.
pub(crate) fn media_type_from_path(path: &str) -> String {
    mime_guess::from_path(path)
        .first_raw()
        .filter(|m| m.starts_with("image/"))
        .unwrap_or("image/jpeg")
        .to_string()
}

/// Build an OpenAI-style image endpoint from a provider base URL.
///
/// Some providers already include `/v1` in their configured base URL while
/// others expect clients to append it. Keep this logic shared so image tools
/// do not drift.
pub(crate) fn image_api_endpoint_url(api_base_url: &str, path: &str) -> String {
    let base = api_base_url.trim_end_matches('/');
    if has_version_like_path_suffix(base) {
        format!("{base}{path}")
    } else {
        format!("{base}/v1{path}")
    }
}

fn has_version_like_path_suffix(api_base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(api_base_url) else {
        return false;
    };
    url.path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        .is_some_and(is_version_like_path_segment)
}

fn is_version_like_path_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    matches!(chars.next(), Some('v')) && matches!(chars.next(), Some(c) if c.is_ascii_digit())
}

pub(crate) async fn persist_generated_image_bytes(
    bytes: &[u8],
    media_type: &str,
    base_dir: Option<&Path>,
) -> Result<String, crate::tools::ToolError> {
    let root = base_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(crate::bootstrap::ironclaw_base_dir);
    let output_dir = root.join("generated-images");
    tokio::fs::create_dir_all(&output_dir).await.map_err(|e| {
        crate::tools::ToolError::ExecutionFailed(format!(
            "Failed to create generated image directory '{}': {e}",
            output_dir.display()
        ))
    })?;

    let extension = match media_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "bin",
    };
    let path = output_dir.join(format!("generated-{}.{}", uuid::Uuid::new_v4(), extension));
    tokio::fs::write(&path, bytes).await.map_err(|e| {
        crate::tools::ToolError::ExecutionFailed(format!(
            "Failed to persist generated image '{}': {e}",
            path.display()
        ))
    })?;

    Ok(path.to_string_lossy().into_owned())
}

pub(crate) async fn persist_generated_image_base64(
    image_b64: &str,
    media_type: &str,
    base_dir: Option<&Path>,
) -> Result<String, crate::tools::ToolError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image_b64)
        .map_err(|e| {
            crate::tools::ToolError::ExecutionFailed(format!(
                "Failed to decode generated image bytes: {e}"
            ))
        })?;
    persist_generated_image_bytes(&bytes, media_type, base_dir).await
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    #[tokio::test]
    async fn persist_generated_image_bytes_writes_under_generated_images_dir() {
        let dir = tempdir().unwrap();

        let path =
            super::persist_generated_image_bytes(b"png-bytes", "image/png", Some(dir.path()))
                .await
                .expect("persist image");

        assert!(path.ends_with(".png"));
        assert!(path.contains("generated-images"));
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"png-bytes");
    }
}
