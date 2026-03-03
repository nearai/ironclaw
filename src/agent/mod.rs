//! Core agent logic.
//!
//! The agent orchestrates:
//! - Message routing from channels
//! - Job scheduling and execution
//! - Tool invocation with safety
//! - Self-repair for stuck jobs
//! - Proactive heartbeat execution
//! - Routine-based scheduled and reactive jobs
//! - Turn-based session management with undo
//! - Context compaction for long conversations

mod agent_loop;
mod commands;
pub mod compaction;
pub mod context_monitor;
pub mod cost_guard;
mod dispatcher;
mod heartbeat;
pub mod job_monitor;
mod router;
pub mod routine;
pub mod routine_engine;
mod scheduler;
mod self_repair;
pub mod session;
mod session_manager;
pub mod submission;
pub mod task;
pub(crate) mod thread_ops;
pub mod undo;
pub mod worker;

pub(crate) use agent_loop::truncate_for_preview;
pub use agent_loop::{Agent, AgentDeps};
pub use compaction::{CompactionResult, ContextCompactor};
pub use context_monitor::{CompactionStrategy, ContextBreakdown, ContextMonitor};
pub use heartbeat::{HeartbeatConfig, HeartbeatResult, HeartbeatRunner, spawn_heartbeat};
pub use router::{MessageIntent, Router};
pub use routine::{Routine, RoutineAction, RoutineRun, Trigger};
pub use routine_engine::RoutineEngine;
pub use scheduler::Scheduler;
pub use self_repair::{BrokenTool, RepairResult, RepairTask, SelfRepair, StuckJob};
pub use session::{PendingApproval, PendingAuth, Session, Thread, ThreadState, Turn, TurnState};
pub use session_manager::SessionManager;
pub use submission::{Submission, SubmissionParser, SubmissionResult};
pub use task::{Task, TaskContext, TaskHandler, TaskOutput};
pub use undo::{Checkpoint, UndoManager};
pub use worker::{Worker, WorkerDeps};

#[cfg(test)]
mod thread_ops_tests;
