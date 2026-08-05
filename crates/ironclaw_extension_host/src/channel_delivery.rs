//! Composition wiring for the generic delivery coordinator (§5.4): the
//! deployment-first channel resolver (with an active-snapshot compatibility
//! fallback) and the ingress `reply_context` read half
//! (ING-11). All delivery semantics live in
//! `ironclaw_assistant::DeliveryCoordinator`; this module only
//! implements its ports over the extension host.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::product_adapter::AdapterInstallationId;
use ironclaw_product_contracts::delivery::{
    ChannelDeliveryResolver, DeliveryReplyContextSource, ResolvedChannelDelivery,
};

use crate::egress::{ChannelEgressTransport, DeclaredChannelEgress, PolicyEnforcedChannelEgress};
use crate::ingress::{ReplyContextKey, ReplyContextStore};
use crate::{DeploymentChannelRegistry, SnapshotWatch};

/// Resolves one extension's delivery half from its deployment binding, or
/// from the active snapshot for compatibility with dynamically supplied
/// channels. Both paths return owned `Arc`s so in-flight delivery survives a
/// later registry or snapshot change.
pub struct SnapshotChannelDeliveryResolver {
    watch: SnapshotWatch,
    deployment_channels: Arc<DeploymentChannelRegistry>,
    transport: Arc<dyn ChannelEgressTransport>,
}

impl SnapshotChannelDeliveryResolver {
    pub fn new(watch: SnapshotWatch, transport: Arc<dyn ChannelEgressTransport>) -> Self {
        Self {
            watch,
            deployment_channels: Arc::new(DeploymentChannelRegistry::default()),
            transport,
        }
    }

    pub fn with_deployment_channels(
        mut self,
        deployment_channels: Arc<DeploymentChannelRegistry>,
    ) -> Self {
        self.deployment_channels = deployment_channels;
        self
    }
}

/// Both identity fields of `ResolvedChannelDelivery` are typed, and the
/// snapshot/registry rows they come from are still bare strings, so the
/// conversion happens here. It is deliberately **fail-closed**: an id the
/// boundary vocabulary rejects resolves to "no such channel" — the same
/// `None` a deactivated extension produces — rather than being force-fit into
/// an envelope that would then address the wrong installation.
fn delivery_identity(
    extension_id: &str,
    installation_id: &str,
) -> Option<(ExtensionId, AdapterInstallationId)> {
    match (
        ExtensionId::new(extension_id),
        AdapterInstallationId::new(installation_id),
    ) {
        (Ok(extension), Ok(installation)) => Some((extension, installation)),
        _ => {
            tracing::debug!(
                target: "ironclaw::reborn::channel_delivery",
                extension_id,
                "channel delivery identity is not boundary vocabulary; channel unresolved"
            );
            None
        }
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
            // A deployment-bound channel has no installation record, so its
            // extension id *is* its installation id. With distinct newtypes
            // that reuse has to be spelled out, which is the point: it used to
            // be indistinguishable from a copy-paste of the line above.
            let (extension_id, installation_id) =
                delivery_identity(&extension.extension_id, &extension.extension_id)?;
            return Some(ResolvedChannelDelivery {
                extension_id,
                installation_id,
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
        let (extension_id, installation_id) =
            delivery_identity(&extension.extension_id, &extension.installation_id)?;
        Some(ResolvedChannelDelivery {
            extension_id,
            installation_id,
            adapter,
            egress,
        })
    }
}

/// The delivery-time read half of the ingress router's `reply_context`
/// storage: the opaque vendor context an adapter attached to the originating
/// inbound message, keyed by conversation fingerprint.
pub struct IngressReplyContextSource {
    store: Arc<dyn ReplyContextStore>,
}

impl IngressReplyContextSource {
    pub fn new(store: Arc<dyn ReplyContextStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl DeliveryReplyContextSource for IngressReplyContextSource {
    async fn reply_context(
        &self,
        extension_id: &ExtensionId,
        installation_id: &AdapterInstallationId,
        conversation_fingerprint: &str,
    ) -> Option<Vec<u8>> {
        self.store
            .get(&ReplyContextKey {
                extension_id: extension_id.as_str().to_string(),
                installation_id: installation_id.as_str().to_string(),
                conversation: conversation_fingerprint.to_string(),
            })
            .await
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct SharedLogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    struct SharedLogWriterGuard(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for SharedLogWriterGuard {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogWriter {
        type Writer = SharedLogWriterGuard;

        fn make_writer(&'a self) -> Self::Writer {
            SharedLogWriterGuard(std::sync::Arc::clone(&self.0))
        }
    }

    impl SharedLogWriter {
        fn contents(&self) -> String {
            String::from_utf8(
                self.0
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone(),
            )
            .expect("tracing output is UTF-8")
        }
    }

    /// The only way an unusable id reaches this seam is a durable
    /// installation record that predates — or violates — the boundary
    /// vocabulary, which is exactly when guessing is worst: the alternative to
    /// refusing is addressing an envelope to an identity nothing validated.
    ///
    /// Refusing is also **silent to the caller** — it is the same `None` a
    /// deactivated channel produces — so the debug event is the only thing
    /// that tells an operator "this channel has a corrupted installation row"
    /// apart from "this channel is not installed". A subscriber has to be
    /// installed for that half to mean anything: `tracing` short-circuits on
    /// the null dispatcher, so without one the macro body never runs and the
    /// test could not tell "logged it" from "dropped it". Scoped with
    /// `with_default` so parallel tests are unaffected.
    #[test]
    fn delivery_identity_refuses_ids_the_boundary_vocabulary_rejects_and_says_which() {
        let (extension, installation) =
            delivery_identity("slack", "inst-1").expect("both ids are valid vocabulary");
        assert_eq!(extension.as_str(), "slack");
        assert_eq!(installation.as_str(), "inst-1");

        let logs = SharedLogWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(logs.clone())
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            // `ExtensionId` is a name segment: uppercase and spaces are out.
            assert!(delivery_identity("Slack Channel", "inst-1").is_none());
            // `AdapterInstallationId` is permissive but not empty.
            assert!(delivery_identity("slack", "").is_none());
            // Both bad is still one refusal, not a partial resolve.
            assert!(delivery_identity("", "").is_none());
        });

        let rendered = logs.contents();
        assert_eq!(
            rendered.lines().count(),
            3,
            "every refusal is announced, not just the first: {rendered}"
        );
        assert!(
            rendered.contains("Slack Channel"),
            "the event must name the id it rejected, or an operator cannot tell a \
             corrupted installation row from an uninstalled channel: {rendered}"
        );
    }
}
