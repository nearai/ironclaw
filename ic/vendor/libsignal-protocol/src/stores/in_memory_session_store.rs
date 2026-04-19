use crate::{
    stores::{SerializedSession, SessionStore},
    Address, InternalError,
};
use std::{collections::HashMap, sync::Mutex};

/// An in-memory [`SessionStore`].
#[derive(Debug, Default)]
pub struct InMemorySessionStore {
    sessions: Mutex<HashMap<Address, SerializedSession>>,
}

impl SessionStore for InMemorySessionStore {
    fn load_session(&self, address: Address) -> Result<Option<SerializedSession>, InternalError> {
        Ok(self.sessions.lock().unwrap().get(&address).cloned())
    }

    fn get_sub_device_sessions(&self, name: &[u8]) -> Result<Vec<i32>, InternalError> {
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .keys()
            .filter_map(|addr| {
                if addr.bytes() == name {
                    Some(addr.device_id())
                } else {
                    None
                }
            })
            .collect())
    }

    fn store_session(
        &self,
        addr: Address,
        session: SerializedSession,
    ) -> Result<(), InternalError> {
        self.sessions.lock().unwrap().insert(addr, session);
        Ok(())
    }

    fn delete_session(&self, addr: Address) -> Result<(), InternalError> {
        self.sessions.lock().unwrap().remove(&addr);
        Ok(())
    }

    fn delete_all_sessions(&self, name: &[u8]) -> Result<usize, InternalError> {
        let mut sessions = self.sessions.lock().unwrap();

        let to_delete: Vec<_> = sessions
            .keys()
            .filter(|addr| addr.bytes() == name)
            .cloned()
            .collect();

        for addr in &to_delete {
            sessions.remove(addr);
        }

        Ok(to_delete.len())
    }

    fn contains_session(&self, addr: Address) -> Result<bool, InternalError> {
        Ok(self.sessions.lock().unwrap().contains_key(&addr))
    }
}
