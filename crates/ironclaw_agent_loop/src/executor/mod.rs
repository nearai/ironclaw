//! Canonical agent-loop executor.
//!
//! The executor owns loop mechanics: checkpointing, host-port calls, strategy
//! sequencing, and safety-net exits. Planners remain pure strategy
//! composition.

use async_trait::async_trait;
use ironclaw_turns::{
    LoopFailureKind, LoopGateRef, LoopMessageRef, run_profile::AgentLoopDriverHost,
};

use crate::{
    planner::AgentLoopPlanner,
    state::{CheckpointKind, LoopExecutionState},
};

mod canonical;
mod capability;
mod drain;
mod lifecycle;
mod model;
#[cfg(test)]
mod tests;
mod util;

/// Drives the canonical loop tick against a planner and host facade.
#[async_trait]
pub trait AgentLoopExecutor: Send + Sync {
    /// See master spec §8 for the canonical iteration algorithm.
    async fn execute(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: &mut LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError>;
}

/// Loop exit produced by the canonical framework executor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopExit {
    Completed(CompletionKind),
    Failed { kind: FailureKind },
    Blocked { gate_ref: LoopGateRef },
    Cancelled(CancelledKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    NaturalEnd,
    GracefulStop,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    IterationLimitReached,
    NoProgressDetected,
    /// `BudgetStrategy::wall_clock_limit` exceeded before the loop reached a
    /// natural terminal state. Distinct from `IterationLimitReached` so a
    /// profile that opted into a wall-clock cap can tell time-bound vs
    /// step-bound exhaustion apart (master spec §6 — `BudgetStrategy`).
    WallClockLimitReached,
    Other(LoopFailureKind),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CancelledKind {
    pub interrupted_message_refs: Vec<LoopMessageRef>,
}

/// Sanitized executor errors. Loop-level failures should usually be returned
/// as [`LoopExit::Failed`]; this type is reserved for cases where the executor
/// cannot produce a normal loop exit.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgentLoopExecutorError {
    #[error("host port returned an unrecoverable error at {stage:?}")]
    HostUnavailable { stage: HostStage },
    #[error("planner returned a contract violation: {detail}")]
    PlannerContract { detail: &'static str },
    #[error("checkpoint write failed at {stage:?}")]
    CheckpointFailed { stage: CheckpointKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStage {
    Prompt,
    Model,
    Capability,
    Transcript,
    Checkpoint,
    Input,
}

/// The reference executor. Implements the canonical tick from master spec §8.
#[derive(Debug, Default, Clone, Copy)]
pub struct CanonicalAgentLoopExecutor;

#[async_trait]
impl AgentLoopExecutor for CanonicalAgentLoopExecutor {
    async fn execute(
        &self,
        planner: &dyn AgentLoopPlanner,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: &mut LoopExecutionState,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        self.execute_canonical(planner, host, state).await
    }
}

#[allow(dead_code)]
fn _check(_: &dyn AgentLoopExecutor) {}
