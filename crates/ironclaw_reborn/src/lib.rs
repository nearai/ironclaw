//! Standalone Reborn composition and adapter wiring.
//!
//! This crate is the Reborn-side home for adapters that intentionally bridge
//! to existing root IronClaw services while keeping the normal `/src` app graph
//! free of Reborn loop-support wiring.

pub mod driver_registry;
pub mod loop_driver_host;
pub mod loop_exit_applier;
pub mod turn_runner;

#[cfg(feature = "root-llm-provider")]
pub mod model_gateway;

#[cfg(feature = "libsql-secrets")]
pub mod secrets_store;

#[cfg(feature = "root-llm-provider")]
pub use model_gateway::{
    LlmModelProfilePolicy, LlmProviderModelGateway, ThreadBackedLoopModelGateway,
};

#[cfg(feature = "libsql-secrets")]
pub use secrets_store::{LibSqlLoopSecretStore, LibSqlLoopSecretStoreConfig};
