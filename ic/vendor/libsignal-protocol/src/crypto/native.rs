use aes::{Aes128, Aes192, Aes256};
use cbc::cipher::{
    block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit, StreamCipher,
};
use ctr::Ctr128BE;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};

use crate::{
    crypto::{Crypto, Sha256Hmac, Sha512Digest, SignalCipherType},
    errors::InternalError,
};

// FWI, PKCS5 padding is a subset of PKCS7
type Aes128CbcEnc = cbc::Encryptor<Aes128>;
type Aes128CbcDec = cbc::Decryptor<Aes128>;
type Aes192CbcEnc = cbc::Encryptor<Aes192>;
type Aes192CbcDec = cbc::Decryptor<Aes192>;
type Aes256CbcEnc = cbc::Encryptor<Aes256>;
type Aes256CbcDec = cbc::Decryptor<Aes256>;
type Aes128Ctr = Ctr128BE<Aes128>;
type Aes192Ctr = Ctr128BE<Aes192>;
type Aes256Ctr = Ctr128BE<Aes256>;

// Create alias for HMAC-SHA256
type HmacSha256 = Hmac<Sha256>;

struct NativeHmacSha256 {
    key: Vec<u8>,
    inner: HmacSha256,
}

impl NativeHmacSha256 {
    fn new(key: &[u8]) -> Result<Self, InternalError> {
        let inner = HmacSha256::new_from_slice(key).map_err(|_| InternalError::Unknown)?;
        Ok(Self {
            key: key.to_vec(),
            inner,
        })
    }
}

#[derive(Copy, Clone, Ord, PartialOrd, PartialEq, Eq)]
pub enum Mode {
    Encrypt,
    Decrypt,
}

/// Cryptography routines using native Rust crates.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct DefaultCrypto;

impl DefaultCrypto {
    fn crypter(
        self,
        mode: Mode,
        cipher: SignalCipherType,
        key: &[u8],
        iv: &[u8],
        data: &[u8],
    ) -> Result<Vec<u8>, InternalError> {
        let result = match (cipher, key.len()) {
            (SignalCipherType::AesCtrNoPadding, 16) => {
                let mut buf = data.to_vec();
                let mut c =
                    Aes128Ctr::new_from_slices(key, iv).map_err(|_| InternalError::Unknown)?;
                c.apply_keystream(&mut buf);
                buf
            }
            (SignalCipherType::AesCtrNoPadding, 24) => {
                let mut buf = data.to_vec();
                let mut c =
                    Aes192Ctr::new_from_slices(key, iv).map_err(|_| InternalError::Unknown)?;
                c.apply_keystream(&mut buf);
                buf
            }
            (SignalCipherType::AesCtrNoPadding, 32) => {
                let mut buf = data.to_vec();
                let mut c =
                    Aes256Ctr::new_from_slices(key, iv).map_err(|_| InternalError::Unknown)?;
                c.apply_keystream(&mut buf);
                buf
            }
            (SignalCipherType::AesCbcPkcs5, 16) => match mode {
                Mode::Encrypt => Aes128CbcEnc::new_from_slices(key, iv)
                    .map_err(|_| InternalError::Unknown)?
                    .encrypt_padded_vec_mut::<Pkcs7>(data),
                Mode::Decrypt => Aes128CbcDec::new_from_slices(key, iv)
                    .map_err(|_| InternalError::Unknown)?
                    .decrypt_padded_vec_mut::<Pkcs7>(data)
                    .map_err(|_| InternalError::Unknown)?,
            },
            (SignalCipherType::AesCbcPkcs5, 24) => match mode {
                Mode::Encrypt => Aes192CbcEnc::new_from_slices(key, iv)
                    .map_err(|_| InternalError::Unknown)?
                    .encrypt_padded_vec_mut::<Pkcs7>(data),
                Mode::Decrypt => Aes192CbcDec::new_from_slices(key, iv)
                    .map_err(|_| InternalError::Unknown)?
                    .decrypt_padded_vec_mut::<Pkcs7>(data)
                    .map_err(|_| InternalError::Unknown)?,
            },
            (SignalCipherType::AesCbcPkcs5, 32) => match mode {
                Mode::Encrypt => Aes256CbcEnc::new_from_slices(key, iv)
                    .map_err(|_| InternalError::Unknown)?
                    .encrypt_padded_vec_mut::<Pkcs7>(data),
                Mode::Decrypt => Aes256CbcDec::new_from_slices(key, iv)
                    .map_err(|_| InternalError::Unknown)?
                    .decrypt_padded_vec_mut::<Pkcs7>(data)
                    .map_err(|_| InternalError::Unknown)?,
            },
            (cipher, size) => unreachable!(
                "A combination of {:?} and {} doesn't make sense",
                cipher, size
            ),
        };
        Ok(result)
    }
}

#[cfg(feature = "crypto-native")]
impl Crypto for DefaultCrypto {
    fn fill_random(&self, buffer: &mut [u8]) -> Result<(), InternalError> {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        rng.fill_bytes(buffer);
        Ok(())
    }

    fn hmac_sha256(&self, key: &[u8]) -> Result<Box<dyn Sha256Hmac>, InternalError> {
        Ok(Box::new(NativeHmacSha256::new(key)?))
    }

    fn sha512_digest(&self) -> Result<Box<dyn Sha512Digest>, InternalError> {
        Ok(Box::new(Sha512::new()))
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

#[cfg(feature = "crypto-native")]
impl Default for DefaultCrypto {
    fn default() -> Self {
        Self
    }
}

#[cfg(feature = "crypto-native")]
impl Sha512Digest for Sha512 {
    fn update(&mut self, data: &[u8]) -> Result<(), InternalError> {
        Digest::update(self, data);
        Ok(())
    }

    fn finalize(&mut self) -> Result<Vec<u8>, InternalError> {
        let result = Digest::finalize_reset(self);
        Ok(result.to_vec())
    }
}

#[cfg(feature = "crypto-native")]
impl Sha256Hmac for NativeHmacSha256 {
    fn update(&mut self, data: &[u8]) -> Result<(), InternalError> {
        Mac::update(&mut self.inner, data);
        Ok(())
    }

    fn finalize(&mut self) -> Result<Vec<u8>, InternalError> {
        let result = self.inner.clone().finalize().into_bytes();
        self.inner = HmacSha256::new_from_slice(&self.key).map_err(|_| InternalError::Unknown)?;
        Ok(result.to_vec())
    }
}
