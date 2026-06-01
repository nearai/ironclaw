//! Immediate-ACK dispatch path for native ProductAdapter webhooks.
//!
//! Protocols such as Slack must receive a 2xx response before the ProductWorkflow
//! result is available. This extension keeps the verified parse/stamp/admission
//! preparation shared with the synchronous runner path, then schedules bounded
//! workflow dispatch after the protocol ACK is safe to return.

use std::sync::Arc;

use ironclaw_product_adapters::{InboundRetryDisposition, ProtocolAuthEvidence};

use crate::runner::{NativeProductAdapterRunner, RunnerError, WebhookProcessOutcome};

impl NativeProductAdapterRunner {
    /// Verify, parse, stamp, and schedule workflow dispatch without waiting for
    /// the workflow result. This is the path for protocols that require an
    /// immediate webhook ACK after authentication and syntactic normalization.
    pub async fn process_webhook_immediate_ack(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<WebhookProcessOutcome, RunnerError> {
        let evidence = self.verify_webhook_auth(headers, body)?;
        self.process_verified_webhook_immediate_ack(body, &evidence)
            .await
    }

    /// Schedule a previously verified webhook payload. Exposed for
    /// protocol-specific handlers that must verify once, handle a special
    /// synchronous protocol handshake, and then continue into the normal async
    /// ProductWorkflow dispatch path for ordinary events.
    pub async fn process_verified_webhook_immediate_ack(
        &self,
        body: &[u8],
        evidence: &ProtocolAuthEvidence,
    ) -> Result<WebhookProcessOutcome, RunnerError> {
        let (envelope, permit) = self.prepare_inbound_envelope(body, evidence).await?;
        let workflow = Arc::clone(&self.workflow);
        let workflow_timeout = self.config.workflow_timeout;
        let mut tasks = self.immediate_ack_tasks.lock().await;
        while let Some(result) = tasks.try_join_next() {
            if let Err(error) = result {
                tracing::debug!(
                    target = "ironclaw::product_adapter::runner",
                    error = %error,
                    "tracked async webhook workflow dispatch task finished with join error"
                );
            }
        }
        tasks.spawn(async move {
            let _permit = permit;
            match tokio::time::timeout(workflow_timeout, workflow.accept_inbound(envelope)).await {
                Ok(Ok(ack)) if ack.retry_disposition() == InboundRetryDisposition::Retry => {
                    tracing::warn!(
                        target = "ironclaw::product_adapter::runner",
                        "async webhook workflow dispatch requested retry after protocol ack; event was not retried by protocol transport"
                    );
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    tracing::debug!(
                        target = "ironclaw::product_adapter::runner",
                        error = %error,
                        "async webhook workflow dispatch failed after protocol ack"
                    );
                }
                Err(_) => {
                    tracing::debug!(
                        target = "ironclaw::product_adapter::runner",
                        timeout_secs = workflow_timeout.as_secs(),
                        "async webhook workflow dispatch timed out after protocol ack"
                    );
                }
            }
        });
        Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch)
    }

