use crate::{
    default_context, errors::FromInternalErrorCode, raw_ptr::Raw, ContextInner, Serializable,
    SessionState,
};
use failure::Error;
use std::ptr;
use std::rc::Rc;

/// The serialized state of a session.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub(crate) raw: Raw<sys::session_record>,
    pub(crate) ctx: Rc<ContextInner>,
}

impl SessionRecord {
    /// Get the state.
    pub fn state(&self) -> SessionState {
        unsafe {
            let raw = sys::session_record_get_state(self.raw.as_ptr());
            assert!(!raw.is_null());
            SessionState {
                raw: Raw::copied_from(raw),
                _ctx: Rc::clone(&self.ctx),
            }
        }
    }
}

impl Serializable for SessionRecord {
    fn deserialize(data: &[u8]) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let ctx = default_context()?;
        unsafe {
            let mut raw = ptr::null_mut();
            sys::session_record_deserialize(&mut raw, data.as_ptr(), data.len(), ctx.raw())
                .into_result()?;

            Ok(Self {
                raw: Raw::from_ptr(raw),
                ctx: Rc::clone(&ctx.0),
            })
        }
    }

    fn serialize(&self) -> Result<crate::Buffer, Error> {
        unsafe {
            let mut buffer = ptr::null_mut();
            sys::session_record_serialize(&mut buffer, self.raw.as_const_ptr()).into_result()?;
            Ok(crate::Buffer::from_raw(buffer))
        }
    }
}
