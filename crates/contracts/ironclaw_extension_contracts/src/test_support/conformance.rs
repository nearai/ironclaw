//! The exported channel-surface conformance suite (extension-runtime §8,
//! TEST-1): ONE behavioral contract every channel implementation runs against
//! a scripted vendor server. Concrete adapter crates (and the invented-vendor
//! integration fixture) call [`run_channel_adapter_conformance`] from their
//! own tests; a new channel ships by passing this suite plus its own
//! vendor-shape fixtures — no bespoke harness per channel.
//!
//! **The suite is keyed on the halves the channel actually implements**
//! ([`ChannelSurfaces`]), not on one fused adapter. A channel declaring
//! `[channel.reply] transport = "stream"` implements no reply half at all, so
//! there is nothing to drive there — and that absence is asserted rather than
//! stubbed. The suite exercises exactly the halves present and asserts the
//! absent ones stay absent.
//!
//! Covered: inbound outcomes are bounded and well-formed (and malformed input
//! never panics), reply/delivery honor the envelope with structured per-part
//! reports, deferred post-ack fetch handles fail cleanly when unimplemented,
//! and unsupported surfaces error rather than panic.
//!
//! Not covered, deliberately: vendor-side ingress registration. That stopped
//! being adapter behavior when `activate`/`cleanup` became the
//! `[channel.ingress.registration]` / `[channel.ingress.deregistration]`
//! recipes — there is no per-adapter implementation left to conform, and the
//! generic executor is covered once, host-side, in
//! `ironclaw_extension_host::lifecycle`.

use std::sync::{Arc, Mutex};

use crate::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use async_trait::async_trait;

use crate::channel_adapter::{
    ChannelSurfaces, InboundOutcome, OutboundEnvelope, OutboundPart, PartDeliveryOutcome,
    VerifiedInbound,
};

/// One host-verified inbound request fixture.
pub struct ConformanceInbound {
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

/// The per-adapter fixture: the halves under test plus the vendor-shaped
/// inputs and the scripted vendor server that satisfies them.
pub struct ChannelAdapterConformance {
    /// The halves this channel implements. `None` entries are asserted absent
    /// rather than skipped: a missing half is a declaration, not a gap.
    pub surfaces: ChannelSurfaces,
    pub extension_id: String,
    pub installation_id: String,
    /// A vendor-valid inbound request that must normalize to `Messages`.
    /// `None` for a channel whose input arrives on the authenticated session
    /// door instead of a vendor payload.
    pub message_inbound: Option<ConformanceInbound>,
    /// A vendor challenge that must produce a bounded immediate `Respond`,
    /// when the protocol has one.
    pub challenge_inbound: Option<ConformanceInbound>,
    /// An envelope every implemented outbound half must fully deliver against
    /// the scripted vendor server.
    pub outbound_envelope: OutboundEnvelope,
    /// The scripted vendor server: a pure request→response script standing
    /// in for the vendor API behind restricted egress.
    #[allow(clippy::type_complexity)]
    pub vendor_responses:
        Arc<dyn Fn(&RestrictedEgressRequest) -> RestrictedEgressResponse + Send + Sync>,
    /// Non-secret operator config supplied to the inbound context.
    pub config: Vec<(String, String)>,
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
        surfaces,
        extension_id,
        installation_id,
        message_inbound,
        challenge_inbound,
        outbound_envelope,
        vendor_responses,
        config,
    } = conformance;
    let server = ScriptedVendorServer::new(Arc::clone(&vendor_responses));

    if !surfaces.has_outbound() && surfaces.ingress.is_none() {
        panic!("conformance: a channel must implement at least one half");
    }

