//! Slack inbound wiring for the GENERIC extension ingress router
//! (extension-runtime P4 cutover).
//!
//! The former Slack-owned Events API route mount, installation resolver, and
//! per-installation webhook dispatcher were deleted in the P4 cutover:
//! signature verification now runs in the host's generic recipe verifier
//! (from the Slack manifest's `[channel.ingress.verification]`), envelope
//! parsing lives in `ironclaw_slack_v2_adapter::SlackChannelAdapter`, and
//! durable admission + turn submission flow through the generic inbound sink
//! over the existing product workflow. What remains here is the Slack-shaped
//! glue the generic pieces are parameterized with (classifier, evidence
//! shape, observer adapter) plus the one-release legacy path alias.
//!
//! Transitional (deleted with `composition/src/slack/**` in P6).

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_host::ingress::ExtensionIngressRouter;
use ironclaw_product_adapters::{ProductInboundAck, ProductInboundEnvelope};

use crate::extension_host::extension_ingress::{
    ChannelIngressDrain, InboundPayloadClassifier, PostAdmissionObserver, VerifiedEvidenceMint,
};
use crate::webui::webui_serve::PublicRouteMount;

mod installation;
pub use installation::{
    SlackApiAppId, SlackChannelId, SlackEnterpriseId, SlackInstallationSelector, SlackTeamId,
    SlackUserId,
};

/// Legacy fixed path, now a ONE-RELEASE forwarding alias to the canonical
/// `/webhooks/extensions/slack/events` route (migration MIG-5). Delete the
/// alias — and this constant — in the first release after the cutover ships;
/// vendors reconfigure their event URL to the canonical path.
pub const SLACK_EVENTS_PATH: &str = "/webhooks/slack/events";

const SLACK_SIGNATURE_HEADER: &str = "X-Slack-Signature";
const SLACK_TIMESTAMP_HEADER: &str = "X-Slack-Request-Timestamp";

/// The Slack-shaped evidence the generic sink stamps on admitted messages —
/// mirrors the manifest's `hmac_sha256` verification recipe.
pub(crate) fn slack_evidence_mint() -> VerifiedEvidenceMint {
    VerifiedEvidenceMint::RequestSignature {
        signature_header: SLACK_SIGNATURE_HEADER.to_string(),
        timestamp_header: Some(SLACK_TIMESTAMP_HEADER.to_string()),
    }
}

/// Slack's gate-resolution reclassification for the generic sink
/// (`approve` / `deny [gate:<ref>]` / `auth deny <ref>` replies).
pub(crate) fn slack_inbound_classifier() -> Arc<InboundPayloadClassifier> {
    Arc::new(|message| {
        ironclaw_slack_v2_adapter::classify_interaction_resolution(&message.text, message.trigger)
    })
}

/// Adapts the generic run-delivery observer to the post-admission observer
/// seam of the generic ingress sink.
pub(crate) struct RunDeliveryObserverAdapter(
    pub(crate) Arc<ironclaw_product_workflow::RunDeliveryObserver>,
);

#[async_trait]
impl PostAdmissionObserver for RunDeliveryObserverAdapter {
    async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        self.0.observe_ack(envelope, ack).await;
    }

    async fn observe_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ironclaw_product_adapters::ProductAdapterError,
    ) {
        self.0.observe_error(envelope, error).await;
    }
}

/// Build the one-release `/webhooks/slack/events` forwarding alias. The
/// implementation lives in the generic legacy-alias home
/// (`extension_host/legacy_ingress_aliases.rs`, MIG-5); this wrapper keeps
/// the retiring lane's call sites compiling until the lane deletes.
pub(crate) fn slack_events_alias_mount(
    router: Arc<ExtensionIngressRouter>,
    drain: Option<Arc<dyn ChannelIngressDrain>>,
) -> Result<PublicRouteMount, crate::RebornBuildError> {
    crate::extension_host::legacy_ingress_aliases::legacy_channel_ingress_alias_mounts(
        &router, drain,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| crate::RebornBuildError::InvalidConfig {
        reason: "legacy channel ingress alias table has no slack entry".to_string(),
    })
}

#[cfg(test)]
mod e2e_tests;
