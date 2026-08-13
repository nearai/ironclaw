//! Deployment session-channel directory.
//!
//! Implements the product-declared [`SessionChannelDirectory`] over the
//! deployment channel registry: an extension is a session channel iff its
//! resolved manifest declares an inbound channel whose ingress verification
//! recipe is the authenticated-session entrypoint. Derived from resolved
//! manifests only — no install-state coupling, mirroring how webhook route
//! resolution reads the same registry.

use std::sync::Arc;

use ironclaw_product_contracts::session_ingress::SessionChannelDirectory;

use crate::deployment_channels::DeploymentChannelRegistry;

/// [`SessionChannelDirectory`] over the deployment channel registry.
pub struct DeploymentSessionChannelDirectory {
    registry: Arc<DeploymentChannelRegistry>,
}

impl DeploymentSessionChannelDirectory {
    pub fn new(registry: Arc<DeploymentChannelRegistry>) -> Self {
        Self { registry }
    }
}

impl SessionChannelDirectory for DeploymentSessionChannelDirectory {
    fn is_session_channel(&self, extension_id: &str) -> bool {
        let Some(binding) = self.registry.extension(extension_id) else {
            return false;
        };
        let Some(channel) = binding.resolved.channel.as_ref() else {
            return false;
        };
        let Some(ingress) = channel.ingress.as_ref() else {
            return false;
        };
        channel.supports_inbound() && ingress.verification.is_authenticated_session()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deployment_channels::DeploymentChannelBinding;

    #[test]
    fn only_authenticated_session_channels_resolve() {
        let session_manifest = Arc::new(crate::test_support::session_channel_manifest());
        let webhook_manifest = Arc::new(crate::test_support::channel_only_manifest());
        let registry = Arc::new(
            DeploymentChannelRegistry::try_new([
                DeploymentChannelBinding::new(
                    Arc::clone(&session_manifest),
                    crate::test_support::FakeChannelAdapter::delivery_only(),
                )
                .expect("session channel binding validates"),
                DeploymentChannelBinding::new(
                    Arc::clone(&webhook_manifest),
                    crate::test_support::FakeChannelAdapter::all_halves(),
                )
                .expect("webhook channel binding validates"),
            ])
            .expect("registry validates"),
        );
        let directory = DeploymentSessionChannelDirectory::new(registry);

        assert!(directory.is_session_channel(session_manifest.id.as_str()));
        assert!(
            !directory.is_session_channel(webhook_manifest.id.as_str()),
            "a webhook channel must never admit session submissions"
        );
        assert!(!directory.is_session_channel("unknown-extension"));
    }
}
