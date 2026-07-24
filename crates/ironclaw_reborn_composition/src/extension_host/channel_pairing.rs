//! Composition adapters for product-workflow-owned channel pairing.
//!
//! The pairing state machine, durable completion outbox, manifest-declared
//! inbound command interpretation, and retry policy live in
//! `ironclaw_product`. This module only adapts composition-owned
//! installation, administrator-configuration, identity, and direct-target
//! stores into those product ports.

use std::sync::Arc;

#[cfg(test)]
use std::collections::BTreeMap;

use async_trait::async_trait;
#[cfg(test)]
use ironclaw_auth::{AuthContinuationEvent, AuthContinuationRef, AuthFlowId};
#[cfg(test)]
use ironclaw_conversations::{
    AdapterKind, ConversationActorPairingService, ExpectedExternalActorOwner,
};
use ironclaw_extensions::ExtensionInstallationStore;
#[cfg(test)]
use ironclaw_filesystem::RootFilesystem;
#[cfg(test)]
use ironclaw_host_api::TenantId;
use ironclaw_host_api::{ExtensionId, UserId};
use ironclaw_product::AdapterInstallationId;

use ironclaw_product::{
    ChannelPairingDirectTargetStore, ChannelPairingIdentityBindOutcome,
    ChannelPairingIdentityStore, ChannelPairingInstallationSource, ChannelPairingTemplateValues,
};

use crate::extension_host::channel_dm_targets::FilesystemChannelDmTargetStore;
#[cfg(test)]
use crate::product_auth::api::auth::RebornAuthContinuationDispatcher;
use crate::provider_identity::{
    RebornIdentityProviderId, RebornIdentityProviderUserId, RebornUserIdentityBinding,
    RebornUserIdentityBindingDeleteStore, RebornUserIdentityBindingError,
    RebornUserIdentityBindingStore, RebornUserIdentityLookup, installation_scoped_provider_user_id,
};

/// Pairing installation lookup over the durable lifecycle store. Pairing is
/// setup work performed after install and before readiness, so the active-host
/// snapshot is intentionally too narrow for this adapter.
pub(crate) struct StoredPairingInstallationSource {
    store: Arc<dyn ExtensionInstallationStore>,
    extension_id: ExtensionId,
}

impl StoredPairingInstallationSource {
    pub(crate) fn new(
        store: Arc<dyn ExtensionInstallationStore>,
        extension_id: ExtensionId,
    ) -> Self {
        Self {
            store,
            extension_id,
        }
    }
}

#[async_trait]
impl ChannelPairingInstallationSource for StoredPairingInstallationSource {
    async fn current_installation(
        &self,
        caller: &UserId,
    ) -> Result<Option<AdapterInstallationId>, String> {
        let installation = self
            .store
            .list_installations()
            .await
            .map_err(|error| format!("installation lookup failed: {error}"))?
            .into_iter()
            .find(|installation| {
                installation.extension_id() == &self.extension_id
                    && installation.owner().visible_to(caller)
            });
        installation
            .map(|installation| AdapterInstallationId::new(installation.installation_id().as_str()))
            .transpose()
            .map_err(|error| format!("installed installation id invalid: {error}"))
    }
}

/// Pairing deep-link values over the extension's saved non-secret
/// administrator configuration. The manifest template determines which
/// values are requested; composition does not name a provider or field.
pub(crate) struct AdminConfigurationPairingTemplateValues {
    admin_configuration_resolver: Arc<
        crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver,
    >,
    extension_id: ExtensionId,
}

impl AdminConfigurationPairingTemplateValues {
    pub(crate) fn new(
        admin_configuration_resolver: Arc<
            crate::extension_host::admin_configuration::ComposedExtensionAdminConfigurationResolver,
        >,
        extension_id: ExtensionId,
    ) -> Self {
        Self {
            admin_configuration_resolver,
            extension_id,
        }
    }
}

#[async_trait]
impl ChannelPairingTemplateValues for AdminConfigurationPairingTemplateValues {
    async fn template_value(&self, handle: &str) -> Result<Option<String>, String> {
        self.admin_configuration_resolver
            .non_secret_value(&self.extension_id, handle)
            .await
            .map_err(|error| error.to_string())
    }
}

