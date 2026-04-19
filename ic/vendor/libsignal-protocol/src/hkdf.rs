use crate::{
    context::ContextInner,
    errors::{FromInternalErrorCode, InternalError},
    raw_ptr::Raw,
    Context,
};
use failure::Error;
use std::{ptr, rc::Rc};

/// Context for a HMAC-based Key Derivation Function.
#[derive(Debug, Clone)]
pub struct HMACBasedKeyDerivationFunction {
    pub(crate) raw: Raw<sys::hkdf_context>,
    ctx: Rc<ContextInner>,
}

impl HMACBasedKeyDerivationFunction {
    pub(crate) fn new(
        version: i32,
        ctx: &Context,
    ) -> Result<HMACBasedKeyDerivationFunction, Error> {
        unsafe {
            let mut raw = ptr::null_mut();
            sys::hkdf_create(&mut raw, version as _, ctx.raw()).into_result()?;

            Ok(HMACBasedKeyDerivationFunction {
                raw: Raw::from_ptr(raw),
                ctx: Rc::clone(&ctx.0),
            })
        }
    }

    /// Derive a new secret by cryptographically "stretching" the provided
    /// information to the expected length.
    pub fn derive_secrets(
        &self,
        secret_length: usize,
        input_key_material: &[u8],
        salt: &[u8],
        info: &[u8],
    ) -> Result<Vec<u8>, Error> {
        unsafe {
            let mut secret = ptr::null_mut();
            let prk_len = sys::hkdf_derive_secrets(
                self.raw.as_ptr(),
                &mut secret,
                input_key_material.as_ptr(),
                input_key_material.len(),
                salt.as_ptr(),
                salt.len(),
                info.as_ptr(),
                info.len(),
                secret_length,
            );

            if prk_len < 0 {
                return Err(InternalError::from_error_code(prk_len as i32)
                    .unwrap_or(InternalError::Unknown)
                    .into());
            }

            // Note: I'm not 100% sure this is sound. `secret` was allocated
            // using malloc, but the allocator used to free our Vec is
            // unspecified...
            let secret = std::slice::from_raw_parts_mut(secret, secret_length);
            Ok(Vec::from(Box::from_raw(secret)))
        }
    }
}
