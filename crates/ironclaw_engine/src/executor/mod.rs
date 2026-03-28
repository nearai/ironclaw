//! Step execution.
//!
//! - [`ExecutionLoop`] — core loop replacing `run_agentic_loop()`
//! - [`structured`] — Tier 0 action execution (structured tool calls)
//! - [`context`] — context building for LLM calls
//! - [`intent`] — tool intent nudge detection

pub mod compaction;
pub mod context;
pub mod intent;
pub mod loop_engine;
pub mod orchestrator;
pub mod prompt;
pub mod scripting;
pub mod structured;
pub mod trace;

pub use loop_engine::ExecutionLoop;
