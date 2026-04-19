use crate::{keys::PreKey, raw_ptr::Raw};
use std::fmt::{self, Debug, Formatter};

/// A list of pre-keys.
pub struct PreKeyList {
    head: *mut sys::signal_protocol_key_helper_pre_key_list_node,
    current: *mut sys::signal_protocol_key_helper_pre_key_list_node,
}

impl PreKeyList {
    pub(crate) const fn from_raw(
        head: *mut sys::signal_protocol_key_helper_pre_key_list_node,
    ) -> PreKeyList {
        PreKeyList {
            head,
            current: head,
        }
    }
}

impl Drop for PreKeyList {
    fn drop(&mut self) {
        unsafe {
            sys::signal_protocol_key_helper_key_list_free(self.head);
        }
    }
}

impl Debug for PreKeyList {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PreKeyList").finish()
    }
}

impl Iterator for PreKeyList {
    type Item = PreKey;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_null() {
            return None;
        }

        unsafe {
            let elem = sys::signal_protocol_key_helper_key_list_element(self.current);
            assert!(!elem.is_null());

            self.current = sys::signal_protocol_key_helper_key_list_next(self.current);

            Some(PreKey {
                raw: Raw::copied_from(elem),
            })
        }
    }
}
