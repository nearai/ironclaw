use async_trait::async_trait;
use chrono::Utc;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeliveryStatus, ExternalActorRef,
    ExternalConversationRef, ExternalEventId, OutboundDeliverySink, ParsedProductInbound,
    ProductAdapter, ProductAdapterCapabilities, ProductAdapterError, ProductAdapterHealth,
    ProductAdapterId, ProductInboundEnvelope, ProductInboundPayload, ProductOutboundEnvelope,
    ProductRenderOutcome, ProductSurfaceKind, ProductTriggerReason, ProjectionCursor,
    ProjectionSubscriptionPayload, ProtocolAuthEvidence, ProtocolAuthFailure, ProtocolHttpEgress,
    TrustedInboundContext, UserMessagePayload,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct RebornTestProductAdapter {
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    capabilities: ProductAdapterCapabilities,
    auth_requirement: AuthRequirement,
}

impl RebornTestProductAdapter {
    pub fn new(
        adapter_id: impl Into<String>,
        installation_id: impl Into<String>,
    ) -> Result<Self, ProductAdapterError> {
        Ok(Self {
            adapter_id: ProductAdapterId::new(adapter_id.into())?,
            installation_id: AdapterInstallationId::new(installation_id.into())?,
            capabilities: ProductAdapterCapabilities::external_channel_default(),
            auth_requirement: AuthRequirement::BearerToken,
        })
    }

    pub fn text_payload(
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        text: &str,
    ) -> Result<Vec<u8>, ProductAdapterError> {
        Self::text_payload_with_trigger(
            event_id,
            user_id,
            thread_id,
            text,
            ProductTriggerReason::DirectChat,
        )
    }

    pub fn text_payload_with_trigger(
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        text: &str,
        trigger: ProductTriggerReason,
    ) -> Result<Vec<u8>, ProductAdapterError> {
        serde_json::to_vec(&RebornTestInboundPayload {
            kind: RebornTestInboundKind::UserMessage,
            event_id,
            user_id,
            thread_id,
            text: Some(text),
            trigger: Some(trigger),
            thread_id_hint: None,
            after_cursor: None,
        })
        .map_err(|error| ProductAdapterError::MalformedInboundPayload {
            reason: ironclaw_product_adapters::RedactedString::new(error.to_string()),
        })
    }

    pub fn subscription_payload(
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        thread_id_hint: Option<&str>,
        after_cursor: Option<ProjectionCursor>,
    ) -> Result<Vec<u8>, ProductAdapterError> {
        serde_json::to_vec(&RebornTestInboundPayload {
            kind: RebornTestInboundKind::SubscriptionRequest,
            event_id,
            user_id,
            thread_id,
            text: None,
            trigger: None,
            thread_id_hint,
            after_cursor,
        })
        .map_err(|error| ProductAdapterError::MalformedInboundPayload {
            reason: ironclaw_product_adapters::RedactedString::new(error.to_string()),
        })
    }
}

#[async_trait]
impl ProductAdapter for RebornTestProductAdapter {
    fn adapter_id(&self) -> &ProductAdapterId {
        &self.adapter_id
    }

    fn installation_id(&self) -> &AdapterInstallationId {
        &self.installation_id
    }

    fn surface_kind(&self) -> ProductSurfaceKind {
        ProductSurfaceKind::ExternalChannel
    }

    fn capabilities(&self) -> &ProductAdapterCapabilities {
        &self.capabilities
    }

    fn auth_requirement(&self) -> &AuthRequirement {
        &self.auth_requirement
    }

    fn parse_inbound(
        &self,
        raw_payload: &[u8],
        auth_evidence: &ProtocolAuthEvidence,
    ) -> Result<ParsedProductInbound, ProductAdapterError> {
        if !auth_evidence.is_verified() {
            return Err(ProductAdapterError::Authentication(
                auth_evidence
                    .failure()
                    .cloned()
                    .unwrap_or(ProtocolAuthFailure::Missing),
            ));
        }
        let payload: OwnedRebornTestInboundPayload =
            serde_json::from_slice(raw_payload).map_err(|error| {
                ProductAdapterError::MalformedInboundPayload {
                    reason: ironclaw_product_adapters::RedactedString::new(error.to_string()),
                }
            })?;
        let claim = auth_evidence
            .claim()
            .ok_or(ProductAdapterError::Authentication(
                ProtocolAuthFailure::Missing,
            ))?;
        if claim.subject() != payload.user_id {
            return Err(ProductAdapterError::Authentication(
                ProtocolAuthFailure::Other {
                    detail: ironclaw_product_adapters::RedactedString::new(
                        "verified subject does not match inbound actor",
                    ),
                },
            ));
        }
        let inbound_payload = match payload.kind {
            RebornTestInboundKind::UserMessage => {
                ProductInboundPayload::UserMessage(UserMessagePayload::new(
                    payload
                        .text
                        .ok_or_else(|| ProductAdapterError::MalformedInboundPayload {
                            reason: ironclaw_product_adapters::RedactedString::new(
                                "user message payload missing text",
                            ),
                        })?,
                    Vec::new(),
                    payload.trigger.ok_or_else(|| {
                        ProductAdapterError::MalformedInboundPayload {
                            reason: ironclaw_product_adapters::RedactedString::new(
                                "user message payload missing trigger",
                            ),
                        }
                    })?,
                )?)
            }
            RebornTestInboundKind::SubscriptionRequest => {
                ProductInboundPayload::SubscriptionRequest(ProjectionSubscriptionPayload::new(
                    payload.thread_id_hint,
                    payload.after_cursor,
                )?)
            }
        };
        ParsedProductInbound::new(
            ExternalEventId::new(payload.event_id)?,
            ExternalActorRef::new(
                "reborn_test_user",
                payload.user_id.clone(),
                Some(payload.user_id),
            )?,
            ExternalConversationRef::new(None, payload.thread_id, None, None)?,
            inbound_payload,
        )
    }

