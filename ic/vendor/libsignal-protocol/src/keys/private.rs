use crate::{errors::FromInternalErrorCode, keys::PublicKey, raw_ptr::Raw, Context};
use failure::Error;
use std::{
    cmp::{Ord, Ordering},
    ptr,
};

/// The private half of an elliptic curve key pair.
#[derive(Clone, Debug)]
pub struct PrivateKey {
    pub(crate) raw: Raw<sys::ec_private_key>,
}

impl PrivateKey {
    /// Decode a [`PrivateKey`] from raw key data.
    pub fn decode_point(ctx: &Context, key: &[u8]) -> Result<PrivateKey, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::curve_decode_private_point(&mut raw, key.as_ptr(), key.len(), ctx.raw())
                .into_result()?;

            Ok(PrivateKey {
                raw: Raw::from_ptr(raw),
            })
        }
    }

    /// Derive the public part of this key pair.
    pub fn generate_public_key(&self) -> Result<PublicKey, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::curve_generate_public_key(&mut raw, self.raw.as_const_ptr()).into_result()?;

            Ok(PublicKey {
                raw: Raw::from_ptr(raw),
            })
        }
    }
}

impl Ord for PrivateKey {
    fn cmp(&self, other: &PrivateKey) -> Ordering {
        let cmp = unsafe {
            sys::ec_private_key_compare(self.raw.as_const_ptr(), other.raw.as_const_ptr())
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

impl PartialEq for PrivateKey {
    fn eq(&self, other: &PrivateKey) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for PrivateKey {}

impl PartialOrd for PrivateKey {
    fn partial_cmp(&self, other: &PrivateKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl_serializable!(PrivateKey, ec_private_key_serialize, asd);
