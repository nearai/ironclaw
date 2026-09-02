//! The Web Push channel's **delivery** half: render one envelope into one
//! notification, encrypt it per enrolled client, and POST through restricted
//! egress.
//!
//! **This channel implements exactly one of the three halves, and the two
//! absences are the design rather than gaps.**
//!
//! - No [`ChannelIngress`](ironclaw_extension_contracts::channel_adapter::ChannelIngress):
//!   input arrives on the authenticated session door, whose actor authority
//!   is the authenticated caller — a thing an adapter may never mint from a
//!   payload. There is no webhook mount and no vendor payload to parse.
//! - No [`ChannelReply`](ironclaw_extension_contracts::channel_adapter::ChannelReply):
//!   the manifest declares `[channel.reply] transport = "stream"`, so the
//!   host publishes to the durable projection pipeline and an adapter is
//!   never called. A stub here would be dead code that reads as live.
//!
//! `check_binding` proves both absences against the manifest at activation,
//! so this comment cannot quietly become false.
//!
//! **The adapter holds no store.** Per-user delivery registrations are
//! host-owned (design §8): the coordinator resolves them, hands them over on
//! the envelope, and prunes what this half reports gone — the same shape as
//! every other "the adapter describes, the host writes" rule in the delivery
//! contract.

use async_trait::async_trait;
use ironclaw_extension_contracts::auth_prompt::render_channel_auth_prompt;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelDelivery, ChannelError, DeliveryRegistration, DeliveryReport, OutboundEnvelope,
    OutboundPart, PartDeliveryOutcome,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest,
};
use ironclaw_host_api::action::NetworkMethod;
use ironclaw_host_api::ids::SecretHandle;
use ironclaw_web_app::{
    DEFAULT_TTL_SECONDS, PushEndpoint, PushSubscriptionRecord, PushUrgency, RegistrationDocument,
    WEB_APP_VAPID_CREDENTIAL_HANDLE, WebAppError, WebAppNotificationPayload, build_push_request,
};

/// Deep link a notification opens; the service worker resolves it against
/// the app origin.
const NOTIFICATION_URL: &str = "/automations";
const NOTIFICATION_TITLE: &str = "IronClaw";
/// Fixed run-completion copy (design §7.10): a push payload never carries
/// generated or protected content, only this generic sentence.
const RUN_COMPLETION_BODY: &str = "An agent run finished.";

/// Per-registration send outcomes, folded into one part outcome.
#[derive(Default)]
struct FanOutTally {
    accepted: usize,
    /// Registration ids the push service said are gone. Reported to the host,
    /// which owns the records; this half never writes.
    pruned: Vec<String>,
    ambiguous: Option<String>,
    retryable: Option<String>,
    permanent: Option<String>,
    unauthorized: Option<String>,
}

/// Stateless: everything this half needs arrives on the envelope.
#[derive(Debug, Default, Clone, Copy)]
pub struct WebAppChannelAdapter;

impl WebAppChannelAdapter {
    pub fn new() -> Self {
        Self
    }
}

/// One registration parsed into the vendor record this half sends with.
///
/// The opaque half of a registration is interpreted by the web-app domain's
/// [`RegistrationDocument`] grammar — the one owner the host's enrollment
/// probe reads through as well (design §8: "everything else validates where
/// it is used"). A malformed document fails **this** registration — not the
/// delivery, and not the other registrations — and is reported for pruning
/// on the same path an expired endpoint takes.
fn parse_registration(
    registration: &DeliveryRegistration,
) -> Result<PushSubscriptionRecord, WebAppError> {
    let endpoint = PushEndpoint::new(registration.endpoint.clone())?;
    let document = RegistrationDocument::parse(&registration.document)?;
    Ok(PushSubscriptionRecord::new(
        endpoint,
        document.keys,
        document.user_agent,
        registration.created_at.clone(),
    ))
}

