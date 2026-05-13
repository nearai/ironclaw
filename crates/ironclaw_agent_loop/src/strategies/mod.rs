//! Strategy contracts for the Reborn agent-loop framework.

pub mod budget;
pub mod drain;
pub mod stop;

pub use budget::{BudgetStrategy, UnlimitedBudget};
pub use drain::InputDrainStrategy;
pub use stop::{StopConditionStrategy, StopKind, StopOutcome, TurnEndKind, TurnSummary};
