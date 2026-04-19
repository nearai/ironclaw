use openssl::{
    hash::{Hasher, MessageDigest},
    nid::Nid,
    pkey::{PKey, Private},
    sign::Signer,
    symm::{Cipher, Crypter, Mode},
};

use crate::{
    crypto::{Crypto, Sha256Hmac, Sha512Digest, SignalCipherType},
    errors::InternalError,
};

rental! {
    mod hmac {
        use super::*;
        #[rental]
        pub struct HmacSigner {
            pkey: Box<PKey<Private>>,
            signer: Signer<'pkey>,
        }
    }
}

/// Cryptography routines built on top of the system's `openssl` library.
#[derive(Debug, Copy, Clone)]
pub struct OpenSSLCrypto;

impl OpenSSLCrypto {
    fn crypter(
        &self,
        mode: Mode,
        cipher: SignalCipherType,
        key: &[u8],
        iv: &[u8],
        data: &[u8],
    ) -> Result<Vec<u8>, InternalError> {
        let signal_cipher_type = match (cipher, key.len()) {
            (SignalCipherType::AesCtrNoPadding, 16) => Cipher::aes_128_ctr(),
            (SignalCipherType::AesCtrNoPadding, 24) => {
                let nid = Nid::AES_192_CTR;
                Cipher::from_nid(nid).expect("OpenSSL should have AES_192_CTR !!")
            }
            (SignalCipherType::AesCtrNoPadding, 32) => Cipher::aes_256_ctr(),
            (SignalCipherType::AesCbcPkcs5, 16) => Cipher::aes_128_cbc(),
            (SignalCipherType::AesCbcPkcs5, 24) => {
                let nid = Nid::AES_192_CBC;
                Cipher::from_nid(nid).expect("OpenSSL should have AES_192_CBC !!")
            }
            (SignalCipherType::AesCbcPkcs5, 32) => Cipher::aes_256_cbc(),
            _ => unreachable!(),
        };
        let block_size = signal_cipher_type.block_size();
        let mut crypter = Crypter::new(signal_cipher_type, mode, key, Some(iv))
            .map_err(|_e| InternalError::Unknown)?;
        let mut result = match cipher {
            SignalCipherType::AesCtrNoPadding => {
                crypter.pad(false); // in ctr we need to set padding to false
                vec![0u8; data.len()]
            }
            SignalCipherType::AesCbcPkcs5 => vec![0u8; data.len() + block_size],
        };
        let mut count = crypter
            .update(data, &mut result)
            .map_err(|_e| InternalError::Unknown)?;

        count += crypter
            .finalize(&mut result)
            .map_err(|_e| InternalError::Unknown)?;
        result.truncate(count);
        Ok(result)
    }
}

impl Crypto for OpenSSLCrypto {
    fn fill_random(&self, buffer: &mut [u8]) -> Result<(), InternalError> {
        openssl::rand::rand_bytes(buffer).map_err(|_e| InternalError::Unknown)
    }

    fn hmac_sha256(&self, key: &[u8]) -> Result<Box<dyn Sha256Hmac>, InternalError> {
        let pkey = Box::new(PKey::hmac(key).map_err(|_e| InternalError::Unknown)?);
        let hmac_signer =
            hmac::HmacSigner::try_new(pkey, |pkey| Signer::new(MessageDigest::sha256(), pkey))
                .map_err(|_e| InternalError::Unknown)?;
        Ok(Box::new(hmac_signer))
    }

    fn sha512_digest(&self) -> Result<Box<dyn Sha512Digest>, InternalError> {
        let ty = MessageDigest::sha512();
        let hasher = Hasher::new(ty).map_err(|_e| InternalError::Unknown)?;

        Ok(Box::new(hasher))
    }

    fn encrypt(
        &self,
        cipher: SignalCipherType,
        key: &[u8],
        iv: &[u8],
        data: &[u8],
    ) -> Result<Vec<u8>, InternalError> {
        self.crypter(Mode::Encrypt, cipher, key, iv, data)
    }

    fn decrypt(
        &self,
        cipher: SignalCipherType,
        key: &[u8],
        iv: &[u8],
        data: &[u8],
    ) -> Result<Vec<u8>, InternalError> {
        self.crypter(Mode::Decrypt, cipher, key, iv, data)
    }
}

impl Default for OpenSSLCrypto {
    fn default() -> OpenSSLCrypto {
        OpenSSLCrypto
    }
}

impl Sha256Hmac for hmac::HmacSigner {
    fn update(&mut self, data: &[u8]) -> Result<(), InternalError> {
        self.rent_mut(|signer| signer.update(data))
            .map_err(|_| InternalError::Unknown)
    }

    fn finalize(&mut self) -> Result<Vec<u8>, InternalError> {
        self.rent_mut(|signer| signer.sign_to_vec())
            .map_err(|_| InternalError::Unknown)
    }
}

impl Sha512Digest for Hasher {
    fn update(&mut self, data: &[u8]) -> Result<(), InternalError> {
        self.update(data).map_err(|_| InternalError::Unknown)
    }

    fn finalize(&mut self) -> Result<Vec<u8>, InternalError> {
        self.finish()
            .map(|bytes| bytes.as_ref().to_vec())
            .map_err(|_| InternalError::Unknown)
    }
}
