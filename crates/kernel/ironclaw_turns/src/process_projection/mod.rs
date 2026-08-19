//! Agent-turn adapter for the canonical process journal.
//!
//! Runtime orchestration, durable metadata, checkpoint storage, and the store
//! adapter are kept in separate modules so each contract can evolve without
//! turning the projection entrypoint into another composition root.

mod event_projection;
mod loop_checkpoint;
mod metadata;
mod runtime;
mod store_adapter;

pub use event_projection::{
    TurnEventProjectionFromProcessJournal, turn_event_page_from_process_journal,
    turn_lifecycle_event_from_process_journal_entry, turn_status_from_process_status,
};
pub use loop_checkpoint::ProcessLoopCheckpointStore;
pub use metadata::{
    AgentTurnProcessMetadata, AgentTurnProcessStateMetadata, agent_turn_metadata_from_claimed,
};
pub use runtime::*;
pub use store_adapter::{
    ProcessJournalStoreTurnAdapter, turn_error_from_process_journal_store_error,
};
