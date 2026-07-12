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

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Router, body::Bytes, extract::State, http::HeaderMap, response::Response, routing::post,
};
use ironclaw_extension_host::ingress::ExtensionIngressRouter;
use ironclaw_product_adapters::{ProductInboundAck, ProductInboundEnvelope};
use ironclaw_wasm_product_adapters::ImmediateAckWorkflowObserver;

use crate::extension_host::extension_ingress::{
    ChannelIngressDrain, InboundPayloadClassifier, PostAdmissionObserver, VerifiedEvidenceMint,
    forward_alias_request,
};
use crate::webui::webui_serve::{PublicRouteDrain, PublicRouteMount};

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

const SLACK_EVENTS_ROUTE_ID: &str = "slack.events";
const SLACK_EXTENSION_ID: &str = "slack";
const SLACK_EVENTS_ROUTE_SUFFIX: &str = "events";
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

/// Adapts the Slack final-reply delivery observer (an
/// [`ImmediateAckWorkflowObserver`]) to the generic post-admission observer
/// seam. Deleted with the P5 delivery-coordinator cutover.
pub(crate) struct ImmediateAckObserverAdapter(pub(crate) Arc<dyn ImmediateAckWorkflowObserver>);

#[async_trait]
impl PostAdmissionObserver for ImmediateAckObserverAdapter {
    async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck) {
        self.0.observe_workflow_ack(envelope, ack).await;
    }

    async fn observe_error(
        &self,
        envelope: ProductInboundEnvelope,
        error: ironclaw_product_adapters::ProductAdapterError,
    ) {
        self.0.observe_workflow_error(envelope, error).await;
    }
}

/// Build the one-release `/webhooks/slack/events` forwarding alias: the
/// handler drives the SAME generic router as the canonical
/// `/webhooks/extensions/slack/events` route (an internal forward — Slack
/// does not follow HTTP redirects for event delivery). Removal note: delete
/// in the first release after the P4 cutover ships (MIG-5).
pub(crate) fn slack_events_alias_mount(
    router: Arc<ExtensionIngressRouter>,
    drain: Option<Arc<dyn ChannelIngressDrain>>,
) -> Result<PublicRouteMount, crate::RebornBuildError> {
    let descriptor = crate::host_ingress::bundled_channel_ingress_descriptor(
        crate::extension_host::available_extensions::slack_manifest_toml(),
        SLACK_EVENTS_ROUTE_ID,
        SLACK_EVENTS_PATH,
    )
    .map_err(|error| crate::RebornBuildError::InvalidConfig {
        reason: format!("legacy channel events alias descriptor invalid: {error}"),
    })?;
    let axum_router = Router::new()
        .route(SLACK_EVENTS_PATH, post(alias_handler))
        .with_state(router);
    let mut mount = PublicRouteMount::new(axum_router, vec![descriptor]);
    if let Some(drain) = drain {
        mount = mount.with_drain(Arc::new(AliasDrain(drain)));
    }
    Ok(mount)
}

async fn alias_handler(
    State(router): State<Arc<ExtensionIngressRouter>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_alias_request(
        &router,
        SLACK_EXTENSION_ID,
        SLACK_EVENTS_ROUTE_SUFFIX,
        &headers,
        body,
    )
    .await
}

struct AliasDrain(Arc<dyn ChannelIngressDrain>);

impl PublicRouteDrain for AliasDrain {
    fn drain<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(self.0.drain())
    }
}

#[cfg(test)]
mod e2e_tests;
