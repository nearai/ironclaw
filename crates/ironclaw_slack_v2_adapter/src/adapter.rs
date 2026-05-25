//! Slack v2 ProductAdapter implementation.

use async_trait::async_trait;
use ironclaw_product_adapters::redaction::RedactedString;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, DeclaredEgressTarget,
    DeliveryStatus, EgressCredentialHandle, OutboundDeliverySink, ParsedProductInbound,
    ProductAdapter, ProductAdapterCapabilities, ProductAdapterError, ProductAdapterId,
    ProductOutboundEnvelope, ProductOutboundPayload, ProductRenderOutcome, ProductSurfaceKind,
    ProtocolAuthEvidence, ProtocolHttpEgress, ProtocolHttpEgressError,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};
use serde::Deserialize;

use crate::payload::{SLACK_API_HOST, SlackPayloadParseError, parse_slack_event};
use crate::render::{SlackRenderError, render_final_reply};

#[derive(Debug, Clone)]
pub struct SlackV2AdapterConfig {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub egress_credential_handle: EgressCredentialHandle,
    pub auth_requirement: AuthRequirement,
}

pub struct SlackV2Adapter {
    config: SlackV2AdapterConfig,
    capabilities: ProductAdapterCapabilities,
    declared_egress: Vec<DeclaredEgressTarget>,
}

impl SlackV2Adapter {
    pub fn new(config: SlackV2AdapterConfig) -> Self {
        let declared_egress = vec![DeclaredEgressTarget::new(
            DeclaredEgressHost::new(SLACK_API_HOST).expect("static Slack host valid"), // safety: compile-time const "slack.com" satisfies DeclaredEgressHost validation
            Some(config.egress_credential_handle.clone()),
        )];
        Self {
            config,
            capabilities: slack_default_capabilities(),
            declared_egress,
        }
    }

    pub fn config(&self) -> &SlackV2AdapterConfig {
        &self.config
    }
}

pub fn slack_default_capabilities() -> ProductAdapterCapabilities {
    ProductAdapterCapabilities::external_channel_default()
}

pub fn slack_request_signature_auth_requirement() -> AuthRequirement {
    AuthRequirement::RequestSignature {
        header_name: "X-Slack-Signature".into(),
        timestamp_header_name: Some("X-Slack-Request-Timestamp".into()),
    }
}

pub fn slack_declared_egress_hosts() -> Vec<DeclaredEgressHost> {
    vec![DeclaredEgressHost::new(SLACK_API_HOST).expect("static Slack host valid")] // safety: compile-time const "slack.com" satisfies DeclaredEgressHost validation
}

#[async_trait]
impl ProductAdapter for SlackV2Adapter {
    fn adapter_id(&self) -> &ProductAdapterId {
        &self.config.adapter_id
    }

    fn installation_id(&self) -> &AdapterInstallationId {
        &self.config.installation_id
    }

    fn surface_kind(&self) -> ProductSurfaceKind {
        ProductSurfaceKind::ExternalChannel
    }

    fn capabilities(&self) -> &ProductAdapterCapabilities {
        &self.capabilities
    }

    fn auth_requirement(&self) -> &AuthRequirement {
        &self.config.auth_requirement
    }

    fn declared_egress(&self) -> &[DeclaredEgressTarget] {
        &self.declared_egress
    }

    fn parse_inbound(
        &self,
        raw_payload: &[u8],
        auth_evidence: &ProtocolAuthEvidence,
    ) -> Result<ParsedProductInbound, ProductAdapterError> {
        parse_slack_event(raw_payload, auth_evidence, &self.config.installation_id).map_err(|err| {
            match err {
                SlackPayloadParseError::UnauthenticatedPayload => {
                    ProductAdapterError::Authentication(
                        ironclaw_product_adapters::ProtocolAuthFailure::Missing,
                    )
                }
                SlackPayloadParseError::InvalidJson { reason } => {
                    ProductAdapterError::MalformedInboundPayload {
                        reason: RedactedString::new(reason),
                    }
                }
                SlackPayloadParseError::InvalidExternalRef { kind, reason } => {
                    ProductAdapterError::InvalidIdentifier { kind, reason }
                }
            }
        })
    }

