//! Reborn loop drivers, host factory, and model-gateway bridge.
//!
//! This crate is an **internal** assembly building block. The only sanctioned
//! downstream consumer is `ironclaw_reborn_composition`, which composes the
//! items declared here with substrate facades into a runnable agent and
//! re-exposes a task-level handle. The dependency boundary tests in
//! `ironclaw_architecture` enforce that nothing else takes a normal cargo
//! dependency on this crate.
//!
//! The public surface here is intentionally a **directory of modules**, not a
//! shopping list of types. Each module is reachable by path
//! (`ironclaw_reborn::driver_registry::DriverRegistry`,
//! `ironclaw_reborn::model_gateway::LlmProviderModelGateway`, …) so that a
//! glance at this file tells a reader what areas exist without enumerating
//! every type. We deliberately do **not** flatten the modules via a wall of
//! `pub use` re-exports — that was the noisy "speculative public API" pattern
//! the boundary tests are designed to prevent.

pub mod app_loop_family;
pub mod driver_registry;
pub mod loop_driver_host;
pub mod loop_exit_applier;
pub mod milestone_events;
pub mod model_routes;
pub mod planned_driver;
pub mod planned_driver_factory;
pub mod production_readiness;
pub mod runtime;
pub mod text_loop_driver;
pub mod turn_runner;

#[cfg(feature = "root-llm-provider")]
pub mod model_gateway;
#[cfg(feature = "libsql-secrets")]
pub mod secrets;
