use crate::{
    default_context, errors::FromInternalErrorCode, keys::KeyPair, raw_ptr::Raw, Serializable,
};
use failure::Error;
use std::{
    fmt::{self, Debug, Formatter},
    ptr,
    time::{Duration, SystemTime},
};

/// A signed pre-key.
#[derive(Clone)]
pub struct SessionSignedPreKey {
    pub(crate) raw: Raw<sys::session_signed_pre_key>,
}

impl SessionSignedPreKey {
    /// Create a new [`SessionSignedPreKey`] out of an existing [`KeyPair`] and
    /// its signature.
    pub fn new(
        id: u32,
        timestamp: SystemTime,
        key_pair: &KeyPair,
        signature: &[u8],
    ) -> Result<SessionSignedPreKey, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            let elapsed = timestamp.duration_since(SystemTime::UNIX_EPOCH)?;

            sys::session_signed_pre_key_create(
                &mut raw,
                id,
                elapsed.as_secs(),
                key_pair.raw.as_ptr(),
                signature.as_ptr(),
                signature.len(),
            )
            .into_result()?;

            Ok(SessionSignedPreKey {
                raw: Raw::from_ptr(raw),
            })
        }
    }

    /// Get the signed pre-key's ID.
    pub fn id(&self) -> u32 {
        unsafe { sys::session_signed_pre_key_get_id(self.raw.as_const_ptr()) }
    }

    /// When was this key signed?
    pub fn timestamp(&self) -> SystemTime {
        unsafe {
            let ts = sys::session_signed_pre_key_get_timestamp(self.raw.as_const_ptr());
            SystemTime::UNIX_EPOCH + Duration::from_secs(ts)
        }
    }

    /// Get the key pair which has been signed.
    pub fn key_pair(&self) -> KeyPair {
        unsafe {
            let raw = sys::session_signed_pre_key_get_key_pair(self.raw.as_const_ptr());
            assert!(!raw.is_null());
            KeyPair {
                raw: Raw::copied_from(raw),
            }
        }
    }

    /// Get the signature.
    pub fn signature(&self) -> &[u8] {
        unsafe {
            let len = sys::session_signed_pre_key_get_signature_len(self.raw.as_const_ptr());
            let ptr = sys::session_signed_pre_key_get_signature(self.raw.as_const_ptr());

            std::slice::from_raw_parts(ptr, len)
        }
    }
}

impl Serializable for SessionSignedPreKey {
    fn deserialize(data: &[u8]) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let ctx = default_context()?;
        unsafe {
            let mut raw = ptr::null_mut();
            sys::session_signed_pre_key_deserialize(&mut raw, data.as_ptr(), data.len(), ctx.raw())
                .into_result()?;

            Ok(Self {
                raw: Raw::from_ptr(raw),
            })
        }
    }

    fn serialize(&self) -> Result<crate::Buffer, Error> {
        unsafe {
            let mut buffer = ptr::null_mut();
            sys::session_signed_pre_key_serialize(&mut buffer, self.raw.as_const_ptr())
                .into_result()?;
            Ok(crate::Buffer::from_raw(buffer))
        }
    }
}

impl Debug for SessionSignedPreKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionSignedPreKey")
            .field("key_pair", &self.key_pair())
            .field("id", &self.id())
            .field("timestamp", &self.timestamp())
            .finish()
    }
}
