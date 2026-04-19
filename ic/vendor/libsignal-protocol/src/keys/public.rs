use std::{
    cmp::{Ord, Ordering},
    ptr,
};

use failure::Error;

use crate::{
    errors::{FromInternalErrorCode, InternalError},
    raw_ptr::Raw,
    Context,
};

/// The public part of an elliptic curve key pair.
#[derive(Clone, Debug)]
pub struct PublicKey {
    pub(crate) raw: Raw<sys::ec_public_key>,
}

impl PublicKey {
    /// Deserialize a [`PublicKey`] from the raw key data.
    pub fn decode_point(ctx: &Context, key: &[u8]) -> Result<PublicKey, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::curve_decode_point(&mut raw, key.as_ptr(), key.len(), ctx.raw()).into_result()?;

            Ok(PublicKey {
                raw: Raw::from_ptr(raw),
            })
        }
    }

    /// Use this public key to check whether a message matches its signature.
    pub fn verify_signature(&self, message: &[u8], signature: &[u8]) -> Result<(), Error> {
        unsafe {
            let result = sys::curve_verify_signature(
                self.raw.as_const_ptr(),
                message.as_ptr(),
                message.len(),
                signature.as_ptr(),
                signature.len(),
            );

            if result == 1 {
                Ok(())
            } else if result == 0 {
                Err(failure::err_msg("Invalid signature"))
            } else if let Some(err) = InternalError::from_error_code(result) {
                Err(err.into())
            } else {
                Err(failure::format_err!("Unknown error code: {}", result))
            }
        }
    }
}

impl Ord for PublicKey {
    fn cmp(&self, other: &PublicKey) -> Ordering {
        let cmp = unsafe {
            sys::ec_public_key_compare(self.raw.as_const_ptr(), other.raw.as_const_ptr())
        };

        if cmp < 0 {
            Ordering::Less
        } else if cmp > 0 {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl PartialEq for PublicKey {
    fn eq(&self, other: &PublicKey) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for PublicKey {}

impl PartialOrd for PublicKey {
    fn partial_cmp(&self, other: &PublicKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl_serializable!(PublicKey, ec_public_key_serialize, asd);

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(feature = "crypto-native", feature = "crypto-openssl"))]
    #[test]
    fn decode_from_binary() {
        cfg_if::cfg_if! {
            if #[cfg(feature = "crypto-native")] {
                type Crypto = crate::crypto::DefaultCrypto;
            } else if #[cfg(feature = "crypto-openssl")] {
                type Crypto = crate::crypto::OpenSSLCrypto;
            } else {
                compile_error!("These tests require one of the crypto features to be enabled");
            }
        }
        let ctx = Context::new(Crypto::default()).unwrap();
        let public = &[
            0x05, 0x1b, 0xb7, 0x59, 0x66, 0xf2, 0xe9, 0x3a, 0x36, 0x91, 0xdf, 0xff, 0x94, 0x2b,
            0xb2, 0xa4, 0x66, 0xa1, 0xc0, 0x8b, 0x8d, 0x78, 0xca, 0x3f, 0x4d, 0x6d, 0xf8, 0xb8,
            0xbf, 0xa2, 0xe4, 0xee, 0x28,
        ];

        let _got = PublicKey::decode_point(&ctx, public).unwrap();
    }
}