#[async_trait]
impl ChannelDelivery for WebAppChannelAdapter {
    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        // Render every deliverable part into one notification body; push is
        // a coalesced surface, not a message stream.
        let mut lines: Vec<String> = Vec::new();
        let mut urgency = PushUrgency::Normal;
        let mut part_supported: Vec<Result<(), &'static str>> = Vec::new();
        // §7.10: a typed run-completion part builds its own fixed-copy v2
        // payload; it never mixes with free-text rendering.
        let mut run_completion: Option<WebAppNotificationPayload> = None;
        for part in &envelope.parts {
            match part {
                OutboundPart::Text(text) => {
                    lines.push(text.clone());
                    part_supported.push(Ok(()));
                }
                OutboundPart::AuthPrompt {
                    view,
                    direct_message,
                } => {
                    lines.push(render_channel_auth_prompt(view, *direct_message));
                    urgency = PushUrgency::High;
                    part_supported.push(Ok(()));
                }
                OutboundPart::RunCompletion(view) => {
                    // Fixed copy only (design §7.10): the payload carries no
                    // generated or protected content, and the URL derives
                    // from the typed thread id inside the payload builder.
                    run_completion = Some(WebAppNotificationPayload::run_completion(
                        NOTIFICATION_TITLE,
                        RUN_COMPLETION_BODY,
                        view.thread_id.as_str(),
                        view.notice_id.clone(),
                        view.opaque_thread_tag.clone(),
                        view.unread_count_for_thread,
                    ));
                    part_supported.push(Ok(()));
                }
                OutboundPart::File(_) => {
                    part_supported.push(Err("attachments are not supported by browser push"));
                }
                OutboundPart::Retract { .. } => {
                    part_supported.push(Err("retraction is not supported by browser push"));
                }
                OutboundPart::React { .. } => {
                    part_supported.push(Err("reactions are not supported by browser push"));
                }
            }
        }

        // §7.10: a typed run-completion push is a complete, fixed-copy
        // payload. Mixing it with free-text parts would send ONE push and
        // then report `Sent` for text the push service never accepted —
        // forged delivery evidence. Reject the mixed shape outright.
        if run_completion.is_some() && lines.iter().any(|line| !line.is_empty()) {
            return Ok(DeliveryReport::from_parts(
                envelope
                    .parts
                    .iter()
                    .map(|_| PartDeliveryOutcome::Permanent {
                        reason: "run-completion pushes cannot be mixed with other parts"
                            .to_string(),
                    })
                    .collect(),
            ));
        }
        let has_deliverable = part_supported.iter().any(Result::is_ok);
        if !has_deliverable {
            return Ok(DeliveryReport::from_parts(
                part_supported
                    .into_iter()
                    .map(|supported| match supported {
                        Ok(()) => PartDeliveryOutcome::Permanent {
                            reason: "nothing to deliver".to_string(),
                        },
                        Err(reason) => PartDeliveryOutcome::Permanent {
                            reason: reason.to_string(),
                        },
                    })
                    .collect(),
            ));
        }

        let mut tally = FanOutTally::default();
        let deliverable_outcome = if envelope.registrations.is_empty() {
            // The coordinator resolves zero registrations to a "no target"
            // outcome before it ever calls this half, so reaching here means
            // the channel declared no enrollment requirement. Say so plainly
            // rather than pretending a send was attempted.
            PartDeliveryOutcome::Permanent {
                reason: "no clients are enrolled for browser push".to_string(),
            }
        } else {
            let payload = match run_completion {
                Some(payload) => payload,
                None => WebAppNotificationPayload::new(
                    NOTIFICATION_TITLE,
                    lines.join("\n\n"),
                    NOTIFICATION_URL,
                    None,
                ),
            };
            tally = self
                .fan_out(&envelope.registrations, &payload, urgency, egress)
                .await;
            fold_tally(&tally)
        };

        Ok(DeliveryReport {
            parts: part_supported
                .into_iter()
                .map(|supported| match supported {
                    Ok(()) => deliverable_outcome.clone(),
                    Err(reason) => PartDeliveryOutcome::Permanent {
                        reason: reason.to_string(),
                    },
                })
                .collect(),
            prune_registrations: tally.pruned,
        })
    }
}

