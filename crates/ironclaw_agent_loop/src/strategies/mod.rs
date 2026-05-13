//! Strategy trait contracts for the Reborn agent-loop framework.
//!
//! Each strategy receives `&LoopExecutionState` and returns an outcome enum
//! that carries the new value of its own slot. The executor swaps the slot
//! into the next whole state. See `docs/reborn/agent-loop-skeleton.md` §6
//! ("Strategy decomposition") and §8 ("Outcome enums").
//!
//! WS-1 lands the context / capability / model axis (α).
//! WS-2 lands the batch / gate / recovery axis (β).
//! WS-3 lands the stop / drain / budget axis (γ).
//! `Default*` impls land in WS-5; the executor body that consumes these
//! outcomes lands in WS-6.

mod capability;
mod context;
mod model;
pub mod batch;
pub mod budget;
pub mod drain;
pub mod gate;
pub mod recovery;
pub mod stop;

pub use capability::{CapabilityFilter, CapabilityStrategy};
pub use context::ContextStrategy;
pub use model::{ModelPreference, ModelStrategy};
pub use batch::{BatchPolicy, BatchPolicyStrategy, CapabilityCallSummary, ConcurrencyHint};
pub use budget::{BudgetStrategy, UnlimitedBudget};
pub use drain::InputDrainStrategy;
pub use gate::{GateHandlingStrategy, GateKind, GateOutcome, GateSummary};
pub use recovery::{
    CapabilityErrorClass, CapabilityErrorSummary, ModelErrorClass, ModelErrorSummary,
    RecoveryOutcome, RecoveryStrategy, RetryAlteration,
};
pub use stop::{StopConditionStrategy, StopKind, StopOutcome, TurnEndKind, TurnSummary};
