//! Thread lifecycle management.
//!
//! - [`ThreadManager`] — top-level orchestrator for spawning and supervising threads
//! - [`ThreadTree`] — parent-child relationship tracking
//! - [`messaging`] — inter-thread signal channel

pub mod conversation;
pub mod manager;
pub mod messaging;
pub mod mission;
pub mod tree;

pub use conversation::ConversationManager;
pub use manager::ThreadManager;
pub use messaging::ThreadOutcome;
pub use mission::MissionManager;
pub use tree::ThreadTree;
