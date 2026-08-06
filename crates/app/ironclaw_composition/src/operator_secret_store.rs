//! The production implementation of
//! [`ironclaw_product_contracts::operator_secrets::OperatorSecretValueStore`],
//! over `ironclaw_secrets::SecretStorePort` (CHECKLIST WS3, PROPOSAL §6.2.2 /
//! §8.2 / §12.1b).
//!
//! It lives here because assembly is the only layer that may name both sides:
//! the port is products-tier vocabulary and the store is a substrate, and §8.2's
//! product row says the products tier no longer holds the substrate. This is the
//! same placement as `OperatorStatusService`, the other operator port whose
//! implementor is composition rather than `ironclaw_operator`.
//!
//! **Three things this adapter owns that the operator used to.**
//!
//! 1. **The scope.** Every operation is at [`ResourceScope::system`] — LLM
//!    configuration is operator-wide, a single instance config rather than
//!    per-user. Fixing it here is what stops a products-tier caller addressing
//!    a tenant's scope through the same handle.
//! 2. **The lease protocol.** `read` is `lease_once` + `consume`. That pair
//!    looks like a one-shot consume and is not: the underlying secret persists,
//!    so a read is repeatable across provider reloads — which is the contract
//!    `OperatorSecretValueStore::read` states and the property
//!    `read_is_repeatable_across_reloads` pins below. An unknown secret is
//!    `Ok(None)`, not an error, because a provider with no operator-set key is
//!    an ordinary state.
//! 3. **Error classification.** `SecretStoreError::stable_reason()` is the only
//!    thing that crosses the port; handle names, backend detail and the
//!    substrate's `Display` stay on this side of it.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{ids::SecretHandle, resource::ResourceScope};
use ironclaw_product_contracts::operator_secrets::{
    OperatorSecretValueStore, OperatorSecretValueStoreError,
};
use ironclaw_secrets::{SecretMaterial, SecretStoreError, SecretStorePort};
use secrecy::SecretString;

/// `OperatorSecretValueStore` over the instance's shared secret store.
pub struct RuntimeOperatorSecretValueStore {
    store: Arc<dyn SecretStorePort>,
}

impl RuntimeOperatorSecretValueStore {
    /// Wrap the instance's shared secret store as the operator-facing port.
    ///
    /// Named `shared` rather than `new` because it hands back the *port*, not
    /// the concrete adapter: no caller has a reason to hold this type, and the
    /// only thing it is ever used for is being injected as
    /// `Arc<dyn OperatorSecretValueStore>`.
    pub fn shared(store: Arc<dyn SecretStorePort>) -> Arc<dyn OperatorSecretValueStore> {
        Arc::new(Self { store })
    }
}

/// The one scope the operator control plane addresses. See the module docs.
fn scope() -> ResourceScope {
    ResourceScope::system()
}

fn classify(error: &SecretStoreError) -> OperatorSecretValueStoreError {
    OperatorSecretValueStoreError::new(error.stable_reason())
}

#[async_trait]
impl OperatorSecretValueStore for RuntimeOperatorSecretValueStore {
    async fn put(
        &self,
        handle: &SecretHandle,
        value: SecretString,
    ) -> Result<(), OperatorSecretValueStoreError> {
        self.store
            .put(scope(), handle.clone(), SecretMaterial::from(value), None)
            .await
            .map_err(|error| classify(&error))?;
        Ok(())
    }

    async fn contains(&self, handle: &SecretHandle) -> Result<bool, OperatorSecretValueStoreError> {
        Ok(self
            .store
            .metadata(&scope(), handle)
            .await
            .map_err(|error| classify(&error))?
            .is_some())
    }

    async fn handles(&self) -> Result<Vec<SecretHandle>, OperatorSecretValueStoreError> {
        Ok(self
            .store
            .metadata_for_scope(&scope())
            .await
            .map_err(|error| classify(&error))?
            .into_iter()
            .map(|metadata| metadata.handle)
            .collect())
    }

