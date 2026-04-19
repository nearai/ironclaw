//! Places to store Signal Protocol state.

pub(crate) mod identity_key_store;
mod in_memory_identity_key_store;
mod in_memory_pre_key_stores;
mod in_memory_session_store;
pub(crate) mod pre_key_store;
pub(crate) mod session_store;
pub(crate) mod signed_pre_key_store;

pub use self::{
    identity_key_store::IdentityKeyStore,
    in_memory_identity_key_store::InMemoryIdentityKeyStore,
    in_memory_pre_key_stores::{InMemoryPreKeyStore, InMemorySignedPreKeyStore},
    in_memory_session_store::InMemorySessionStore,
    pre_key_store::PreKeyStore,
    session_store::{SerializedSession, SessionStore},
    signed_pre_key_store::SignedPreKeyStore,
};
