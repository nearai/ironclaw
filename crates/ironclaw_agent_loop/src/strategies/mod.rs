//! Crate-internal strategy trait contracts for the Reborn agent-loop framework.
//!
//! Each strategy receives `&LoopExecutionState` and returns either a pure policy
//! value or an outcome enum carrying the new value of its own state slot. The
//! executor swaps the slot into the next whole state. See
//! `docs/reborn/agent-loop-skeleton.md` section 6.

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

pub(crate) use batch::{BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary};
pub(crate) use budget::BudgetStrategy;
pub(crate) use capability::{CapabilityFilter, CapabilityStrategy};
pub(crate) use context::ContextStrategy;
pub(crate) use drain::InputDrainStrategy;
pub(crate) use gate::{GateHandlingStrategy, GateKind, GateOutcome, GateSummary};
pub(crate) use ironclaw_turns::run_profile::ConcurrencyHint;
pub(crate) use model::{ModelPreference, ModelStrategy};
pub(crate) use recovery::{
    BackoffDelayMs, CapabilityErrorClass, CapabilityErrorSummary, ModelErrorClass,
    ModelErrorSummary, RecoveryOutcome, RecoveryStrategy, RetryAlteration, RetryScope,
    SanitizedStrategySummary,
};
pub(crate) use stop::{StopConditionStrategy, StopKind, StopOutcome, TurnEndKind, TurnSummary};
