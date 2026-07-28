//! NEAR vertical: decode / render / sign / broadcast / policy.
//!
//! ## Scope note
//!
//! The signing primitive (ed25519) uses the vendored `ed25519-dalek`, so this
//! PR does NOT pull `near-primitives` / `near-crypto`. Signing operates on the
//! shared [`ironclaw_attestation::canonical_signing_bytes`] (review finding #4);
//! a full `near-primitives::Transaction` borsh round-trip producing a
//! directly-broadcastable signature is the immediate next slice flagged in the
//! PR body.

pub mod broadcast;
pub mod decode;
pub mod policy;
pub mod render;
pub(crate) mod sign;

pub use broadcast::{NearBroadcastOutcome, NearBroadcaster};
pub use policy::check_network;
pub use render::render_near;
// Only the signature *result* type is public; the signing primitives in
// [`sign`] are `pub(crate)` (review finding #5).
pub use sign::NearSignature;
