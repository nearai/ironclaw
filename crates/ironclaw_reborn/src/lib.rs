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
mod planned_driver_factory;
pub mod production_readiness;
mod runtime;
mod text_loop_driver;
pub mod turn_runner;

#[cfg(feature = "root-llm-provider")]
mod model_gateway;
#[cfg(feature = "libsql-secrets")]
pub mod secrets;

pub use app_loop_family::build_loop_family_registry;
pub use ironclaw_loop_support::HostRuntimeLoopCapabilityPortFactory as HostRuntimeCapabilityPortFactory;
pub use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileFilter,
    CapabilitySurfaceProfileResolver, HostRuntimeLoopCapabilityPort, LoopCapabilityInputResolver,
    LoopCapabilityResultWriter,
};
pub use loop_driver_host::{
    DEFAULT_HOOK_CAPABILITY_INPUT_MAX_BYTES, HookCapabilityInputResolverAdapter,
    HostManagedLoopCheckpointPort, HostManagedLoopProgressPort, LoopCapabilityPortFactory,
    NoExtraLoopInputPort, RebornLoopDriverHost, RebornLoopDriverHostError,
    RebornLoopDriverHostFactory, RebornLoopDriverHostRequest, TextOnlyLoopHostConfig,
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
pub use planned_driver_factory::{
    DefaultPlannedDriverBuild, DefaultPlannedDriverRegistrationError, PLANNED_DEFAULT_PROFILE_ID,
    PLANNED_DRIVER_CHECKPOINT_SCHEMA_ID, PLANNED_DRIVER_CHECKPOINT_SCHEMA_VERSION,
    PLANNED_DRIVER_DEFAULT_ID, PLANNED_DRIVER_DEFAULT_VERSION, default_planned_driver,
    default_planned_run_profile_resolver, planned_default_profile_id,
    planned_driver_checkpoint_schema_id, planned_driver_checkpoint_schema_version,
    planned_driver_default_id, planned_driver_default_version, planned_driver_descriptor,
    register_default_planned_driver, register_default_planned_profile,
    register_default_text_only_driver,
};
pub use runtime::{
    DefaultPlannedRuntimeBuildError, DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts,
    ProductLiveRuntimeBuildError, ProductLiveRuntimeReadinessComponent,
    RebornRuntimeLoopComposition, build_default_planned_runtime,
    build_product_live_planned_runtime,
};
pub use text_loop_driver::{TextOnlyModelReplyDriver, TextOnlyModelReplyDriverConfig};
