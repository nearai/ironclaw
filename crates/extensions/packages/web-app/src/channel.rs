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
    DEFAULT_TTL_SECONDS, PushEndpoint, PushSubscriptionKeys, PushSubscriptionRecord, PushUrgency,
    WEB_APP_VAPID_CREDENTIAL_HANDLE, WebAppError, WebAppNotificationPayload, build_push_request,
};

/// Deep link a notification opens; the service worker resolves it against
/// the app origin.
const NOTIFICATION_URL: &str = "/automations";
const NOTIFICATION_TITLE: &str = "IronClaw";

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
/// This is where the opaque half of a registration is interpreted, and the
/// only place it ever is (design §8: "everything else validates where it is
/// used"). A malformed document fails **this** registration — not the
/// delivery, and not the other registrations — and is reported for pruning on
/// the same path an expired endpoint takes.
fn parse_registration(
    registration: &DeliveryRegistration,
) -> Result<PushSubscriptionRecord, WebAppError> {
    #[derive(serde::Deserialize)]
    struct RegistrationDocument {
        keys: RegistrationKeys,
        #[serde(default)]
        user_agent: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct RegistrationKeys {
        p256dh: String,
        auth: String,
    }

    let endpoint = PushEndpoint::new(registration.endpoint.clone())?;
    let document: RegistrationDocument =
        serde_json::from_str(&registration.document).map_err(|error| {
            WebAppError::InvalidSubscription {
                reason: format!("registration document is not a push subscription: {error}"),
            }
        })?;
    let keys = PushSubscriptionKeys::new(document.keys.p256dh, document.keys.auth)?;
    Ok(PushSubscriptionRecord::new(
        endpoint,
        keys,
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
            let payload = WebAppNotificationPayload::new(
                NOTIFICATION_TITLE,
                lines.join("\n\n"),
                NOTIFICATION_URL,
                None,
            );
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
