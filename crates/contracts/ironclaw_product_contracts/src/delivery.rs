//! Delivery-resolution ports (PROPOSAL §6.1.3).
//!
//! The outbound delivery coordinator is product-tier *semantics* and stays in
//! `ironclaw_assistant`. What crosses the product boundary is the pair of ports
//! it reads through: "which channel extension is active right now" and "what
//! opaque vendor reply context did that extension attach to the originating
//! inbound message". Both are implemented **below** product by
//! `ironclaw_extension_host`, which owns the active snapshot and the
//! reply-context store — so defining them here is what lets the extension host
//! satisfy the coordinator without depending on it.
//!
//! Never here: the coordinator, delivery attempt persistence, retry policy, or
//! any implementation of these ports.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::channel_adapter::ChannelAdapter;
use ironclaw_extension_contracts::tool_adapter::RestrictedEgress;
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::product_adapter::AdapterInstallationId;

/// One channel's delivery half, resolved from a single active-snapshot read
/// (generation-pinned: an in-flight delivery keeps these `Arc`s across an
/// upgrade).
///
/// The two identifiers are **distinct newtypes on purpose**. They travel
/// together through every delivery hop — the reply-context lookup and the
/// outbound envelope both take them adjacently — and while both were `String`
/// a transposition was a silent mis-delivery to another installation rather
/// than a compile error. `AdapterInstallationId` is the same installation type
/// `ironclaw_extension_contracts::channel_identity` and `preference_target`
/// already speak, so this is the boundary's existing vocabulary, not a new one.
#[derive(Clone)]
pub struct ResolvedChannelDelivery {
    pub extension_id: ExtensionId,
    pub installation_id: AdapterInstallationId,
    pub adapter: Arc<dyn ChannelAdapter>,
    /// Policy-enforced egress built from the same snapshot read.
    pub egress: Arc<dyn RestrictedEgress>,
}

/// Resolver port: the coordinator's view of the active extension set.
/// Defined here (the coordinator is the consumer); implemented over the
/// extension host's snapshot.
pub trait ChannelDeliveryResolver: Send + Sync {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery>;
}

/// Read half of the host-side `reply_context` storage (ING-11): the opaque
/// vendor context an adapter attached to the originating inbound message,
/// handed back at delivery time.
///
/// The two identity arguments are typed for the same reason the fields of
/// [`ResolvedChannelDelivery`] are: they are read straight off that struct and
/// handed here adjacently, so a bare-`&str` triple made a transposition a
/// silent wrong-anchor lookup. The fingerprint stays a `&str` — it is a
/// content hash, not an identity, and with two distinct identity types beside
/// it no pair of the three can be swapped without a compile error.
#[async_trait]
pub trait DeliveryReplyContextSource: Send + Sync {
    async fn reply_context(
        &self,
        extension_id: &ExtensionId,
        installation_id: &AdapterInstallationId,
        conversation_fingerprint: &str,
    ) -> Option<Vec<u8>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use std::sync::Mutex;

    // Every consumer holds these as `Arc<dyn _>`, so dyn-safety is part of the
    // contract, not an implementation detail: a signature change that breaks it
    // fails here rather than at the far-away wiring site.
    static_assertions::assert_obj_safe!(ChannelDeliveryResolver, DeliveryReplyContextSource);

    /// Records the coordinates each port is asked about. The point under test
    /// is the *seam*, not the lookup: both ports key on identifiers the
    /// coordinator passes through. The identity pair is typed, so a
    /// transposition no longer compiles — but the *pass-through* still needs
    /// pinning (nothing in the type system stops an implementation from being
    /// handed one id and looking up another), and the fingerprint is still a
    /// bare string. These pin the order and the pass-through.
    #[derive(Default)]
    struct RecordingResolver {
        resolved: Mutex<Vec<String>>,
        contexts: Mutex<Vec<(String, String, String)>>,
    }

    impl ChannelDeliveryResolver for RecordingResolver {
        fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
            self.resolved
                .lock()
                .expect("lock")
                .push(extension_id.to_string());
            // Absence is expressible without an error on purpose: a channel
            // that is not in the active snapshot is a normal outcome (it was
            // just deactivated, or never installed), not a delivery failure.
            None
        }
    }

    #[async_trait]
    impl DeliveryReplyContextSource for RecordingResolver {
        async fn reply_context(
            &self,
            extension_id: &ExtensionId,
            installation_id: &AdapterInstallationId,
            conversation_fingerprint: &str,
        ) -> Option<Vec<u8>> {
            self.contexts.lock().expect("lock").push((
                extension_id.as_str().to_string(),
                installation_id.as_str().to_string(),
                conversation_fingerprint.to_string(),
            ));
            None
        }
    }

    #[test]
    fn the_resolver_receives_the_extension_id_verbatim_and_may_answer_none() {
        let recorder = Arc::new(RecordingResolver::default());
        let resolver: Arc<dyn ChannelDeliveryResolver> = recorder.clone();

        assert!(resolver.resolve_channel_delivery("slack").is_none());
        assert!(resolver.resolve_channel_delivery("telegram").is_none());

        assert_eq!(
            *recorder.resolved.lock().expect("lock"),
            vec!["slack".to_string(), "telegram".to_string()],
            "the port must hand the implementation the id it was asked about"
        );
    }

    #[tokio::test]
    async fn reply_context_keeps_extension_installation_and_fingerprint_in_order() {
        // The identity pair is typed, so a swap of those two is a compile
        // error rather than a wrong-anchor lookup; this pins that the
        // implementation is handed each one verbatim and in position. `None`
        // (no stored anchor) is also deliberately distinct from `Some(vec![])`
        // (a stored but empty anchor).
        let recorder = Arc::new(RecordingResolver::default());
        let source: Arc<dyn DeliveryReplyContextSource> = recorder.clone();

        let extension_id = ExtensionId::new("slack").expect("valid extension id");
        let installation_id = AdapterInstallationId::new("inst-1").expect("valid installation id");
        assert_eq!(
            source
                .reply_context(&extension_id, &installation_id, "fp-9")
                .await,
            None
        );

        assert_eq!(
            *recorder.contexts.lock().expect("lock"),
            vec![(
                "slack".to_string(),
                "inst-1".to_string(),
                "fp-9".to_string()
            )],
        );
    }
}
