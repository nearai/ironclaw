//! Deployment-owned channel bindings.
//!
//! Channel ingress is deployment infrastructure: an operator can configure a
//! manifest-declared channel before any user installs the extension.  This
//! registry therefore stays deliberately separate from [`crate::ActiveSnapshot`],
//! which remains the user installation/tool activation projection.

use std::collections::BTreeMap;
use std::sync::Arc;

use ironclaw_extensions::ResolvedExtensionManifest;
use ironclaw_product::ChannelAdapter;

/// One manifest-declared channel paired with the adapter linked by the
/// assembling binary.
pub struct DeploymentChannelBinding {
    pub extension_id: String,
    pub resolved: Arc<ResolvedExtensionManifest>,
    pub adapter: Arc<dyn ChannelAdapter>,
}

impl DeploymentChannelBinding {
    pub fn new(
        resolved: Arc<ResolvedExtensionManifest>,
        adapter: Arc<dyn ChannelAdapter>,
    ) -> Result<Self, DeploymentChannelRegistryError> {
        let extension_id = resolved.id.as_str().to_string();
        let Some(channel) = resolved.channel.as_ref() else {
            return Err(DeploymentChannelRegistryError::MissingChannel { extension_id });
        };
        if !channel.inbound || channel.ingress.is_none() {
            return Err(DeploymentChannelRegistryError::MissingInboundIngress { extension_id });
        }
        Ok(Self {
            extension_id,
            resolved,
            adapter,
        })
    }
}

/// Immutable deployment channel set. It is assembled once from catalog data
/// and binary-linked adapters; no install or activation transition mutates it.
#[derive(Default)]
pub struct DeploymentChannelRegistry {
    bindings: BTreeMap<String, Arc<DeploymentChannelBinding>>,
}

impl DeploymentChannelRegistry {
    pub fn try_new(
        bindings: impl IntoIterator<Item = DeploymentChannelBinding>,
    ) -> Result<Self, DeploymentChannelRegistryError> {
        let mut by_id = BTreeMap::new();
        for binding in bindings {
            let extension_id = binding.extension_id.clone();
            if by_id
                .insert(extension_id.clone(), Arc::new(binding))
                .is_some()
            {
                return Err(DeploymentChannelRegistryError::DuplicateExtension { extension_id });
            }
        }
        Ok(Self { bindings: by_id })
    }

    pub fn extension(&self, extension_id: &str) -> Option<Arc<DeploymentChannelBinding>> {
        self.bindings.get(extension_id).cloned()
    }

    pub fn extension_ids(&self) -> Vec<String> {
        self.bindings.keys().cloned().collect()
    }

    pub fn resolve_channel_ingress(
        &self,
        extension_id: &str,
        route_suffix: &str,
    ) -> Option<Arc<DeploymentChannelBinding>> {
        let binding = self.bindings.get(extension_id)?;
        let channel = binding.resolved.channel.as_ref()?;
        let ingress = channel.ingress.as_ref()?;
        if !channel.inbound || ingress.route_suffix.as_str() != route_suffix {
            return None;
        }
        Some(Arc::clone(binding))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeploymentChannelRegistryError {
    #[error("extension `{extension_id}` does not declare a channel")]
    MissingChannel { extension_id: String },
    #[error("extension `{extension_id}` does not declare inbound channel ingress")]
    MissingInboundIngress { extension_id: String },
    #[error("deployment channel `{extension_id}` is bound more than once")]
    DuplicateExtension { extension_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_manifest_route_without_an_active_installation() {
        let manifest = Arc::new(crate::test_support::channel_only_manifest());
        let registry = DeploymentChannelRegistry::try_new([DeploymentChannelBinding::new(
            Arc::clone(&manifest),
            Arc::new(crate::test_support::FakeChannelAdapter::default()),
        )
        .expect("channel binding validates")])
        .expect("deployment registry validates");

        let resolved = registry
            .resolve_channel_ingress("acme-chat", "events")
            .expect("manifest route resolves");
        assert_eq!(resolved.resolved.id, manifest.id);
        assert!(
            registry
                .resolve_channel_ingress("acme-chat", "wrong")
                .is_none()
        );
    }

    #[test]
    fn duplicate_extension_bindings_fail_closed() {
        let manifest = Arc::new(crate::test_support::channel_only_manifest());
        let binding = || {
            DeploymentChannelBinding::new(
                Arc::clone(&manifest),
                Arc::new(crate::test_support::FakeChannelAdapter::default()),
            )
            .expect("channel binding validates")
        };

        assert!(matches!(
            DeploymentChannelRegistry::try_new([binding(), binding()]),
            Err(DeploymentChannelRegistryError::DuplicateExtension { extension_id })
                if extension_id == "acme-chat"
        ));
    }
}
