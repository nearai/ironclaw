use crate::{
    stores::{PreKeyStore, SignedPreKeyStore},
    InternalError,
};
use std::{
    collections::HashMap,
    io::{self, Write},
    sync::Mutex,
};

/// An in-memory [`PreKeyStore`].
#[derive(Debug, Default)]
pub struct InMemoryPreKeyStore(Inner);

impl PreKeyStore for InMemoryPreKeyStore {
    fn load(&self, id: u32, writer: &mut dyn Write) -> io::Result<()> {
        self.0.load(id, writer)
    }

    fn store(&self, id: u32, body: &[u8]) -> Result<(), InternalError> {
        self.0.store(id, body)
    }

    fn contains(&self, id: u32) -> bool {
        self.0.contains(id)
    }

    fn remove(&self, id: u32) -> Result<(), InternalError> {
        self.0.remove(id)
    }
}

/// An in-memory [`SignedPreKeyStore`].
#[derive(Debug, Default)]
pub struct InMemorySignedPreKeyStore(Inner);

impl SignedPreKeyStore for InMemorySignedPreKeyStore {
    fn load(&self, id: u32, writer: &mut dyn Write) -> io::Result<()> {
        self.0.load(id, writer)
    }

    fn store(&self, id: u32, body: &[u8]) -> Result<(), InternalError> {
        self.0.store(id, body)
    }

    fn contains(&self, id: u32) -> bool {
        self.0.contains(id)
    }

    fn remove(&self, id: u32) -> Result<(), InternalError> {
        self.0.remove(id)
    }
}

#[derive(Debug, Default)]
struct Inner {
    keys: Mutex<HashMap<u32, Vec<u8>>>,
}

impl Inner {
    fn load(&self, id: u32, writer: &mut dyn Write) -> io::Result<()> {
        match self.keys.lock().unwrap().get(&id) {
            Some(bytes) => writer.write_all(bytes),
            None => unimplemented!(),
        }
    }

    fn store(&self, id: u32, body: &[u8]) -> Result<(), InternalError> {
        self.keys.lock().unwrap().insert(id, body.to_vec());
        Ok(())
    }

    fn contains(&self, id: u32) -> bool {
        self.keys.lock().unwrap().contains_key(&id)
    }

    fn remove(&self, id: u32) -> Result<(), InternalError> {
        self.keys.lock().unwrap().remove(&id);
        Ok(())
    }
}
