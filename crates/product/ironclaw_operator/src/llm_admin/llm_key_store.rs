//! Operator-scoped storage for LLM provider API-key **values**.
//!
//! The reborn provider catalog (`providers.json`) only ever references an
//! `api_key_env` *name* and the config selection only carries names — inline
//! secret values are rejected. When the webui2 settings surface lets an
//! operator paste an actual key, the value lands here instead: stored behind
//! the [`OperatorSecretValueStore`] port under a fixed per-provider handle, and
//! injected into the resolved `LlmConfig` at provider-build / reload time.
//!
//! **This crate does not name a secret substrate.** WS3 removed the direct
//! `ironclaw_secrets` edge (PROPOSAL §8.2's product row, §12.1b): the port is
//! declared in `ironclaw_product_contracts` and implemented by
//! `ironclaw_composition::RuntimeOperatorSecretValueStore`, which is the
//! only layer allowed to name both sides. What stays here is the *policy* —
//! the handle a provider id maps to, and what an absent value means.
//!
//! The handle is derived from the provider id: `llm_provider_<id>_api_key`.
//! LLM configuration is operator-wide (a single instance config, not per-user),
//! so there is exactly one scope and the port fixes it; this crate cannot name
//! a scope at all, which is a tightening the port bought rather than a
//! restatement of what was here before.

use std::collections::HashSet;
use std::sync::Arc;

use ironclaw_host_api::ids::SecretHandle;
use ironclaw_product_contracts::operator_secrets::{
    OperatorSecretValueStore, OperatorSecretValueStoreError,
};
use secrecy::SecretString;
use thiserror::Error;

/// Operator-scoped LLM API-key policy over the
/// [`OperatorSecretValueStore`] port.
#[derive(Clone)]
pub struct LlmKeyStore {
    store: Arc<dyn OperatorSecretValueStore>,
}

impl LlmKeyStore {
    /// Wrap the instance's operator secret-value store.
    pub fn new(store: Arc<dyn OperatorSecretValueStore>) -> Self {
        Self { store }
    }

    /// Store (or replace) the API-key value for `provider_id`.
    pub async fn put(
        &self,
        provider_id: &str,
        value: SecretString,
    ) -> Result<(), LlmKeyStoreError> {
        let handle = handle_for(provider_id)?;
        self.store
            .put(&handle, value)
            .await
            .map_err(|source| LlmKeyStoreError::Store {
                operation: "put",
                source,
            })?;
        Ok(())
    }

    /// [`Self::put`] taking a plain `String` rather than a [`SecretString`] —
    /// for callers outside this crate (namely `ironclaw_cli::onboard`)
    /// that must not depend on a secret substrate directly (see
    /// `crates/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs::reborn_cli_binary_crate_stays_separate_from_v1_root`,
    /// which pins `ironclaw_cli`'s allowed workspace dependency set).
    pub async fn put_plaintext(
        &self,
        provider_id: &str,
        value: String,
    ) -> Result<(), LlmKeyStoreError> {
        self.put(provider_id, SecretString::from(value)).await
    }

    /// Whether a stored key exists for `provider_id` (without revealing it).
    pub async fn exists(&self, provider_id: &str) -> Result<bool, LlmKeyStoreError> {
        let handle = handle_for(provider_id)?;
        self.store
            .contains(&handle)
            .await
            .map_err(|source| LlmKeyStoreError::Store {
                operation: "contains",
                source,
            })
    }

    /// Provider ids that have an operator-stored key.
    pub async fn stored_provider_ids(&self) -> Result<HashSet<String>, LlmKeyStoreError> {
        const PREFIX: &str = "llm_provider_";
        const SUFFIX: &str = "_api_key";

        Ok(self
            .store
            .handles()
            .await
            .map_err(|source| LlmKeyStoreError::Store {
                operation: "handles",
                source,
            })?
            .into_iter()
            .filter_map(|handle| {
                handle
                    .as_str()
                    .strip_prefix(PREFIX)?
                    .strip_suffix(SUFFIX)
                    .map(ToString::to_string)
            })
            .collect())
    }

    /// Read back the stored key value for `provider_id`, if any.
    ///
    /// Returns `Ok(None)` when no key is stored. The port contracts this read
    /// as repeatable, so a provider rebuild or a live reload can call it again;
    /// the lease protocol that makes that true lives behind the port.
    pub async fn read(&self, provider_id: &str) -> Result<Option<SecretString>, LlmKeyStoreError> {
        let handle = handle_for(provider_id)?;
        self.store
            .read(&handle)
            .await
            .map_err(|source| LlmKeyStoreError::Store {
                operation: "read",
                source,
            })
    }

    /// Delete the stored key for `provider_id`. Returns whether one existed.
    pub async fn delete(&self, provider_id: &str) -> Result<bool, LlmKeyStoreError> {
        let handle = handle_for(provider_id)?;
        self.store
            .delete(&handle)
            .await
            .map_err(|source| LlmKeyStoreError::Store {
                operation: "delete",
                source,
            })
    }
}

