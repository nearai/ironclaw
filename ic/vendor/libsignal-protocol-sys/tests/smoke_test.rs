extern crate libsignal_protocol_sys as sys;

use std::{ffi::c_void, ptr};

use sys::signal_context;

#[test]
fn library_initialization() {
    unsafe {
        let mut global_context: *mut signal_context = ptr::null_mut();
        let provider = sys::signal_crypto_provider {
            user_data: ptr::null_mut(),
            decrypt_func: None,
            encrypt_func: None,
            hmac_sha256_cleanup_func: Some(hmac_sha256_cleanup_func),
            hmac_sha256_final_func: Some(hmac_sha256_final_func),
            hmac_sha256_init_func: Some(hmac_sha256_init_func),
            hmac_sha256_update_func: Some(hmac_sha256_update_func),
            random_func: None,
            sha512_digest_cleanup_func: None,
            sha512_digest_final_func: None,
            sha512_digest_init_func: None,
            sha512_digest_update_func: None,
        };

        let ret = sys::signal_context_create(&mut global_context, ptr::null_mut());
        assert_eq!(ret, 0);
        let ret = sys::signal_context_set_crypto_provider(global_context, &provider);
        assert_eq!(ret, 0);
        let ret = sys::signal_context_set_locking_functions(
            global_context,
            Some(lock_function),
            Some(unlock_function),
        );
        assert_eq!(ret, 0);
    }
}

unsafe extern "C" fn lock_function(_: *mut c_void) {}
unsafe extern "C" fn unlock_function(_: *mut c_void) {}

unsafe extern "C" fn hmac_sha256_cleanup_func(_: *mut std::ffi::c_void, _: *mut std::ffi::c_void) {}

unsafe extern "C" fn hmac_sha256_final_func(
    _: *mut c_void,
    _: *mut *mut sys::signal_buffer,
    _: *mut c_void,
) -> i32 {
    0
}

unsafe extern "C" fn hmac_sha256_init_func(
    _: *mut *mut c_void,
    _: *const u8,
    _: usize,
    _: *mut c_void,
) -> i32 {
    0
}

unsafe extern "C" fn hmac_sha256_update_func(
    _: *mut c_void,
    _: *const u8,
    _: usize,
    _: *mut c_void,
) -> i32 {
    0
}
