//! Host runtime glue for IronClaw Reborn WASM v2 product adapters.
//!
//! # Trust-model warning
//!
//! The native runner in this crate executes `Arc<dyn ProductAdapter>` in the
//! host process. It is not a WASM sandbox. Only run trusted native adapters
//! there. Installable external adapters should use the wasmtime component-model
//! runtime exposed by [`ProductAdapterComponentRuntime`].
//!
//! This crate is the boundary where the trusted host (Rust) verifies protocol
//! authentication, normalizes egress to declared hosts, and exposes a small
//! constrained capability set to WASM v2 components.
//!
//! What this crate ships:
//!
//! * `WebhookAuthVerifier` — trait + helpers for HMAC + shared-secret-header
//!   verification. Production hosts use these to mint
//!   [`ironclaw_product_adapters::ProtocolAuthEvidence::Verified`] before any
//!   adapter parse step.
//! * `WebhookAuth` — bridge that returns a `Verified` evidence constructed via
//!   the public `mark_*_verified` helpers in `ironclaw_product_adapters::auth`.
//! * `EgressPolicy` — declared-host + credential-handle enforcement for adapter
//!   manifests and host-mediated egress.
//! * `ProductAdapterComponentRuntime` — wasmtime component loader for
//!   `wit/product_adapter.wit` that reads the manifest and calls `parse-inbound`
//!   / `render-outbound` through the component boundary.
//! * Native `ProductAdapter` runner that wires a Rust adapter implementation
//!   to a `ProductWorkflow` + `ProtocolHttpEgress`. Telegram v2 ships here
//!   today; it will move into a WASM component once a component artifact lands.

#![forbid(unsafe_code)]
#![warn(unreachable_pub)]

mod auth_verifier;
mod bindings;
mod component_runtime;
mod config;
mod egress_policy;
mod runner;
mod store;

pub use auth_verifier::{
    Clock, HmacWebhookAuth, SharedSecretHeaderAuth, VerificationOutcome, WebhookAuthVerifier,
};
pub use component_runtime::{
    ComponentManifest, ParsedInboundResult, PreparedProductAdapterComponent,
    ProductAdapterComponentRuntime, RenderOutboundResult, RuntimeError,
};
pub use config::{
    PRODUCT_ADAPTER_WIT_VERSION, ProductAdapterComponentLimits,
    ProductAdapterComponentRuntimeConfig,
};
pub use egress_policy::{EgressPolicy, EgressPolicyError, EgressPolicyTarget};
pub use runner::{
    DEFAULT_MAX_IN_FLIGHT_WEBHOOKS, DEFAULT_WEBHOOK_WORKFLOW_TIMEOUT, NativeProductAdapterRunner,
    NativeProductAdapterRunnerConfig, RunnerError, WebhookAuth, WebhookProcessOutcome,
    evidence_from_bearer_subject, evidence_from_session_subject,
};
