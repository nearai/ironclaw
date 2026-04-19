use crate::{
    default_context,
    errors::FromInternalErrorCode,
    keys::{PrivateKey, PublicKey},
    raw_ptr::Raw,
    Serializable,
};
use failure::Error;
use std::{
    fmt::{self, Debug, Formatter},
    ptr,
};

/// A "ratcheting" key pair.
#[derive(Clone)]
pub struct IdentityKeyPair {
    pub(crate) raw: Raw<sys::ratchet_identity_key_pair>,
}

impl IdentityKeyPair {
    /// Create a new [`IdentityKeyPair`] out of its public and private keys.
    pub fn new(public_key: &PublicKey, private_key: &PrivateKey) -> Result<IdentityKeyPair, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::ratchet_identity_key_pair_create(
                &mut raw,
                public_key.raw.as_ptr(),
                private_key.raw.as_ptr(),
            )
            .into_result()?;

            Ok(IdentityKeyPair {
                raw: Raw::from_ptr(raw),
            })
        }
    }

    /// Get the public part of this key pair.
    pub fn public(&self) -> PublicKey {
        unsafe {
            let raw = sys::ratchet_identity_key_pair_get_public(self.raw.as_const_ptr());
            assert!(!raw.is_null());
            PublicKey {
                raw: Raw::copied_from(raw),
            }
        }
    }

    /// Get the public part of this key pair.
    pub fn private(&self) -> PrivateKey {
        unsafe {
            let raw = sys::ratchet_identity_key_pair_get_private(self.raw.as_const_ptr());
            assert!(!raw.is_null());
            PrivateKey {
                raw: Raw::copied_from(raw),
            }
        }
    }
}

impl Debug for IdentityKeyPair {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityKeyPair")
            .field("public", &self.public())
            .field("private", &self.private())
            .finish()
    }
}

impl Serializable for IdentityKeyPair {
    fn deserialize(data: &[u8]) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let ctx = default_context()?;
        unsafe {
            let mut raw = ptr::null_mut();
            sys::ratchet_identity_key_pair_deserialize(
                &mut raw,
                data.as_ptr(),
                data.len(),
                ctx.raw(),
            )
            .into_result()?;

            Ok(Self {
                raw: Raw::from_ptr(raw),
            })
        }
    }

    fn serialize(&self) -> Result<crate::Buffer, Error> {
        unsafe {
            let mut buffer = ptr::null_mut();
            sys::ratchet_identity_key_pair_serialize(&mut buffer, self.raw.as_const_ptr())
                .into_result()?;
            Ok(crate::Buffer::from_raw(buffer))
        }
    }
}
