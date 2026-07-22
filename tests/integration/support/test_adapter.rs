//! Test inbound-injection for the integration harness.
//!
//! `RebornTestIngress` builds verified [`ProductInboundEnvelope`]s directly on
//! the LIVE product-workflow inbound contract: it constructs a
//! [`ParsedProductInbound`] and stamps it with a host-verified
//! [`TrustedInboundContext`] via
//! [`ProductInboundEnvelope::from_trusted_parse`] — the same path the
//! production `ChannelAdapter` ingress bridge takes
//! (`ironclaw_reborn_composition::extension_host::extension_ingress`).
//!
//! Ported in P7b (DEL-5): the retired `ProductAdapter` trait's `parse_inbound`
//! used to produce the parsed value here. The harness never needed the trait —
//! only the LIVE envelope it wrapped — so the parsed inbound is now built
//! directly. Coverage that exercised the retired trait's own `parse_inbound` /
//! `render_outbound` moved to the `ChannelAdapter` conformance suite
//! (`ironclaw_product_adapters::test_support::run_channel_adapter_conformance`);
//! see that PR's DEL-5 notes.

use chrono::Utc;
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ParsedProductInbound, ProductAdapterError, ProductAdapterId,
    ProductInboundEnvelope, ProductInboundPayload, ProductTriggerReason, ProjectionCursor,
    ProjectionSubscriptionPayload, ProtocolAuthEvidence, TrustedInboundContext, UserMessagePayload,
};

/// Builds verified inbound envelopes for the integration harness.
///
/// Holds only the host-stamped identity (`adapter_id` / `installation_id`); the
/// envelopes it produces flow through the same
/// [`TrustedInboundContext::from_verified_evidence`] +
/// [`ProductInboundEnvelope::from_trusted_parse`] path as the production
/// `ChannelAdapter` ingress bridge, so nothing here forges trusted context.
#[derive(Debug, Clone)]
pub struct RebornTestIngress {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
}

impl RebornTestIngress {
    pub fn new(
        adapter_id: impl Into<String>,
        installation_id: impl Into<String>,
    ) -> Result<Self, ProductAdapterError> {
        Ok(Self {
            adapter_id: ProductAdapterId::new(adapter_id.into())?,
            installation_id: AdapterInstallationId::new(installation_id.into())?,
        })
    }

    pub fn adapter_id(&self) -> &ProductAdapterId {
        &self.adapter_id
    }

    pub fn installation_id(&self) -> &AdapterInstallationId {
        &self.installation_id
    }

    /// Directly construct the adapter-shaped `ParsedProductInbound` a real
    /// adapter would hand the host, using the harness's stable synthetic actor
    /// (`reborn_test_user` / `user_id`) and conversation (`thread_id`).
    fn parsed_inbound(
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        payload: ProductInboundPayload,
    ) -> Result<ParsedProductInbound, ProductAdapterError> {
        ParsedProductInbound::new(
            ExternalEventId::new(event_id)?,
            ExternalActorRef::new("reborn_test_user", user_id, Some(user_id.to_string()))?,
            ExternalConversationRef::new(None, thread_id.to_string(), None, None)?,
            payload,
        )
    }

    /// The shared trusted-context wrapper: stamps host-verified evidence for
    /// `user_id` and wraps a directly-built [`ParsedProductInbound`] into a
    /// [`ProductInboundEnvelope`] exactly as the production ingress bridge does.
    fn envelope_from_parsed(
        &self,
        user_id: &str,
        parsed: ParsedProductInbound,
    ) -> Result<ProductInboundEnvelope, ProductAdapterError> {
        let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, user_id);
        let context = TrustedInboundContext::from_verified_evidence(
            self.adapter_id.clone(),
            self.installation_id.clone(),
            Utc::now(),
            &evidence,
        )?;
        ProductInboundEnvelope::from_trusted_parse(context, parsed)
    }

    pub fn verified_text_envelope(
        &self,
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        text: &str,
    ) -> Result<ProductInboundEnvelope, ProductAdapterError> {
        self.verified_text_envelope_with_trigger(
            event_id,
            user_id,
            thread_id,
            text,
            ProductTriggerReason::DirectChat,
        )
    }

    pub fn verified_text_envelope_with_trigger(
        &self,
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        text: &str,
        trigger: ProductTriggerReason,
    ) -> Result<ProductInboundEnvelope, ProductAdapterError> {
        let payload = ProductInboundPayload::UserMessage(UserMessagePayload::new(
            text.to_string(),
            Vec::new(),
            trigger,
        )?);
        let parsed = Self::parsed_inbound(event_id, user_id, thread_id, payload)?;
        self.envelope_from_parsed(user_id, parsed)
    }

    pub fn verified_subscription_envelope(
        &self,
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        thread_id_hint: Option<&str>,
        after_cursor: Option<ProjectionCursor>,
    ) -> Result<ProductInboundEnvelope, ProductAdapterError> {
        let payload =
            ProductInboundPayload::SubscriptionRequest(ProjectionSubscriptionPayload::new(
                thread_id_hint.map(|hint| hint.to_string()),
                after_cursor,
            )?);
        let parsed = Self::parsed_inbound(event_id, user_id, thread_id, payload)?;
        self.envelope_from_parsed(user_id, parsed)
    }

    /// A verified `ApprovalResolution` envelope for `submit_inbound`, the real
    /// dispatch arm an adapter's "approve"/"deny" reply hits.
    pub fn verified_approval_resolution_envelope(
        &self,
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        gate_ref: &str,
        decision: ApprovalDecision,
    ) -> Result<ProductInboundEnvelope, ProductAdapterError> {
        let payload = ProductInboundPayload::ApprovalResolution(ApprovalResolutionPayload::new(
            gate_ref, decision,
        )?);
        let parsed = Self::parsed_inbound(event_id, user_id, thread_id, payload)?;
        self.envelope_from_parsed(user_id, parsed)
    }

    /// A verified `AuthResolution` envelope for `submit_inbound`, the real
    /// dispatch arm an adapter's auth-gate reply hits.
    pub fn verified_auth_resolution_envelope(
        &self,
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        auth_request_ref: &str,
        result: AuthResolutionResult,
    ) -> Result<ProductInboundEnvelope, ProductAdapterError> {
        let payload = ProductInboundPayload::AuthResolution(AuthResolutionPayload::new(
            auth_request_ref,
            result,
        )?);
        let parsed = Self::parsed_inbound(event_id, user_id, thread_id, payload)?;
        self.envelope_from_parsed(user_id, parsed)
    }
}
