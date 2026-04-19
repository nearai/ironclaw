use crate::{
    address::Address,
    context::{Context, ContextInner},
    errors::{FromInternalErrorCode, InternalError},
    pre_key_bundle::PreKeyBundle,
    store_context::{StoreContext, StoreContextInner},
};
use std::{
    fmt::{self, Debug, Formatter},
    ptr,
    rc::Rc,
};

/// Create a new session.
pub struct SessionBuilder {
    raw: *mut sys::session_builder,
    // both these fields must outlive `session_builder`
    _store_ctx: Rc<StoreContextInner>,
    _ctx: Rc<ContextInner>,
    address: Address,
}

impl SessionBuilder {
    /// Create a new session builder for communication with the user with the
    /// specified address.
    pub fn new(ctx: &Context, store_context: &StoreContext, address: &Address) -> SessionBuilder {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::session_builder_create(&mut raw, store_context.raw(), address.raw(), ctx.raw());

            SessionBuilder {
                raw,
                _store_ctx: Rc::clone(&store_context.0),
                _ctx: Rc::clone(&ctx.0),
                address: address.clone(),
            }
        }
    }

    /// Build a session using a pre-key retrieved from the server.
    pub fn process_pre_key_bundle(
        &self,
        pre_key_bundle: &PreKeyBundle,
    ) -> Result<(), InternalError> {
        unsafe {
            sys::session_builder_process_pre_key_bundle(self.raw, pre_key_bundle.raw.as_ptr())
                .into_result()
        }
    }
}

impl Drop for SessionBuilder {
    fn drop(&mut self) {
        unsafe {
            sys::session_builder_free(self.raw);
        }
    }
}

impl Debug for SessionBuilder {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionBuilder")
            .field("address", &self.address)
            .finish()
    }
}
