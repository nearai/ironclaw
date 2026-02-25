//! DM pairing for channels.
//!
//! Gates DMs from unknown senders. Only approved senders can message the agent.
//! Unknown senders receive a pairing code and must be approved via `clawyer pairing approve`.
//!
//! cLawyer reference: src/pairing/pairing-store.ts

mod store;

pub use store::{PairingRequest, PairingStore, PairingStoreError, UpsertResult};
