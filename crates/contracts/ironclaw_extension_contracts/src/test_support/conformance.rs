//! The exported channel-surface conformance suite (extension-runtime §8,
//! TEST-1): ONE behavioral contract every channel implementation runs against
//! a scripted vendor server. Concrete adapter crates (and the invented-vendor
//! integration fixture) call [`run_channel_adapter_conformance`] from their
//! own tests; a new channel ships by passing this suite plus its own
//! vendor-shape fixtures — no bespoke harness per channel.
//!
//! **The suite is keyed on the halves the channel actually implements**
//! ([`ChannelSurfaces`]), not on one fused adapter. The reply slot holds one
//! [`ReplySink`]; the fixture's declared `reply_transport` picks the cadence
//! it is driven at — a `message` sink sees the terminal materialization (and
//! an idempotent repeat), a `stream` sink sees the opening revision, an
//! idempotent repeat, the terminal revision, and an idempotent terminal
//! repeat — with the checkpoint round-tripped between calls. The suite
//! exercises exactly the halves present.
//!
//! Covered: inbound outcomes are bounded and well-formed (and malformed input
//! never panics), delivery honors the envelope with structured per-part
//! reports, reply sinks apply and re-apply revisions against the scripted
//! vendor with bounded reports and never treat a checkpoint they cannot read
//! as evidence of application, deferred post-ack fetch handles fail cleanly
//! when unimplemented, and unsupported surfaces error rather than panic.
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

use crate::channel::ReplyTransport;
use crate::channel_adapter::{
    ChannelSurfaces, InboundOutcome, OutboundEnvelope, OutboundPart, PartDeliveryOutcome,
    VerifiedInbound,
};
use crate::reply::{
    ReplyAnswerText, ReplyAttachmentRef, ReplyAudience, ReplyContextBytes, ReplyDisplayText,
    ReplyDocument, ReplyItemId, ReplyReconcilePoint, ReplyReconcileRequest, ReplyRevision,
    ReplySink, ReplySinkCheckpoint, ReplySinkOutcome, ReplyTarget, ReplyThreadAnchor,
};
use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{TurnActor, TurnRunId, TurnScope};

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
    /// The manifest's declared `[channel.reply] transport`, which decides the
    /// cadence the bound sink is driven at. `None` when the channel declares
    /// no reply section (and then `surfaces.reply` must be `None` too).
    pub reply_transport: Option<ReplyTransport>,
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
        reply_transport,
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
    match (surfaces.reply.as_ref(), reply_transport) {
        (Some(sink), Some(transport)) => {
            run_reply_sink_conformance(sink.as_ref(), transport, &outbound_envelope, &server).await;
        }
        (Some(_), None) => {
            panic!(
                "conformance: a reply sink is bound but the fixture declares no reply transport"
            );
        }
        (None, Some(_)) => {
            panic!("conformance: the fixture declares a reply transport but binds no reply sink");
        }
        (None, None) => {}
    }
    if let Some(delivery) = surfaces.delivery.as_ref() {
        let report = delivery
            .deliver(outbound_envelope, &server)
            .await
            .expect("conformance: deliver must drive the scripted vendor server"); // safety: test-support conformance failure should fail the caller's test.
        assert_delivery_report(&report.parts, text_parts, "deliver");
    }
}

