//! History and persistence layer.
//!
//! Stores job history, conversations, and actions in PostgreSQL for:
//! - Audit trail
//! - Learning from past executions
//! - Analytics and metrics

mod analytics;
mod store;

pub use analytics::{JobStats, ToolStats};
pub use store::{
    ConversationMessage, ConversationSummary, JobEventRecord, LlmCallRecord, SandboxJobRecord,
    SandboxJobSummary, Store,
};
