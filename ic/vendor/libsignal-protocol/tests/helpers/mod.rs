#![allow(dead_code)]

use libsignal_protocol::{
    crypto::{Crypto, Sha256Hmac, Sha512Digest, SignalCipherType},
    InternalError,
};
use std::{panic::RefUnwindSafe, sync::Mutex};

pub(crate) struct MockCrypto<C> {
    inner: C,
    random_func:
        Option<Box<dyn Fn(&mut [u8]) -> Result<(), InternalError> + RefUnwindSafe + 'static>>,
}

impl<C: Crypto> MockCrypto<C> {
    pub fn new(inner: C) -> MockCrypto<C> {
        MockCrypto {
            inner,
            random_func: None,
        }
    }

    pub fn random_func<F>(mut self, func: F) -> Self
    where
        F: Fn(&mut [u8]) -> Result<(), InternalError> + RefUnwindSafe + 'static,
    {
        self.random_func = Some(Box::new(func));
        self
    }
}

impl<C: Crypto> Crypto for MockCrypto<C> {
    fn fill_random(&self, buffer: &mut [u8]) -> Result<(), InternalError> {
        if let Some(ref random_func) = self.random_func {
            random_func(buffer)
        } else {
            self.inner.fill_random(buffer)
        }
    }

    fn hmac_sha256(&self, key: &[u8]) -> Result<Box<dyn Sha256Hmac>, InternalError> {
        self.inner.hmac_sha256(key)
    }

    fn sha512_digest(&self) -> Result<Box<dyn Sha512Digest>, InternalError> {
        self.inner.sha512_digest()
    }

    fn encrypt(
        &self,
        cipher: SignalCipherType,
        key: &[u8],
        iv: &[u8],
        data: &[u8],
    ) -> Result<Vec<u8>, InternalError> {
        self.inner.encrypt(cipher, key, iv, data)
    }

    fn decrypt(
        &self,
        cipher: SignalCipherType,
        key: &[u8],
        iv: &[u8],
        data: &[u8],
    ) -> Result<Vec<u8>, InternalError> {
        self.inner.decrypt(cipher, key, iv, data)
    }
}

pub fn fake_random_generator() -> impl Fn(&mut [u8]) -> Result<(), InternalError> {
    let test_next_random = Mutex::new(0);

    move |data| {
        let mut test_next_random = test_next_random.lock().unwrap();

        for i in 0..data.len() {
            data[i] = *test_next_random;
            *test_next_random = test_next_random.wrapping_add(1);
        }

        Ok(())
    }
}
