use crate::{
    context::{Context, ContextInner},
    errors::FromInternalErrorCode,
    keys::PublicKey,
    messages::{CiphertextMessage, CiphertextType, SignalMessage},
    raw_ptr::Raw,
};
use failure::Error;
use std::{convert::TryFrom, ptr, rc::Rc};

/// A message containing everything necessary to establish a session.
#[derive(Debug, Clone)]
pub struct PreKeySignalMessage {
    pub(crate) raw: Raw<sys::pre_key_signal_message>,
    pub(crate) _ctx: Rc<ContextInner>,
}

impl PreKeySignalMessage {
    /// Deserialize a pre-key signal message with the provided context.
    pub fn deserialize_with_context(ctx: &Context, data: &[u8]) -> Result<Self, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::pre_key_signal_message_deserialize(&mut raw, data.as_ptr(), data.len(), ctx.raw())
                .into_result()?;
            Ok(Self {
                raw: Raw::from_ptr(raw),
                _ctx: Rc::clone(&ctx.0),
            })
        }
    }

    /// Get the message format version.
    pub fn message_version(&self) -> u8 {
        unsafe { sys::pre_key_signal_message_get_message_version(self.raw.as_const_ptr()) }
    }

    /// The sender's public pre-key.
    pub fn identity_key(&self) -> PublicKey {
        unsafe {
            let ptr = sys::pre_key_signal_message_get_identity_key(self.raw.as_const_ptr());
            assert!(!ptr.is_null());
            PublicKey {
                raw: Raw::copied_from(ptr),
            }
        }
    }

    /// The registration ID for this pre-key.
    pub fn registration_id(&self) -> u32 {
        unsafe { sys::pre_key_signal_message_get_registration_id(self.raw.as_const_ptr()) }
    }

    /// Does this message contain a pre-key ID?
    pub fn has_pre_key_id(&self) -> bool {
        unsafe { sys::pre_key_signal_message_has_pre_key_id(self.raw.as_const_ptr()) != 0 }
    }

    /// Get the pre-key's ID.
    pub fn pre_key_id(&self) -> Option<u32> {
        if !self.has_pre_key_id() {
            return None;
        }

        unsafe {
            Some(sys::pre_key_signal_message_get_pre_key_id(
                self.raw.as_const_ptr(),
            ))
        }
    }

    /// Get the signed pre-key ID.
    pub fn signed_pre_key_id(&self) -> u32 {
        unsafe { sys::pre_key_signal_message_get_signed_pre_key_id(self.raw.as_const_ptr()) }
    }

    /// Get this message's base key.
    pub fn base_key(&self) -> PublicKey {
        unsafe {
            let raw = sys::pre_key_signal_message_get_base_key(self.raw.as_const_ptr());
            assert!(!raw.is_null());
            PublicKey {
                raw: Raw::copied_from(raw),
            }
        }
    }

    /// Get the internal [`SignalMessage`].
    pub fn signal_message(&self) -> SignalMessage {
        unsafe {
            let raw = sys::pre_key_signal_message_get_signal_message(self.raw.as_const_ptr());
            assert!(!raw.is_null());
            SignalMessage {
                raw: Raw::copied_from(raw),
                _ctx: Rc::clone(&self._ctx),
            }
        }
    }
}

impl TryFrom<CiphertextMessage> for PreKeySignalMessage {
    type Error = Error;

    fn try_from(other: CiphertextMessage) -> Result<Self, Self::Error> {
        if other.get_type()? != CiphertextType::PreKey {
            Err(failure::err_msg("Expected a pre-key ciphertext message"))
        } else {
            // safety: the `CiphertextType` check tells us this is actually a
            // pointer to a `pre_key_signal_message`
            let raw =
                unsafe { Raw::copied_from(other.raw.as_ptr() as *mut sys::pre_key_signal_message) };
            Ok(PreKeySignalMessage {
                raw,
                _ctx: other._ctx,
            })
        }
    }
}

impl From<PreKeySignalMessage> for CiphertextMessage {
    fn from(other: PreKeySignalMessage) -> CiphertextMessage {
        CiphertextMessage {
            raw: other.raw.upcast(),
            _ctx: other._ctx,
        }
    }
}

impl_is_a!(sys::pre_key_signal_message => sys::ciphertext_message);