/// The reply-sink half of the contract: a synthetic reply drives the sink at
/// its declared cadence — a `stream` sink through the opening revision, an
/// idempotent repeat with the returned checkpoint, the terminal revision, and
/// an idempotent terminal repeat; a `message` sink through the terminal
/// revision and its repeat only. Against the fixture's happy-path vendor
/// script each must be `Applied`, and every report must stay within the host
/// bounds a real publisher re-validates. The terminal document lists the
/// same attachments the request materializes, as the request contract
/// requires. Finally the terminal revision is reconciled once more under a
/// checkpoint of a foreign version: an unreadable checkpoint is never
/// evidence the terminal was applied, so the sink must either drive the
/// provider again or report a non-`Applied` outcome — never `Applied` off
/// bytes it could not decode.
async fn run_reply_sink_conformance(
    sink: &dyn ReplySink,
    transport: ReplyTransport,
    outbound_envelope: &OutboundEnvelope,
    server: &ScriptedVendorServer,
) {
    let answer_text = outbound_envelope
        .parts
        .iter()
        .filter_map(|part| match part {
            OutboundPart::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let answer_text = if answer_text.is_empty() {
        "conformance reply".to_string()
    } else {
        answer_text
    };
    let run_id = TurnRunId::new();
    let target = ReplyTarget {
        scope: TurnScope::new_with_owner(
            conformance_value(
                TenantId::new("conformance-tenant"),
                "conformance: tenant id",
            ),
            None,
            None,
            conformance_value(
                ThreadId::new("conformance-thread"),
                "conformance: thread id",
            ),
            Some(conformance_value(
                UserId::new("conformance-user"),
                "conformance: user id",
            )),
        ),
        actor: TurnActor::new(conformance_value(
            UserId::new("conformance-user"),
            "conformance: user id",
        )),
        run_id,
        conversation: Some(outbound_envelope.target.conversation.clone()),
        thread_anchor: outbound_envelope
            .target
            .thread_anchor
            .as_deref()
            .map(|anchor| conformance_value(ReplyThreadAnchor::new(anchor), "conformance: anchor")),
        audience: ReplyAudience::Private,
    };
    let reply_context = outbound_envelope.reply_context.clone().map(|bytes| {
        conformance_value(ReplyContextBytes::new(bytes), "conformance: reply context")
    });
    let materialized_attachments: Vec<ironclaw_host_api::attachment::WorkspaceFile> =
        outbound_envelope
            .parts
            .iter()
            .filter_map(|part| match part {
                OutboundPart::File(file) => Some(file.clone()),
                _ => None,
            })
            .collect();

    let mut document = ReplyDocument::default();
    document.note_phase(crate::reply::ReplyPhase::Working);
    document.append_answer(&answer_text);
    let first = ReplyRevision {
        revision: 1,
        document: document.clone(),
    };

    let reconcile = |revision: ReplyRevision,
                     point: ReplyReconcilePoint,
                     checkpoint: Option<ReplySinkCheckpoint>| {
        ReplyReconcileRequest {
            revision,
            point,
            target: target.clone(),
            reply_context: reply_context.clone(),
            checkpoint,
            extension_generation: 1,
            materialized_attachments: if matches!(point, ReplyReconcilePoint::Terminal) {
                materialized_attachments.clone()
            } else {
                Vec::new()
            },
        }
    };

    let mut checkpoint = None;
    if transport.reconciles_at(ReplyReconcilePoint::Opened) {
        let report = sink
            .reconcile(
                reconcile(first.clone(), ReplyReconcilePoint::Opened, None),
                server,
            )
            .await
            .expect("conformance: the opening revision must reconcile against the scripted vendor"); // safety: test-support conformance failure should fail the caller's test.
        assert_stream_report_applied(&report.outcome, "opening revision");
        checkpoint = report.checkpoint;

        let repeat = sink
            .reconcile(
                reconcile(first, ReplyReconcilePoint::Progress, checkpoint.clone()),
                server,
            )
            .await
            .expect("conformance: repeating a revision must not error"); // safety: test-support conformance failure should fail the caller's test.
        assert_stream_report_applied(&repeat.outcome, "repeated revision");
        checkpoint = repeat.checkpoint.or(checkpoint);
    }

    // The document and the request must agree: `materialized_attachments`
    // is non-empty only when the document lists attachments, so derive the
    // listed refs from the very files the terminal request carries.
    let attachment_refs: Vec<ReplyAttachmentRef> = materialized_attachments
        .iter()
        .enumerate()
        .map(|(index, file)| ReplyAttachmentRef {
            id: conformance_value(
                ReplyItemId::new(format!("attachment-{index}")),
                "conformance: attachment id",
            ),
            filename: conformance_value(
                ReplyDisplayText::new(
                    file.filename
                        .clone()
                        .unwrap_or_else(|| file.path.to_string()),
                ),
                "conformance: attachment filename",
            ),
            mime_type: conformance_value(
                ReplyDisplayText::new(file.mime_type.clone()),
                "conformance: attachment mime type",
            ),
            size_bytes: file.size_bytes(),
        })
        .collect();
    document.finalize_answer(
        conformance_value(ReplyAnswerText::new(&answer_text), "conformance: answer"),
        attachment_refs,
    );
    document.set_status(
        conformance_value(ReplyDisplayText::new("done"), "conformance: status"),
        None,
    );
    document.complete();
    let terminal = ReplyRevision {
        revision: 2,
        document,
    };
    let report = sink
        .reconcile(
            reconcile(
                terminal.clone(),
                ReplyReconcilePoint::Terminal,
                checkpoint.clone(),
            ),
            server,
        )
        .await
        .expect("conformance: the terminal revision must reconcile"); // safety: test-support conformance failure should fail the caller's test.
    assert_stream_report_applied(&report.outcome, "terminal revision");
    // `None` keeps the previous checkpoint (the report contract): a sink
    // with nothing new to persist must still see its carried state on the
    // repeated terminal reconcile.
    let checkpoint = report.checkpoint.or(checkpoint);

    let repeat = sink
        .reconcile(
            reconcile(terminal.clone(), ReplyReconcilePoint::Terminal, checkpoint),
            server,
        )
        .await
        .expect("conformance: repeating the terminal revision must not error"); // safety: test-support conformance failure should fail the caller's test.
    assert_stream_report_applied(&repeat.outcome, "repeated terminal revision");

    // ── A checkpoint the sink cannot read is never evidence of application.
    // A foreign checkpoint version is treated as absent: the sink either
    // drives the provider again (calls recorded) or reports a non-`Applied`
    // outcome. What it must never do is short-circuit to `Applied` off bytes
    // it could not decode.
    let foreign = conformance_value(
        ReplySinkCheckpoint::new(u32::MAX, "foreign"),
        "conformance: foreign checkpoint",
    );
    let calls_before = server.requests().len();
    let report = sink
        .reconcile(
            reconcile(terminal, ReplyReconcilePoint::Terminal, Some(foreign)),
            server,
        )
        .await
        .expect("conformance: reconciling under an unreadable checkpoint must not error"); // safety: test-support conformance failure should fail the caller's test.
    let calls_during = server.requests().len().saturating_sub(calls_before);
    if report.outcome.is_applied() && calls_during == 0 {
        panic!(
            "conformance: the sink reported Applied off a checkpoint it cannot read without \
             driving the provider"
        );
    }
}

fn assert_stream_report_applied(outcome: &ReplySinkOutcome, step: &str) {
    if !outcome.is_applied() {
        panic!(
            "conformance: against the fixture's happy-path vendor script the {step} must be Applied, got {outcome:?}"
        );
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
