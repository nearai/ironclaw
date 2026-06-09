//! Slack v2 ProductAdapter implementation.

use async_trait::async_trait;
use ironclaw_product_adapters::redaction::RedactedString;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, DeclaredEgressTarget,
    DeliveryStatus, EgressCredentialHandle, EgressRequest, OutboundDeliverySink,
    ParsedProductInbound, ProductAdapter, ProductAdapterCapabilities, ProductAdapterError,
    ProductAdapterId, ProductCapabilityFlag, ProductOutboundEnvelope, ProductOutboundPayload,
    ProductOutboundTarget, ProductRenderOutcome, ProductSurfaceKind, ProtocolAuthEvidence,
    ProtocolHttpEgress, ProtocolHttpEgressError,
};
use ironclaw_turns::TurnRunId;
use serde::Deserialize;

use crate::payload::{SLACK_API_HOST, SlackPayloadParseError, parse_slack_event};
use crate::render::{SlackRenderError, render_auth_prompt, render_final_reply, render_gate_prompt};

/// Timeout for recording a delivery status to the sink.
/// Guards against a hung sink blocking the delivery hot path indefinitely.
const SINK_RECORD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Maximum accepted byte length for a Slack `chat.postMessage` response body.
/// Protects against WAF/proxy responses (e.g. large HTML error pages on 200 OK)
/// causing full-allocation and O(n) deserialization on the delivery hot path.
const MAX_SLACK_RESPONSE_BYTES: usize = 64 * 1024; // 64 KB

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
        .without(ProductCapabilityFlag::InboundCommands)
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
        let run_id = payload_run_id(&envelope.payload);

        let request = match render_supported_payload(
            &envelope.target,
            &envelope.payload,
            self.config.egress_credential_handle.clone(),
        ) {
            Ok(Some(request)) => request,
            Ok(None) => {
                record_status(
                    delivery_sink,
                    DeliveryStatus::Deferred {
                        attempt_id,
                        target: target_binding,
                        run_id,
                        reason: RedactedString::new(
                            "slack first slice only renders final reply and prompt envelopes",
                        ),
                    },
                )
                .await;
                return Ok(ProductRenderOutcome::Deferred);
            }
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
        };

        let response = match egress.send(request).await {
            Ok(response) => response,
            Err(egress_err) => {
                let failure = SlackDeliveryFailureKind::from_egress_error(&egress_err);
                let reason = RedactedString::new(egress_err.to_string());
                let status = match failure {
                    SlackDeliveryFailureKind::Retryable => DeliveryStatus::FailedRetryable {
                        attempt_id,
                        target: target_binding.clone(),
                        run_id,
                        reason: reason.clone(),
                    },
                    SlackDeliveryFailureKind::Unauthorized => DeliveryStatus::FailedUnauthorized {
                        attempt_id,
                        target: target_binding.clone(),
                        run_id,
                        reason: reason.clone(),
                    },
                    SlackDeliveryFailureKind::Permanent => DeliveryStatus::FailedPermanent {
                        attempt_id,
                        target: target_binding.clone(),
                        run_id,
                        reason: reason.clone(),
                    },
                };
                record_status(delivery_sink, status).await;
                return Err(failure.to_adapter_error(reason));
            }
        };

        if !(200..300).contains(&response.status()) {
            let reason = RedactedString::new(format!(
                "slack web api returned status {}",
                response.status()
            ));
            let failure = SlackDeliveryFailureKind::from_http_status(response.status());
            let status = match failure {
                SlackDeliveryFailureKind::Retryable => DeliveryStatus::FailedRetryable {
                    attempt_id,
                    target: target_binding.clone(),
                    run_id,
                    reason: reason.clone(),
                },
                SlackDeliveryFailureKind::Unauthorized => DeliveryStatus::FailedUnauthorized {
                    attempt_id,
                    target: target_binding.clone(),
                    run_id,
                    reason: reason.clone(),
                },
                SlackDeliveryFailureKind::Permanent => DeliveryStatus::FailedPermanent {
                    attempt_id,
                    target: target_binding.clone(),
                    run_id,
                    reason: reason.clone(),
                },
            };
            record_status(delivery_sink, status).await;
            return Err(failure.to_adapter_error(reason));
        }

        if let Err(slack_err) = slack_post_message_result(response.body()) {
            let reason = slack_err.reason; // already RedactedString — wrapped at construction
            match slack_err.kind {
                SlackDeliveryFailureKind::Unauthorized => {
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
                    return Err(ProductAdapterError::EgressDenied { reason });
                }
                SlackDeliveryFailureKind::Retryable => {
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
                SlackDeliveryFailureKind::Permanent => {
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
                    return Err(ProductAdapterError::EgressDenied { reason });
                }
            }
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

fn render_supported_payload(
    target: &ProductOutboundTarget,
    payload: &ProductOutboundPayload,
    credential_handle: EgressCredentialHandle,
) -> Result<Option<EgressRequest>, SlackRenderError> {
    match payload {
        ProductOutboundPayload::FinalReply(view) => {
            render_final_reply(target, view, credential_handle).map(Some)
        }
        ProductOutboundPayload::GatePrompt(view) => {
            render_gate_prompt(target, view, credential_handle).map(Some)
        }
        ProductOutboundPayload::AuthPrompt(view) => {
            render_auth_prompt(target, view, credential_handle).map(Some)
        }
        ProductOutboundPayload::Progress(_)
        | ProductOutboundPayload::CapabilityActivity(_)
        | ProductOutboundPayload::CapabilityDisplayPreview(_)
        | ProductOutboundPayload::ProjectionSnapshot { .. }
        | ProductOutboundPayload::ProjectionUpdate { .. }
        | ProductOutboundPayload::KeepAlive => Ok(None),
    }
}

async fn record_status(sink: &dyn OutboundDeliverySink, status: DeliveryStatus) {
    // silent-ok: sink timeout guard — hung sink must not block the delivery hot path.
    let _ = tokio::time::timeout(SINK_RECORD_TIMEOUT, sink.record(status)).await;
}

/// Extracts the `TurnRunId` from an outbound payload without consuming it.
/// Centralises the borrow-match so the consuming match that dispatches to
/// protocol-specific rendering does not need to replicate this mapping.
fn payload_run_id(payload: &ProductOutboundPayload) -> Option<TurnRunId> {
    match payload {
        ProductOutboundPayload::FinalReply(v) => Some(v.turn_run_id),
        ProductOutboundPayload::Progress(v) => Some(v.turn_run_id),
        ProductOutboundPayload::GatePrompt(v) => Some(v.turn_run_id),
        ProductOutboundPayload::AuthPrompt(v) => Some(v.turn_run_id),
        ProductOutboundPayload::CapabilityActivity(_)
        | ProductOutboundPayload::CapabilityDisplayPreview(_)
        | ProductOutboundPayload::ProjectionSnapshot { .. }
        | ProductOutboundPayload::ProjectionUpdate { .. }
        | ProductOutboundPayload::KeepAlive => None,
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

fn slack_post_message_result(body: &[u8]) -> Result<(), SlackPostMessageFailure> {
    if body.len() > MAX_SLACK_RESPONSE_BYTES {
        return Err(SlackPostMessageFailure::permanent(
            "response body too large",
        ));
    }
    let parsed: SlackPostMessageResponse = serde_json::from_slice(body).map_err(|err| {
        // A truncated/empty body from a proxy/LB timeout is a transient infra
        // condition; treat as retryable rather than permanently abandoning.
        SlackPostMessageFailure {
            reason: RedactedString::new(format!(
                "Slack chat.postMessage response was not valid JSON: {err}"
            )),
            kind: SlackDeliveryFailureKind::Retryable,
        }
    })?;
    if parsed.ok {
        Ok(())
    } else {
        let error = parsed.error.unwrap_or_else(|| "unknown_error".to_string());
        Err(SlackPostMessageFailure {
            reason: RedactedString::new(format!("Slack rejected chat.postMessage ({})", error)),
            kind: slack_error_kind(&error),
        })
    }
}

#[derive(Debug, Deserialize)]
struct SlackPostMessageResponse {
    ok: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlackDeliveryFailureKind {
    Unauthorized,
    Retryable,
    Permanent,
}

impl SlackDeliveryFailureKind {
    fn from_egress_error(err: &ProtocolHttpEgressError) -> Self {
        match err {
            ProtocolHttpEgressError::Timeout
            | ProtocolHttpEgressError::Network(_)
            | ProtocolHttpEgressError::LeakDetected => Self::Retryable,
            ProtocolHttpEgressError::UnknownCredentialHandle { .. }
            | ProtocolHttpEgressError::UnauthorizedCredentialHandle { .. } => Self::Unauthorized,
            ProtocolHttpEgressError::UndeclaredHost { .. }
            | ProtocolHttpEgressError::PolicyDenied { .. } => Self::Permanent,
        }
    }

    fn from_http_status(status: u16) -> Self {
        if status >= 500 || status == 429 {
            Self::Retryable
        } else if status == 401 || status == 403 {
            Self::Unauthorized
        } else {
            Self::Permanent
        }
    }

    fn to_adapter_error(self, reason: RedactedString) -> ProductAdapterError {
        match self {
            Self::Retryable => ProductAdapterError::EgressTransient { reason },
            Self::Unauthorized | Self::Permanent => ProductAdapterError::EgressDenied { reason },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SlackPostMessageFailure {
    reason: RedactedString,
    kind: SlackDeliveryFailureKind,
}

impl SlackPostMessageFailure {
    fn permanent(reason: impl Into<String>) -> Self {
        Self {
            reason: RedactedString::new(reason.into()),
            kind: SlackDeliveryFailureKind::Permanent,
        }
    }
}

fn slack_error_kind(error: &str) -> SlackDeliveryFailureKind {
    match error {
        "not_authed"
        | "invalid_auth"
        | "account_inactive"
        | "token_revoked"
        | "missing_scope"
        | "no_permission"
        | "is_bot"
        | "not_allowed_token_type" => SlackDeliveryFailureKind::Unauthorized,
        "fatal_error"
        | "internal_error"
        | "service_unavailable"
        | "request_timeout"
        | "ratelimited" => SlackDeliveryFailureKind::Retryable,
        _ => SlackDeliveryFailureKind::Permanent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ironclaw_product_adapters::auth::mark_request_signature_verified;
    use ironclaw_product_adapters::{
        DeliveryStatus, ExternalConversationRef, FakeOutboundDeliverySink, FakeProtocolHttpEgress,
        FinalReplyView, ProductOutboundTarget,
    };
    use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId};

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

    fn final_reply_payload(text: &str) -> ProductOutboundPayload {
        ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: text.to_string(),
            generated_at: Utc::now(),
        })
    }

    #[test]
    fn metadata_declares_signature_auth_and_paired_egress() {
        let adapter = SlackV2Adapter::new(config());

        assert_eq!(adapter.surface_kind(), ProductSurfaceKind::ExternalChannel);
        assert_eq!(
            adapter.auth_requirement(),
            &slack_request_signature_auth_requirement()
        );
        assert!(
            adapter
                .capabilities()
                .contains(ProductCapabilityFlag::InboundMessages)
        );
        assert!(
            adapter
                .capabilities()
                .contains(ProductCapabilityFlag::InboundAttachments)
        );
        assert!(
            !adapter
                .capabilities()
                .contains(ProductCapabilityFlag::InboundCommands)
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

    #[test]
    fn parse_inbound_maps_slack_parse_errors() {
        let adapter = SlackV2Adapter::new(config());

        let malformed = adapter
            .parse_inbound(
                br#"{"type":"event_callback","event_id":]"#,
                &mark_request_signature_verified(
                    "X-Slack-Signature",
                    Some("X-Slack-Request-Timestamp".to_string()),
                    "T123",
                ),
            )
            .expect_err("malformed JSON must fail at adapter boundary");
        assert!(matches!(
            malformed,
            ProductAdapterError::MalformedInboundPayload { .. }
        ));

        let unauthenticated = adapter
            .parse_inbound(
                br#"{"type":"event_callback","event_id":"EvNoAuth"}"#,
                &ProtocolAuthEvidence::failed(
                    ironclaw_product_adapters::ProtocolAuthFailure::Missing,
                ),
            )
            .expect_err("unverified auth evidence must fail at adapter boundary");
        assert!(matches!(
            unauthenticated,
            ProductAdapterError::Authentication(
                ironclaw_product_adapters::ProtocolAuthFailure::Missing
            )
        ));
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
        assert_eq!(body["mrkdwn"], true);
        assert_eq!(body["thread_ts"], "1710000000.000001");
        assert!(matches!(
            sink.statuses().as_slice(),
            [DeliveryStatus::Delivered { run_id: Some(delivered), .. }] if delivered == &run_id
        ));
    }

    #[tokio::test]
    async fn render_outbound_rejects_mismatched_envelope_ids_without_egress() {
        let adapter = SlackV2Adapter::new(config());
        let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
        egress.allow_credential_handle("slack_bot_token");
        let sink = FakeOutboundDeliverySink::new();

        let mut wrong_adapter = envelope(final_reply_payload("wrong adapter"));
        wrong_adapter.adapter_id = ProductAdapterId::new("other_slack").expect("valid");
        let err = adapter
            .render_outbound(wrong_adapter, &egress, &sink)
            .await
            .expect_err("wrong adapter id must fail");
        assert!(matches!(
            err,
            ProductAdapterError::InvalidIdentifier {
                kind: "envelope.adapter_id",
                ..
            }
        ));

        let mut wrong_installation = envelope(final_reply_payload("wrong installation"));
        wrong_installation.installation_id =
            AdapterInstallationId::new("other_installation").expect("valid");
        let err = adapter
            .render_outbound(wrong_installation, &egress, &sink)
            .await
            .expect_err("wrong installation id must fail");
        assert!(matches!(
            err,
            ProductAdapterError::InvalidIdentifier {
                kind: "envelope.installation_id",
                ..
            }
        ));

        assert!(egress.calls().is_empty());
        assert!(sink.statuses().is_empty());
    }

    #[tokio::test]
    async fn auth_prompts_render_to_slack_http_egress() {
        let adapter = SlackV2Adapter::new(config());
        let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
        egress.allow_credential_handle("slack_bot_token");
        let sink = FakeOutboundDeliverySink::new();
        let run_id = TurnRunId::new();
        let payload =
            ProductOutboundPayload::AuthPrompt(ironclaw_product_adapters::AuthPromptView {
                turn_run_id: run_id,
                auth_request_ref: "auth-1".to_string(),
                headline: "Auth required".to_string(),
                body: "Open WebUI".to_string(),
                challenge_kind: None,
                provider: None,
                account_label: None,
                authorization_url: None,
                expires_at: None,
            });

        let outcome = adapter
            .render_outbound(envelope(payload), &egress, &sink)
            .await
            .expect("auth prompt renders");

        assert_eq!(outcome, ProductRenderOutcome::DeliveryRecorded);
        let calls = egress.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].path, "/api/chat.postMessage");
        let body: serde_json::Value = serde_json::from_slice(&calls[0].body).expect("body json");
        assert_eq!(body["channel"], "C123");
        assert_eq!(
            body["text"],
            "Auth required\n\nOpen WebUI\n\nMention this app in this Slack thread with `auth deny auth-1` to cancel this blocked run."
        );
        assert_eq!(body["thread_ts"], "1710000000.000001");
        assert!(matches!(
            sink.statuses().as_slice(),
            [DeliveryStatus::Delivered { run_id: Some(delivered), .. }] if delivered == &run_id
        ));
    }

    #[tokio::test]
    async fn render_outbound_records_status_for_slack_http_failures() {
        for (status, expected) in [
            (429, "retryable"),
            (500, "retryable"),
            (401, "unauthorized"),
            (403, "unauthorized"),
            (400, "permanent"),
        ] {
            let adapter = SlackV2Adapter::new(config());
            let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
            egress.allow_credential_handle("slack_bot_token");
            egress.program_response(
                SLACK_API_HOST,
                Ok(ironclaw_product_adapters::EgressResponse::new(
                    status,
                    br#"{"ok":false,"error":"http_error"}"#.to_vec(),
                )),
            );
            let sink = FakeOutboundDeliverySink::new();

            let err = adapter
                .render_outbound(envelope(final_reply_payload("hello Slack")), &egress, &sink)
                .await
                .expect_err("HTTP failure status must fail");

            match expected {
                "retryable" => {
                    assert!(matches!(err, ProductAdapterError::EgressTransient { .. }));
                    assert!(matches!(
                        sink.statuses().as_slice(),
                        [DeliveryStatus::FailedRetryable { .. }]
                    ));
                }
                "unauthorized" => {
                    assert!(matches!(err, ProductAdapterError::EgressDenied { .. }));
                    assert!(matches!(
                        sink.statuses().as_slice(),
                        [DeliveryStatus::FailedUnauthorized { .. }]
                    ));
                }
                "permanent" => {
                    assert!(matches!(err, ProductAdapterError::EgressDenied { .. }));
                    assert!(matches!(
                        sink.statuses().as_slice(),
                        [DeliveryStatus::FailedPermanent { .. }]
                    ));
                }
                other => panic!("unknown expectation {other}"),
            }
        }
    }

    #[tokio::test]
    async fn slack_ok_false_auth_failure_records_unauthorized_without_token_leak() {
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

        let err = adapter
            .render_outbound(envelope(final_reply_payload("hello Slack")), &egress, &sink)
            .await
            .expect_err("Slack ok=false must fail");

        let rendered = err.to_string();
        assert!(rendered.contains(RedactedString::placeholder()));
        assert!(!rendered.contains("missing_scope"));
        assert!(!rendered.contains("xoxb"));
        assert!(matches!(
            sink.statuses().as_slice(),
            [DeliveryStatus::FailedUnauthorized { reason, .. }] if reason.to_string() == RedactedString::placeholder()
        ));
    }

    #[tokio::test]
    async fn slack_ok_false_retryable_error_records_retryable() {
        let adapter = SlackV2Adapter::new(config());
        let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
        egress.allow_credential_handle("slack_bot_token");
        egress.program_response(
            SLACK_API_HOST,
            Ok(ironclaw_product_adapters::EgressResponse::new(
                200,
                br#"{"ok":false,"error":"internal_error"}"#.to_vec(),
            )),
        );
        let sink = FakeOutboundDeliverySink::new();

        let err = adapter
            .render_outbound(envelope(final_reply_payload("hello Slack")), &egress, &sink)
            .await
            .expect_err("Slack retryable ok=false must fail");

        assert!(matches!(err, ProductAdapterError::EgressTransient { .. }));
        assert!(matches!(
            sink.statuses().as_slice(),
            [DeliveryStatus::FailedRetryable { .. }]
        ));
    }

    // ── High/Tests: render error paths ──────────────────────────────────────────

    #[tokio::test]
    async fn render_outbound_final_reply_render_error_records_failed_permanent() {
        let adapter = SlackV2Adapter::new(config());
        let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
        egress.allow_credential_handle("slack_bot_token");
        let sink = FakeOutboundDeliverySink::new();
        let run_id = TurnRunId::new();

        // "not-a-channel" fails looks_like_slack_id in render_final_reply.
        let mut bad_target = envelope(ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: run_id,
            text: "hello".to_string(),
            generated_at: Utc::now(),
        }));
        bad_target.target = ProductOutboundTarget::new(
            ReplyTargetBindingRef::new("reply:bad").expect("valid"),
            ExternalConversationRef::new(Some("T123"), "not-a-channel", None, None).expect("valid"),
            None,
        );

        let err = adapter
            .render_outbound(bad_target, &egress, &sink)
            .await
            .expect_err("invalid channel must fail");

        assert!(matches!(
            err,
            ProductAdapterError::InvalidIdentifier {
                kind: "reply_target",
                ..
            }
        ));
        // No HTTP call must have been made.
        assert!(egress.calls().is_empty());
        // Delivery status must record FailedPermanent with the correct run_id.
        assert!(matches!(
            sink.statuses().as_slice(),
            [DeliveryStatus::FailedPermanent { run_id: Some(r), .. }] if r == &run_id
        ));
    }

    #[tokio::test]
    async fn render_outbound_egress_transport_errors_classified() {
        use ironclaw_product_adapters::ProtocolHttpEgressError;

        type EgressCase = (
            ProtocolHttpEgressError,
            fn(&ProductAdapterError) -> bool,
            fn(&DeliveryStatus) -> bool,
        );
        let cases: &[EgressCase] = &[
            (
                ProtocolHttpEgressError::Timeout,
                |e| matches!(e, ProductAdapterError::EgressTransient { .. }),
                |s| matches!(s, DeliveryStatus::FailedRetryable { .. }),
            ),
            (
                ProtocolHttpEgressError::UnknownCredentialHandle {
                    handle: "slack_bot_token".into(),
                },
                |e| matches!(e, ProductAdapterError::EgressDenied { .. }),
                |s| matches!(s, DeliveryStatus::FailedUnauthorized { .. }),
            ),
            (
                ProtocolHttpEgressError::PolicyDenied {
                    reason: ironclaw_product_adapters::RedactedString::new("blocked"),
                },
                |e| matches!(e, ProductAdapterError::EgressDenied { .. }),
                |s| matches!(s, DeliveryStatus::FailedPermanent { .. }),
            ),
        ];

        for (egress_err, check_err, check_status) in cases {
            let adapter = SlackV2Adapter::new(config());
            let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
            egress.allow_credential_handle("slack_bot_token");
            egress.program_response(SLACK_API_HOST, Err(egress_err.clone()));
            let sink = FakeOutboundDeliverySink::new();

            let err = adapter
                .render_outbound(envelope(final_reply_payload("hi")), &egress, &sink)
                .await
                .expect_err("egress error must fail");

            assert!(check_err(&err), "unexpected error variant: {err:?}");
            let statuses = sink.statuses();
            assert_eq!(statuses.len(), 1, "expected exactly one delivery status");
            assert!(
                check_status(&statuses[0]),
                "unexpected status: {:?}",
                statuses[0]
            );
        }
    }

    #[tokio::test]
    async fn render_outbound_progress_and_keepalive_are_deferred() {
        use ironclaw_product_adapters::{ProgressKind, ProgressUpdateView};

        let payloads = [
            ProductOutboundPayload::Progress(ProgressUpdateView {
                turn_run_id: TurnRunId::new(),
                kind: ProgressKind::Typing,
                generated_at: Utc::now(),
            }),
            ProductOutboundPayload::KeepAlive,
        ];

        for payload in payloads {
            let adapter = SlackV2Adapter::new(config());
            let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
            let sink = FakeOutboundDeliverySink::new();

            let outcome = adapter
                .render_outbound(envelope(payload), &egress, &sink)
                .await
                .expect("unsupported payloads must return Ok(Deferred)");

            assert_eq!(outcome, ProductRenderOutcome::Deferred);
            assert!(
                egress.calls().is_empty(),
                "deferred payloads must not trigger HTTP egress"
            );
            assert!(
                matches!(
                    sink.statuses().as_slice(),
                    [DeliveryStatus::Deferred { .. }]
                ),
                "deferred payloads must record Deferred status"
            );
        }
    }

    #[tokio::test]
    async fn render_outbound_2xx_invalid_json_records_retryable() {
        let adapter = SlackV2Adapter::new(config());
        let egress = FakeProtocolHttpEgress::new(vec![SLACK_API_HOST.to_string()]);
        egress.allow_credential_handle("slack_bot_token");
        // Truncated JSON body (simulates proxy/LB cutting off a 200 response).
        egress.program_response(
            SLACK_API_HOST,
            Ok(ironclaw_product_adapters::EgressResponse::new(
                200,
                br#"{"ok":tr"#.to_vec(),
            )),
        );
        let sink = FakeOutboundDeliverySink::new();

        let err = adapter
            .render_outbound(envelope(final_reply_payload("hello Slack")), &egress, &sink)
            .await
            .expect_err("invalid JSON on 200 must fail");

        // Truncated body is a transient infra condition — must be retryable.
        assert!(
            matches!(err, ProductAdapterError::EgressTransient { .. }),
            "invalid JSON on 200 must be EgressTransient, got {err:?}"
        );
        assert!(
            matches!(
                sink.statuses().as_slice(),
                [DeliveryStatus::FailedRetryable { .. }]
            ),
            "expected FailedRetryable, got {:?}",
            sink.statuses()
        );
    }
}
