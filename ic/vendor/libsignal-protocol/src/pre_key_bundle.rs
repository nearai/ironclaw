use crate::{keys::PublicKey, raw_ptr::Raw};
use failure::Error;
use std::{
    fmt::{self, Debug, Formatter},
    ptr,
};

/// The session state used when sending a message to another user.
#[derive(Clone)]
pub struct PreKeyBundle {
    pub(crate) raw: Raw<sys::session_pre_key_bundle>,
}

impl PreKeyBundle {
    /// Get a builder struct for the [`PreKeyBundle`].
    pub fn builder() -> PreKeyBundleBuilder {
        PreKeyBundleBuilder::default()
    }

    /// Get the registration ID.
    pub fn registration_id(&self) -> u32 {
        unsafe { sys::session_pre_key_bundle_get_registration_id(self.raw.as_const_ptr()) }
    }

    /// Get the device ID.
    pub fn device_id(&self) -> i32 {
        unsafe { sys::session_pre_key_bundle_get_device_id(self.raw.as_const_ptr()) }
    }

    /// Get the pre-key ID.
    pub fn pre_key_id(&self) -> u32 {
        unsafe { sys::session_pre_key_bundle_get_pre_key_id(self.raw.as_const_ptr()) }
    }

    /// Get the pre-key itself.
    pub fn pre_key(&self) -> Result<PublicKey, Error> {
        unsafe {
            let raw = sys::session_pre_key_bundle_get_pre_key(self.raw.as_const_ptr());
            if raw.is_null() {
                Err(failure::err_msg("Unable to get the pre-key"))
            } else {
                Ok(PublicKey {
                    raw: Raw::copied_from(raw),
                })
            }
        }
    }

    /// Get the signed pre-key id.
    pub fn signed_pre_key_id(&self) -> u32 {
        unsafe { sys::session_pre_key_bundle_get_signed_pre_key_id(self.raw.as_const_ptr()) }
    }

    /// Get the signed pre-key.
    pub fn signed_pre_key(&self) -> Result<PublicKey, Error> {
        unsafe {
            let raw = sys::session_pre_key_bundle_get_signed_pre_key(self.raw.as_const_ptr());
            if raw.is_null() {
                Err(failure::err_msg("Unable to get the signed pre-key"))
            } else {
                Ok(PublicKey {
                    raw: Raw::copied_from(raw),
                })
            }
        }
    }

    /// Get the identity key.
    pub fn identity_key(&self) -> Result<PublicKey, Error> {
        unsafe {
            let raw = sys::session_pre_key_bundle_get_identity_key(self.raw.as_const_ptr());
            if raw.is_null() {
                Err(failure::err_msg("Unable to get the identity key"))
            } else {
                Ok(PublicKey {
                    raw: Raw::copied_from(raw),
                })
            }
        }
    }
}

impl Debug for PreKeyBundle {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PreKeyBundle").finish()
    }
}

/// A builder type for the [`PreKeyBundle`].
#[derive(Debug, Default)]
pub struct PreKeyBundleBuilder {
    registration_id: Option<u32>,
    device_id: Option<i32>,
    pre_key_id: Option<u32>,
    pre_key_public: Option<PublicKey>,
    signed_pre_key_id: Option<u32>,
    signed_pre_key_public: Option<PublicKey>,
    signature: Option<Vec<u8>>,
    identity_key: Option<PublicKey>,
}

impl PreKeyBundleBuilder {
    /// Set the recipient's public pre-key.
    pub fn pre_key(mut self, id: u32, public_key: &PublicKey) -> Self {
        self.pre_key_id = Some(id);
        self.pre_key_public = Some(public_key.clone());

        self
    }

    /// Set the signed pre-key.
    pub fn signed_pre_key(mut self, id: u32, signed_public_key: &PublicKey) -> Self {
        self.signed_pre_key_id = Some(id);
        self.signed_pre_key_public = Some(signed_public_key.clone());

        self
    }

    /// Set the signed pre-key's signature.
    pub fn signature(mut self, sig: &[u8]) -> Self {
        self.signature = Some(sig.to_vec());
        self
    }

    /// Set the registration ID.
    pub fn registration_id(mut self, id: u32) -> Self {
        self.registration_id = Some(id);
        self
    }

    /// Set the device ID.
    pub fn device_id(mut self, id: i32) -> Self {
        self.device_id = Some(id);
        self
    }

    /// Set the user's identity key.
    pub fn identity_key(mut self, identity_key: &PublicKey) -> Self {
        self.identity_key = Some(identity_key.clone());
        self
    }

    fn get_registration_id(&self) -> Result<u32, Error> {
        self.registration_id
            .ok_or_else(|| failure::err_msg("a registration ID is required"))
    }

    fn get_device_id(&self) -> Result<i32, Error> {
        self.device_id
            .ok_or_else(|| failure::err_msg("a device ID is required"))
    }

    fn get_identity_key(&self) -> Result<*mut sys::ec_public_key, Error> {
        match self.identity_key {
            Some(ref key) => Ok(key.raw.as_ptr()),
            None => Err(failure::err_msg("Identity key is required")),
        }
    }

    fn get_pre_key(&self) -> (u32, *mut sys::ec_public_key) {
        if let PreKeyBundleBuilder {
            pre_key_id: Some(id),
            pre_key_public: Some(ref public),
            ..
        } = self
        {
            (*id, public.raw.as_ptr())
        } else {
            (0, ptr::null_mut())
        }
    }

    fn get_signed_pre_key(&self) -> (u32, *mut sys::ec_public_key) {
        if let PreKeyBundleBuilder {
            signed_pre_key_id: Some(id),
            signed_pre_key_public: Some(ref public),
            ..
        } = self
        {
            (*id, public.raw.as_ptr())
        } else {
            (0, ptr::null_mut())
        }
    }

    /// Actually build the [`PreKeyBundle`].
    pub fn build(self) -> Result<PreKeyBundle, Error> {
        let registration_id = self.get_registration_id()?;
        let device_id = self.get_device_id()?;
        let (pre_key_id, pre_key_public) = self.get_pre_key();
        let (signed_pre_key_id, signed_pre_key_public) = self.get_signed_pre_key();
        let signature = self.signature.as_ref().map(Vec::as_slice).unwrap_or(&[]);
        let identity_key = self.get_identity_key()?;

        unsafe {
            let mut raw = ptr::null_mut();

            sys::session_pre_key_bundle_create(
                &mut raw,
                registration_id,
                device_id,
                pre_key_id,
                pre_key_public,
                signed_pre_key_id,
                signed_pre_key_public,
                signature.as_ptr(),
                signature.len(),
                identity_key,
            );
            Ok(PreKeyBundle {
                raw: Raw::from_ptr(raw),
            })
        }
    }
}
