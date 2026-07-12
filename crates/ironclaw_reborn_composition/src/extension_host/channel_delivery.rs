//! Composition wiring for the generic delivery coordinator (§5.4): the
//! snapshot-backed channel resolver (generation-pinned adapter + policy
//! egress from ONE snapshot read) and the ingress `reply_context` read half
//! (ING-11). All delivery semantics live in
//! `ironclaw_product_workflow::DeliveryCoordinator`; this module only
//! implements its ports over the extension host.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_host::SnapshotWatch;
use ironclaw_extension_host::egress::{
    ChannelEgressTransport, DeclaredChannelEgress, PolicyEnforcedChannelEgress,
};
use ironclaw_extension_host::ingress::{ReplyContextKey, ReplyContextStore};
use ironclaw_product_workflow::{
    ChannelDeliveryResolver, DeliveryReplyContextSource, ResolvedChannelDelivery,
};

/// Resolves one extension's delivery half from the active snapshot: the
/// bound channel adapter plus a policy-enforced egress built from the SAME
/// snapshot read, so an in-flight delivery survives an upgrade on the `Arc`s
/// it resolved.
pub(crate) struct SnapshotChannelDeliveryResolver {
    watch: SnapshotWatch,
    transport: Arc<dyn ChannelEgressTransport>,
}

impl SnapshotChannelDeliveryResolver {
    pub(crate) fn new(watch: SnapshotWatch, transport: Arc<dyn ChannelEgressTransport>) -> Self {
        Self { watch, transport }
    }
}

impl ChannelDeliveryResolver for SnapshotChannelDeliveryResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
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
    store: Arc<dyn ReplyContextStore>,
}

impl IngressReplyContextSource {
    pub(crate) fn new(store: Arc<dyn ReplyContextStore>) -> Self {
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
