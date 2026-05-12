//! Standalone Reborn composition and adapter wiring.
//!
//! This crate is the Reborn-side home for adapters that intentionally bridge
//! to existing root IronClaw services while keeping the normal `/src` app graph
//! free of Reborn loop-support wiring.

pub mod driver_registry;
pub mod loop_driver_host;
pub mod loop_exit_applier;
pub mod model_routes;
pub mod text_loop_driver;
pub mod turn_runner;

#[cfg(feature = "root-llm-provider")]
pub mod model_gateway;
#[cfg(feature = "libsql-secrets")]
pub mod secrets;

pub use loop_driver_host::{
    HostManagedLoopCheckpointPort, HostManagedLoopProgressPort, HostRuntimeLoopCapabilityPort,
    LoopCapabilityInputResolver, LoopCapabilityResultWriter, NoExtraLoopInputPort,
    RebornLoopDriverHost, RebornLoopDriverHostError, RebornLoopDriverHostFactory,
    RebornLoopDriverHostRequest, TextOnlyLoopHostConfig,
};
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
pub use text_loop_driver::{TextOnlyModelReplyDriver, TextOnlyModelReplyDriverConfig};
