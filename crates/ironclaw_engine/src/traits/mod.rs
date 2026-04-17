//! External dependency traits.
//!
//! The engine defines these traits; the host (main ironclaw crate)
//! implements them via bridge adapters over existing infrastructure.

pub mod effect;
pub mod llm;
pub mod store;
pub mod workspace;
