//! Crate-internal strategy trait contracts for the Reborn agent-loop framework.

// WS-1 lands these sealed contracts before WS-4/WS-6 consume them.
#![allow(dead_code, unused_imports)]

mod capability;
mod context;
mod model;

pub(crate) use capability::{CapabilityFilter, CapabilityStrategy};
pub(crate) use context::ContextStrategy;
pub(crate) use model::{ModelPreference, ModelStrategy};
