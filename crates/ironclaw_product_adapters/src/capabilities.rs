//! Typed adapter capabilities.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Capabilities a product adapter may declare. The workflow uses these to
/// pick safe presentation/delivery defaults — for example, ambient progress
/// pushes are only attempted when the adapter advertises [`ProductCapabilityFlag::ExternalProgressPush`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductCapabilityFlag {
    /// Inbound user messages are supported.
    InboundMessages,
    /// Inbound bot commands are supported.
    InboundCommands,
    /// Inbound message attachments are supported.
    InboundAttachments,
    /// Adapter can push final replies to the external surface.
    ExternalFinalReplyPush,
    /// Adapter can push progress (typing indicators, etc.) to the external
    /// surface.
    ExternalProgressPush,
    /// Adapter can deliver approval/auth gate prompts (deferred until #3094).
    ExternalGatePush,
    /// Adapter consumes projection subscriptions (Web/CLI/API).
    ProjectionSubscription,
    /// Adapter can wait synchronously on a projection (synchronous APIs).
    SynchronousWait,
    /// Adapter reports per-push delivery status to the egress sink.
    DeliveryStatusReporting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProductAdapterCapabilities {
    flags: BTreeSet<ProductCapabilityFlag>,
}

impl ProductAdapterCapabilities {
    pub fn new(flags: impl IntoIterator<Item = ProductCapabilityFlag>) -> Self {
        Self {
            flags: flags.into_iter().collect(),
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn contains(&self, flag: ProductCapabilityFlag) -> bool {
        self.flags.contains(&flag)
    }

    pub fn iter(&self) -> impl Iterator<Item = ProductCapabilityFlag> + '_ {
        self.flags.iter().copied()
    }

    pub fn with(mut self, flag: ProductCapabilityFlag) -> Self {
        self.flags.insert(flag);
        self
    }

    pub fn without(mut self, flag: ProductCapabilityFlag) -> Self {
        self.flags.remove(&flag);
        self
    }

    /// Convenience preset for an external chat channel that reports delivery
    /// status and supports inbound messages, commands, and attachments, plus
    /// final-reply push (no progress push by default — opt-in per #3266).
    pub fn external_channel_default() -> Self {
        Self::new([
            ProductCapabilityFlag::InboundMessages,
            ProductCapabilityFlag::InboundCommands,
            ProductCapabilityFlag::InboundAttachments,
            ProductCapabilityFlag::ExternalFinalReplyPush,
            ProductCapabilityFlag::DeliveryStatusReporting,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_channel_default_excludes_progress_and_gate_push() {
        let caps = ProductAdapterCapabilities::external_channel_default();
        assert!(!caps.contains(ProductCapabilityFlag::ExternalProgressPush));
        assert!(!caps.contains(ProductCapabilityFlag::ExternalGatePush));
        assert!(caps.contains(ProductCapabilityFlag::ExternalFinalReplyPush));
    }

    #[test]
    fn capability_round_trips() {
        let caps = ProductAdapterCapabilities::external_channel_default()
            .with(ProductCapabilityFlag::ExternalProgressPush);
        let json = serde_json::to_string(&caps).expect("serialize");
        let parsed: ProductAdapterCapabilities = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(caps, parsed);
    }
}