    async fn render_outbound(
        &self,
        envelope: ProductOutboundEnvelope,
        egress: &dyn ProtocolHttpEgress,
        delivery_sink: &dyn OutboundDeliverySink,
    ) -> Result<ProductRenderOutcome, ProductAdapterError> {
        if envelope.adapter_id != self.config.adapter_id {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "envelope.adapter_id",
                reason: format!(
                    "envelope adapter_id `{}` does not match this adapter `{}`",
                    envelope.adapter_id.as_str(),
                    self.config.adapter_id.as_str(),
                ),
            });
        }
        if envelope.installation_id != self.config.installation_id {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "envelope.installation_id",
                reason: format!(
                    "envelope installation_id `{}` does not match this installation `{}`",
                    envelope.installation_id.as_str(),
                    self.config.installation_id.as_str(),
                ),
            });
        }

        let attempt_id = envelope.delivery_attempt_id;
        let target_binding = envelope.target.reply_target_binding_ref.clone();
        let run_id = run_id_for_payload(&envelope.payload);

        let request = match envelope.payload {
            ProductOutboundPayload::FinalReply(view) => {
                match render_final_reply(
                    &envelope.target,
                    &view,
                    self.config.egress_credential_handle.clone(),
                ) {
                    Ok(req) => req,
                    Err(render_err) => {
                        record_status(
                            delivery_sink,
                            DeliveryStatus::FailedPermanent {
                                attempt_id,
                                target: target_binding.clone(),
                                run_id,
                                reason: RedactedString::new(render_err.to_string()),
                            },
                        )
                        .await;
                        return Err(map_render_error(render_err));
                    }
                }
            }
            ProductOutboundPayload::GatePrompt(_) | ProductOutboundPayload::AuthPrompt(_) => {
                record_status(
                    delivery_sink,
                    DeliveryStatus::Deferred {
                        attempt_id,
                        target: target_binding,
                        run_id,
                        reason: RedactedString::new("gate/auth prompts deferred to #3094 on Slack"),
                    },
                )
                .await;
                return Ok(ProductRenderOutcome::Deferred);
            }
            ProductOutboundPayload::Progress(_)
            | ProductOutboundPayload::CapabilityActivity(_)
            | ProductOutboundPayload::CapabilityDisplayPreview(_)
            | ProductOutboundPayload::ProjectionSnapshot { .. }
            | ProductOutboundPayload::ProjectionUpdate { .. }
            | ProductOutboundPayload::KeepAlive => {
                record_status(
                    delivery_sink,
                    DeliveryStatus::Deferred {
                        attempt_id,
                        target: target_binding,
                        run_id,
                        reason: RedactedString::new(
                            "slack first slice only renders final reply envelopes",
                        ),
                    },
                )
                .await;
                return Ok(ProductRenderOutcome::Deferred);
            }
        };

        let response = match egress.send(request).await {
            Ok(response) => response,
            Err(egress_err) => {
                record_status(
                    delivery_sink,
                    egress_err_to_delivery_status(
                        &egress_err,
                        attempt_id,
                        target_binding.clone(),
                        run_id,
                    ),
                )
                .await;
                return Err(map_egress_error(egress_err));
            }
        };

        if !(200..300).contains(&response.status()) {
            let reason = RedactedString::new(format!(
                "slack web api returned status {}",
                response.status()
            ));
            if response.status() >= 500 || response.status() == 429 {
                record_status(
                    delivery_sink,
                    DeliveryStatus::FailedRetryable {
                        attempt_id,
                        target: target_binding.clone(),
                        run_id,
                        reason: reason.clone(),
                    },
                )
                .await;
                return Err(ProductAdapterError::EgressTransient { reason });
            }
            if response.status() == 401 || response.status() == 403 {
                record_status(
                    delivery_sink,
                    DeliveryStatus::FailedUnauthorized {
                        attempt_id,
                        target: target_binding.clone(),
                        run_id,
                        reason: reason.clone(),
                    },
                )
                .await;
            } else {
                record_status(
                    delivery_sink,
                    DeliveryStatus::FailedPermanent {
                        attempt_id,
                        target: target_binding.clone(),
                        run_id,
                        reason: reason.clone(),
                    },
                )
                .await;
            }
            return Err(ProductAdapterError::EgressDenied { reason });
        }

        if let Err(slack_err) = slack_post_message_result(response.body()) {
            record_status(
                delivery_sink,
                DeliveryStatus::FailedPermanent {
                    attempt_id,
                    target: target_binding.clone(),
                    run_id,
                    reason: RedactedString::new(slack_err.clone()),
                },
            )
            .await;
            return Err(ProductAdapterError::EgressDenied {
                reason: RedactedString::new(slack_err),
            });
        }

        record_status(
            delivery_sink,
            DeliveryStatus::Delivered {
                attempt_id,
                target: target_binding,
                run_id,
            },
        )
        .await;
        Ok(ProductRenderOutcome::DeliveryRecorded)
    }
}