impl WebAppChannelAdapter {
    async fn fan_out(
        &self,
        registrations: &[DeliveryRegistration],
        payload: &WebAppNotificationPayload,
        urgency: PushUrgency,
        egress: &dyn RestrictedEgress,
    ) -> FanOutTally {
        let mut tally = FanOutTally::default();
        let Ok(credential_handle) = SecretHandle::new(WEB_APP_VAPID_CREDENTIAL_HANDLE) else {
            tally.permanent = Some("VAPID credential handle is invalid".to_string());
            return tally;
        };
        for registration in registrations {
            let record = match parse_registration(registration) {
                Ok(record) => record,
                Err(error) => {
                    // A record this half cannot parse can never be delivered
                    // to, so it is pruned on the same path an expired
                    // endpoint takes rather than failing the whole send.
                    tracing::debug!(
                        target: "ironclaw::web_app",
                        error = %error,
                        "pruning an unparseable delivery registration"
                    );
                    tally.pruned.push(registration.registration_id.clone());
                    continue;
                }
            };
            let plan = match build_push_request(&record, payload, DEFAULT_TTL_SECONDS, urgency) {
                Ok(plan) => plan,
                Err(error) => {
                    tally.permanent = Some(format!("push request planning failed: {error}"));
                    continue;
                }
            };
            let request = RestrictedEgressRequest {
                method: NetworkMethod::Post,
                url: record.endpoint.as_str().to_string(),
                headers: plan
                    .headers
                    .iter()
                    .map(|(name, value)| ((*name).to_string(), value.clone()))
                    .collect(),
                body: Some(plan.body),
                credential: Some(credential_handle.clone()),
                body_credentials: Vec::new(),
            };
            match egress.send(request).await {
                Ok(response) => match response.status {
                    200..=299 => tally.accepted += 1,
                    404 | 410 => {
                        // The push service says this registration no longer
                        // exists. Report it; the host prunes.
                        tally.pruned.push(registration.registration_id.clone());
                    }
                    401 | 403 => {
                        tally.unauthorized = Some(format!(
                            "push service rejected VAPID authorization (status {})",
                            response.status
                        ));
                    }
                    413 => {
                        tally.permanent =
                            Some("push payload exceeds the push service limit".to_string());
                    }
                    429 => {
                        tally.retryable = Some("push service rate limited the send".to_string());
                    }
                    500..=599 => {
                        tally.retryable = Some(format!(
                            "push service unavailable (status {})",
                            response.status
                        ));
                    }
                    other => {
                        tally.permanent = Some(format!(
                            "push service rejected the request (status {other})"
                        ));
                    }
                },
                Err(RestrictedEgressError::AuthRequired { .. }) => {
                    tally.unauthorized = Some("VAPID key material is not available".to_string());
                }
                Err(error @ RestrictedEgressError::Transport { .. }) => {
                    tally.ambiguous = Some(error.to_string());
                }
                Err(error) => {
                    tally.permanent = Some(error.to_string());
                }
            }
        }
        tally
    }
}