    async fn read(
        &self,
        handle: &SecretHandle,
    ) -> Result<Option<SecretString>, OperatorSecretValueStoreError> {
        let scope = scope();
        let lease = match self.store.lease_once(&scope, handle).await {
            Ok(lease) => lease,
            Err(error) if error.is_unknown_secret() => return Ok(None),
            Err(error) => return Err(classify(&error)),
        };
        let material = self
            .store
            .consume(&scope, lease.id)
            .await
            .map_err(|error| classify(&error))?;
        Ok(Some(material))
    }

    async fn delete(&self, handle: &SecretHandle) -> Result<bool, OperatorSecretValueStoreError> {
        self.store
            .delete(&scope(), handle)
            .await
            .map_err(|error| classify(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_filesystem::{Fault, FaultInjecting, FilesystemOperation, InMemoryBackend};
    use ironclaw_secrets::SecretStore;
    use secrecy::ExposeSecret;

    fn handle(name: &str) -> SecretHandle {
        SecretHandle::new(name).expect("handle")
    }

    fn store() -> Arc<dyn OperatorSecretValueStore> {
        RuntimeOperatorSecretValueStore::shared(Arc::new(SecretStore::ephemeral()))
    }

    /// The property `ironclaw_operator::LlmKeyStore` used to pin on its own
    /// side and can no longer reach: `read` is `lease_once` + `consume`, which
    /// must NOT destroy the underlying secret. Every provider build and every
    /// live reload reads the key again.
    #[tokio::test]
    async fn read_is_repeatable_across_reloads() {
        let store = store();
        let handle = handle("llm_provider_acme_api_key");
        store
            .put(&handle, SecretString::from("sk-test-value"))
            .await
            .expect("put");

        for pass in 0..3 {
            let value = store
                .read(&handle)
                .await
                .expect("read")
                .unwrap_or_else(|| panic!("read {pass} must still find the secret"));
            assert_eq!(value.expose_secret(), "sk-test-value");
        }
    }

    #[tokio::test]
    async fn absent_handle_reads_as_none_rather_than_an_error() {
        assert!(
            store()
                .read(&handle("llm_provider_absent_api_key"))
                .await
                .expect("read must not error on an absent handle")
                .is_none()
        );
    }

    #[tokio::test]
    async fn put_contains_handles_delete_round_trip_at_the_system_scope() {
        let store = store();
        let handle = handle("llm_provider_acme_api_key");
        assert!(!store.contains(&handle).await.expect("contains"));
        assert!(store.handles().await.expect("handles").is_empty());

        store
            .put(&handle, SecretString::from("sk-test-value"))
            .await
            .expect("put");

        assert!(store.contains(&handle).await.expect("contains"));
        assert_eq!(
            store.handles().await.expect("handles"),
            vec![handle.clone()]
        );
        assert!(store.delete(&handle).await.expect("delete"));
        assert!(!store.delete(&handle).await.expect("delete again"));
        assert!(!store.contains(&handle).await.expect("contains"));
    }

    /// The seam half of the fail-closed chain the operator's provider-delete
    /// path depends on: a real backend fault must arrive at the port as an
    /// error carrying the substrate's stable classification — and nothing else.
    /// `ironclaw_operator` pins the other half (that it fails closed when this
    /// port errors); together they cover
    /// `FilesystemError::Backend -> SecretStoreError::StoreUnavailable ->
    /// OperatorSecretValueStoreError`.
    #[tokio::test]
    async fn a_backend_fault_surfaces_as_the_substrate_stable_reason() {
        let backend = Arc::new(
            FaultInjecting::new(InMemoryBackend::new()).with_fault(
                Fault::on(FilesystemOperation::Delete)
                    .path("secrets")
                    .backend("secret delete unavailable"),
            ),
        );
        let store =
            RuntimeOperatorSecretValueStore::shared(Arc::new(SecretStore::ephemeral_over(backend)));
        let handle = handle("llm_provider_acme_api_key");
        store
            .put(&handle, SecretString::from("sk-test-value"))
            .await
            .expect("put");

        let error = store
            .delete(&handle)
            .await
            .expect_err("delete must surface the backend fault");
        assert_eq!(error.stable_reason(), "BackendUnavailable");
        // The substrate's own detail must not cross the port.
        assert!(!error.to_string().contains("secret delete unavailable"));
        assert!(!error.to_string().contains("llm_provider_acme_api_key"));
    }
}
