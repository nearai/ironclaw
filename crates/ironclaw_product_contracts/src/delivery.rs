//! Delivery-resolution ports (PROPOSAL §6.1.3).
//!
//! The outbound delivery coordinator is product-tier *semantics* and stays in
//! `ironclaw_product`. What crosses the product boundary is the pair of ports
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

/// One channel's delivery half, resolved from a single active-snapshot read
/// (generation-pinned: an in-flight delivery keeps these `Arc`s across an
/// upgrade).
#[derive(Clone)]
pub struct ResolvedChannelDelivery {
    pub extension_id: String,
    pub installation_id: String,
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
#[async_trait]
pub trait DeliveryReplyContextSource: Send + Sync {
    async fn reply_context(
        &self,
        extension_id: &str,
        installation_id: &str,
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
    /// coordinator passes through, and both currently carry them as bare
    /// strings, so a transposed argument is a silent mis-delivery rather than
    /// a compile error. These pin the order and the pass-through.
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
            extension_id: &str,
            installation_id: &str,
            conversation_fingerprint: &str,
        ) -> Option<Vec<u8>> {
            self.contexts.lock().expect("lock").push((
                extension_id.to_string(),
                installation_id.to_string(),
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
        // All three are bare strings today, so nothing but this test stops a
        // transposition. `None` (no stored anchor) is also deliberately
        // distinct from `Some(vec![])` (a stored but empty anchor).
        let recorder = Arc::new(RecordingResolver::default());
        let source: Arc<dyn DeliveryReplyContextSource> = recorder.clone();

        assert_eq!(source.reply_context("slack", "inst-1", "fp-9").await, None);

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
