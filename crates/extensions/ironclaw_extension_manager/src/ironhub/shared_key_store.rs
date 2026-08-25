//! Operator-scoped storage for the IronHub agent shared key.
//! `IRONHUB_AGENT_SHARED_KEY` keeps the higher precedence.

use std::sync::Arc;

use ironclaw_host_api::{ids::SecretHandle, resource::ResourceScope};
use ironclaw_secrets::{SecretMaterial, SecretStoreError, SecretStorePort};
use thiserror::Error;

const HANDLE: &str = "ironhub_agent_shared_key";

#[derive(Clone)]
pub struct IronhubSharedKeyStore {
    store: Arc<dyn SecretStorePort>,
}

impl IronhubSharedKeyStore {
    pub fn new(store: Arc<dyn SecretStorePort>) -> Self {
        Self { store }
    }

    pub async fn put(&self, value: SecretMaterial) -> Result<(), IronhubSharedKeyStoreError> {
        self.store
            .put(scope(), handle()?, value, None)
            .await
            .map_err(IronhubSharedKeyStoreError::Store)?;
        Ok(())
    }

    pub async fn exists(&self) -> Result<bool, IronhubSharedKeyStoreError> {
        Ok(self
            .store
            .metadata(&scope(), &handle()?)
            .await
            .map_err(IronhubSharedKeyStoreError::Store)?
            .is_some())
    }

    pub async fn read(&self) -> Result<Option<SecretMaterial>, IronhubSharedKeyStoreError> {
        let scope = scope();
        let lease = match self.store.lease_once(&scope, &handle()?).await {
            Ok(lease) => lease,
            Err(error) if error.is_unknown_secret() => return Ok(None),
            Err(error) => return Err(IronhubSharedKeyStoreError::Store(error)),
        };
        let material = self
            .store
            .consume(&scope, lease.id)
            .await
            .map_err(IronhubSharedKeyStoreError::Store)?;
        Ok(Some(material))
    }

    pub async fn delete(&self) -> Result<bool, IronhubSharedKeyStoreError> {
        self.store
            .delete(&scope(), &handle()?)
            .await
            .map_err(IronhubSharedKeyStoreError::Store)
    }
}

fn handle() -> Result<SecretHandle, IronhubSharedKeyStoreError> {
    SecretHandle::new(HANDLE).map_err(|source| IronhubSharedKeyStoreError::InvalidHandle {
        reason: source.to_string(),
    })
}

fn scope() -> ResourceScope {
    ResourceScope::system()
}

#[derive(Debug, Error)]
pub enum IronhubSharedKeyStoreError {
    #[error("invalid secret handle for IronHub agent shared key: {reason}")]
    InvalidHandle { reason: String },
    #[error("secret store error: {0}")]
    Store(#[source] SecretStoreError),
}

#[cfg(test)]
mod tests {
    use ironclaw_secrets::SecretStore;

    use super::*;

    const KEY: &str = "ihub_sk_TestSharedKey00000000000000000000000";

    fn store() -> IronhubSharedKeyStore {
        IronhubSharedKeyStore::new(Arc::new(SecretStore::ephemeral()))
    }

    #[tokio::test]
    async fn put_then_read_round_trips() {
        let secret = store();
        assert!(!secret.exists().await.expect("exists"));
        assert!(secret.read().await.expect("read").is_none());

        secret.put(SecretMaterial::from(KEY)).await.expect("put");

        assert!(secret.exists().await.expect("exists"));
        let value = secret.read().await.expect("read").expect("some");
        assert_eq!(secrecy::ExposeSecret::expose_secret(&value), KEY);
    }

    #[tokio::test]
    async fn read_is_repeatable_across_reloads() {
        let secret = store();
        secret.put(SecretMaterial::from(KEY)).await.expect("put");
        assert!(secret.read().await.expect("read 1").is_some());
        assert!(secret.read().await.expect("read 2").is_some());
    }

    #[tokio::test]
    async fn rotate_replaces_the_previous_key() {
        let secret = store();
        secret.put(SecretMaterial::from(KEY)).await.expect("put");
        let rotated = "ihub_sk_RotatedSharedKey000000000000000000";
        secret
            .put(SecretMaterial::from(rotated))
            .await
            .expect("rotate");

        let value = secret.read().await.expect("read").expect("some");
        assert_eq!(secrecy::ExposeSecret::expose_secret(&value), rotated);
    }

    #[tokio::test]
    async fn delete_removes_secret() {
        let secret = store();
        secret.put(SecretMaterial::from(KEY)).await.expect("put");
        assert!(secret.delete().await.expect("delete"));
        assert!(!secret.exists().await.expect("exists"));
        assert!(!secret.delete().await.expect("delete again"));
    }
}
