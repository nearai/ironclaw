//! NIP-04 encrypted direct messages.
//!
//! Encryption: ECDH shared secret (secp256k1), then AES-256-CBC.
//!   shared_x = ECDH(sender_sk, recipient_pk).x-coordinate
//!   aes_key  = SHA-256(shared_x)
//!   ciphertext = AES-256-CBC(plaintext, aes_key, random_iv)
//!   output = base64(iv || ciphertext)   (iv is 16 bytes, PKCS7 padded)

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::Engine;
use k256::ecdh::diffie_hellman;
use k256::elliptic_curve::point::DecompressPoint;
use k256::schnorr::SigningKey;
use k256::sha2::{Digest, Sha256};
use k256::AffinePoint;

type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

/// Compute the NIP-04 shared secret: ECDH(sk, pk) -> SHA-256(x-coordinate).
/// Returns the 32-byte AES key.
fn compute_shared_aes_key(
    sk_bytes: &[u8; 32],
    recipient_pk_bytes: &[u8; 32],
) -> Result<[u8; 32], String> {
    // Build our signing key (contains the scalar)
    let signing_key =
        SigningKey::from_bytes(sk_bytes).map_err(|e| format!("invalid secret key: {e}"))?;
    let scalar = signing_key.as_nonzero_scalar();

    // Decompress recipient's x-only pubkey to an AffinePoint.
    // Try y_is_odd = false first. Since we only use the x-coordinate
    // of the DH result, both y-parities give the same shared secret.
    let pk_point = {
        let x_bytes = k256::elliptic_curve::generic_array::GenericArray::from_slice(recipient_pk_bytes);
        let ct_option = AffinePoint::decompress(
            x_bytes,
            k256::elliptic_curve::subtle::Choice::from(0),
        );
        if bool::from(ct_option.is_some()) {
            ct_option.unwrap()
        } else {
            return Err("failed to decompress recipient pubkey (invalid x-coordinate)".into());
        }
    };

    // ECDH
    let shared = diffie_hellman(scalar, &pk_point);
    let shared_x = shared.raw_secret_bytes();

    // AES key = SHA-256(shared_x)
    let mut hasher = Sha256::new();
    hasher.update(shared_x);
    let aes_key = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&aes_key);
    Ok(key)
}

/// Encrypt a plaintext string for NIP-04.
/// Returns base64(IV || ciphertext) where IV is 16 bytes.
///
/// `iv` must be 16 bytes of cryptographically random data.
pub fn encrypt(
    plaintext: &str,
    sk_bytes: &[u8; 32],
    recipient_pk_bytes: &[u8; 32],
    iv: &[u8; 16],
) -> Result<String, String> {
    let aes_key = compute_shared_aes_key(sk_bytes, recipient_pk_bytes)?;

    // AES-256-CBC encrypt with PKCS7 padding
    let encryptor =
        Aes256CbcEnc::new_from_slices(&aes_key, iv).map_err(|e| format!("AES init error: {e}"))?;

    let plaintext_bytes = plaintext.as_bytes();
    let mut buf = vec![0u8; plaintext_bytes.len() + 16]; // room for padding
    let ciphertext = encryptor
        .encrypt_padded_b2b_mut::<Pkcs7>(plaintext_bytes, &mut buf)
        .map_err(|e| format!("AES encrypt error: {e}"))?;

    // Prepend IV
    let mut output = Vec::with_capacity(16 + ciphertext.len());
    output.extend_from_slice(iv);
    output.extend_from_slice(ciphertext);

    Ok(base64::engine::general_purpose::STANDARD.encode(&output))
}

/// Decrypt a NIP-04 encrypted message.
/// Input is base64(IV || ciphertext).
pub fn decrypt(
    encrypted: &str,
    sk_bytes: &[u8; 32],
    sender_pk_bytes: &[u8; 32],
) -> Result<String, String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encrypted.trim())
        .map_err(|e| format!("base64 decode error: {e}"))?;

    if raw.len() < 32 {
        return Err(
            "encrypted message too short (need IV + at least 16 bytes ciphertext)".into(),
        );
    }

    let iv_arr: [u8; 16] = raw[..16].try_into().map_err(|_| "IV must be 16 bytes")?;
    let ciphertext = &raw[16..];

    let aes_key = compute_shared_aes_key(sk_bytes, sender_pk_bytes)?;

    let decryptor =
        Aes256CbcDec::new_from_slices(&aes_key, &iv_arr).map_err(|e| format!("AES init error: {e}"))?;

    let mut buf = ciphertext.to_vec();
    let plaintext = decryptor
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| format!("AES decrypt error: {e}"))?;

    String::from_utf8(plaintext.to_vec()).map_err(|e| format!("UTF-8 decode: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        // Generate two keypairs
        let alice_sk = [0x01u8; 32];
        let bob_sk = [0x02u8; 32];

        let alice_signing = SigningKey::from_bytes(&alice_sk).unwrap();
        let bob_signing = SigningKey::from_bytes(&bob_sk).unwrap();

        let alice_pk: [u8; 32] = alice_signing.verifying_key().to_bytes().into();
        let bob_pk: [u8; 32] = bob_signing.verifying_key().to_bytes().into();

        let iv = [0xABu8; 16];
        let message = "Hello from NIP-04!";

        // Alice encrypts for Bob
        let encrypted = encrypt(message, &alice_sk, &bob_pk, &iv).unwrap();

        // Bob decrypts from Alice
        let decrypted = decrypt(&encrypted, &bob_sk, &alice_pk).unwrap();
        assert_eq!(decrypted, message);
    }

    #[test]
    fn test_encrypt_produces_base64() {
        let sk = [0x42u8; 32];
        let signing = SigningKey::from_bytes(&sk).unwrap();
        let pk: [u8; 32] = signing.verifying_key().to_bytes().into();
        let iv = [0u8; 16];

        let encrypted = encrypt("test message", &sk, &pk, &iv).unwrap();
        // Should be valid base64
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encrypted)
            .unwrap();
        // IV (16) + at least one block of ciphertext (16)
        assert!(decoded.len() >= 32);
    }
}
