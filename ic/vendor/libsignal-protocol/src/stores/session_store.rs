use crate::{errors::InternalError, Address, Buffer};
use std::{
    os::raw::{c_char, c_int, c_void},
    panic::RefUnwindSafe,
};

/// A serialized session.
#[derive(Debug, Clone, PartialEq)]
pub struct SerializedSession {
    /// The session itself.
    pub session: Buffer,
    /// Extra data attached by the user (e.g. a name or other information).
    pub extra_data: Option<Buffer>,
}

/// Something which can store the sessions established with recipients.
pub trait SessionStore: RefUnwindSafe {
    /// Get a copy of the serialized session record corresponding to the
    /// provided recipient [`Address`].
    fn load_session(&self, address: Address) -> Result<Option<SerializedSession>, InternalError>;

    /// Get the IDs of all known devices with active sessions for a recipient.
    fn get_sub_device_sessions(&self, name: &[u8]) -> Result<Vec<i32>, InternalError>;

    /// Determine whether there is a committed session record for a
    /// recipient ID + device ID tuple.
    fn contains_session(&self, addr: Address) -> Result<bool, InternalError>;

    /// Commit to storage the session record for a given recipient ID + device
    /// ID tuple.
    fn store_session(&self, addr: Address, session: SerializedSession)
        -> Result<(), InternalError>;

    /// Remove a session record for a recipient ID + device ID tuple.
    fn delete_session(&self, addr: Address) -> Result<(), InternalError>;

    /// Remove the session records corresponding to all devices of a recipient
    /// ID.
    ///
    /// Returns the number of deleted sessions.
    fn delete_all_sessions(&self, name: &[u8]) -> Result<usize, InternalError>;
}

pub(crate) fn new_vtable<S: SessionStore + 'static>(
    session_store: S,
) -> sys::signal_protocol_session_store {
    let state: Box<State> = Box::new(State(Box::new(session_store)));

    sys::signal_protocol_session_store {
        user_data: Box::into_raw(state) as *mut c_void,
        load_session_func: Some(load_session_func),
        get_sub_device_sessions_func: Some(get_sub_device_sessions_func),
        store_session_func: Some(store_session_func),
        contains_session_func: Some(contains_session_func),
        delete_session_func: Some(delete_session_func),
        delete_all_sessions_func: Some(delete_all_sessions_func),
        destroy_func: Some(destroy_func),
    }
}

struct State(Box<dyn SessionStore>);

unsafe extern "C" fn load_session_func(
    record: *mut *mut sys::signal_buffer,
    user_record: *mut *mut sys::signal_buffer,
    address: *const sys::signal_protocol_address,
    user_data: *mut c_void,
) -> c_int {
    signal_assert!(!record.is_null());
    signal_assert!(!user_record.is_null());
    signal_assert!(!address.is_null());
    signal_assert!(!user_data.is_null());

    let state = &*(user_data as *const State);
    let address = Address::from_ptr(address);

    match signal_catch_unwind!(state.0.load_session(address)) {
        Ok(Some(SerializedSession {
            session,
            extra_data,
        })) => {
            *record = session.into_raw();
            if let Some(extra_data) = extra_data {
                *user_record = extra_data.into_raw();
            }

            1
        }
        Ok(None) => 0,
        Err(e) => e.code(),
    }
}

unsafe extern "C" fn get_sub_device_sessions_func(
    sessions: *mut *mut sys::signal_int_list,
    name: *const c_char,
    name_len: usize,
    user_data: *mut c_void,
) -> c_int {
    signal_assert!(!sessions.is_null());
    signal_assert!(!name.is_null());
    signal_assert!(!user_data.is_null());

    let state = &*(user_data as *const State);
    let name = std::slice::from_raw_parts(name as *const _, name_len);

    match signal_catch_unwind!(state.0.get_sub_device_sessions(name)) {
        Ok(got) => {
            let list = sys::signal_int_list_alloc();
            if list.is_null() {
                return InternalError::NoMemory.code();
            }

            for device_id in got {
                sys::signal_int_list_push_back(list, device_id);
            }

            *sessions = list;
            sys::SG_SUCCESS as _
        }
        Err(e) => e.code(),
    }
}

unsafe extern "C" fn store_session_func(
    address: *const sys::signal_protocol_address,
    record: *mut u8,
    record_len: usize,
    user_record: *mut u8,
    user_record_len: usize,
    user_data: *mut c_void,
) -> c_int {
    signal_assert!(!address.is_null());
    signal_assert!(!record.is_null());
    signal_assert!(!user_data.is_null());

    let state = &*(user_data as *const State);
    let addr = Address::from_ptr(address);
    let record = std::slice::from_raw_parts(record, record_len);
    let user_record = if user_record.is_null() {
        None
    } else {
        Some(std::slice::from_raw_parts(user_record, user_record_len))
    };

    let session = SerializedSession {
        session: Buffer::from(record),
        extra_data: user_record.map(Buffer::from),
    };

    match signal_catch_unwind!(state.0.store_session(addr, session)) {
        Ok(_) => sys::SG_SUCCESS as _,
        Err(e) => e.code(),
    }
}

unsafe extern "C" fn contains_session_func(
    address: *const sys::signal_protocol_address,
    user_data: *mut c_void,
) -> c_int {
    signal_assert!(!address.is_null());
    signal_assert!(!user_data.is_null());

    let state = &*(user_data as *const State);
    let addr = Address::from_ptr(address);

    match signal_catch_unwind!(state.0.contains_session(addr)) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(e) => e.code(),
    }
}

unsafe extern "C" fn delete_session_func(
    address: *const sys::signal_protocol_address,
    user_data: *mut c_void,
) -> c_int {
    signal_assert!(!address.is_null());
    signal_assert!(!user_data.is_null());

    let state = &*(user_data as *const State);
    let addr = Address::from_ptr(address);

    match signal_catch_unwind!(state.0.delete_session(addr)) {
        Ok(_) => sys::SG_SUCCESS as _,
        Err(e) => e.code(),
    }
}

unsafe extern "C" fn delete_all_sessions_func(
    name: *const c_char,
    name_len: usize,
    user_data: *mut c_void,
) -> c_int {
    signal_assert!(!name.is_null());
    signal_assert!(!user_data.is_null());

    let state = &*(user_data as *const State);
    let name = std::slice::from_raw_parts(name as *const _, name_len);

    match signal_catch_unwind!(state.0.delete_all_sessions(name)) {
        Ok(_) => sys::SG_SUCCESS as _,
        Err(e) => e.code(),
    }
}

unsafe extern "C" fn destroy_func(user_data: *mut c_void) {
    let user_data = Box::from_raw(user_data as *mut State);
    drop(user_data);
}