fn fold_tally(tally: &FanOutTally) -> PartDeliveryOutcome {
    if let Some(reason) = &tally.ambiguous {
        return PartDeliveryOutcome::Ambiguous {
            reason: reason.clone(),
        };
    }
    if tally.accepted > 0 {
        if let Some(cause) = tally
            .unauthorized
            .as_deref()
            .or(tally.retryable.as_deref())
            .or(tally.permanent.as_deref())
        {
            // Partial fan-out settles Permanent (already-accepted browsers
            // must never be double-pushed by a retry) — but the durable
            // attempt record keeps the failing cause, so an operator can
            // tell a rate limit from a rejected key.
            return PartDeliveryOutcome::Permanent {
                reason: format!(
                    "browser push was accepted by only part of the enrolled client fanout ({cause})"
                ),
            };
        }
        // Push services return 201/202 with no durable message reference the
        // adapter is allowed to read (response headers are host-withheld),
        // so the honest evidence is Sent-without-ref: acceptance by the push
        // service, not device receipt.
        return PartDeliveryOutcome::Sent {
            vendor_message_ref: None,
        };
    }
    if let Some(reason) = &tally.unauthorized {
        return PartDeliveryOutcome::Unauthorized {
            reason: reason.clone(),
        };
    }
    if let Some(reason) = &tally.retryable {
        return PartDeliveryOutcome::Retryable {
            reason: reason.clone(),
        };
    }
    if let Some(reason) = &tally.permanent {
        return PartDeliveryOutcome::Permanent {
            reason: reason.clone(),
        };
    }
    if !tally.pruned.is_empty() {
        return PartDeliveryOutcome::Permanent {
            reason: "every enrolled client registration has expired".to_string(),
        };
    }
    PartDeliveryOutcome::Permanent {
        reason: "no push delivery was attempted".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::channel_adapter::{
        OutboundEnvelope, OutboundTarget, OutboundVisibility, RunCompletionNoticeView,
    };
    use ironclaw_extension_contracts::external::ExternalConversationRef;
    use std::sync::Mutex;

    #[derive(Default)]
    struct ScriptedEgress {
        requests: Mutex<Vec<RestrictedEgressRequest>>,
    }

    #[async_trait]
    impl RestrictedEgress for ScriptedEgress {
        async fn send(
            &self,
            request: RestrictedEgressRequest,
        ) -> Result<
            ironclaw_extension_contracts::tool_adapter::RestrictedEgressResponse,
            RestrictedEgressError,
        > {
            self.requests.lock().expect("lock").push(request);
            Ok(
                ironclaw_extension_contracts::tool_adapter::RestrictedEgressResponse {
                    status: 201,
                    body: Vec::new(),
                },
            )
        }
    }

    fn envelope(parts: Vec<OutboundPart>) -> OutboundEnvelope {
        OutboundEnvelope {
            target: OutboundTarget {
                conversation: ExternalConversationRef::new(None, "web-app", None, None)
                    .expect("conversation"),
                thread_anchor: None,
            },
            parts,
            reply_context: None,
            registrations: Vec::new(),
            visibility: OutboundVisibility::Public,
        }
    }

    fn completion_part() -> OutboundPart {
        OutboundPart::RunCompletion(Box::new(RunCompletionNoticeView {
            notice_id: "rcn-test".to_string(),
            thread_id: ironclaw_host_api::ids::ThreadId::new("thread-rc").expect("thread id"),
            opaque_thread_tag: "rct-test".to_string(),
            unread_count_for_thread: 2,
        }))
    }

    #[tokio::test]
    async fn mixed_completion_and_text_envelopes_are_rejected_wholesale() {
        // §7.10: one push per envelope. Reporting `Sent` for a text part the
        // push service never separately accepted would forge delivery
        // evidence, so the mixed shape fails as Permanent for every part.
        let egress = ScriptedEgress::default();
        let report = WebAppChannelAdapter::new()
            .deliver(
                envelope(vec![
                    completion_part(),
                    OutboundPart::Text("free text".to_string()),
                ]),
                &egress,
            )
            .await
            .expect("deliver reports");
        assert_eq!(report.parts.len(), 2);
        for part in &report.parts {
            assert!(
                matches!(
                    part,
                    PartDeliveryOutcome::Permanent { reason }
                        if reason.contains("cannot be mixed")
                ),
                "unexpected outcome: {part:?}"
            );
        }
        assert!(
            egress.requests.lock().expect("lock").is_empty(),
            "no push may be attempted for a rejected mixed envelope"
        );
    }

    #[tokio::test]
    async fn completion_part_alone_reports_no_enrollment_without_forged_send() {
        // With zero registrations the honest outcome is Permanent
        // ("no clients enrolled"), never a fabricated Sent.
        let egress = ScriptedEgress::default();
        let report = WebAppChannelAdapter::new()
            .deliver(envelope(vec![completion_part()]), &egress)
            .await
            .expect("deliver reports");
        assert_eq!(report.parts.len(), 1);
        assert!(
            matches!(
                &report.parts[0],
                PartDeliveryOutcome::Permanent { reason }
                    if reason.contains("enrolled")
            ),
            "unexpected outcome: {:?}",
            report.parts[0]
        );
        assert!(egress.requests.lock().expect("lock").is_empty());
    }
}