fn run_id_for_payload(payload: &ProductOutboundPayload) -> Option<TurnRunId> {
    match payload {
        ProductOutboundPayload::FinalReply(view) => Some(view.turn_run_id),
        ProductOutboundPayload::Progress(view) => Some(view.turn_run_id),
        ProductOutboundPayload::GatePrompt(view) => Some(view.turn_run_id),
        ProductOutboundPayload::AuthPrompt(view) => Some(view.turn_run_id),
        ProductOutboundPayload::CapabilityActivity(_)
        | ProductOutboundPayload::CapabilityDisplayPreview(_)
        | ProductOutboundPayload::ProjectionSnapshot { .. }
        | ProductOutboundPayload::ProjectionUpdate { .. }
        | ProductOutboundPayload::KeepAlive => None,
    }
}

async fn record_status(sink: &dyn OutboundDeliverySink, status: DeliveryStatus) {
    sink.record(status).await;
}

fn egress_err_to_delivery_status(
    err: &ProtocolHttpEgressError,
    attempt_id: ironclaw_product_adapters::DeliveryAttemptId,
    target: ReplyTargetBindingRef,
    run_id: Option<TurnRunId>,
) -> DeliveryStatus {
    let reason = RedactedString::new(err.to_string());
    match err {
        ProtocolHttpEgressError::Timeout
        | ProtocolHttpEgressError::Network(_)
        | ProtocolHttpEgressError::LeakDetected => DeliveryStatus::FailedRetryable {
            attempt_id,
            target,
            run_id,
            reason,
        },
        ProtocolHttpEgressError::UnknownCredentialHandle { .. }
        | ProtocolHttpEgressError::UnauthorizedCredentialHandle { .. } => {
            DeliveryStatus::FailedUnauthorized {
                attempt_id,
                target,
                run_id,
                reason,
            }
        }
        ProtocolHttpEgressError::UndeclaredHost { .. }
        | ProtocolHttpEgressError::PolicyDenied { .. } => DeliveryStatus::FailedPermanent {
            attempt_id,
            target,
            run_id,
            reason,
        },
    }
}

fn map_render_error(err: SlackRenderError) -> ProductAdapterError {
    match err {
        SlackRenderError::InvalidReplyTarget { .. } => ProductAdapterError::InvalidIdentifier {
            kind: "reply_target",
            reason: err.to_string(),
        },
        SlackRenderError::Serialization { .. } => ProductAdapterError::Internal {
            detail: RedactedString::new(err.to_string()),
        },
    }
}

fn map_egress_error(err: ProtocolHttpEgressError) -> ProductAdapterError {
    let reason = RedactedString::new(err.to_string());
    match err {
        ProtocolHttpEgressError::Timeout
        | ProtocolHttpEgressError::Network(_)
        | ProtocolHttpEgressError::LeakDetected => ProductAdapterError::EgressTransient { reason },
        ProtocolHttpEgressError::UndeclaredHost { .. }
        | ProtocolHttpEgressError::UnknownCredentialHandle { .. }
        | ProtocolHttpEgressError::UnauthorizedCredentialHandle { .. }
        | ProtocolHttpEgressError::PolicyDenied { .. } => {
            ProductAdapterError::EgressDenied { reason }
        }
    }
}

fn slack_post_message_result(body: &[u8]) -> Result<(), String> {
    let parsed: SlackPostMessageResponse = serde_json::from_slice(body)
        .map_err(|err| format!("Slack chat.postMessage response was not valid JSON: {err}"))?;
    if parsed.ok {
        Ok(())
    } else {
        Err(format!(
            "Slack rejected chat.postMessage ({})",
            parsed.error.unwrap_or_else(|| "unknown_error".to_string())
        ))
    }
}

#[derive(Debug, Deserialize)]
struct SlackPostMessageResponse {
    ok: bool,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_product_adapters::{
        DeliveryStatus, ExternalConversationRef, FakeOutboundDeliverySink, FakeProtocolHttpEgress,
        FinalReplyView, ProductOutboundTarget,
    };
    use ironclaw_turns::ReplyTargetBindingRef;

    fn config() -> SlackV2AdapterConfig {
        SlackV2AdapterConfig {
            adapter_id: ProductAdapterId::new("slack_v2").expect("valid"),
            installation_id: AdapterInstallationId::new("slack_install_beta").expect("valid"),
            egress_credential_handle: EgressCredentialHandle::new("slack_bot_token")
                .expect("valid"),
            auth_requirement: slack_request_signature_auth_requirement(),
        }
    }

