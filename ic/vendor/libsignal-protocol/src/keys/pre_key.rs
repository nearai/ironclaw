use crate::{
    default_context, errors::FromInternalErrorCode, keys::KeyPair, raw_ptr::Raw, Serializable,
};
use failure::Error;
use std::{
    fmt::{self, Debug, Formatter},
    ptr,
};

/// An unsigned pre-key.
#[derive(Clone)]
pub struct PreKey {
    pub(crate) raw: Raw<sys::session_pre_key>,
}

impl PreKey {
    /// Create a new pre-key based on an identity key-pair.
    pub fn new(id: u32, key_pair: &KeyPair) -> Result<PreKey, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::session_pre_key_create(&mut raw, id, key_pair.raw.as_ptr()).into_result()?;

            Ok(PreKey {
                raw: Raw::from_ptr(raw),
            })
        }
    }

    /// Get the pre-key ID.
    pub fn id(&self) -> u32 {
        unsafe { sys::session_pre_key_get_id(self.raw.as_const_ptr()) }
    }

    /// Get this pre-key's key pair.
    pub fn key_pair(&self) -> KeyPair {
        unsafe {
            let raw = sys::session_pre_key_get_key_pair(self.raw.as_const_ptr());
            assert!(!raw.is_null());
            KeyPair {
                raw: Raw::copied_from(raw),
            }
        }
    }
}

impl Serializable for PreKey {
    fn deserialize(data: &[u8]) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let ctx = default_context()?;
        unsafe {
            let mut raw = ptr::null_mut();
            sys::session_pre_key_deserialize(&mut raw, data.as_ptr(), data.len(), ctx.raw())
                .into_result()?;

            Ok(Self {
                raw: Raw::from_ptr(raw),
            })
        }
    }

    fn serialize(&self) -> Result<crate::Buffer, Error> {
        unsafe {
            let mut buffer = ptr::null_mut();
            sys::session_pre_key_serialize(&mut buffer, self.raw.as_const_ptr()).into_result()?;
            Ok(crate::Buffer::from_raw(buffer))
        }
    }
}

impl Debug for PreKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreKey")
            .field("key_pair", &self.key_pair())
            .field("id", &self.id())
            .finish()
    }
}
