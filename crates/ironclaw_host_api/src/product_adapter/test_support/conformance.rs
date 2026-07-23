//! The exported channel-adapter conformance suite (extension-runtime §8,
//! TEST-1): ONE behavioral contract every `ChannelAdapter` implementation
//! runs against a scripted vendor server. Concrete adapter crates (and the
//! invented-vendor integration fixture) call
//! [`run_channel_adapter_conformance`] from their own tests; a new channel
//! ships by passing this suite plus its own vendor-shape fixtures — no
//! bespoke harness per channel.
//!
//! Covered: inbound outcomes are bounded and well-formed (and malformed
//! input never panics), `deliver` honors the envelope with structured
//! per-part reports, `activate`/`cleanup` are idempotent against the same
//! scripted vendor server, and unsupported surfaces fail cleanly.

use std::sync::{Arc, Mutex};

use crate::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use async_trait::async_trait;

use crate::product_adapter::{
    ChannelAdapter, ChannelContext, ChannelError, InboundOutcome, OutboundEnvelope, OutboundPart,
    PartDeliveryOutcome, TargetQuery, VerifiedInbound,
};

/// One host-verified inbound request fixture.
pub struct ConformanceInbound {
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

/// The per-adapter fixture: the adapter under test plus the vendor-shaped
/// inputs and the scripted vendor server that satisfies it.
pub struct ChannelAdapterConformance {
    pub adapter: Arc<dyn ChannelAdapter>,
    pub extension_id: String,
    pub installation_id: String,
    /// A vendor-valid inbound request that must normalize to `Messages`.
    pub message_inbound: ConformanceInbound,
    /// A vendor challenge that must produce a bounded immediate `Respond`,
    /// when the protocol has one.
    pub challenge_inbound: Option<ConformanceInbound>,
    /// An envelope the adapter must fully deliver against the scripted
    /// vendor server.
    pub outbound_envelope: OutboundEnvelope,
    /// The scripted vendor server: a pure request→response script standing
    /// in for the vendor API behind restricted egress.
    #[allow(clippy::type_complexity)]
    pub vendor_responses:
        Arc<dyn Fn(&RestrictedEgressRequest) -> RestrictedEgressResponse + Send + Sync>,
    /// Non-secret operator config for the activation/cleanup context.
    pub config: Vec<(String, String)>,
    /// Whether free target listing (no query) is expected to fail cleanly
    /// with `Unsupported` (adapters with real listing set this false and
    /// are covered by their own fixtures).
    pub expects_unsupported_free_target_listing: bool,
}

fn conformance_value<T, E: std::fmt::Debug>(result: Result<T, E>, message: &'static str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{message}: {error:?}"),
    }
}

/// Scripted vendor server over the restricted-egress seam: records every
/// request and answers from the fixture's script.
pub struct ScriptedVendorServer {
    #[allow(clippy::type_complexity)]
    responder: Arc<dyn Fn(&RestrictedEgressRequest) -> RestrictedEgressResponse + Send + Sync>,
    requests: Mutex<Vec<RestrictedEgressRequest>>,
}

