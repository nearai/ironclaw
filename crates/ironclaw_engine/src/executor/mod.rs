//! Step execution.
//!
//! - [`ExecutionLoop`] — core loop replacing `run_agentic_loop()`
//! - [`structured`] — Tier 0 action execution (structured tool calls)
//! - [`context`] — context building for LLM calls
//! - [`trace_dump`] — append-only JSONL dump of LLM / code-step I/O for dev

pub mod context;
pub mod loop_engine;
pub mod orchestrator;
pub mod prompt;
pub mod scripting;
pub mod structured;
pub mod trace;
pub mod trace_dump;

pub use loop_engine::ExecutionLoop;
pub use scripting::validate_python_syntax;