    /// Wait for currently tracked immediate-ACK dispatches to finish. Hosts can
    /// call this during graceful shutdown after ingress stops accepting new
    /// webhooks.
    pub async fn drain_immediate_ack_tasks(&self) {
        let mut tasks = self.immediate_ack_tasks.lock().await;
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                tracing::debug!(
                    target = "ironclaw::product_adapter::runner",
                    error = %error,
                    "tracked async webhook workflow dispatch task finished with join error"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use ironclaw_product_adapters::capabilities::ProductAdapterCapabilities;
    use ironclaw_product_adapters::external::{
        ExternalActorRef, ExternalConversationRef, ExternalEventId,
    };
    use ironclaw_product_adapters::identity::{
        AdapterInstallationId, ProductAdapterId, ProductSurfaceKind,
    };
    use ironclaw_product_adapters::{
        AuthRequirement, OutboundDeliverySink, ParsedProductInbound, ProductAdapter,
        ProductAdapterError, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
        ProductOutboundEnvelope, ProductRenderOutcome, ProductTriggerReason,
        ProjectionSubscriptionRequest, ProtocolAuthEvidence, ProtocolAuthFailure,
        ProtocolHttpEgress, UserMessagePayload,
    };

    use crate::auth_verifier::SharedSecretHeaderAuth;
    use crate::runner::{
        NativeProductAdapterRunner, NativeProductAdapterRunnerConfig, RunnerError, WebhookAuth,
        evidence_from_bearer_subject,
    };

    struct StaticAdapter {
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        capabilities: ProductAdapterCapabilities,
        parse_count: Arc<AtomicUsize>,
    }

    impl StaticAdapter {
        fn new(parse_count: Arc<AtomicUsize>) -> Self {
            Self {
                adapter_id: ProductAdapterId::new("slack_v2").expect("valid adapter id"),
                installation_id: AdapterInstallationId::new("install_alpha")
                    .expect("valid installation id"),
                capabilities: ProductAdapterCapabilities::empty(),
                parse_count,
            }
        }
    }

    #[async_trait]
    impl ProductAdapter for StaticAdapter {
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
            static AUTH: std::sync::LazyLock<AuthRequirement> =
                std::sync::LazyLock::new(|| AuthRequirement::SharedSecretHeader {
                    header_name: "X-Test-Secret".into(),
                });
            &AUTH
        }

        fn parse_inbound(
            &self,
            _raw_payload: &[u8],
            _auth_evidence: &ProtocolAuthEvidence,
        ) -> Result<ParsedProductInbound, ProductAdapterError> {
            self.parse_count.fetch_add(1, Ordering::SeqCst);
            ParsedProductInbound::new(
                ExternalEventId::new("slack-event-1").expect("valid event id"),
                ExternalActorRef::new("slack_user", "U123", None::<String>)
                    .expect("valid actor ref"),
                ExternalConversationRef::new(None, "C123", None::<&str>, None::<&str>)
                    .expect("valid conversation ref"),
                ProductInboundPayload::UserMessage(
                    UserMessagePayload::new("hello", Vec::new(), ProductTriggerReason::DirectChat)
                        .expect("valid user message"),
                ),
            )
        }

        async fn render_outbound(
            &self,
            _envelope: ProductOutboundEnvelope,
            _egress: &dyn ProtocolHttpEgress,
            _delivery_sink: &dyn OutboundDeliverySink,
        ) -> Result<ProductRenderOutcome, ProductAdapterError> {
            Ok(ProductRenderOutcome::DeliveryRecorded)
        }
    }

    struct AckWorkflow;

    #[async_trait]
    impl ironclaw_product_adapters::ProductWorkflow for AckWorkflow {
        async fn accept_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            Ok(ProductInboundAck::NoOp)
        }

        async fn resolve_projection_subscription(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
            Err(ProductAdapterError::Internal {
                detail: ironclaw_product_adapters::redaction::RedactedString::new(
                    "test stub: resolve_projection_subscription not supported",
                ),
            })
        }
    }

    fn runner(parse_count: Arc<AtomicUsize>) -> NativeProductAdapterRunner {
        NativeProductAdapterRunner::with_config(
            Arc::new(StaticAdapter::new(parse_count)),
            Arc::new(AckWorkflow),
            WebhookAuth::SharedSecretHeader(SharedSecretHeaderAuth {
                header_name: "X-Test-Secret".into(),
                expected_secret: "topsecret".into(),
                subject: "slack_install_alpha".into(),
            }),
            NativeProductAdapterRunnerConfig::new(
                Duration::from_secs(1),
                std::num::NonZeroUsize::new(1).expect("nonzero"),
            ),
        )
    }

    #[tokio::test]
    async fn process_verified_webhook_immediate_ack_rejects_mismatched_evidence_type() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let runner = runner(Arc::clone(&parse_count));
        let err = runner
            .process_verified_webhook_immediate_ack(b"{}", &evidence_from_bearer_subject("user"))
            .await
            .expect_err("mismatched evidence should fail closed");

        assert!(matches!(err, RunnerError::AuthenticationFailed { .. }));
        assert_eq!(parse_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_verified_webhook_immediate_ack_rejects_none_claim_evidence() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let runner = runner(Arc::clone(&parse_count));
        let evidence = ProtocolAuthEvidence::failed(ProtocolAuthFailure::Missing);
        let err = runner
            .process_verified_webhook_immediate_ack(b"{}", &evidence)
            .await
            .expect_err("failed evidence should fail closed");

        assert!(matches!(err, RunnerError::AuthenticationFailed { .. }));
        assert_eq!(parse_count.load(Ordering::SeqCst), 0);
    }
}
