//! EVM vertical: decode / render / sign / broadcast / policy.
//!
//! Signing is secp256k1 over the keccak signing hash of the transaction
//! RECONSTRUCTED from the decoded projection ([`decode::rebuild_signable`]),
//! with a mandatory ecrecover signer-binding check. The raw signing primitives
//! and key parsing live in [`sign`] as `pub(crate)` — reachable only through the
//! guarded `CustodialSigner` flow (review finding #5).

pub mod broadcast;
pub(crate) mod decode;
pub mod policy;
pub mod render;
pub(crate) mod sign;

pub use broadcast::{EvmBroadcastOutcome, EvmBroadcaster};
pub use policy::{
    EvmHiddenFields, EvmTokenMetadata, check_chain_id, check_token_metadata, hidden_fields,
};
pub use render::render_evm;
// `address_of` is a public-key derivation used for binding setup (never exposes
// secret material). `decode_*` produce the SDK-free projection. The signature
// result type is public; the signing primitives are not.
pub use decode::{decode_eip1559, decode_eip2930, decode_legacy};
pub use sign::{EvmSignature, address_of};
