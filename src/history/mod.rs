//! History and persistence layer.
//!
//! Stores job history, conversations, and actions in PostgreSQL for:
//! - Audit trail
//! - Learning from past executions
//! - Analytics and metrics

#[cfg(feature = "postgres")]
mod analytics;
mod store;

#[cfg(feature = "postgres")]
pub use analytics::{JobStats, ToolStats};
#[cfg(feature = "postgres")]
pub use store::Store;
pub(crate) use store::job_result_event_message;
pub use store::{
    AgentJobRecord, AgentJobSummary, ConversationMessage, ConversationSummary, JobEventRecord,
    LlmCallRecord, SandboxJobRecord, SandboxJobSummary, SandboxRestartParams, SettingRow,
};
