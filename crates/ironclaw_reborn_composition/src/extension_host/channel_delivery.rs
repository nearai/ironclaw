//! Composition wiring for the generic delivery coordinator (§5.4): the
//! deployment-first channel resolver (with an active-snapshot compatibility
//! fallback) and the ingress `reply_context` read half
//! (ING-11). All delivery semantics live in
//! `ironclaw_product::DeliveryCoordinator`; this module only
//! implements its ports over the extension host.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_host::egress::{
    ChannelEgressTransport, DeclaredChannelEgress, PolicyEnforcedChannelEgress,
};
use ironclaw_extension_host::ingress::{ReplyContextKey, ReplyContextStorePort};
use ironclaw_extension_host::{DeploymentChannelRegistry, SnapshotWatch};
use ironclaw_product::{
    ChannelDeliveryResolver, DeliveryReplyContextSource, ResolvedChannelDelivery,
};

/// Resolves one extension's delivery half from its deployment binding, or
/// from the active snapshot for compatibility with dynamically supplied
/// channels. Both paths return owned `Arc`s so in-flight delivery survives a
/// later registry or snapshot change.
pub(crate) struct SnapshotChannelDeliveryResolver {
    watch: SnapshotWatch,
    deployment_channels: Arc<DeploymentChannelRegistry>,
    transport: Arc<dyn ChannelEgressTransport>,
}

impl SnapshotChannelDeliveryResolver {
    pub(crate) fn new(watch: SnapshotWatch, transport: Arc<dyn ChannelEgressTransport>) -> Self {
        Self {
            watch,
            deployment_channels: Arc::new(DeploymentChannelRegistry::default()),
            transport,
        }
    }

    pub(crate) fn with_deployment_channels(
        mut self,
        deployment_channels: Arc<DeploymentChannelRegistry>,
    ) -> Self {
        self.deployment_channels = deployment_channels;
        self
    }
}

impl ChannelDeliveryResolver for SnapshotChannelDeliveryResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        if let Some(extension) = self.deployment_channels.extension(extension_id) {
            let declared: Vec<DeclaredChannelEgress> = extension
                .resolved
                .channel
                .as_ref()
                .map(|channel| {
                    channel
                        .egress
                        .iter()
                        .map(DeclaredChannelEgress::from_descriptor)
                        .collect()
                })
                .unwrap_or_default();
            let egress = Arc::new(PolicyEnforcedChannelEgress::new(
                extension.extension_id.clone(),
                extension.extension_id.clone(),
                declared,
                Arc::clone(&self.transport),
            ));
            return Some(ResolvedChannelDelivery {
                extension_id: extension.extension_id.clone(),
                installation_id: extension.extension_id.clone(),
                adapter: Arc::clone(&extension.adapter),
                egress,
            });
        }
        let snapshot = self.watch.current();
        let extension = snapshot.extension(extension_id)?;
        let adapter = extension.channel.clone()?;
        let declared: Vec<DeclaredChannelEgress> = extension
            .resolved
            .channel
            .as_ref()
            .map(|channel| {
                channel
                    .egress
                    .iter()
                    .map(DeclaredChannelEgress::from_descriptor)
                    .collect()
            })
            .unwrap_or_default();
        let egress = Arc::new(PolicyEnforcedChannelEgress::new(
            extension.extension_id.clone(),
            extension.installation_id.clone(),
            declared,
            Arc::clone(&self.transport),
        ));
        Some(ResolvedChannelDelivery {
            extension_id: extension.extension_id.clone(),
            installation_id: extension.installation_id.clone(),
            adapter,
            egress,
        })
    }
}

/// The delivery-time read half of the ingress router's `reply_context`
/// storage: the opaque vendor context an adapter attached to the originating
/// inbound message, keyed by conversation fingerprint.
pub(crate) struct IngressReplyContextSource {
    store: Arc<dyn ReplyContextStorePort>,
}

impl IngressReplyContextSource {
    pub(crate) fn new(store: Arc<dyn ReplyContextStorePort>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl DeliveryReplyContextSource for IngressReplyContextSource {
    async fn reply_context(
        &self,
        extension_id: &str,
        installation_id: &str,
        conversation_fingerprint: &str,
    ) -> Option<Vec<u8>> {
        self.store
            .get(&ReplyContextKey {
                extension_id: extension_id.to_string(),
                installation_id: installation_id.to_string(),
                conversation: conversation_fingerprint.to_string(),
            })
            .await
            .ok()
            .flatten()
    }
}
