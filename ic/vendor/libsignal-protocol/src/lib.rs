//! A Rust interface to the [libsignal-protocol-c] library.
//!
//! A ratcheting forward secrecy protocol that works in synchronous and
//! asynchronous messaging environments.
//!
//! # Key Concepts
//!
//! ## PreKeys
//!
//! This protocol uses a concept called "*PreKeys*". A PreKey is a
//! [`keys::PublicKey`] and an associated unique ID which are stored together by
//! a server. PreKeys can also be signed.
//!
//! At install time, clients generate a single signed PreKey, as well as a large
//! list of unsigned PreKeys, and transmit all of them to the server.
//!
//! ## Sessions
//!
//! The Signal Protocol is session-oriented. Clients establish a "session"
//! which is then used for all subsequent encrypt/decrypt operations. There is
//! no need to ever tear down a session once one has been established.
//!
//! Sessions are established in one of three ways:
//!
//! 1. [`PreKeyBundle`]. A client that wishes to send a message to a recipient
//!    can establish a session by retrieving a [`PreKeyBundle`] for that
//!    recipient from the server.
//! 2. [`PreKeySignalMessage`]s.  A client can receive a [`PreKeySignalMessage`]
//!    from a recipient and use it to establish a session.
//! 3. KeyExchangeMessages. Two clients can exchange KeyExchange messages to
//!    establish a session.
//!
//! ## State
//!
//! An established session encapsulates a lot of state between two clients. That
//! state is maintained in durable records which need to be kept for the life of
//! the session.
//!
//! State is kept in the following places:
//!
//! 1. Identity State. Clients will need to maintain the state of their own
//!    identity key pair, as well as identity keys received from other clients
//!    (saved in an [`IdentityKeyStore`]).
//! 1. PreKey State. Clients will need to maintain the state of their generated
//!    PreKeys in a [`PreKeyStore`].
//! 1. Signed PreKey States. Clients will need to maintain the state of their
//!    signed PreKeys using a [`SignedPreKeyStore`].
//! 1. Session State. Clients will need to maintain the state of the sessions
//!    they have established using a [`SessionStore`].
//!
//! [libsignal-protocol-c]: https://github.com/signalapp/libsignal-protocol-c

#![deny(
    missing_docs,
    missing_debug_implementations,
    missing_copy_implementations,
    elided_lifetimes_in_paths,
    rust_2018_idioms,
    clippy::cargo_common_metadata,
    clippy::fallible_impl_from,
    clippy::missing_const_for_fn,
    intra_doc_link_resolution_failure
)]

// we use the *-sys crate everywhere so give it a shorter name
#[allow(unused_extern_crates)]
extern crate libsignal_protocol_sys as sys;
#[cfg(feature = "crypto-openssl")]
#[macro_use]
extern crate rental;

use std::io::Write;

use failure::Error;

pub use crate::{
    address::Address,
    buffer::Buffer,
    context::*,
    errors::{FromInternalErrorCode, InternalError, IntoInternalErrorCode},
    hkdf::HMACBasedKeyDerivationFunction,
    pre_key_bundle::{PreKeyBundle, PreKeyBundleBuilder},
    session_builder::SessionBuilder,
    session_cipher::SessionCipher,
    session_record::SessionRecord,
    session_state::SessionState,
    store_context::StoreContext,
};
// bring into scope for rustdoc
#[allow(unused_imports)]
use crate::messages::PreKeySignalMessage;
// so rustdoc can resolve links
#[allow(unused_imports)]
use crate::stores::{IdentityKeyStore, PreKeyStore, SessionStore, SignedPreKeyStore};

#[macro_use]
mod macros;

mod address;
mod buffer;
mod context;
pub mod crypto;
mod errors;
mod hkdf;
pub mod keys;
pub mod messages;
mod pre_key_bundle;
pub(crate) mod raw_ptr;
mod session_builder;
mod session_cipher;
mod session_record;
mod session_state;
mod store_context;
pub mod stores;

/// A helper trait for something which can be serialized to protobufs.
pub trait Serializable {
    /// Serialize the object to a buffer.
    fn serialize(&self) -> Result<Buffer, Error>;

    /// Parse the provided data in the protobuf format.
    fn deserialize(data: &[u8]) -> Result<Self, Error>
    where
        Self: Sized;

    /// Helper for serializing to anything which implements [`Write`].
    fn serialize_to<W: Write>(&self, mut writer: W) -> Result<(), Error> {
        let buffer = self.serialize()?;
        writer.write_all(buffer.as_slice())?;

        Ok(())
    }
}

/// Create a default Signal context suitable for standalone deserialization.
pub(crate) fn default_context() -> Result<Context, Error> {
    #[cfg(feature = "crypto-native")]
    {
        return Context::new(crypto::DefaultCrypto::default());
    }

    #[cfg(all(not(feature = "crypto-native"), feature = "crypto-openssl"))]
    {
        return Context::new(crypto::OpenSSLCrypto::default());
    }

    #[allow(unreachable_code)]
    Err(failure::err_msg(
        "libsignal-protocol requires a crypto backend for deserialization",
    ))
}
