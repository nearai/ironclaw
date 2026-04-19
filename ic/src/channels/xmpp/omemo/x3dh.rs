//! X3DH (Extended Triple Diffie-Hellman) key agreement for OMEMO session initiation.
#![allow(dead_code)]

use ed25519_dalek::Verifier;
use hkdf::Hkdf;
use sha2::Sha256;
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};

use super::store::{IdentityBundle, OmemoStore, OmemoStoreError};

#[derive(Debug, Error)]
pub enum X3dhError {
    #[error("Invalid signature on signed prekey")]
    InvalidSignature,
    #[error("Prekey not found: {0}")]
    PrekeyNotFound(u32),
    #[error("Store error: {0}")]
    Store(#[from] OmemoStoreError),
    #[error("Key format error: {0}")]
    KeyFormat(String),
}

/// A remote device's public key bundle fetched from their PEP node.
#[derive(Debug, Clone)]
pub struct RemoteBundle {
    pub jid: String,
    pub device_id: u32,
    /// X25519 Diffie-Hellman public key.
    pub ik_pub: [u8; 32],
    /// Ed25519 verifying key — used to verify the SPK signature.
    pub ik_sig_pub: [u8; 32],
    pub spk_pub: [u8; 32],
    pub spk_id: u32,
    pub spk_sig: [u8; 64],
    pub opk_pub: Option<[u8; 32]>,
    pub opk_id: Option<u32>,
}

/// Output of X3DH sender-side initiation.
pub struct X3dhOutput {
    /// 32-byte root key for Double Ratchet init.
    pub shared_secret: [u8; 32],
    /// Ephemeral public key to include in the PreKeyMessage.
    pub ek_pub: [u8; 32],
    /// Which one-time prekey was used (if any).
    pub used_opk_id: Option<u32>,
}

/// Initiate a session as the sender.
/// Verifies the SPK signature before proceeding.
pub fn x3dh_init_sender(
    our_identity: &IdentityBundle,
    remote: &RemoteBundle,
) -> Result<X3dhOutput, X3dhError> {
    use secrecy::ExposeSecret;

    // Verify SPK signature using the remote's published Ed25519 verifying key
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&remote.ik_sig_pub)
        .map_err(|_| X3dhError::InvalidSignature)?;
    let sig = ed25519_dalek::Signature::from_bytes(&remote.spk_sig);
    verifying_key
        .verify(&remote.spk_pub, &sig)
        .map_err(|_| X3dhError::InvalidSignature)?;

    // Generate ephemeral key
    let ek_priv = StaticSecret::random_from_rng(rand::thread_rng());
    let ek_pub = PublicKey::from(&ek_priv);

    let our_ik = StaticSecret::from(*our_identity.ik_priv.expose_secret());
    let their_ik_pub = PublicKey::from(remote.ik_pub);
    let their_spk_pub = PublicKey::from(remote.spk_pub);

    // DH1 = DH(IK_A, SPK_B)
    let dh1 = our_ik.diffie_hellman(&their_spk_pub);
    // DH2 = DH(EK_A, IK_B)
    let dh2 = ek_priv.diffie_hellman(&their_ik_pub);
    // DH3 = DH(EK_A, SPK_B)
    let dh3 = ek_priv.diffie_hellman(&their_spk_pub);

    let mut dh_concat = Vec::with_capacity(128);
    dh_concat.extend_from_slice(dh1.as_bytes());
    dh_concat.extend_from_slice(dh2.as_bytes());
    dh_concat.extend_from_slice(dh3.as_bytes());

    let used_opk_id = if let Some(opk_pub) = remote.opk_pub {
        // DH4 = DH(EK_A, OPK_B)
        let their_opk = PublicKey::from(opk_pub);
        let dh4 = ek_priv.diffie_hellman(&their_opk);
        dh_concat.extend_from_slice(dh4.as_bytes());
        remote.opk_id
    } else {
        None
    };

    let shared_secret = kdf_x3dh(&dh_concat);

    Ok(X3dhOutput {
        shared_secret,
        ek_pub: ek_pub.to_bytes(),
        used_opk_id,
    })
}

