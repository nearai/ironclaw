//! Core agent logic.
//!
//! The agent orchestrates:
//! - Message routing from channels
//! - Job scheduling and execution
//! - Tool invocation with safety
//! - Self-repair for stuck jobs

mod agent_loop;
mod router;
mod scheduler;
mod self_repair;
mod worker;

pub use agent_loop::Agent;
pub use router::{MessageIntent, Router};
pub use scheduler::Scheduler;
pub use self_repair::{RepairResult, RepairTask, SelfRepair, StuckJob};
pub use worker::Worker;
