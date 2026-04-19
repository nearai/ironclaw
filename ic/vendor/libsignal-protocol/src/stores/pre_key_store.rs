use crate::{buffer::Buffer, errors::InternalError};
use std::{
    io::{self, Write},
    os::raw::{c_int, c_void},
    panic::RefUnwindSafe,
};

/// Something which can store [`crate::keys::PreKey`]s without inspecting their
/// contents.
pub trait PreKeyStore: RefUnwindSafe {
    /// Load a pre-key.
    fn load(&self, id: u32, writer: &mut dyn Write) -> io::Result<()>;
    /// Store a pre-key.
    fn store(&self, id: u32, body: &[u8]) -> Result<(), InternalError>;
    /// Is the pre-key with this ID present in the store?
    fn contains(&self, id: u32) -> bool;
    /// Remove a pre-key from the store.
    fn remove(&self, id: u32) -> Result<(), InternalError>;
}

pub(crate) fn new_vtable<P: PreKeyStore + 'static>(store: P) -> sys::signal_protocol_pre_key_store {
    let state: Box<State> = Box::new(State(Box::new(store)));

    sys::signal_protocol_pre_key_store {
        user_data: Box::into_raw(state) as *mut c_void,
        load_pre_key: Some(load_pre_key),
        store_pre_key: Some(store_pre_key),
        contains_pre_key: Some(contains_pre_key),
        remove_pre_key: Some(remove_pre_key),
        destroy_func: Some(destroy_func),
    }
}

struct State(Box<dyn PreKeyStore>);

unsafe extern "C" fn load_pre_key(
    record: *mut *mut sys::signal_buffer,
    pre_key_id: u32,
    user_data: *mut c_void,
) -> c_int {
    signal_assert!(!user_data.is_null());
    signal_assert!(!record.is_null());
    let user_data = &*(user_data as *const State);

    let got = signal_catch_unwind!({
        let mut buffer = Buffer::new();
        match user_data.0.load(pre_key_id, &mut buffer) {
            Ok(_) => Ok(buffer),
            Err(e) => {
                log::error!(
                    "An error occurred while trying to load pre-key {}: {}",
                    pre_key_id,
                    e
                );
                Err(InternalError::Unknown)
            }
        }
    });

    match got {
        Ok(buffer) => {
            *record = buffer.into_raw();
            sys::SG_SUCCESS as c_int
        }
        Err(_) => InternalError::Unknown.code(),
    }
}

unsafe extern "C" fn store_pre_key(
    pre_key_id: u32,
    record: *mut u8,
    record_len: usize,
    user_data: *mut c_void,
) -> c_int {
    signal_assert!(!user_data.is_null());
    signal_assert!(!record.is_null());

    let user_data = &*(user_data as *const State);
    let data = std::slice::from_raw_parts(record, record_len);

    match signal_catch_unwind!(user_data.0.store(pre_key_id, data)) {
        Ok(_) => sys::SG_SUCCESS as c_int,
        Err(e) => e.code(),
    }
}

unsafe extern "C" fn contains_pre_key(pre_key_id: u32, user_data: *mut c_void) -> c_int {
    signal_assert!(!user_data.is_null());

    let user_data = &*(user_data as *const State);

    signal_catch_unwind!(user_data.0.contains(pre_key_id)) as c_int
}

unsafe extern "C" fn remove_pre_key(pre_key_id: u32, user_data: *mut c_void) -> c_int {
    signal_assert!(!user_data.is_null());

    let user_data = &*(user_data as *const State);

    match signal_catch_unwind!(user_data.0.remove(pre_key_id)) {
        Ok(_) => sys::SG_SUCCESS as c_int,
        Err(e) => e.code(),
    }
}

unsafe extern "C" fn destroy_func(user_data: *mut c_void) {
    if !user_data.is_null() {
        let user_data = Box::from_raw(user_data as *mut State);
        drop(user_data);
    }
}
