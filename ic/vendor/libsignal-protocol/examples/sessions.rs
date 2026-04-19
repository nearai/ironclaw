//! Create a signal session.
//!
//! A rust equivalent of the [Building A Session][bas] example from
//! `libsignal-protocol-c`'s README.
//!
//! ```c
//! /* Create the data store context, and add all the callbacks to it */
//! signal_protocol_store_context *store_context;
//! signal_protocol_store_context_create(&store_context, context);
//! signal_protocol_store_context_set_session_store(store_context, &session_store);
//! signal_protocol_store_context_set_pre_key_store(store_context, &pre_key_store);
//! signal_protocol_store_context_set_signed_pre_key_store(store_context, &signed_pre_key_store);
//! signal_protocol_store_context_set_identity_key_store(store_context, &identity_key_store);
//!
//! /* Instantiate a session_builder for a recipient address. */
//! signal_protocol_address address = {
//!     "+14159998888", 12, 1
//! };
//! session_builder *builder;
//! session_builder_create(&builder, store_context, &address, global_context);
//!
//! /* Build a session with a pre key retrieved from the server. */
//! session_builder_process_pre_key_bundle(builder, retrieved_pre_key);
//!
//! /* Create the session cipher and encrypt the message */
//! session_cipher *cipher;
//! session_cipher_create(&cipher, store_context, &address, global_context);
//!
//! ciphertext_message *encrypted_message;
//! session_cipher_encrypt(cipher, message, message_len, &encrypted_message);
//!
//! /* Get the serialized content and deliver it */
//! signal_buffer *serialized = ciphertext_message_get_serialized(encrypted_message);
//!
//! deliver(signal_buffer_data(serialized), signal_buffer_len(serialized));
//!
//! /* Cleanup */
//! SIGNAL_UNREF(encrypted_message);
//! session_cipher_free(cipher);
//! session_builder_free(builder);
//! signal_protocol_store_context_destroy(store_context);
//! ```
//!
//! [bas]: https://github.com/signalapp/libsignal-protocol-c#building-a-session

extern crate libsignal_protocol as sig;

use std::time::SystemTime;

use failure::{Error, ResultExt};

use sig::{
    stores::{
        InMemoryIdentityKeyStore, InMemoryPreKeyStore, InMemorySessionStore,
        InMemorySignedPreKeyStore,
    },
    Address, Context, PreKeyBundle, Serializable, SessionBuilder, SessionCipher,
};

#[path = "../tests/helpers/mod.rs"]
mod helpers;

cfg_if::cfg_if! {
    if #[cfg(feature = "crypto-native")] {
        type Crypto = sig::crypto::DefaultCrypto;
    } else if #[cfg(feature = "crypto-openssl")] {
        type Crypto = sig::crypto::OpenSSLCrypto;
    } else {
        compile_error!("These tests require one of the crypto features to be enabled");
    }
}
fn main() -> Result<(), Error> {
    env_logger::init();
    let ctx = Context::new(Crypto::default()).unwrap();

    // first we'll need a copy of bob's public key and some of his pre-keys
    let bob_address = Address::new("+14159998888", 1);
    let bob_identity_keys =
        sig::generate_identity_key_pair(&ctx).context("Unable to generate bob's keys")?;
    let bob_public_identity_key = bob_identity_keys.public();
    let bob_pre_keys: Vec<_> = sig::generate_pre_keys(&ctx, 0, 10)
        .context("Unable to generate bob's pre-keys")?
        .collect();
    let pre_key = &bob_pre_keys[0];
    let bob_signed_pre_key =
        sig::generate_signed_pre_key(&ctx, &bob_identity_keys, 12, SystemTime::now())
            .context("Unable to generate a signed pre-key for bob")?;

    // alice will need an identity
    let alice_registration_id = sig::generate_registration_id(&ctx, 0)?;
    let alice_identity = sig::generate_identity_key_pair(&ctx)?;

    // set up some key stores for alice
    let alice_store_ctx = sig::store_context(
        &ctx,
        InMemoryPreKeyStore::default(),
        InMemorySignedPreKeyStore::default(),
        InMemorySessionStore::default(),
        InMemoryIdentityKeyStore::new(alice_registration_id, &alice_identity),
    )?;

    // Instantiate a session_builder for a recipient address.
    let alice_session_builder = SessionBuilder::new(&ctx, &alice_store_ctx, &bob_address);

    let pre_key_bundle = PreKeyBundle::builder()
        .registration_id(42)
        .device_id(bob_address.device_id())
        .identity_key(&bob_public_identity_key)
        .pre_key(pre_key.id(), &pre_key.key_pair().public())
        .signed_pre_key(
            bob_signed_pre_key.id(),
            &bob_signed_pre_key.key_pair().public(),
        )
        .signature(bob_signed_pre_key.signature())
        .build()
        .context("Unable to generate the pre-key bundle")?;

    // Create a session using a pre key retrieved from the server.
    alice_session_builder
        .process_pre_key_bundle(&pre_key_bundle)
        .context("Unable to create a session with bob")?;

    // Now we've established a session alice can start encrypting messages to
    // send to bob
    let cipher = SessionCipher::new(&ctx, &alice_store_ctx, &bob_address)?;
    let message = "Hello, World!";
    let encrypted_message = cipher
        .encrypt(message.as_bytes())
        .context("Encryption failed")?;

    let serialized = encrypted_message
        .serialize()
        .context("Unable to serialize the message for transmission")?;

    println!("Encrypted Message: {:?}", serialized.as_slice());

    Ok(())
}
