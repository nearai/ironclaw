//! Agent-loop framework state and strategy contracts for IronClaw Reborn.
//!
//! This crate owns the framework layer above `ironclaw_turns`. The master
//! architecture is `docs/reborn/agent-loop-skeleton.md`; workstream briefs live
//! under `docs/reborn/agent-loop-briefs/`.

pub mod default_planner;
pub mod executor;
pub mod planner;
pub mod state;
pub mod strategies;

pub use default_planner::DefaultPlanner;
pub use executor::{AgentLoopExecutor, CanonicalAgentLoopExecutor};
pub use planner::{AgentLoopPlanner, PlannerId, PlannerIdError};
pub use strategies::{
    BatchPolicy, BatchPolicyStrategy, BudgetStrategy, CapabilityCallSummary, CapabilityErrorClass,
    CapabilityErrorSummary, CapabilityFilter, CapabilityStrategy, ConcurrencyHint, ContextStrategy,
    DefaultBatchPolicyStrategy, DefaultBudgetStrategy, DefaultCapabilityStrategy,
    DefaultContextStrategy, DefaultGateHandlingStrategy, DefaultInputDrainStrategy,
    DefaultModelStrategy, DefaultRecoveryStrategy, DefaultStopConditionStrategy,
    GateHandlingStrategy, GateKind, GateOutcome, GateSummary, InputDrainStrategy, ModelErrorClass,
    ModelErrorSummary, ModelPreference, ModelStrategy, RecoveryOutcome, RecoveryStrategy,
    RetryAlteration, StopConditionStrategy, StopKind, StopOutcome, TurnEndKind, TurnSummary,
    UnlimitedBudget,
};