    fn envelope(payload: ProductOutboundPayload) -> ProductOutboundEnvelope {
        ProductOutboundEnvelope {
            adapter_id: ProductAdapterId::new("slack_v2").expect("valid"),
            installation_id: AdapterInstallationId::new("slack_install_beta").expect("valid"),
            target: ProductOutboundTarget::new(
                ReplyTargetBindingRef::new("reply:slack-test").expect("valid"),
                ExternalConversationRef::new(
                    Some("T123"),
                    "C123",
                    Some("1710000000.000001"),
                    Some("1710000000.000002"),
                )
                .expect("valid"),
                None,
            ),
            projection_cursor: ironclaw_product_adapters::ProjectionCursor::new("cursor:slack")
                .expect("valid"),
            delivery_attempt_id: uuid::Uuid::new_v4(),
            payload,
        }
    }

    #[test]
    fn metadata_declares_signature_auth_and_paired_egress() {
        let adapter = SlackV2Adapter::new(config());

        assert_eq!(adapter.surface_kind(), ProductSurfaceKind::ExternalChannel);
        assert_eq!(
            adapter.auth_requirement(),
            &slack_request_signature_auth_requirement()
        );
        assert_eq!(adapter.declared_egress().len(), 1);
        assert_eq!(adapter.declared_egress()[0].host.as_str(), SLACK_API_HOST);
        assert_eq!(
            adapter.declared_egress()[0]
                .credential_handle
                .as_ref()
                .map(EgressCredentialHandle::as_str),
            Some("slack_bot_token")
        );
    }

    #[tokio::test]
    async fn final_reply_renders_and_records_delivery() {
        let adapter = SlackV2Adapter::new(config());
        let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
        egress.allow_credential_handle("slack_bot_token");
        let sink = FakeOutboundDeliverySink::new();
        let run_id = TurnRunId::new();
        let payload = ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: run_id,
            text: "hello Slack".to_string(),
            generated_at: Utc::now(),
        });

        let outcome = adapter
            .render_outbound(envelope(payload), &egress, &sink)
            .await
            .expect("render outbound");

        assert_eq!(outcome, ProductRenderOutcome::DeliveryRecorded);
        let calls = egress.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].host, SLACK_API_HOST);
        assert_eq!(calls[0].path, "/api/chat.postMessage");
        assert_eq!(
            calls[0].credential_handle.as_deref(),
            Some("slack_bot_token")
        );
        let body: serde_json::Value = serde_json::from_slice(&calls[0].body).expect("body json");
        assert_eq!(body["channel"], "C123");
        assert_eq!(body["text"], "hello Slack");
        assert_eq!(body["thread_ts"], "1710000000.000001");
        assert!(matches!(
            sink.statuses().as_slice(),
            [DeliveryStatus::Delivered { run_id: Some(delivered), .. }] if delivered == &run_id
        ));
    }

    #[tokio::test]
    async fn gate_prompts_are_deferred_to_approval_service() {
        let adapter = SlackV2Adapter::new(config());
        let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
        let sink = FakeOutboundDeliverySink::new();
        let payload =
            ProductOutboundPayload::AuthPrompt(ironclaw_product_adapters::AuthPromptView {
                turn_run_id: TurnRunId::new(),
                auth_request_ref: "auth-1".to_string(),
                headline: "Auth required".to_string(),
                body: "Open WebUI".to_string(),
            });

        let outcome = adapter
            .render_outbound(envelope(payload), &egress, &sink)
            .await
            .expect("deferred");

        assert_eq!(outcome, ProductRenderOutcome::Deferred);
        assert!(egress.calls().is_empty());
        assert!(matches!(
            sink.statuses().as_slice(),
            [DeliveryStatus::Deferred { reason, .. }] if reason.to_string() == RedactedString::placeholder()
        ));
    }

    #[tokio::test]
    async fn slack_ok_false_records_permanent_failure_without_token_leak() {
        let adapter = SlackV2Adapter::new(config());
        let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            SLACK_API_HOST,
            Ok(ironclaw_product_adapters::EgressResponse::new(
                200,
                br#"{"ok":false,"error":"missing_scope"}"#.to_vec(),
            )),
        );
        let sink = FakeOutboundDeliverySink::new();
        let payload = ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "hello Slack".to_string(),
            generated_at: Utc::now(),
        });

        let err = adapter
            .render_outbound(envelope(payload), &egress, &sink)
            .await
            .expect_err("Slack ok=false must fail");

        let rendered = err.to_string();
        assert!(rendered.contains(RedactedString::placeholder()));
        assert!(!rendered.contains("missing_scope"));
        assert!(!rendered.contains("xoxb"));
        assert!(matches!(
            sink.statuses().as_slice(),
            [DeliveryStatus::FailedPermanent { reason, .. }] if reason.to_string() == RedactedString::placeholder()
        ));
    }
}
