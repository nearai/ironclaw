//! Double Ratchet state machine for OMEMO per-message forward secrecy.
#![allow(dead_code)]

use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use thiserror::Error;
use x25519_dalek::{PublicKey, StaticSecret};

type HmacSha256 = Hmac<Sha256>;

const MAX_SKIP: u32 = 1000;

#[derive(Debug, Error)]
pub enum RatchetError {
    #[error("Decryption failed")]
    DecryptionFailed,
    #[error("Maximum skipped messages exceeded")]
    TooManySkippedMessages,
    #[error("AEAD error")]
    Aead,
    #[error("Key bytes wrong length")]
    BadKeyLength,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RatchetState {
    pub root_key: [u8; 32],
    pub send_chain_key: [u8; 32],
    pub recv_chain_key: Option<[u8; 32]>,
    pub send_ratchet_pub: [u8; 32],
    /// Sensitive but stored for persistence. The store file should be protected at the OS level.
    pub send_ratchet_priv: [u8; 32],
    pub recv_ratchet_pub: Option<[u8; 32]>,
    pub send_msg_n: u32,
    pub recv_msg_n: u32,
    pub prev_send_chain_n: u32,
    /// Skipped message keys keyed by recv_msg_n.
    pub skipped_keys: HashMap<u32, [u8; 32]>,
    /// Whether this side initiated (sender = true) or received (false) first.
    pub is_sender: bool,
}

impl RatchetState {
    /// Initialize for sender after X3DH.
    pub fn init_sender(shared_secret: [u8; 32], remote_ratchet_pub: [u8; 32]) -> Self {
        // Generate our initial ratchet key pair
        let our_ratchet_priv = StaticSecret::random_from_rng(rand::thread_rng());
        let our_ratchet_pub = PublicKey::from(&our_ratchet_priv);

        // First DH ratchet step to derive chain keys
        let dh = our_ratchet_priv.diffie_hellman(&PublicKey::from(remote_ratchet_pub));
        let (new_root, send_chain) = kdf_rk(&shared_secret, dh.as_bytes());

        Self {
            root_key: new_root,
            send_chain_key: send_chain,
            recv_chain_key: None,
            send_ratchet_pub: our_ratchet_pub.to_bytes(),
            send_ratchet_priv: *our_ratchet_priv.as_bytes(),
            recv_ratchet_pub: Some(remote_ratchet_pub),
            send_msg_n: 0,
            recv_msg_n: 0,
            prev_send_chain_n: 0,
            skipped_keys: HashMap::new(),
            is_sender: true,
        }
    }

    /// Initialize for receiver after X3DH.
    pub fn init_receiver(shared_secret: [u8; 32], our_ratchet_priv_bytes: [u8; 32]) -> Self {
        let our_ratchet_priv = StaticSecret::from(our_ratchet_priv_bytes);
        let our_ratchet_pub = PublicKey::from(&our_ratchet_priv);
        Self {
            root_key: shared_secret,
            send_chain_key: [0u8; 32],
            recv_chain_key: None,
            send_ratchet_pub: our_ratchet_pub.to_bytes(),
            send_ratchet_priv: our_ratchet_priv_bytes,
            recv_ratchet_pub: None,
            send_msg_n: 0,
            recv_msg_n: 0,
            prev_send_chain_n: 0,
            skipped_keys: HashMap::new(),
            is_sender: false,
        }
    }
}

/// Advance chain key, return (new_chain_key, message_key).
fn advance_chain(chain_key: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let msg_key = hmac_sha256(chain_key, &[0x01]);
    let new_chain = hmac_sha256(chain_key, &[0x02]);
    (new_chain, msg_key)
}

/// KDF_RK: derive new root key and chain key from root key + DH output.
fn kdf_rk(root_key: &[u8; 32], dh_out: &[u8]) -> ([u8; 32], [u8; 32]) {
    let hk = Hkdf::<Sha256>::new(Some(root_key), dh_out);
    let mut okm = [0u8; 64];
    hk.expand(b"OMEMO Double Ratchet Root", &mut okm)
        .expect("HKDF expand");
    let mut new_root = [0u8; 32];
    let mut chain = [0u8; 32];
    new_root.copy_from_slice(&okm[..32]);
    chain.copy_from_slice(&okm[32..]);
    (new_root, chain)
}

fn hmac_sha256(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as hmac::Mac>::new_from_slice(key).expect("HMAC key length");
    mac.update(data);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Encrypt a 32-byte message key using AES-256-GCM with a zero nonce.
/// Returns 48 bytes (32 plaintext + 16 AEAD tag).
fn aes_encrypt_key(aes_key: &[u8; 32], plaintext: &[u8; 32]) -> Result<[u8; 48], RatchetError> {
    let cipher = Aes256Gcm::new_from_slice(aes_key).map_err(|_| RatchetError::Aead)?;
    let nonce = Nonce::from_slice(&[0u8; 12]);
    let ct = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| RatchetError::Aead)?;
    // ct = ciphertext (32) + tag (16) = 48 bytes
    let mut out = [0u8; 48];
    out.copy_from_slice(&ct);
    Ok(out)
}

/// Decrypt 48-byte ciphertext back to 32-byte message key.
fn aes_decrypt_key(aes_key: &[u8; 32], ciphertext: &[u8; 48]) -> Result<[u8; 32], RatchetError> {
    let cipher = Aes256Gcm::new_from_slice(aes_key).map_err(|_| RatchetError::Aead)?;
    let nonce = Nonce::from_slice(&[0u8; 12]);
    let pt = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| RatchetError::DecryptionFailed)?;
    if pt.len() != 32 {
        return Err(RatchetError::DecryptionFailed);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&pt);
    Ok(out)
}

