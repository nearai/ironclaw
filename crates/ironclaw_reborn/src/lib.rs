//! Standalone Reborn composition and adapter wiring.
//!
//! This crate is the Reborn-side home for adapters that intentionally bridge
//! to existing root IronClaw services while keeping the normal `/src` app graph
//! free of Reborn loop-support wiring.

mod app_loop_family;
pub mod driver_registry;
mod loop_driver_host;
pub mod loop_exit_applier;
mod milestone_events;
mod model_routes;
mod planned_driver;
pub mod production_readiness;
mod text_loop_driver;
pub mod turn_runner;

#[cfg(feature = "root-llm-provider")]
mod model_gateway;
#[cfg(feature = "libsql-secrets")]
pub mod secrets;

pub use app_loop_family::build_loop_family_registry;
pub use loop_driver_host::{
    HostManagedLoopCheckpointPort, HostManagedLoopProgressPort, HostRuntimeLoopCapabilityPort,
    LoopCapabilityInputResolver, LoopCapabilityResultWriter, NoExtraLoopInputPort,
    RebornLoopDriverHost, RebornLoopDriverHostError, RebornLoopDriverHostFactory,
    RebornLoopDriverHostRequest, TextOnlyLoopHostConfig,
};
pub use milestone_events::{DurableLoopHostMilestoneScope, DurableLoopHostMilestoneSink};
#[cfg(feature = "root-llm-provider")]
pub use model_gateway::{
    LlmModelProfilePolicy, LlmProviderModelGateway, ModelRouteProviderPool,
    RoutedLlmProviderModelGateway, StaticModelRouteProviderPool, ThreadBackedLoopModelGateway,
};
pub use model_routes::{
    ActiveModelRouteSettings, ModelRoute, ModelRouteError, ModelRoutePolicy, ModelRouteProviderKey,
    ModelRouteResolver, ModelRouteSource, ModelSelectionMode, ModelSlot,
    ResolvedModelRouteSnapshot, StaticModelRouteResolver,
};
pub use planned_driver::PlannedDriver;
pub use text_loop_driver::{TextOnlyModelReplyDriver, TextOnlyModelReplyDriverConfig};
