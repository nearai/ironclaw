use crate::{
    context::ContextInner,
    errors::{FromInternalErrorCode, InternalError},
    raw_ptr::Raw,
    Address, SessionRecord,
};
use failure::Error;
use std::{
    fmt::{self, Debug, Formatter},
    ptr,
    rc::Rc,
};

/// Something which contains state used by the signal protocol.
///
/// Under the hood this contains several "Stores" for various keys and session
/// state (e.g. which identities are trusted, and their pre-keys).
#[derive(Debug, Clone)]
pub struct StoreContext(pub(crate) Rc<StoreContextInner>);

impl StoreContext {
    pub(crate) fn new(
        raw: *mut sys::signal_protocol_store_context,
        ctx: &Rc<ContextInner>,
    ) -> StoreContext {
        StoreContext(Rc::new(StoreContextInner {
            raw,
            ctx: Rc::clone(ctx),
        }))
    }

    /// Get the registration ID.
    pub fn registration_id(&self) -> Result<u32, Error> {
        unsafe {
            let mut id = 0;
            sys::signal_protocol_identity_get_local_registration_id(self.raw(), &mut id)
                .into_result()?;

            Ok(id)
        }
    }

    /// Does this store already contain a session with the provided recipient?
    pub fn contains_session(&self, addr: &Address) -> Result<bool, Error> {
        unsafe {
            match sys::signal_protocol_session_contains_session(self.raw(), addr.raw()) {
                0 => Ok(false),
                1 => Ok(true),
                code => Err(Error::from(
                    InternalError::from_error_code(code).unwrap_or(InternalError::Unknown),
                )),
            }
        }
    }

    /// Load the session corresponding to the provided recipient.
    pub fn load_session(&self, addr: &Address) -> Result<SessionRecord, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::signal_protocol_session_load_session(self.raw(), &mut raw, addr.raw())
                .into_result()?;

            Ok(SessionRecord {
                raw: Raw::from_ptr(raw),
                ctx: Rc::clone(&self.0.ctx),
            })
        }
    }

    pub(crate) fn raw(&self) -> *mut sys::signal_protocol_store_context {
        self.0.raw
    }
}

pub(crate) struct StoreContextInner {
    raw: *mut sys::signal_protocol_store_context,
    // the global context must outlive `signal_protocol_store_context`
    #[allow(dead_code)]
    ctx: Rc<ContextInner>,
}

impl Drop for StoreContextInner {
    fn drop(&mut self) {
        unsafe {
            sys::signal_protocol_store_context_destroy(self.raw);
        }
    }
}

impl Debug for StoreContextInner {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("StoreContextInner").finish()
    }
}
