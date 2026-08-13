//! Deployment-owned channel bindings.
//!
//! Channel ingress is deployment infrastructure: an operator can configure a
//! manifest-declared channel before any user installs the extension.  This
//! registry therefore stays deliberately separate from [`crate::ActiveSnapshot`],
//! which remains the user installation/tool activation projection.

use std::collections::BTreeMap;
use std::sync::Arc;

use ironclaw_extension_contracts::channel_adapter::ChannelSurfaces;
use ironclaw_extension_registry::ResolvedExtensionManifest;

/// One manifest-declared channel paired with the adapter linked by the
/// assembling binary.
pub struct DeploymentChannelBinding {
    pub extension_id: String,
    pub resolved: Arc<ResolvedExtensionManifest>,
    pub surfaces: ChannelSurfaces,
}

impl DeploymentChannelBinding {
    pub fn new(
        resolved: Arc<ResolvedExtensionManifest>,
        surfaces: ChannelSurfaces,
    ) -> Result<Self, DeploymentChannelRegistryError> {
        let extension_id = resolved.id.as_str().to_string();
        let Some(channel) = resolved.channel.as_ref() else {
            return Err(DeploymentChannelRegistryError::MissingChannel { extension_id });
        };
        // Presence of `[channel.ingress]` IS the inbound declaration now, so
        // the old "claims inbound but declares no ingress" contradiction is
        // unrepresentable. What remains worth checking is that the binding
        // does *something*: an outbound-only channel (browser push) is a
        // legitimate deployment binding with nothing to mount — delivery
        // resolution still needs its egress declarations — but a channel that
        // neither receives nor emits is a manifest mistake.
        if !channel.supports_inbound() && !channel.supports_outbound() {
            return Err(DeploymentChannelRegistryError::MissingInboundIngress { extension_id });
        }
        crate::entrypoint::check_channel_halves(channel, &surfaces).map_err(|source| {
            DeploymentChannelRegistryError::InvalidChannelBinding {
                extension_id: extension_id.clone(),
                source,
            }
        })?;
        Ok(Self {
            extension_id,
            resolved,
            surfaces,
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
        // authenticated_session ingress carries no route_suffix and never
        // matches a mounted webhook route — fail closed.
        let declared_suffix = ingress.route_suffix.as_ref()?;
        if !channel.supports_inbound() || declared_suffix.as_str() != route_suffix {
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
    #[error("deployment channel `{extension_id}` has an invalid axis binding: {source}")]
    InvalidChannelBinding {
        extension_id: String,
        #[source]
        source: crate::entrypoint::BindError,
    },
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
            crate::test_support::FakeChannelAdapter::all_halves(),
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
    fn outbound_only_channel_binds_without_ingress_and_never_resolves_ingress() {
        let manifest = Arc::new(crate::test_support::outbound_only_channel_manifest());
        let registry = DeploymentChannelRegistry::try_new([DeploymentChannelBinding::new(
            Arc::clone(&manifest),
            crate::test_support::FakeChannelAdapter::delivery_only(),
        )
        .expect("an outbound-only channel is a legitimate deployment binding")])
        .expect("deployment registry validates");

        let binding = registry
            .extension("acme-push")
            .expect("outbound-only binding resolves for delivery");
        assert_eq!(binding.resolved.id, manifest.id);
        assert!(
            registry
                .resolve_channel_ingress("acme-push", "events")
                .is_none(),
            "an outbound-only channel mounts no ingress route"
        );
    }

    #[test]
    fn deployment_binding_rejects_a_declared_axis_without_its_half() {
        let manifest = Arc::new(crate::test_support::outbound_only_channel_manifest());
        assert!(
            DeploymentChannelBinding::new(manifest, ChannelSurfaces::default()).is_err(),
            "[channel.delivery] must not enter the deployment registry without ChannelDelivery"
        );
    }

    #[test]
    fn duplicate_extension_bindings_fail_closed() {
        let manifest = Arc::new(crate::test_support::channel_only_manifest());
        let binding = || {
            DeploymentChannelBinding::new(
                Arc::clone(&manifest),
                crate::test_support::FakeChannelAdapter::all_halves(),
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
