use crate::{
    keys::IdentityKeyPair, stores::IdentityKeyStore, Address, Buffer, InternalError, Serializable,
};
use std::{collections::HashMap, sync::Mutex};

/// An in-memory [`IdentityKeyStore`].
#[derive(Debug)]
pub struct InMemoryIdentityKeyStore {
    registration_id: u32,
    identity: IdentityKeyPair,
    trusted_identities: Mutex<HashMap<Address, Vec<u8>>>,
    /// Should recipients be trusted the first time they are contacted?
    pub trust_on_first_use: bool,
}

impl InMemoryIdentityKeyStore {
    /// Create a new [`InMemoryIdentityKeyStore`].
    pub fn new(registration_id: u32, identity: &IdentityKeyPair) -> InMemoryIdentityKeyStore {
        InMemoryIdentityKeyStore {
            registration_id,
            trust_on_first_use: true,
            identity: identity.clone(),
            trusted_identities: Default::default(),
        }
    }
}

impl IdentityKeyStore for InMemoryIdentityKeyStore {
    fn local_registration_id(&self) -> Result<u32, InternalError> {
        Ok(self.registration_id)
    }

    fn identity_key_pair(&self) -> Result<(Buffer, Buffer), InternalError> {
        let public = self
            .identity
            .public()
            .serialize()
            .map_err(|_| InternalError::Unknown)?;
        let private = self
            .identity
            .private()
            .serialize()
            .map_err(|_| InternalError::Unknown)?;

        Ok((public, private))
    }

    fn is_trusted_identity(
        &self,
        address: Address,
        identity_key: &[u8],
    ) -> Result<bool, InternalError> {
        let identities = self.trusted_identities.lock().unwrap();

        if let Some(identity) = identities.get(&address) {
            Ok(identity_key == identity.as_slice())
        } else {
            Ok(self.trust_on_first_use)
        }
    }

    fn save_identity(&self, addr: Address, identity_key: &[u8]) -> Result<(), InternalError> {
        self.trusted_identities
            .lock()
            .unwrap()
            .insert(addr, identity_key.to_vec());

        Ok(())
    }
}
