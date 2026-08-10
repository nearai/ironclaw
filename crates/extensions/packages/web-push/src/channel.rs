//! The Web Push `ChannelAdapter`: render one envelope into one notification,
//! encrypt it per enrolled browser, and POST through restricted egress.

use async_trait::async_trait;
use ironclaw_extension_contracts::auth_prompt::render_channel_auth_prompt;
use ironclaw_extension_contracts::channel_adapter::{
    ChannelAdapter, ChannelError, DeliveryReport, InboundOutcome, OutboundEnvelope, OutboundPart,
    PartDeliveryOutcome, VerifiedInbound,
};
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest,
};
use ironclaw_host_api::action::NetworkMethod;
use ironclaw_host_api::ids::{InvocationId, SecretHandle};
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_web_push::{
    DEFAULT_TTL_SECONDS, PushSubscriptionRecord, PushUrgency, WEB_PUSH_VAPID_CREDENTIAL_HANDLE,
    WebPushNotificationPayload, WebPushRuntimeSlot, build_push_request, decode_web_push_target_ref,
};

/// Deep link a notification opens; the service worker resolves it against
/// the app origin.
const NOTIFICATION_URL: &str = "/automations";
const NOTIFICATION_TITLE: &str = "IronClaw";

/// Per-subscription send outcomes, folded into one part outcome.
#[derive(Default)]
struct FanOutTally {
    accepted: usize,
    pruned: usize,
    retryable: Option<String>,
    permanent: Option<String>,
    unauthorized: Option<String>,
}

pub struct WebPushChannelAdapter {
    runtime: WebPushRuntimeSlot,
}

impl WebPushChannelAdapter {
    pub fn new(runtime: WebPushRuntimeSlot) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl ChannelAdapter for WebPushChannelAdapter {
    /// Web Push has no inbound surface: enrollment happens in the
    /// authenticated WebUI session, never through channel ingress.
    fn inbound(&self, _request: VerifiedInbound<'_>) -> Result<InboundOutcome, ChannelError> {
        Err(ChannelError::Unsupported)
    }

    async fn deliver(
        &self,
        envelope: OutboundEnvelope,
        egress: &dyn RestrictedEgress,
    ) -> Result<DeliveryReport, ChannelError> {
        let conversation_id = envelope.target.conversation.conversation_id();
        let Some((tenant_id, user_id)) = decode_web_push_target_ref(conversation_id) else {
            return Err(ChannelError::Render {
                reason: "delivery target is not a web-push conversation".to_string(),
            });
        };
        let runtime = self
            .runtime
            .get()
            .map_err(|_| ChannelError::Configuration {
                reason: "web push runtime is not wired yet".to_string(),
            })?;
        let scope = ResourceScope {
            tenant_id,
            user_id,
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let subscriptions = runtime
            .subscriptions
            .list_subscriptions(&scope)
            .await
            .map_err(|error| ChannelError::Configuration {
                reason: format!("push subscription lookup failed: {error}"),
            })?;

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
            return Ok(DeliveryReport {
                parts: part_supported
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
            });
        }

        let deliverable_outcome = if subscriptions.is_empty() {
            PartDeliveryOutcome::Permanent {
                reason: "no browsers are enrolled for web push".to_string(),
            }
        } else {
            let payload = WebPushNotificationPayload::new(
                NOTIFICATION_TITLE,
                lines.join("\n\n"),
                NOTIFICATION_URL,
                None,
            );
            let tally = self
                .fan_out(&runtime, &scope, &subscriptions, &payload, urgency, egress)
                .await?;
            fold_tally(tally)
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
        })
    }
}

impl WebPushChannelAdapter {
    async fn fan_out(
        &self,
        runtime: &ironclaw_web_push::WebPushRuntime,
        scope: &ResourceScope,
        subscriptions: &[PushSubscriptionRecord],
        payload: &WebPushNotificationPayload,
        urgency: PushUrgency,
        egress: &dyn RestrictedEgress,
    ) -> Result<FanOutTally, ChannelError> {
        let credential_handle =
            SecretHandle::new(WEB_PUSH_VAPID_CREDENTIAL_HANDLE).map_err(|_| {
                ChannelError::Configuration {
                    reason: "VAPID credential handle is invalid".to_string(),
                }
            })?;
        let mut tally = FanOutTally::default();
        for subscription in subscriptions {
            let plan = match build_push_request(subscription, payload, DEFAULT_TTL_SECONDS, urgency)
            {
                Ok(plan) => plan,
                Err(error) => {
                    tally.permanent = Some(format!("push request planning failed: {error}"));
                    continue;
                }
            };
            let request = RestrictedEgressRequest {
                method: NetworkMethod::Post,
                url: subscription.endpoint.as_str().to_string(),
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
                        // The push service says this subscription no longer
                        // exists — prune it so future sends stop trying.
                        // Prune failure must not fail the delivery.
                        if let Err(error) = runtime
                            .subscriptions
                            .remove_subscription(scope, &subscription.endpoint)
                            .await
                        {
                            tracing::debug!(
                                target: "ironclaw::web_push",
                                error = %error,
                                "failed to prune an expired push subscription"
                            );
                        }
                        tally.pruned += 1;
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
                    tally.retryable = Some(error.to_string());
                }
                Err(error) => {
                    tally.permanent = Some(error.to_string());
                }
            }
        }
        Ok(tally)
    }
}

fn fold_tally(tally: FanOutTally) -> PartDeliveryOutcome {
    if tally.accepted > 0 {
        // Push services return 201/202 with no durable message reference the
        // adapter is allowed to read (response headers are host-withheld),
        // so the honest evidence is Sent-without-ref: acceptance by the push
        // service, not device receipt.
        return PartDeliveryOutcome::Sent {
            vendor_message_ref: None,
        };
    }
    if let Some(reason) = tally.unauthorized {
        return PartDeliveryOutcome::Unauthorized { reason };
    }
    if let Some(reason) = tally.retryable {
        return PartDeliveryOutcome::Retryable { reason };
    }
    if let Some(reason) = tally.permanent {
        return PartDeliveryOutcome::Permanent { reason };
    }
    if tally.pruned > 0 {
        return PartDeliveryOutcome::Permanent {
            reason: "every enrolled browser subscription has expired".to_string(),
        };
    }
    PartDeliveryOutcome::Permanent {
        reason: "no push delivery was attempted".to_string(),
    }
}