    async fn render_outbound(
        &self,
        envelope: ProductOutboundEnvelope,
        _egress: &dyn ProtocolHttpEgress,
        delivery_sink: &dyn OutboundDeliverySink,
    ) -> Result<ProductRenderOutcome, ProductAdapterError> {
        delivery_sink
            .record(DeliveryStatus::Delivered {
                attempt_id: envelope.delivery_attempt_id,
                target: envelope.target.reply_target_binding_ref,
                run_id: None,
            })
            .await;
        Ok(ProductRenderOutcome::DeliveryRecorded)
    }

    fn health(&self) -> ProductAdapterHealth {
        ProductAdapterHealth::Healthy
    }
}

#[derive(Debug, Clone)]
pub struct RebornTestIngress {
    adapter: RebornTestProductAdapter,
}

impl RebornTestIngress {
    pub fn new(adapter: RebornTestProductAdapter) -> Self {
        Self { adapter }
    }

    pub fn adapter(&self) -> &RebornTestProductAdapter {
        &self.adapter
    }

    pub fn verified_text_envelope(
        &self,
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        text: &str,
    ) -> Result<ProductInboundEnvelope, ProductAdapterError> {
        self.verified_text_envelope_with_trigger(
            event_id,
            user_id,
            thread_id,
            text,
            ProductTriggerReason::DirectChat,
        )
    }

    pub fn verified_text_envelope_with_trigger(
        &self,
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        text: &str,
        trigger: ProductTriggerReason,
    ) -> Result<ProductInboundEnvelope, ProductAdapterError> {
        let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, user_id);
        let raw = RebornTestProductAdapter::text_payload_with_trigger(
            event_id, user_id, thread_id, text, trigger,
        )?;
        let parsed = self.adapter.parse_inbound(&raw, &evidence)?;
        let context = TrustedInboundContext::from_verified_evidence(
            self.adapter.adapter_id().clone(),
            self.adapter.installation_id().clone(),
            Utc::now(),
            &evidence,
        )?;
        ProductInboundEnvelope::from_trusted_parse(context, parsed)
    }

    pub fn verified_subscription_envelope(
        &self,
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        thread_id_hint: Option<&str>,
        after_cursor: Option<ProjectionCursor>,
    ) -> Result<ProductInboundEnvelope, ProductAdapterError> {
        let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, user_id);
        let raw = RebornTestProductAdapter::subscription_payload(
            event_id,
            user_id,
            thread_id,
            thread_id_hint,
            after_cursor,
        )?;
        let parsed = self.adapter.parse_inbound(&raw, &evidence)?;
        let context = TrustedInboundContext::from_verified_evidence(
            self.adapter.adapter_id().clone(),
            self.adapter.installation_id().clone(),
            Utc::now(),
            &evidence,
        )?;
        ProductInboundEnvelope::from_trusted_parse(context, parsed)
    }

    pub fn failed_auth_payload(
        &self,
        raw_payload: &[u8],
    ) -> Result<ParsedProductInbound, ProductAdapterError> {
        let evidence = ProtocolAuthEvidence::failed(ProtocolAuthFailure::Missing);
        self.adapter.parse_inbound(raw_payload, &evidence)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RebornTestInboundKind {
    UserMessage,
    SubscriptionRequest,
}

fn default_inbound_kind() -> RebornTestInboundKind {
    RebornTestInboundKind::UserMessage
}

#[derive(Debug, Serialize)]
struct RebornTestInboundPayload<'a> {
    kind: RebornTestInboundKind,
    event_id: &'a str,
    user_id: &'a str,
    thread_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trigger: Option<ProductTriggerReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id_hint: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Deserialize)]
struct OwnedRebornTestInboundPayload {
    #[serde(default = "default_inbound_kind")]
    kind: RebornTestInboundKind,
    event_id: String,
    user_id: String,
    thread_id: String,
    text: Option<String>,
    trigger: Option<ProductTriggerReason>,
    thread_id_hint: Option<String>,
    after_cursor: Option<ProjectionCursor>,
}
