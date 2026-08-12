//! Narrow initialization seam for binary-linked first-party channels.
//!
//! Composition supplies shared credential storage and collects the resulting
//! non-secret client bootstrap document. The binding owns every
//! extension-specific decision: credential handles, material shape, and
//! bootstrap shape never enter generic composition.

use std::collections::BTreeMap;

use async_trait::async_trait;
use ironclaw_host_api::{
    ids::{ExtensionId, SecretHandle},
    resource::ResourceScope,
};
use ironclaw_secrets::{SecretMaterial, SecretStorePort};

use crate::input::ChannelExtensionBinding;

/// Shared host resources available to a binary-linked channel initializer.
#[derive(Clone)]
pub struct FirstPartyChannelInitializationContext<'a> {
    secret_store: &'a dyn SecretStorePort,
    credential_scope: ResourceScope,
}

impl<'a> FirstPartyChannelInitializationContext<'a> {
    pub(crate) fn new(
        secret_store: &'a dyn SecretStorePort,
        credential_scope: ResourceScope,
    ) -> Self {
        Self {
            secret_store,
            credential_scope,
        }
    }

    /// Store extension-owned credential material only when the handle is
    /// absent. The secret store arbitrates concurrent replica initialization.
    pub async fn store_credential_if_absent(
        &self,
        handle: SecretHandle,
        material: String,
    ) -> Result<bool, FirstPartyChannelInitializationError> {
        self.secret_store
            .put_if_absent(
                self.credential_scope.clone(),
                handle,
                SecretMaterial::from(material),
                None,
            )
            .await
            .map_err(|error| {
                FirstPartyChannelInitializationError::failed(format!(
                    "credential storage failed: {}",
                    error.stable_reason()
                ))
            })
    }

    /// Read extension-owned credential material through the one-shot lease
    /// protocol. The caller must keep the returned value secret.
    pub async fn read_credential_once(
        &self,
        handle: &SecretHandle,
    ) -> Result<secrecy::SecretString, FirstPartyChannelInitializationError> {
        let lease = self
            .secret_store
            .lease_once(&self.credential_scope, handle)
            .await
            .map_err(|error| {
                FirstPartyChannelInitializationError::failed(format!(
                    "credential lease failed: {}",
                    error.stable_reason()
                ))
            })?;
        self.secret_store
            .consume(&self.credential_scope, lease.id)
            .await
            .map_err(|error| {
                FirstPartyChannelInitializationError::failed(format!(
                    "credential read failed: {}",
                    error.stable_reason()
                ))
            })
    }
}

/// One binary-linked channel's optional startup initialization.
#[async_trait]
pub trait FirstPartyChannelInitializer: Send + Sync {
    /// Initialize extension-owned state and return the optional non-secret
    /// client bootstrap document published by notification setup.
    async fn initialize(
        &self,
        context: &FirstPartyChannelInitializationContext<'_>,
    ) -> Result<Option<serde_json::Value>, FirstPartyChannelInitializationError>;
}

/// Sanitized startup failure from a first-party channel initializer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("first-party channel initialization failed: {reason}")]
pub struct FirstPartyChannelInitializationError {
    reason: String,
}

impl FirstPartyChannelInitializationError {
    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Bootstrap documents produced by successfully initialized channel bindings.
#[derive(Debug, Default)]
pub(crate) struct InitializedChannelBootstraps {
    documents: BTreeMap<ExtensionId, serde_json::Value>,
}

impl ironclaw_assistant::DeliveryClientBootstrap for InitializedChannelBootstraps {
    fn bootstrap(
        &self,
        extension_id: &str,
    ) -> Result<Option<serde_json::Value>, ironclaw_assistant::DeliveryClientBootstrapError> {
        let extension_id = ExtensionId::new(extension_id)
            .map_err(|_| ironclaw_assistant::DeliveryClientBootstrapError)?;
        Ok(self.documents.get(&extension_id).cloned())
    }
}

pub(crate) async fn initialize_first_party_channels(
    bindings: &[ChannelExtensionBinding],
    secret_store: &dyn SecretStorePort,
    credential_scope: ResourceScope,
) -> Result<InitializedChannelBootstraps, FirstPartyChannelInitializationError> {
    let context = FirstPartyChannelInitializationContext::new(secret_store, credential_scope);
    let mut documents = BTreeMap::new();
    for binding in bindings {
        let Some(initializer) = binding.first_party_initializer.as_ref() else {
            continue;
        };
        let bootstrap = initializer.initialize(&context).await.map_err(|error| {
            FirstPartyChannelInitializationError::failed(format!(
                "initializer for extension `{}` failed: {error}",
                binding.extension_id
            ))
        })?;
        if let Some(bootstrap) = bootstrap {
            documents.insert(binding.extension_id.clone(), bootstrap);
        }
    }
    Ok(InitializedChannelBootstraps { documents })
}

#[cfg(test)]
mod tests;
