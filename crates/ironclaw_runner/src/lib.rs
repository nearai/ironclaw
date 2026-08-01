//! Reborn loop drivers, host factory, and model-gateway bridge.
//!
//! This crate is an **internal** assembly building block. The only sanctioned
//! downstream consumer is `ironclaw_reborn_composition`, which composes the
//! items declared here with substrate services into a runnable agent and
//! re-exposes a task-level handle. The dependency boundary tests in
//! `ironclaw_architecture` enforce that nothing else takes a normal cargo
//! dependency on this crate.
//!
//! The public surface here is intentionally a **directory of modules**, not a
//! shopping list of types. Each module is reachable by path
//! (`ironclaw_runner::driver_registry::DriverRegistry`,
//! `ironclaw_runner::model_gateway::LlmProviderModelGateway`, …) so that a
//! glance at this file tells a reader what areas exist without enumerating
//! every type. We deliberately do **not** flatten the modules via a wall of
//! `pub use` re-exports — that was the noisy "speculative public API" pattern
//! the boundary tests are designed to prevent.

pub mod after_turn_memory;
pub mod app_loop_family;
mod context_shadow;
pub mod driver_registry;
/// Failure **classification** only — the category identifiers and the
/// category→summary tables moved to `ironclaw_host_api::failure` with WS1.7
/// (PROPOSAL §6.1.1), so this module is now private to the runner.
mod failure_categories;
pub mod failure_lane;
/// The writer half of the checkpoint-rejection envelope; its reader and the
/// summary tables live in `ironclaw_host_api::failure::summary`.
mod failure_summary;
pub mod hook_gate_refs;
pub mod retry_disposition;

// Run-failure classification/summarization over `failure_categories` (moved from
// ironclaw_reborn_composition; they classify runner-owned categories). Re-exported
// at the crate root so intra-cluster `crate::FailureLane` refs resolve and
// composition can re-export them through its service for the CLI.
pub mod loop_driver_host;
pub mod loop_exit_applier;
pub mod milestone_events;
mod model_failure_mapping;
mod model_gateway_error_mapping;
pub mod model_routes;
pub mod planned_driver;
pub mod planned_driver_factory;
pub mod production_readiness;
pub mod runtime;
pub mod steering_reconcile;
pub mod subagent;
pub mod text_loop_driver;
mod tool_disclosure;
pub mod tool_disclosure_bridge;
mod tool_disclosure_port;
pub mod turn_run_executor;
pub mod turn_runner;
pub mod turn_scheduler;

pub mod model_gateway;
