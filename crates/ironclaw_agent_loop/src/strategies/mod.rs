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
//!
//! Checkpoint/observability wire enums are `#[non_exhaustive]`; later
//! workstreams should extend them without forcing consumers to assume the
//! current variants are closed.
//!
//! Pure policy traits with no host or future host consult may stay sync.
//! Gate and recovery traits are async because they can consult host/runtime
//! state such as grant history, auth flow status, route health, or
//! circuit-breaker counters.

// WS-1/2 land crate-internal contracts before WS-4/5/6 compose and execute
// them. Keep the unused lint local to these forward-declared contracts.
#![allow(dead_code, unused_imports)]

pub(crate) mod batch;
mod capability;
mod context;
pub(crate) mod gate;
mod model;
pub(crate) mod recovery;

pub(crate) use batch::{BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary};
pub(crate) use capability::{CapabilityFilter, CapabilityStrategy};
pub(crate) use context::ContextStrategy;
pub(crate) use gate::{GateHandlingStrategy, GateKind, GateOutcome, GateSummary};
pub(crate) use ironclaw_turns::run_profile::ConcurrencyHint;
pub(crate) use model::{ModelPreference, ModelStrategy};
pub(crate) use recovery::{
    BackoffDelayMs, CapabilityErrorClass, CapabilityErrorSummary, ModelErrorClass,
    ModelErrorSummary, RecoveryOutcome, RecoveryStrategy, RetryAlteration, RetryScope,
    SanitizedStrategySummary,
};
