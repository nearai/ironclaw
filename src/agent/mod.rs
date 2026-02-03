//! Core agent logic.
//!
//! The agent orchestrates:
//! - Message routing from channels
//! - Job scheduling and execution
//! - Tool invocation with safety
//! - Self-repair for stuck jobs
//! - Proactive heartbeat execution
//! - Turn-based session management with undo
//! - Context compaction for long conversations

mod agent_loop;
pub mod compaction;
pub mod context_monitor;
mod heartbeat;
mod router;
mod scheduler;
mod self_repair;
pub mod session;
pub mod submission;
pub mod task;
pub mod undo;
mod worker;

pub use agent_loop::Agent;
pub use compaction::{CompactionResult, ContextCompactor};
pub use context_monitor::{CompactionStrategy, ContextBreakdown, ContextMonitor};
pub use heartbeat::{HeartbeatConfig, HeartbeatResult, HeartbeatRunner, spawn_heartbeat};
pub use router::{MessageIntent, Router};
pub use scheduler::Scheduler;
pub use self_repair::{RepairResult, RepairTask, SelfRepair, StuckJob};
pub use session::{Session, Thread, ThreadState, Turn, TurnState};
pub use submission::{Submission, SubmissionResult};
pub use task::{Task, TaskContext, TaskHandler, TaskOutput, TaskStatus};
pub use undo::{Checkpoint, UndoManager};
pub use worker::Worker;
