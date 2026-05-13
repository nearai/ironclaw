//! Agent-loop framework state and strategy contracts for IronClaw Reborn.
//!
//! This crate owns the framework layer above `ironclaw_turns`. The master
//! architecture is `docs/reborn/agent-loop-skeleton.md`; workstream briefs live
//! under `docs/reborn/agent-loop-briefs/`.

mod default_planner;
pub mod executor;
pub mod families;
pub mod family;
pub mod planner;
pub mod state;
pub(crate) mod strategies;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use planner::AgentLoopPlanner;