    if let Some(ingress) = surfaces.ingress.as_ref() {
        let inbound = message_inbound.as_ref().expect(
            "conformance: a channel implementing ChannelIngress must supply a message fixture",
        ); // safety: test-support conformance failure should fail the caller's test.

        // ── Inbound: a vendor-valid message normalizes, bounded and
        // well-formed.
        let outcome = ingress
            .receive(
                VerifiedInbound {
                    extension_id: &extension_id,
                    installation_id: &installation_id,
                    config: &config,
                    body: &inbound.body,
                    headers: &inbound.headers,
                    can_reply_in_threads: true,
                },
                &server,
            )
            .await
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

        // ── Inbound: malformed and truncated bodies fail cleanly, never
        // panic.
        for garbage in [
            &b""[..],
            &b"{"[..],
            &b"\xff\xfe\x00garbage"[..],
            &b"[]"[..],
            &b"{\"unexpected\":true}"[..],
        ] {
            match ingress
                .receive(
                    VerifiedInbound {
                        extension_id: &extension_id,
                        installation_id: &installation_id,
                        config: &config,
                        body: garbage,
                        headers: &[],
                        can_reply_in_threads: true,
                    },
                    &server,
                )
                .await
            {
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
                Ok(InboundOutcome::BatchFragment(fragment)) => {
                    conformance_value(
                        fragment.validate(),
                        "conformance: batch fragments normalized from odd input must satisfy bounds",
                    );
                }
                Ok(InboundOutcome::Ignore) | Err(_) => {}
            }
        }

        // ── Inbound: the protocol's challenge (when it has one) answers
        // immediately, within bounds.
        if let Some(challenge) = challenge_inbound {
            let outcome = ingress
                .receive(
                    VerifiedInbound {
                        extension_id: &extension_id,
                        installation_id: &installation_id,
                        config: &config,
                        body: &challenge.body,
                        headers: &challenge.headers,
                        can_reply_in_threads: true,
                    },
                    &server,
                )
                .await
                .expect("conformance: the challenge fixture must parse"); // safety: test-support conformance failure should fail the caller's test.
            let InboundOutcome::Respond(response) = outcome else {
                panic!("conformance: the challenge fixture must produce an immediate response"); // safety: test-support conformance failure should fail the caller's test.
            };
            response
                .validate()
                .expect("conformance: the challenge response must stay within host bounds"); // safety: test-support conformance failure should fail the caller's test.
        }
    } else {
        if message_inbound.is_some() {
            panic!(
                "conformance: a message fixture was supplied but the channel has no ingress half"
            );
        }
        if challenge_inbound.is_some() {
            panic!(
                "conformance: a challenge fixture was supplied but the channel has no ingress half"
            );
        }
    }

    // ── Outbound: every implemented half fully delivers the envelope with
    // structured per-part reports against the scripted vendor server. Both
    // axes are driven with the SAME envelope on purpose: reply and delivery
    // differ in routing, never in what an envelope means.
    let text_parts = outbound_envelope
        .parts
        .iter()
        .filter(|part| matches!(part, OutboundPart::Text(_)))
        .count();
    if let Some(reply) = surfaces.reply.as_ref() {
        let report = reply
            .send_reply(outbound_envelope.clone(), &server)
            .await
            .expect("conformance: send_reply must drive the scripted vendor server"); // safety: test-support conformance failure should fail the caller's test.
        assert_delivery_report(&report.parts, text_parts, "send_reply");
    }
    if let Some(delivery) = surfaces.delivery.as_ref() {
        let report = delivery
            .deliver(outbound_envelope, &server)
            .await
            .expect("conformance: deliver must drive the scripted vendor server"); // safety: test-support conformance failure should fail the caller's test.
        assert_delivery_report(&report.parts, text_parts, "deliver");
    }
}

fn assert_delivery_report(parts: &[PartDeliveryOutcome], text_parts: usize, half: &str) {
    if parts.is_empty() {
        panic!("conformance: a {half} report must describe at least one part");
    }
    if parts.len() < text_parts {
        panic!("conformance: every envelope part must be accounted for in the {half} report");
    }
    for part in parts {
        if !matches!(part, PartDeliveryOutcome::Sent { .. }) {
            panic!(
                "conformance: against the fixture's happy-path vendor script every {half} part must be Sent, got {part:?}"
            );
        }
    }
}
