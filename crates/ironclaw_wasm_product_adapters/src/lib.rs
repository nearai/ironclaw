//! Stub host runtime for IronClaw Reborn WASM v2 product adapters.
//!
//! # Trust-model warning
//!
//! The native runner in this crate executes `Arc<dyn ProductAdapter>` in the
//! host process. It is not a WASM sandbox. Only run trusted native adapters here;
//! untrusted adapters must wait for the wasmtime component-model path.
//!
//! This crate is the boundary where the trusted host (Rust) verifies protocol
//! authentication, normalizes egress to declared hosts, and exposes a small
//! constrained capability set to WASM v2 components. The first-slice
//! implementation is **deliberately runtime-free**: the wasmtime component
//! glue lives behind a `wasmtime` feature that's not yet wired up, because
//! the tracer-bullet PR for #3285 must boot without requiring a freshly
//! built telegram-v2.wasm binary.
//!
//! What this crate ships in the first slice:
//!
//! * `WebhookAuthVerifier` — trait + helpers for HMAC + shared-secret-header
//!   verification. Production hosts use these to mint
//!   [`ironclaw_product_adapters::ProtocolAuthEvidence::Verified`] before any
//!   adapter parse step.
//! * `WebhookAuth` — bridge that returns a `Verified` evidence constructed via
//!   the public `mark_*_verified` helpers in `ironclaw_product_adapters::auth`.
//! * `EgressPolicy` — declared-host + credential-handle enforcement that the
//!   wasmtime component-model glue will compose with at later landings.
//! * Native `ProductAdapter` runner that wires a Rust adapter implementation
//!   to a `ProductWorkflow` + `ProtocolHttpEgress`. Telegram v2 ships here
//!   today; it will move into a wasmtime component once the WIT/component
//!   tooling lands.

#![forbid(unsafe_code)]
#![warn(unreachable_pub)]

pub mod auth_verifier;
pub mod egress_policy;
pub mod runner;

pub use auth_verifier::{
    HmacWebhookAuth, SharedSecretHeaderAuth, VerificationOutcome, WebhookAuthVerifier,
};
pub use egress_policy::{EgressPolicy, EgressPolicyError, EgressPolicyTarget};
pub use runner::{
    DEFAULT_MAX_IN_FLIGHT_WEBHOOKS, DEFAULT_WEBHOOK_WORKFLOW_TIMEOUT, NativeProductAdapterRunner,
    NativeProductAdapterRunnerConfig, RunnerError, WebhookAuth, WebhookProcessOutcome,
    evidence_from_bearer_subject, evidence_from_session_subject,
};
