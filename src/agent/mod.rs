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
pub mod agentic_loop;
mod attachments;
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
pub(crate) mod scheduler;
mod self_repair;
pub mod session;
mod session_manager;
pub mod submission;
pub mod task;
mod thread_ops;
pub mod undo;

#[cfg(all(test, feature = "libsql"))]
pub(crate) mod test_support {
    use std::sync::Arc;

    use crate::db::Database;

    pub(crate) async fn make_libsql_test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("failed to create temp dir"); // safety: test-only setup helper
        let path = dir.path().join("test.db");
        let backend = crate::db::libsql::LibSqlBackend::new_local(&path)
            .await
            .expect("failed to create test LibSqlBackend"); // safety: test-only setup helper
        backend
            .run_migrations()
            .await
            .expect("failed to run migrations"); // safety: test-only setup helper
        (Arc::new(backend) as Arc<dyn Database>, dir)
    }
}

pub(crate) use agent_loop::truncate_for_preview;
pub use agent_loop::{Agent, AgentDeps};
pub(crate) use attachments::augment_with_attachments;
pub use compaction::{CompactionResult, ContextCompactor};
pub use context_monitor::{CompactionStrategy, ContextBreakdown, ContextMonitor};
pub(crate) use dispatcher::strip_suggestions;
pub use heartbeat::{
    HeartbeatConfig, HeartbeatResult, HeartbeatRunner, spawn_heartbeat, spawn_multi_user_heartbeat,
};
pub use router::{MessageIntent, Router};
pub use routine::{Routine, RoutineAction, RoutineRun, Trigger};
pub use routine_engine::{RoutineEngine, SandboxReadiness};
pub use scheduler::{Scheduler, SchedulerDeps};
pub use self_repair::{BrokenTool, RepairResult, RepairTask, SelfRepair, StuckJob};
pub use session::{
    PendingApproval, PendingAuth, Session, Thread, ThreadState, Turn, TurnOutcome, TurnState,
};
pub use session_manager::SessionManager;
pub use submission::{Submission, SubmissionParser, SubmissionResult};
pub use task::{Task, TaskContext, TaskHandler, TaskOutput};
pub use undo::{Checkpoint, UndoManager};
