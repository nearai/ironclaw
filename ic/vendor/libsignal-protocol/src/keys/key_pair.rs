use crate::{
    errors::FromInternalErrorCode,
    keys::{PrivateKey, PublicKey},
    raw_ptr::Raw,
};
use failure::Error;
use std::{
    fmt::{self, Debug, Formatter},
    ptr,
};

/// A public-private key pair.
#[derive(Clone)]
pub struct KeyPair {
    pub(crate) raw: Raw<sys::ec_key_pair>,
}

impl KeyPair {
    /// Create a new [`KeyPair`] from its public and private keys.
    pub fn new(public_key: &PublicKey, private_key: &PrivateKey) -> Result<KeyPair, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::ec_key_pair_create(&mut raw, public_key.raw.as_ptr(), private_key.raw.as_ptr())
                .into_result()?;

            Ok(KeyPair {
                raw: Raw::from_ptr(raw),
            })
        }
    }

    /// Get the public key.
    pub fn public(&self) -> PublicKey {
        unsafe {
            let raw = sys::ec_key_pair_get_public(self.raw.as_ptr());
            assert!(!raw.is_null());

            PublicKey {
                raw: Raw::copied_from(raw),
            }
        }
    }

    /// Get the private key.
    pub fn private(&self) -> PrivateKey {
        unsafe {
            let raw = sys::ec_key_pair_get_private(self.raw.as_ptr());
            assert!(!raw.is_null());

            PrivateKey {
                raw: Raw::copied_from(raw),
            }
        }
    }
}

impl Debug for KeyPair {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyPair")
            .field("public", &self.public())
            .field("private", &"<elided>")
            .finish()
    }
}
