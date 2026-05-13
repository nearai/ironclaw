//! Crate-internal strategy trait contracts for the Reborn agent-loop framework.
//!
//! Each strategy receives `&LoopExecutionState` and returns either a pure policy
//! value or an outcome enum carrying the new value of its own state slot. The
//! executor swaps the slot into the next whole state. See
//! `docs/reborn/agent-loop-skeleton.md` §6.

// Strategy contracts land before WS-4/WS-6 consume them.
#![allow(dead_code, unused_imports)]

pub(crate) mod batch;
pub(crate) mod budget;
pub(crate) mod capability;
pub(crate) mod context;
pub(crate) mod drain;
pub(crate) mod gate;
pub(crate) mod model;
pub(crate) mod recovery;
pub(crate) mod stop;

pub(crate) use batch::{
    BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary, DefaultBatchPolicyStrategy,
};
pub(crate) use budget::{BudgetStrategy, DefaultBudgetStrategy};
pub(crate) use capability::{CapabilityFilter, CapabilityStrategy, DefaultCapabilityStrategy};
pub(crate) use context::{ContextStrategy, DefaultContextStrategy};
pub(crate) use drain::{DefaultInputDrainStrategy, InputDrainStrategy};
pub(crate) use gate::{
    DefaultGateHandlingStrategy, GateHandlingStrategy, GateKind, GateOutcome, GateSummary,
};
pub(crate) use ironclaw_turns::run_profile::ConcurrencyHint;
pub(crate) use model::{DefaultModelStrategy, ModelPreference, ModelStrategy};
pub(crate) use recovery::{
    CapabilityErrorClass, CapabilityErrorSummary, DefaultRecoveryStrategy, ModelErrorClass,
    ModelErrorSummary, RecoveryOutcome, RecoveryStrategy, RetryAlteration,
};
pub(crate) use stop::{
    DefaultStopConditionStrategy, StopConditionStrategy, StopKind, StopOutcome, TurnEndKind,
    TurnSummary,
};
