//! Thread lifecycle management.
//!
//! - [`ThreadManager`] — top-level orchestrator for spawning and supervising threads
//! - [`ThreadTree`] — parent-child relationship tracking
//! - [`messaging`] — inter-thread signal channel

pub mod conversation;
pub mod internal_write;
pub mod lease_refresh;
pub mod manager;
pub mod messaging;
pub mod mission;
pub mod tree;

pub use conversation::ConversationManager;
pub use internal_write::{
    SelfModifyTestGuard, is_trusted_internal_write_active, self_modify_enabled,
    set_self_modify_for_test, with_trusted_internal_writes,
};
pub use manager::ThreadManager;
pub use messaging::ThreadOutcome;
pub use mission::MissionManager;
pub use tree::ThreadTree;
