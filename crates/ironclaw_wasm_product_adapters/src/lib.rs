//! Stub host runtime for IronClaw Reborn WASM v2 product adapters.
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
//! * `WebhookAuthEvidenceMint` — bridge that returns a `Verified` evidence
//!   constructed via the public `mark_*_verified` helpers in
//!   `ironclaw_product_adapters::auth`.
//! * `EgressPolicy` — declared-host + credential-handle enforcement that the
//!   wasmtime component-model glue will compose with at later landings.
//! * Native `ProductAdapter` runner that wires a Rust adapter implementation
//!   to a `ProductWorkflow` + `ProtocolHttpEgress`. Telegram v2 ships here
//!   today; it will move into a wasmtime component once the WIT/component
//!   tooling lands.

#![forbid(unsafe_code)]

pub mod auth_verifier;
pub mod egress_policy;
pub mod runner;

pub use auth_verifier::{
    HmacWebhookAuth, SharedSecretHeaderAuth, VerificationOutcome, WebhookAuthVerifier,
};
pub use egress_policy::{EgressPolicy, EgressPolicyError, EgressPolicyTarget};
pub use runner::{NativeProductAdapterRunner, RunnerError, WebhookProcessOutcome};