/// Complete session as receiver when an initial message arrives.
pub async fn x3dh_init_receiver(
    our_identity: &IdentityBundle,
    ek_pub: &[u8; 32],
    sender_ik_pub: &[u8; 32],
    opk_id: Option<u32>,
    store: &OmemoStore,
) -> Result<[u8; 32], X3dhError> {
    use secrecy::ExposeSecret;

    let our_ik = StaticSecret::from(*our_identity.ik_priv.expose_secret());
    let our_spk = StaticSecret::from(*our_identity.spk_priv.expose_secret());
    let their_ik_pub = PublicKey::from(*sender_ik_pub);
    let their_ek_pub = PublicKey::from(*ek_pub);

    // DH1 = DH(SPK_B, IK_A)
    let dh1 = our_spk.diffie_hellman(&their_ik_pub);
    // DH2 = DH(IK_B, EK_A)
    let dh2 = our_ik.diffie_hellman(&their_ek_pub);
    // DH3 = DH(SPK_B, EK_A)
    let dh3 = our_spk.diffie_hellman(&their_ek_pub);

    let mut dh_concat = Vec::with_capacity(128);
    dh_concat.extend_from_slice(dh1.as_bytes());
    dh_concat.extend_from_slice(dh2.as_bytes());
    dh_concat.extend_from_slice(dh3.as_bytes());

    if let Some(id) = opk_id {
        let opk = store.consume_prekey(id).await?;
        let opk_priv = StaticSecret::from(opk.priv_bytes()?);
        // DH4 = DH(OPK_B, EK_A)
        let dh4 = opk_priv.diffie_hellman(&their_ek_pub);
        dh_concat.extend_from_slice(dh4.as_bytes());
    }

    Ok(kdf_x3dh(&dh_concat))
}

fn kdf_x3dh(dh_concat: &[u8]) -> [u8; 32] {
    let salt = [0u8; 32];
    let hk = Hkdf::<Sha256>::new(Some(&salt), dh_concat);
    let mut okm = [0u8; 32];
    hk.expand(b"OMEMO X3DH", &mut okm).expect("HKDF expand");
    okm
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::xmpp::omemo::store::{OmemoStore, PreKey};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_x3dh_roundtrip() {
        let alice_dir = TempDir::new().unwrap();
        let bob_dir = TempDir::new().unwrap();
        let alice_store = OmemoStore::new(alice_dir.path().to_path_buf());
        let bob_store = OmemoStore::new(bob_dir.path().to_path_buf());

        let alice = alice_store.load_or_init_identity(1).await.unwrap();
        let bob = bob_store.load_or_init_identity(2).await.unwrap();

        // Generate a one-time prekey for Bob
        let opk_priv = x25519_dalek::StaticSecret::random_from_rng(rand::thread_rng());
        let opk_pub = x25519_dalek::PublicKey::from(&opk_priv);
        let opk = PreKey {
            id: 1,
            priv_key: hex::encode(opk_priv.as_bytes()),
            pub_key: hex::encode(opk_pub.to_bytes()),
        };
        bob_store.save_prekey(&opk).await.unwrap();

        let bob_bundle = RemoteBundle {
            jid: "bob@example.com".into(),
            device_id: bob.device_id,
            ik_pub: bob.ik_pub,
            ik_sig_pub: bob.ik_sig_pub,
            spk_pub: bob.spk_pub,
            spk_id: bob.spk_id,
            spk_sig: bob.spk_sig,
            opk_pub: Some(opk_pub.to_bytes()),
            opk_id: Some(1),
        };

        let out = x3dh_init_sender(&alice, &bob_bundle).unwrap();
        let bob_secret = x3dh_init_receiver(
            &bob,
            &out.ek_pub,
            &alice.ik_pub,
            out.used_opk_id,
            &bob_store,
        )
        .await
        .unwrap();

        assert_eq!(out.shared_secret, bob_secret);
    }

    #[test]
    fn test_x3dh_invalid_spk_sig() {
        let rt = tokio::runtime::Runtime::new().unwrap();

        let alice_dir = TempDir::new().unwrap();
        let bob_dir = TempDir::new().unwrap();
        let alice_store = OmemoStore::new(alice_dir.path().to_path_buf());
        let bob_store = OmemoStore::new(bob_dir.path().to_path_buf());

        let alice = rt.block_on(alice_store.load_or_init_identity(1)).unwrap();
        let bob = rt.block_on(bob_store.load_or_init_identity(2)).unwrap();

        let mut bad_sig = bob.spk_sig;
        bad_sig[0] ^= 0xff;

        let bundle = RemoteBundle {
            jid: "bob@example.com".into(),
            device_id: bob.device_id,
            ik_pub: bob.ik_pub,
            ik_sig_pub: bob.ik_sig_pub,
            spk_pub: bob.spk_pub,
            spk_id: bob.spk_id,
            spk_sig: bad_sig,
            opk_pub: None,
            opk_id: None,
        };
        let result = x3dh_init_sender(&alice, &bundle);
        assert!(matches!(result, Err(X3dhError::InvalidSignature)));
    }
}
