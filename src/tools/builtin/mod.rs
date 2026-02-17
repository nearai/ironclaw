//! Built-in tools that come with the agent.

mod echo;
pub mod extension_tools;
mod file;
mod http;
mod job;
mod json;
mod memory;
pub mod routine;
pub(crate) mod shell;
mod time;

pub use echo::EchoTool;
pub use extension_tools::{
    ToolActivateTool, ToolAuthTool, ToolInstallTool, ToolListTool, ToolRemoveTool, ToolSearchTool,
};
pub use file::{ApplyPatchTool, ListDirTool, ReadFileTool, WriteFileTool};
pub use http::HttpTool;
pub use job::{CancelJobTool, CreateJobTool, JobStatusTool, ListJobsTool};
pub use json::JsonTool;
pub use memory::{MemoryReadTool, MemorySearchTool, MemoryTreeTool, MemoryWriteTool};
pub use routine::{
    RoutineCreateTool, RoutineDeleteTool, RoutineHistoryTool, RoutineListTool, RoutineUpdateTool,
};
pub use shell::ShellTool;
pub use time::TimeTool;

mod html_converter;

pub use html_converter::convert_html_to_markdown;
