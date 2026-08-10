//! The device-link step machine: the durable half of the one auth method
//! whose mechanics an extension implements instead of declaring.
//!
//! **What lives here and what does not.** This module owns the state machine,
//! the revision compare-and-swap, the two clocks, and the credential-flow
//! lifecycle around a link. It owns *no* vendor mechanics: those arrive
//! through [`crate::DeviceLinkDriver`], the port the extension host
//! implements. The `DeviceLinkAdapter` an extension writes is invisible from
//! here by construction, which is exactly what lets the whole machine be
//! tested against a fake without a vendor in the loop.
//!
//! The ADR for the hook — what it costs, and why no host-side control can
//! detect a substituted login — is
//! the device-link auth-hook ADR under `docs/internal/design/`.

pub mod driver;

pub use driver::{
    DEVICE_LINK_FLOW_MAX_TTL_SECONDS, DEVICE_LINK_FLOW_TTL_SECONDS, DEVICE_LINK_MAX_POLL_ATTEMPTS,
    DEVICE_LINK_POLL_INTERVAL_MILLIS, DEVICE_LINK_STEP_TTL_SECONDS, DeviceLinkFlowDriver,
    DeviceLinkStartRequest,
};