/// Encrypt a 32-byte message AES key. Returns (48-byte ciphertext, ratchet_pub, msg_n).
pub fn ratchet_encrypt(
    state: &mut RatchetState,
    message_key_plaintext: &[u8; 32],
) -> Result<([u8; 48], [u8; 32], u32), RatchetError> {
    let (new_chain, msg_key) = advance_chain(&state.send_chain_key);
    state.send_chain_key = new_chain;
    let msg_n = state.send_msg_n;
    state.send_msg_n += 1;

    let ciphertext = aes_encrypt_key(&msg_key, message_key_plaintext)?;
    Ok((ciphertext, state.send_ratchet_pub, msg_n))
}

/// Decrypt a 48-byte ciphertext to recover the 32-byte message AES key.
pub fn ratchet_decrypt(
    state: &mut RatchetState,
    ciphertext: &[u8; 48],
    ratchet_pub: &[u8; 32],
    msg_n: u32,
    our_ratchet_priv: &[u8; 32],
) -> Result<[u8; 32], RatchetError> {
    // Check skipped keys first
    if let Some(key) = state.skipped_keys.remove(&msg_n) {
        return aes_decrypt_key(&key, ciphertext);
    }

    // DH ratchet step if ratchet_pub changed
    let do_dh_step = state.recv_ratchet_pub.as_ref() != Some(ratchet_pub);

    if do_dh_step {
        // Skip ahead in current receive chain if needed
        if state.recv_chain_key.is_some() {
            skip_message_keys(state, msg_n)?;
        }
        // Perform DH ratchet step
        let their_pub = PublicKey::from(*ratchet_pub);
        let our_priv = StaticSecret::from(*our_ratchet_priv);
        let dh = our_priv.diffie_hellman(&their_pub);
        let (new_root, recv_chain) = kdf_rk(&state.root_key, dh.as_bytes());

        // Generate new send ratchet
        let new_send_priv = StaticSecret::random_from_rng(rand::thread_rng());
        let new_send_pub = PublicKey::from(&new_send_priv);
        let dh2 = new_send_priv.diffie_hellman(&their_pub);
        let (new_root2, send_chain) = kdf_rk(&new_root, dh2.as_bytes());

        state.prev_send_chain_n = state.send_msg_n;
        state.send_msg_n = 0;
        state.recv_msg_n = 0;
        state.root_key = new_root2;
        state.recv_chain_key = Some(recv_chain);
        state.send_chain_key = send_chain;
        state.recv_ratchet_pub = Some(*ratchet_pub);
        state.send_ratchet_pub = new_send_pub.to_bytes();
        state.send_ratchet_priv = *new_send_priv.as_bytes();
    }

    // Skip to msg_n in receive chain
    skip_message_keys(state, msg_n)?;

    let recv_chain = state
        .recv_chain_key
        .ok_or(RatchetError::DecryptionFailed)?;
    let (new_chain, msg_key) = advance_chain(&recv_chain);
    state.recv_chain_key = Some(new_chain);
    state.recv_msg_n += 1;

    aes_decrypt_key(&msg_key, ciphertext)
}

fn skip_message_keys(state: &mut RatchetState, until: u32) -> Result<(), RatchetError> {
    if state.recv_msg_n + MAX_SKIP < until {
        return Err(RatchetError::TooManySkippedMessages);
    }
    while state.recv_msg_n < until {
        if let Some(recv_chain) = state.recv_chain_key {
            let (new_chain, msg_key) = advance_chain(&recv_chain);
            state.recv_chain_key = Some(new_chain);
            state.skipped_keys.insert(state.recv_msg_n, msg_key);
            state.recv_msg_n += 1;
        } else {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_shared_secret() -> [u8; 32] {
        [42u8; 32]
    }

    #[test]
    fn test_ratchet_send_recv() {
        // Alice is sender, Bob is receiver
        let shared = make_shared_secret();
        let bob_ratchet_priv = StaticSecret::random_from_rng(rand::thread_rng());
        let bob_ratchet_pub = PublicKey::from(&bob_ratchet_priv);

        let mut alice = RatchetState::init_sender(shared, bob_ratchet_pub.to_bytes());
        let mut bob = RatchetState::init_receiver(shared, *bob_ratchet_priv.as_bytes());

        let msg_key = [1u8; 32];
        let (ct, ratchet_pub, msg_n) = ratchet_encrypt(&mut alice, &msg_key).unwrap();

        let decrypted =
            ratchet_decrypt(&mut bob, &ct, &ratchet_pub, msg_n, bob_ratchet_priv.as_bytes())
                .unwrap();
        assert_eq!(msg_key, decrypted);
    }

    #[test]
    fn test_ratchet_multiple_messages() {
        let shared = make_shared_secret();
        let bob_ratchet_priv = StaticSecret::random_from_rng(rand::thread_rng());
        let bob_ratchet_pub = PublicKey::from(&bob_ratchet_priv);

        let mut alice = RatchetState::init_sender(shared, bob_ratchet_pub.to_bytes());
        let mut bob = RatchetState::init_receiver(shared, *bob_ratchet_priv.as_bytes());

        // Send 3 messages
        for i in 0u8..3 {
            let msg_key = [i; 32];
            let (ct, ratchet_pub, msg_n) = ratchet_encrypt(&mut alice, &msg_key).unwrap();
            let decrypted =
                ratchet_decrypt(&mut bob, &ct, &ratchet_pub, msg_n, bob_ratchet_priv.as_bytes())
                    .unwrap();
            assert_eq!(msg_key, decrypted, "message {i} mismatch");
        }
    }
}
