//! Strategy trait contracts for the Reborn agent loop.
//!
//! Each strategy receives `&LoopExecutionState` and returns an outcome enum
//! that carries the new value of its own slot. The executor swaps the slot
//! into the next whole state. See `docs/reborn/agent-loop-skeleton.md` §6
//! ("Strategy decomposition") and §8 ("Outcome enums").
//!
//! WS-2 lands the trait stubs and outcome enums for the batch / gate /
//! recovery axis. `Default*` impls land in WS-5; the executor body that
//! consumes these outcomes lands in WS-6.

// WS-2 intentionally lands crate-internal contracts before the planner and
// executor consume them in later workstreams.
#![allow(dead_code, unused_imports)]

pub(crate) mod batch;
pub(crate) mod gate;
pub(crate) mod recovery;

pub(crate) use batch::{BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary};
pub(crate) use gate::{GateHandlingStrategy, GateKind, GateOutcome, GateSummary};
pub(crate) use ironclaw_turns::run_profile::ConcurrencyHint;
pub(crate) use recovery::{
    CapabilityErrorClass, CapabilityErrorSummary, ModelErrorClass, ModelErrorSummary,
    RecoveryOutcome, RecoveryStrategy, RetryAlteration,
};
