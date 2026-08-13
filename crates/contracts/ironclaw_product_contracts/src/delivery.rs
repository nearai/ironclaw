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
use ironclaw_extension_contracts::channel::ReplyTransport;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelDelivery, ChannelReply, DeliveryRegistration,
};
use ironclaw_extension_contracts::tool_adapter::RestrictedEgress;
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::ids::{TenantId, UserId};
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
    /// The outbound halves this channel implements. Ingress is deliberately
    /// absent from this product-side resolver: outbound orchestration has no
    /// reason to hold or inspect an inbound adapter.
    pub reply: Option<Arc<dyn ChannelReply>>,
    pub delivery: Option<Arc<dyn ChannelDelivery>>,
    /// Policy-enforced egress built from the same snapshot read.
    pub egress: Arc<dyn RestrictedEgress>,
    /// The channel's declared reply transport (`[channel.reply]`). `None`
    /// means the channel has no reply half at all — it can be an out-of-band
    /// delivery target but cannot answer a run's input.
    ///
    /// A `Stream` reply rides the durable projection pipeline and must never
    /// be sent through the coordinator's adapter path. That is a property of
    /// the **route**, not of the content: a delivery to this channel flows
    /// regardless, which is why the gate keys on `OutboundRoute` rather than
    /// on what is being said.
    pub reply_transport: Option<ReplyTransport>,
    /// The delivery enrollment requirement from the same resolved manifest
    /// generation as the surfaces and egress authority above.
    pub requires_enrollment: bool,
    /// Exact hosts admitted for enrollment endpoints, resolved from the same
    /// manifest generation as the enrollment requirement and egress authority.
    pub declared_egress_hosts: Vec<String>,
}

/// Resolver port: the coordinator's view of the active extension set.
/// Defined here (the coordinator is the consumer); implemented over the
/// extension host's snapshot.
pub trait ChannelDeliveryResolver: Send + Sync {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery>;
}

/// The user one delivery registration belongs to. Both halves come from the
/// authenticated caller (enrollment) or the run's scope owner (delivery) —
/// never from a payload or a vendor-supplied conversation id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeliveryRegistrationScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub extension_id: ExtensionId,
}

/// Host-owned per-user delivery registrations (design §8).
///
/// Declared here because the delivery coordinator and the notification-setup
/// product surface are both consumers while the implementation sits below
/// product — the same inversion the two ports above use. The **records** and
/// the persisted store live in `ironclaw_auth`, which is where a per-user,
/// per-vendor grant that a user can revoke from settings already belongs;
/// it is also the only one of the two candidate domains that can already name
/// `ironclaw_extension_contracts`, which the registration view requires.
#[async_trait]
pub trait DeliveryRegistrationService: Send + Sync {
    /// Every registration this user holds on this channel. An empty list is a
    /// resolvable "no target" *before* any adapter call — not a failure
    /// discovered inside the vendor path.
    async fn list(
        &self,
        scope: &DeliveryRegistrationScope,
    ) -> Result<Vec<DeliveryRegistration>, DeliveryRegistrationError>;

    /// Store one registration, replacing any earlier one for the same
    /// endpoint while retaining its opaque host-minted registration id.
    /// `endpoint` is checked against `declared_egress_hosts` **before**
    /// anything is written; `document` is opaque and only bounded.
    async fn enroll(
        &self,
        scope: &DeliveryRegistrationScope,
        request: DeliveryRegistrationRequest,
    ) -> Result<DeliveryRegistration, DeliveryRegistrationError>;

    /// Drop one registration by its opaque host-minted id. `false` = there
    /// was none. Provider/channel addressing never becomes record identity.
    async fn remove(
        &self,
        scope: &DeliveryRegistrationScope,
        registration_id: &str,
    ) -> Result<bool, DeliveryRegistrationError>;

    /// Drop registrations the vendor reported gone, by host-minted id.
    /// Best-effort: pruning failure never fails the delivery that found it.
    async fn prune(
        &self,
        scope: &DeliveryRegistrationScope,
        registration_ids: &[String],
    ) -> Result<usize, DeliveryRegistrationError>;
}

/// One enrollment submission. Untrusted: every field arrives from a browser
/// or client and is validated host-side before storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryRegistrationRequest {
    /// Absolute URL the channel will deliver to. Checked against the
    /// channel's declared egress hosts before storage — the one
    /// security-critical, generic, pre-storage check (design §8).
    pub endpoint: String,
    /// Channel-opaque remainder (key material, client metadata). Size-bounded
    /// host-side; never parsed by generic code.
    pub document: String,
}

/// Opaque registration-store failure. Deliberately carries a classification
/// and a sanitized reason only: an endpoint URL is user data and must not
/// travel in an error string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeliveryRegistrationError {
    /// Caller-correctable: malformed endpoint, undeclared egress host, an
    /// oversized document, or the per-user registration limit.
    #[error("delivery registration is not acceptable: {reason}")]
    Rejected { reason: String },
    /// Storage trouble; the caller may retry.
    #[error("delivery registration storage is unavailable: {reason}")]
    Unavailable { reason: String },
}

/// Sanitized failure to read a stored opaque reply anchor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("reply context storage is unavailable")]
pub struct DeliveryReplyContextError;

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
    ) -> Result<Option<Vec<u8>>, DeliveryReplyContextError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use std::sync::Mutex;

    // Every consumer holds these as `Arc<dyn _>`, so dyn-safety is part of the
    // contract, not an implementation detail: a signature change that breaks it
    // fails here rather than at the far-away wiring site.
    static_assertions::assert_obj_safe!(
        ChannelDeliveryResolver,
        DeliveryReplyContextSource,
        DeliveryRegistrationService
    );

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
        ) -> Result<Option<Vec<u8>>, DeliveryReplyContextError> {
            self.contexts.lock().expect("lock").push((
                extension_id.as_str().to_string(),
                installation_id.as_str().to_string(),
                conversation_fingerprint.to_string(),
            ));
            Ok(None)
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
            Ok(None)
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