impl ScriptedVendorServer {
    pub fn new(
        responder: Arc<dyn Fn(&RestrictedEgressRequest) -> RestrictedEgressResponse + Send + Sync>,
    ) -> Self {
        Self {
            responder,
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn requests(&self) -> Vec<RestrictedEgressRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl RestrictedEgress for ScriptedVendorServer {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        let response = (self.responder)(&request);
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        Ok(response)
    }
}

/// Run the full conformance contract. Panics with a labeled assertion on
/// the first violation (this is a test-support entry point).
pub async fn run_channel_adapter_conformance(conformance: ChannelAdapterConformance) {
    let ChannelAdapterConformance {
        adapter,
        extension_id,
        installation_id,
        message_inbound,
        challenge_inbound,
        outbound_envelope,
        vendor_responses,
        config,
        expects_unsupported_free_target_listing,
    } = conformance;
    let server = ScriptedVendorServer::new(Arc::clone(&vendor_responses));

    // ── Inbound: a vendor-valid message normalizes, bounded and well-formed.
    let outcome = adapter
        .inbound(VerifiedInbound {
            extension_id: &extension_id,
            installation_id: &installation_id,
            body: &message_inbound.body,
            headers: &message_inbound.headers,
        })
        .expect("conformance: the vendor-valid message fixture must parse"); // safety: test-support conformance failure should fail the caller's test.
    let InboundOutcome::Messages(messages) = outcome else {
        panic!("conformance: the message fixture must normalize to Messages"); // safety: test-support conformance failure should fail the caller's test.
    };
    if messages.is_empty() {
        panic!("conformance: the message fixture must yield at least one message");
    }
    for message in &messages {
        message
            .validate()
            .expect("conformance: normalized messages must satisfy host bounds"); // safety: test-support conformance failure should fail the caller's test.
        if message.text.is_empty() {
            panic!("conformance: the message fixture's text must survive normalization");
        }
    }

    // ── Inbound: malformed and truncated bodies fail cleanly, never panic.
    for garbage in [
        &b""[..],
        &b"{"[..],
        &b"\xff\xfe\x00garbage"[..],
        &b"[]"[..],
        &b"{\"unexpected\":true}"[..],
    ] {
        match adapter.inbound(VerifiedInbound {
            extension_id: &extension_id,
            installation_id: &installation_id,
            body: garbage,
            headers: &[],
        }) {
            Ok(InboundOutcome::Respond(response)) => response
                .validate()
                .expect("conformance: immediate responses must stay within host bounds"), // safety: test-support conformance failure should fail the caller's test.
            Ok(InboundOutcome::Messages(messages)) => {
                for message in &messages {
                    conformance_value(
                        message.validate(),
                        "conformance: messages normalized from odd input must satisfy bounds",
                    );
                }
            }
            Ok(InboundOutcome::Ignore) | Err(_) => {}
        }
    }

    // ── Inbound: the protocol's challenge (when it has one) answers
    // immediately, within bounds.
    if let Some(challenge) = challenge_inbound {
        let outcome = adapter
            .inbound(VerifiedInbound {
                extension_id: &extension_id,
                installation_id: &installation_id,
                body: &challenge.body,
                headers: &challenge.headers,
            })
            .expect("conformance: the challenge fixture must parse"); // safety: test-support conformance failure should fail the caller's test.
        let InboundOutcome::Respond(response) = outcome else {
            panic!("conformance: the challenge fixture must produce an immediate response"); // safety: test-support conformance failure should fail the caller's test.
        };
        response
            .validate()
            .expect("conformance: the challenge response must stay within host bounds"); // safety: test-support conformance failure should fail the caller's test.
    }

    // ── Outbound: the envelope is fully delivered with structured per-part
    // reports against the scripted vendor server.
    let text_parts = outbound_envelope
        .parts
        .iter()
        .filter(|part| matches!(part, OutboundPart::Text(_)))
        .count();
    let report = adapter
        .deliver(outbound_envelope, &server)
        .await
        .expect("conformance: deliver must drive the scripted vendor server"); // safety: test-support conformance failure should fail the caller's test.
    if report.parts.is_empty() {
        panic!("conformance: a delivery report must describe at least one part");
    }
    if report.parts.len() < text_parts {
        panic!("conformance: every envelope part must be accounted for in the report");
    }
    for part in &report.parts {
        if !matches!(part, PartDeliveryOutcome::Sent { .. }) {
            panic!(
                "conformance: against the fixture's happy-path vendor script every part must be Sent, got {part:?}"
            );
        }
    }

    // ── Lifecycle hooks: activate and cleanup are idempotent against the
    // SAME scripted vendor server (a second run must not fail).
    let context = ChannelContext {
        extension_id: &extension_id,
        installation_id: &installation_id,
        config: &config,
    };
    adapter
        .activate(&context, &server)
        .await
        .expect("conformance: activation must succeed against the scripted vendor server"); // safety: test-support conformance failure should fail the caller's test.
    adapter
        .activate(&context, &server)
        .await
        .expect("conformance: activation must be idempotent (second run failed)"); // safety: test-support conformance failure should fail the caller's test.
    adapter
        .cleanup(&context, &server)
        .await
        .expect("conformance: cleanup must succeed against the scripted vendor server"); // safety: test-support conformance failure should fail the caller's test.
    adapter
        .cleanup(&context, &server)
        .await
        .expect("conformance: cleanup must be idempotent (second run failed)"); // safety: test-support conformance failure should fail the caller's test.

    // ── Unsupported surfaces fail cleanly (no panic, typed error).
    if expects_unsupported_free_target_listing {
        let error = adapter
            .list_targets(
                TargetQuery {
                    extension_id: extension_id.clone(),
                    installation_id: installation_id.clone(),
                    query: None,
                    limit: 10,
                },
                &server,
            )
            .await
            .expect_err("conformance: unsupported free target listing must error cleanly");
        if !matches!(error, ChannelError::Unsupported) {
            panic!(
                "conformance: unsupported listing must be ChannelError::Unsupported, got {error:?}"
            );
        }
    }
}