/// Mechanical adapter from the existing host identity stores to the
/// provider-neutral product pairing port.
pub(crate) struct ComposedChannelPairingIdentityStore {
    bind: Arc<dyn RebornUserIdentityBindingStore>,
    lookup: Arc<dyn RebornUserIdentityLookup>,
    delete: Arc<dyn RebornUserIdentityBindingDeleteStore>,
}

impl ComposedChannelPairingIdentityStore {
    pub(crate) fn new(
        bind: Arc<dyn RebornUserIdentityBindingStore>,
        lookup: Arc<dyn RebornUserIdentityLookup>,
        delete: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    ) -> Self {
        Self {
            bind,
            lookup,
            delete,
        }
    }
}

#[async_trait]
impl ChannelPairingIdentityStore for ComposedChannelPairingIdentityStore {
    fn binding_key(
        &self,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
    ) -> String {
        installation_scoped_provider_user_id(installation_id, external_actor_id)
    }

    async fn resolve_user(
        &self,
        extension_id: &ExtensionId,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
    ) -> Result<Option<UserId>, String> {
        let provider_user_id =
            installation_scoped_provider_user_id(installation_id, external_actor_id);
        self.lookup
            .resolve_user_identity(extension_id.as_str(), &provider_user_id)
            .await
            .map_err(|error| error.to_string())
    }

    async fn bind_user(
        &self,
        extension_id: &ExtensionId,
        installation_id: &AdapterInstallationId,
        external_actor_id: &str,
        user_id: UserId,
    ) -> Result<ChannelPairingIdentityBindOutcome, String> {
        let provider = RebornIdentityProviderId::new(extension_id.as_str())
            .map_err(|error| error.to_string())?;
        let provider_user_id = RebornIdentityProviderUserId::new(
            installation_scoped_provider_user_id(installation_id, external_actor_id),
        )
        .map_err(|error| error.to_string())?;
        match self
            .bind
            .bind_user_identity(RebornUserIdentityBinding {
                provider,
                provider_user_id,
                user_id,
            })
            .await
        {
            Ok(()) => Ok(ChannelPairingIdentityBindOutcome::Bound),
            Err(RebornUserIdentityBindingError::ProviderIdentityAlreadyBound) => {
                Ok(ChannelPairingIdentityBindOutcome::AlreadyBoundToOtherUser)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    async fn delete_user_bindings(
        &self,
        extension_id: &ExtensionId,
        user_id: &UserId,
    ) -> Result<(), String> {
        self.delete
            .delete_user_identity_bindings_for_user(extension_id.as_str(), user_id, None)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// Mechanical adapter from the existing canonical channel DM-target store to
/// the provider-neutral product pairing port.
pub(crate) struct ComposedChannelPairingDirectTargetStore {
    inner: Arc<FilesystemChannelDmTargetStore>,
}

impl ComposedChannelPairingDirectTargetStore {
    pub(crate) fn new(inner: Arc<FilesystemChannelDmTargetStore>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ChannelPairingDirectTargetStore for ComposedChannelPairingDirectTargetStore {
    async fn is_connected(
        &self,
        extension_id: &ExtensionId,
        user_id: &UserId,
    ) -> Result<bool, String> {
        self.inner
            .load(extension_id.as_str(), user_id)
            .await
            .map(|target| target.is_some())
            .map_err(|error| error.to_string())
    }

    async fn upsert(
        &self,
        extension_id: &ExtensionId,
        user_id: &UserId,
        external_actor_id: &str,
        conversation_space_id: Option<&str>,
        conversation_id: &str,
    ) -> Result<(), String> {
        self.inner
            .upsert(
                extension_id.as_str(),
                user_id,
                external_actor_id.to_string(),
                crate::extension_host::channel_dm_targets::dm_target_payload(
                    conversation_space_id,
                    conversation_id,
                ),
            )
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn delete(&self, extension_id: &ExtensionId, user_id: &UserId) -> Result<(), String> {
        self.inner
            .delete(extension_id.as_str(), user_id)
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests;
