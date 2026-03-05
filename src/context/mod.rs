//! Per-job context isolation and state management.
//!
//! Each job runs with its own isolated context that includes:
//! - Conversation history
//! - Action history
//! - State machine
//! - Resource tracking

pub mod fallback;
mod manager;
mod memory;
mod state;

pub use fallback::FallbackDeliverable;
pub use manager::ContextManager;
pub use memory::{ActionRecord, ConversationMemory, Memory};
pub use state::{JobContext, JobState, StateTransition};
