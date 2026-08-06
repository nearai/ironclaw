//! Host-managed implementations of two `ironclaw_loop_contracts` ports.
//!
//! These are adapters, not contracts. They live here only because the turn
//! kernel is where they were written; PROPOSAL §6.7.2 assigns them to
//! `ironclaw_loop_host` and CHECKLIST WS4 ("`loop_host` re-charter") moves
//! them. They were deliberately left behind by the WS1.2 contracts extraction,
//! which moved *contract* vocabulary only — PROPOSAL §6.1.4 forbids
//! `ironclaw_loop_contracts` from containing any implementation of its own
//! ports.
//!
//! Nothing new belongs in this module. A new host-port implementation goes to
//! `ironclaw_loop_host`.

mod model;
mod prompt;

pub use model::HostManagedLoopModelPort;
pub use prompt::HostManagedLoopPromptPort;