fn handle_for(provider_id: &str) -> Result<SecretHandle, LlmKeyStoreError> {
    SecretHandle::new(format!("llm_provider_{provider_id}_api_key")).map_err(|source| {
        LlmKeyStoreError::InvalidProviderId {
            provider_id: provider_id.to_string(),
            reason: source.to_string(),
        }
    })
}

/// Errors surfaced when storing or reading LLM key values.
#[derive(Debug, Error)]
pub enum LlmKeyStoreError {
    #[error("invalid provider id `{provider_id}` for secret handle: {reason}")]
    InvalidProviderId { provider_id: String, reason: String },
    /// A port call failed. `operation` names which one.
    ///
    /// All five port calls used to collapse into a bare
    /// `Store(OperatorSecretValueStoreError)`, so the stable reason survived
    /// but the operation did not: a store failure could not be attributed to
    /// `put`, `contains`, `handles`, `read` or `delete` without a stack trace.
    /// The name is `&'static str` rather than an enum because it is diagnostic
    /// only — nothing branches on it, and a caller that needs to branch should
    /// match the source error instead.
    #[error("operator secret {operation} failed: {source}")]
    Store {
        operation: &'static str,
        #[source]
        source: OperatorSecretValueStoreError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_contracts::test_support::fakes::FakeOperatorSecretValueStore;

    fn store() -> LlmKeyStore {
        LlmKeyStore::new(Arc::new(FakeOperatorSecretValueStore::new()))
    }

    #[tokio::test]
    async fn put_then_read_round_trips() {
        let keys = store();
        assert!(!keys.exists("acme").await.expect("exists"));
        assert!(keys.read("acme").await.expect("read").is_none());
        assert!(
            keys.stored_provider_ids()
                .await
                .expect("stored provider ids")
                .is_empty()
        );

        keys.put("acme", SecretString::from("sk-test-value"))
            .await
            .expect("put");

        assert!(keys.exists("acme").await.expect("exists"));
        assert_eq!(
            keys.stored_provider_ids()
                .await
                .expect("stored provider ids"),
            HashSet::from(["acme".to_string()])
        );
        let value = keys.read("acme").await.expect("read").expect("some");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&value),
            "sk-test-value"
        );
    }

    // `read_is_repeatable_across_reloads` moved to
    // `ironclaw_composition::operator_secret_store`: repeatability is a
    // property of the lease protocol, which now lives behind the port, and a
    // fake asserting its own repeatability would prove nothing.

    /// Through the caller: every operation fails closed when the port errors,
    /// and the substrate's classification reaches the caller unchanged so a
    /// log line can name it.
    #[tokio::test]
    async fn store_failures_surface_as_store_errors_with_the_stable_reason() {
        let keys = LlmKeyStore::new(Arc::new(FakeOperatorSecretValueStore::failing(
            "BackendUnavailable",
        )));

        for (error, expected_operation) in [
            keys.put("acme", SecretString::from("v"))
                .await
                .expect_err("put"),
            keys.exists("acme").await.expect_err("exists"),
            keys.stored_provider_ids().await.expect_err("stored ids"),
            keys.read("acme").await.expect_err("read"),
            keys.delete("acme").await.expect_err("delete"),
        ]
        // Paired with the port call each element above made, in order. The
        // operation name is the point of the struct variant: before it, all
        // five collapsed into one indistinguishable error.
        .into_iter()
        .zip(["put", "contains", "handles", "read", "delete"])
        {
            match error {
                LlmKeyStoreError::Store { operation, source } => {
                    assert_eq!(source.stable_reason(), "BackendUnavailable");
                    assert_eq!(
                        operation, expected_operation,
                        "store failure must name the port call it came from"
                    );
                }
                other => panic!("expected a store error, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn delete_removes_key() {
        let keys = store();
        keys.put("acme", SecretString::from("v"))
            .await
            .expect("put");
        assert!(keys.delete("acme").await.expect("delete"));
        assert!(!keys.exists("acme").await.expect("exists"));
        assert!(!keys.delete("acme").await.expect("delete again"));
    }

    #[tokio::test]
    async fn stored_provider_ids_ignores_unrelated_secret_handles() {
        let store = Arc::new(FakeOperatorSecretValueStore::new());
        store
            .put(
                &SecretHandle::new("not_an_llm_key").expect("handle"),
                SecretString::from("unrelated"),
            )
            .await
            .expect("put unrelated");
        let keys = LlmKeyStore::new(store);
        keys.put("openai", SecretString::from("sk-openai"))
            .await
            .expect("put openai");

        assert_eq!(
            keys.stored_provider_ids()
                .await
                .expect("stored provider ids"),
            HashSet::from(["openai".to_string()])
        );
    }

    #[tokio::test]
    async fn unknown_provider_id_is_rejected() {
        let keys = store();
        let err = keys
            .put("bad id!", SecretString::from("v"))
            .await
            .expect_err("must reject");
        assert!(matches!(err, LlmKeyStoreError::InvalidProviderId { .. }));
    }
}
