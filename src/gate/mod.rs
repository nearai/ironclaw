//! Unified execution gate — application-layer pending state management.
//!
//! The engine crate (`ironclaw_engine::gate`) defines the [`ExecutionGate`]
//! trait and evaluation pipeline. This module owns the **pending state store**
//! that bridges gate pauses to user-facing resolution flows.
//!
//! [`ExecutionGate`]: ironclaw_engine::ExecutionGate

pub mod approval;
pub mod pending;
pub mod persistence;
pub mod store;
