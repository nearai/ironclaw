//! Compatibility re-exports for the capability dispatcher routing layer.
//!
//! The routing implementation now lives in `ironclaw_capabilities`, where the
//! authorization/invocation host and dispatch registry are converging. This
//! crate remains temporarily so existing imports can migrate incrementally.

pub use ironclaw_capabilities::{
    BoundCapabilityAdapter, ChainToolResolver, ResolvedCapability, RuntimeAdapterResult,
    RuntimeDispatcher, ToolResolver,
};
pub use ironclaw_host_api::{
    Authorized, CapabilityDispatchRequest, CapabilityDispatchResult, CapabilityDispatcher,
    CapabilityDisplayOutputPreview, DispatchError, DispatchFailureDetail, RuntimeDispatchErrorKind,
};
