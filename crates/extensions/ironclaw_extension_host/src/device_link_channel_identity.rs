//! Transactional channel-identity binding for linked-device auth completion.
//!
//! This module is deliberately vendor-blind. The active manifest supplies the
//! channel provider and strategy; the authenticated adapter supplies the
//! external user reference; the durable auth flow supplies the IronClaw user.

use std::sync::Arc;

use ironclaw_extension_contracts::channel::ChannelConnectionStrategy;
use ironclaw_extension_registry::ResolvedExtensionManifest;
use ironclaw_host_api::{
    ids::UserId,
    product_adapter::AdapterInstallationId,
    user_identity::{
        RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
        RebornUserIdentityLookup,
    },
};

use crate::channel_identity_store::{
    FilesystemChannelIdentityStore, IdentityBindingRollbackReceipt, IdentityBindingTransaction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceLinkChannelIdentityError {
    #[error("device-link channel declaration is inconsistent")]
    InvalidDeclaration,
    #[error("another external identity is already connected for this user")]
    DifferentIdentityConnected,
    #[error("external identity is already connected to another user")]
    IdentityOwnedByAnotherUser,
    #[error("device-link channel identity storage is unavailable")]
    StorageUnavailable,
}

/// Compensation returned only when this completion created a binding.
#[derive(Debug)]
pub struct DeviceLinkChannelIdentityRollback {
    store: Arc<FilesystemChannelIdentityStore>,
    receipt: IdentityBindingRollbackReceipt,
}

impl DeviceLinkChannelIdentityRollback {
    pub async fn rollback(self) -> Result<bool, DeviceLinkChannelIdentityError> {
        self.store
            .rollback_identity_binding(self.receipt)
            .await
            .map_err(|_| DeviceLinkChannelIdentityError::StorageUnavailable)
    }
}

pub struct DeviceLinkChannelIdentityBinder {
    store: Arc<FilesystemChannelIdentityStore>,
}

impl DeviceLinkChannelIdentityBinder {
    pub fn new(store: Arc<FilesystemChannelIdentityStore>) -> Self {
        Self { store }
    }

    async fn rollback_created(
        &self,
        transaction: IdentityBindingTransaction,
    ) -> Result<(), DeviceLinkChannelIdentityError> {
        if let IdentityBindingTransaction::Created(receipt) = transaction {
            self.store
                .rollback_identity_binding(receipt)
                .await
                .map_err(|_| DeviceLinkChannelIdentityError::StorageUnavailable)?;
        }
        Ok(())
    }

    /// Begin the binding transaction when the active channel explicitly
    /// declares `device_link`. Device-link-only extensions and channels with a
    /// different connection strategy retain their existing behavior.
    pub async fn begin(
        &self,
        declaration: &ResolvedExtensionManifest,
        installation_id: &str,
        provider: &str,
        vendor_user_ref: &str,
        user_id: &UserId,
    ) -> Result<Option<DeviceLinkChannelIdentityRollback>, DeviceLinkChannelIdentityError> {
        let Some(connection) = declaration
            .channel
            .as_ref()
            .and_then(|channel| channel.connection.as_ref())
        else {
            tracing::debug!(
                extension_id = %declaration.id,
                "device-link completion has no declared channel connection"
            );
            return Ok(None);
        };
        if connection.strategy != ChannelConnectionStrategy::DeviceLink {
            tracing::debug!(
                extension_id = %declaration.id,
                strategy = ?connection.strategy,
                "device-link completion does not own channel identity for this strategy"
            );
            return Ok(None);
        }
        if connection.provider.as_str() != provider {
            return Err(DeviceLinkChannelIdentityError::InvalidDeclaration);
        }
        let installation = AdapterInstallationId::new(installation_id)
            .map_err(|_| DeviceLinkChannelIdentityError::InvalidDeclaration)?;
        let keyspace = crate::channel_identity::ChannelIdentityKeyspace::DeviceLinkV1;
        let prefix = keyspace.provider_user_id_prefix(&installation);
        let scoped_provider_user_id = keyspace.provider_user_id(&installation, vendor_user_ref);
        let legacy_provider_user_id = crate::channel_identity::ChannelIdentityKeyspace::Legacy
            .provider_user_id(&installation, vendor_user_ref);

        // A retained proof-code binding remains valid for its original bot
        // channel, but it must never be possible to link that same external
        // actor to a different product user under the stronger namespace.
        if self
            .store
            .resolve_user_identity(provider, &legacy_provider_user_id)
            .await
            .map_err(|_| DeviceLinkChannelIdentityError::StorageUnavailable)?
            .is_some_and(|owner| owner != *user_id)
        {
            return Err(DeviceLinkChannelIdentityError::IdentityOwnedByAnotherUser);
        }

        if self
            .store
            .user_has_other_provider_binding(provider, user_id, &prefix, &scoped_provider_user_id)
            .await
            .map_err(|_| DeviceLinkChannelIdentityError::StorageUnavailable)?
        {
            return Err(DeviceLinkChannelIdentityError::DifferentIdentityConnected);
        }

        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new(provider)
                .map_err(|_| DeviceLinkChannelIdentityError::InvalidDeclaration)?,
            provider_user_id: RebornIdentityProviderUserId::new(&scoped_provider_user_id)
                .map_err(|_| DeviceLinkChannelIdentityError::InvalidDeclaration)?,
            user_id: user_id.clone(),
        };
        let transaction = self
            .store
            .bind_user_identity_transactionally(binding)
            .await
            .map_err(|error| match error {
                ironclaw_host_api::user_identity::RebornUserIdentityBindingError::ProviderIdentityAlreadyBound => {
                    DeviceLinkChannelIdentityError::IdentityOwnedByAnotherUser
                }
                _ => DeviceLinkChannelIdentityError::StorageUnavailable,
            })?;

        tracing::debug!(
            extension_id = %declaration.id,
            provider,
            "device-link completion persisted its channel identity"
        );

        let has_other_after_bind = match self
            .store
            .user_has_other_provider_binding(provider, user_id, &prefix, &scoped_provider_user_id)
            .await
        {
            Ok(has_other) => has_other,
            Err(_) => {
                self.rollback_created(transaction).await?;
                return Err(DeviceLinkChannelIdentityError::StorageUnavailable);
            }
        };
        if has_other_after_bind {
            self.rollback_created(transaction).await?;
            return Err(DeviceLinkChannelIdentityError::DifferentIdentityConnected);
        }

        Ok(match transaction {
            IdentityBindingTransaction::Created(receipt) => {
                Some(DeviceLinkChannelIdentityRollback {
                    store: Arc::clone(&self.store),
                    receipt,
                })
            }
            IdentityBindingTransaction::Existing => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::{Fault, FaultInjecting, FilesystemOperation, InMemoryBackend};
    use ironclaw_host_api::{
        ids::{TenantId, UserId},
        product_adapter::AdapterInstallationId,
        user_identity::{
            RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
            RebornUserIdentityBindingStore, RebornUserIdentityLookup,
        },
    };

    use super::*;

    const INSTALLATION: &str = "acme-link-install";

    fn harness() -> (
        DeviceLinkChannelIdentityBinder,
        Arc<FilesystemChannelIdentityStore>,
        ResolvedExtensionManifest,
    ) {
        let store = Arc::new(FilesystemChannelIdentityStore::new(
            Arc::new(InMemoryBackend::new()),
            TenantId::new("tenant-test").expect("tenant"),
            UserId::new("operator").expect("operator"),
        ));
        (
            DeviceLinkChannelIdentityBinder::new(Arc::clone(&store)),
            store,
            crate::test_support::device_link_channel_manifest(),
        )
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("user")
    }

    fn scoped(external: &str) -> String {
        crate::channel_identity::ChannelIdentityKeyspace::DeviceLinkV1.provider_user_id(
            &AdapterInstallationId::new(INSTALLATION).expect("installation"),
            external,
        )
    }

    #[tokio::test]
    async fn new_identity_binds_and_same_identity_relink_adopts_it() {
        let (binder, store, manifest) = harness();
        let rollback = binder
            .begin(&manifest, INSTALLATION, "acme-link", "U123", &user("alice"))
            .await
            .expect("first bind")
            .expect("created binding receipt");
        assert_eq!(
            store
                .resolve_user_identity("acme-link", &scoped("U123"))
                .await
                .expect("resolve"),
            Some(user("alice"))
        );

        assert!(
            binder
                .begin(&manifest, INSTALLATION, "acme-link", "U123", &user("alice"))
                .await
                .expect("same identity relink")
                .is_none(),
            "same-identity relink adopts the binding and needs no rollback"
        );
        assert!(
            !rollback.rollback().await.expect("stale rollback"),
            "an older attempt cannot remove a binding adopted by a later relink"
        );
    }

    #[tokio::test]
    async fn legacy_pairing_identity_gains_a_stronger_device_link_binding() {
        let (binder, store, manifest) = harness();
        store
            .bind_user_identity(RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new("acme-link").expect("provider"),
                provider_user_id: RebornIdentityProviderUserId::new(
                    ironclaw_host_api::user_identity::installation_scoped_provider_user_id(
                        &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                        "U123",
                    ),
                )
                .expect("legacy provider user"),
                user_id: user("alice"),
            })
            .await
            .expect("seed legacy pairing binding");

        binder
            .begin(&manifest, INSTALLATION, "acme-link", "U123", &user("alice"))
            .await
            .expect("device link binds independently")
            .expect("legacy pairing row is not adopted as the new binding");

        let device_link_key = crate::channel_identity::ChannelIdentityKeyspace::DeviceLinkV1
            .provider_user_id(
                &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                "U123",
            );
        assert_eq!(
            store
                .resolve_user_identity("acme-link", &device_link_key)
                .await
                .expect("resolve device-link identity"),
            Some(user("alice"))
        );
        assert_eq!(
            store
                .resolve_user_identity(
                    "acme-link",
                    &crate::channel_identity::ChannelIdentityKeyspace::Legacy.provider_user_id(
                        &AdapterInstallationId::new(INSTALLATION).expect("installation"),
                        "U123",
                    ),
                )
                .await
                .expect("resolve legacy identity"),
            Some(user("alice")),
            "device linking is additive and does not rewrite grandfathered bot authority"
        );
    }

    #[tokio::test]
    async fn legacy_pairing_owned_by_another_user_rejects_device_link() {
        let (binder, store, manifest) = harness();
        let installation = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let legacy_key = crate::channel_identity::ChannelIdentityKeyspace::Legacy
            .provider_user_id(&installation, "U123");
        store
            .bind_user_identity(RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new("acme-link").expect("provider"),
                provider_user_id: RebornIdentityProviderUserId::new(&legacy_key)
                    .expect("legacy provider user"),
                user_id: user("bob"),
            })
            .await
            .expect("seed foreign legacy pairing binding");

        assert_eq!(
            binder
                .begin(&manifest, INSTALLATION, "acme-link", "U123", &user("alice"))
                .await
                .expect_err("device link must not take over a grandfathered actor"),
            DeviceLinkChannelIdentityError::IdentityOwnedByAnotherUser
        );
        assert_eq!(
            store
                .resolve_user_identity("acme-link", &scoped("U123"))
                .await
                .expect("resolve device-link identity"),
            None
        );
    }

    #[tokio::test]
    async fn different_identity_for_the_same_user_is_rejected() {
        let (binder, _store, manifest) = harness();
        binder
            .begin(&manifest, INSTALLATION, "acme-link", "U123", &user("alice"))
            .await
            .expect("first bind");

        assert_eq!(
            binder
                .begin(&manifest, INSTALLATION, "acme-link", "U999", &user("alice"))
                .await
                .expect_err("identity swap must be explicit"),
            DeviceLinkChannelIdentityError::DifferentIdentityConnected
        );
    }

    #[tokio::test]
    async fn identity_owned_by_another_user_is_rejected() {
        let (binder, store, manifest) = harness();
        store
            .bind_user_identity(RebornUserIdentityBinding {
                provider: RebornIdentityProviderId::new("acme-link").expect("provider"),
                provider_user_id: RebornIdentityProviderUserId::new(scoped("U123"))
                    .expect("provider user"),
                user_id: user("bob"),
            })
            .await
            .expect("seed foreign owner");

        assert_eq!(
            binder
                .begin(&manifest, INSTALLATION, "acme-link", "U123", &user("alice"))
                .await
                .expect_err("foreign owner must be preserved"),
            DeviceLinkChannelIdentityError::IdentityOwnedByAnotherUser
        );
        assert_eq!(
            store
                .resolve_user_identity("acme-link", &scoped("U123"))
                .await
                .expect("resolve foreign owner"),
            Some(user("bob"))
        );
    }

    #[tokio::test]
    async fn provider_mismatch_fails_closed_without_a_binding() {
        let (binder, store, manifest) = harness();
        assert_eq!(
            binder
                .begin(
                    &manifest,
                    INSTALLATION,
                    "wrong-provider",
                    "U123",
                    &user("alice")
                )
                .await
                .expect_err("provider mismatch"),
            DeviceLinkChannelIdentityError::InvalidDeclaration
        );
        assert_eq!(
            store
                .resolve_user_identity("acme-link", &scoped("U123"))
                .await
                .expect("resolve after mismatch"),
            None
        );
    }

    #[tokio::test]
    async fn post_bind_conflict_check_failure_rolls_back_the_new_identity() {
        let backend = Arc::new(
            FaultInjecting::new(InMemoryBackend::new()).with_fault(
                Fault::on(FilesystemOperation::ListDir)
                    .path("channel-identities/identities")
                    .nth(2)
                    .backend("post-bind check failed"),
            ),
        );
        let store = Arc::new(FilesystemChannelIdentityStore::new(
            backend,
            TenantId::new("tenant-test").expect("tenant"),
            UserId::new("operator").expect("operator"),
        ));
        let binder = DeviceLinkChannelIdentityBinder::new(Arc::clone(&store));
        let manifest = crate::test_support::device_link_channel_manifest();

        assert_eq!(
            binder
                .begin(&manifest, INSTALLATION, "acme-link", "U123", &user("alice"))
                .await
                .expect_err("failed post-bind verification must fail completion"),
            DeviceLinkChannelIdentityError::StorageUnavailable
        );
        assert_eq!(
            store
                .resolve_user_identity("acme-link", &scoped("U123"))
                .await
                .expect("resolve after compensation"),
            None,
            "a failed post-bind verification must not orphan the new identity"
        );
    }
}
