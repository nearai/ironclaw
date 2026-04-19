use crate::{raw_ptr::Raw, ContextInner};
use std::rc::Rc;

/// The internal state associated with a session.
#[derive(Debug, Clone)]
pub struct SessionState {
    pub(crate) raw: Raw<sys::session_state>,
    pub(crate) _ctx: Rc<ContextInner>,
}

impl SessionState {
    /// Get the session version.
    pub fn version(&self) -> u32 {
        unsafe { sys::session_state_get_session_version(self.raw.as_ptr()) }
    }
}
