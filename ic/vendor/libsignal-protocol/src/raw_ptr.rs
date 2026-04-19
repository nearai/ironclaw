use std::ptr::NonNull;

/// A wrapper around `libsignal-protocol`'s native reference counted pointers.
#[derive(Debug)]
pub struct Raw<T: IsA<sys::signal_type_base>>(NonNull<T>);

impl<T: IsA<sys::signal_type_base>> Raw<T> {
    /// Create a new [`Raw`] from an owned pointer (doesn't affect reference
    /// count).
    ///
    /// # Safety
    ///
    /// This assumes `raw` points to a valid, non-null instance of `T`.
    pub unsafe fn from_ptr(raw: *mut T) -> Raw<T> {
        debug_assert!(!raw.is_null());
        Raw(NonNull::new_unchecked(raw))
    }

    /// Create a new [`Raw`] after bumping the reference count.
    ///
    /// # Safety
    ///
    /// This assumes `raw` points to a valid, non-null instance of `T`.
    pub unsafe fn copied_from(raw: *mut T) -> Raw<T> {
        sys::signal_type_ref(T::upcast(raw));
        Raw::from_ptr(raw)
    }

    /// Go from a `*mut Child` to a `*mut Parent`.
    ///
    /// # Safety
    ///
    /// This call relies on the [`IsA`] implementation being sound.
    pub fn upcast<Parent>(self) -> Raw<Parent>
    where
        T: IsA<Parent>,
        Parent: IsA<sys::signal_type_base>,
    {
        // safety: self.0 is guaranteed to point to a valid `T` instance.
        // It also assums the `upcast()` method is sound.
        unsafe { Raw::from_ptr(T::upcast(self.into_inner())) }
    }

    pub fn as_ptr(&self) -> *mut T {
        self.0.as_ptr()
    }

    /// Consume this [`Raw`] and extract the underlying pointer **without**
    /// decrementing its reference count.
    pub fn into_inner(self) -> *mut T {
        let ptr = self.0.as_ptr();
        std::mem::forget(self);
        ptr
    }

    pub fn as_const_ptr(&self) -> *const T {
        self.as_ptr() as *const T
    }
}

impl<T: IsA<sys::signal_type_base>> Clone for Raw<T> {
    fn clone(&self) -> Raw<T> {
        unsafe { Raw::copied_from(self.as_ptr()) }
    }
}

impl<T: IsA<sys::signal_type_base>> Drop for Raw<T> {
    fn drop(&mut self) {
        unsafe {
            sys::signal_type_unref(T::upcast(self.0.as_ptr()));
        }
    }
}

/// A marker trait which represents the *is-a* relationship, letting us emulate
/// C-style inheritance.
///
/// This is mainly used by the [`Raw::upcast`] method.
///
/// # Safety
///
/// A `*mut Self` pointer **must** also be a valid `*mut Parent` pointer. This
/// is usually implemented by placing a `Parent` as the first field inside a
/// `#[repr(C)]` struct.
pub unsafe trait IsA<Parent> {
    unsafe fn upcast(this: *mut Self) -> *mut Parent;
}

impl_is_a! {
    sys::ciphertext_message => sys::signal_type_base,
    sys::ec_key_pair => sys::signal_type_base,
    sys::ec_private_key => sys::signal_type_base,
    sys::ec_public_key => sys::signal_type_base,
    sys::hkdf_context => sys::signal_type_base,
    sys::pre_key_signal_message => sys::signal_type_base,
    sys::ratchet_identity_key_pair => sys::signal_type_base,
    sys::session_pre_key => sys::signal_type_base,
    sys::session_pre_key_bundle => sys::signal_type_base,
    sys::session_record => sys::signal_type_base,
    sys::session_signed_pre_key => sys::signal_type_base,
    sys::session_state => sys::signal_type_base,
    sys::signal_message => sys::signal_type_base,
}
