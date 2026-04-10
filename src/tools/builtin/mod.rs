//! Built-in tools that come with the agent.

mod abound;
mod echo;
pub mod extension_tools;
mod file;
pub(crate) mod forex;
mod http;
mod job;
mod json;
pub mod memory;
mod message;
pub mod mission;
pub mod path_utils;
mod plan;
mod restart;
pub mod routine;
pub mod secrets_tools;
pub(crate) mod shell;
pub mod skill_tools;
mod time;
mod tool_info;

pub use abound::{
    AboundAccountInfoTool, AboundCreateNotificationTool, AboundExchangeRateTool,
    AboundRateAlertTool, AboundSendWireTool,
};
pub use echo::EchoTool;
pub use extension_tools::{
    ExtensionInfoTool, ToolActivateTool, ToolAuthTool, ToolInstallTool, ToolListTool,
    ToolPermissionSetTool, ToolRemoveTool, ToolSearchTool, ToolUpgradeTool,
};
pub use file::{ApplyPatchTool, ListDirTool, ReadFileTool, WriteFileTool};
pub use forex::{AnalyzeTransferTool, ForexHistoricalDataTool, ValidateTransferTargetTool};
pub use http::{HttpTool, extract_host_from_params};
pub use job::{
    CancelJobTool, CreateJobTool, JobEventsTool, JobPromptTool, JobStatusTool, ListJobsTool,
    PromptQueue, SchedulerSlot,
};
pub use json::JsonTool;
pub use memory::{MemoryReadTool, MemorySearchTool, MemoryTreeTool, MemoryWriteTool};
pub use message::MessageTool;
pub use mission::{
    MissionCreateTool, MissionDeleteTool, MissionFireTool, MissionListTool, MissionPauseTool,
    MissionResumeTool, MissionUpdateTool,
};
pub use plan::PlanUpdateTool;
pub use restart::RestartTool;
pub use routine::{
    EventEmitTool, RoutineCreateTool, RoutineDeleteTool, RoutineFireTool, RoutineHistoryTool,
    RoutineListTool, RoutineUpdateTool,
};
pub use secrets_tools::{SecretDeleteTool, SecretListTool};
pub use shell::ShellTool;
pub use skill_tools::{SkillInstallTool, SkillListTool, SkillRemoveTool, SkillSearchTool};
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

/// Validate that a string is a 3-letter uppercase currency code (ISO 4217).
pub(super) fn validate_currency_code(s: &str) -> Result<String, crate::tools::tool::ToolError> {
    let upper = s.to_uppercase();
    if upper.len() == 3 && upper.chars().all(|c| c.is_ascii_uppercase()) {
        Ok(upper)
    } else {
        Err(crate::tools::tool::ToolError::InvalidParameters(format!(
            "Invalid currency code: {s}"
        )))
    }
}

/// Detect image media type from file extension via `mime_guess`.
/// Falls back to `image/jpeg` for unrecognized or non-image extensions.
pub(crate) fn media_type_from_path(path: &str) -> String {
    mime_guess::from_path(path)
        .first_raw()
        .filter(|m| m.starts_with("image/"))
        .unwrap_or("image/jpeg")
        .to_string()
}
