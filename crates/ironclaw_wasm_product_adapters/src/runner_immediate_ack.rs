//! Immediate-ACK dispatch path for native ProductAdapter webhooks.
//!
//! Protocols such as Slack must receive a 2xx response before the ProductWorkflow
//! result is available. This extension keeps the verified parse/stamp/admission
//! preparation shared with the synchronous runner path, then schedules bounded
//! workflow dispatch after the protocol ACK is safe to return.
//!
//! # Delivery contract
//!
//! Immediate-ACK dispatch deliberately changes retry semantics. Once this path
//! returns a protocol-level 2xx, transports such as Slack will not redeliver the
//! event even if the asynchronous workflow later returns
//! [`InboundRetryDisposition::Retry`]. The runner logs that case as a potential
//! permanent drop, but hosts that need post-ACK retry guarantees must first land
//! the event in a durable queue/tracked runtime before acknowledging the
//! transport.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{
    InboundRetryDisposition, ProductAdapterError, ProductInboundAck, ProductInboundEnvelope,
    ProtocolAuthEvidence,
};

use crate::runner::{NativeProductAdapterRunner, RunnerError, WebhookProcessOutcome};

/// Observer for workflow outcomes scheduled after an immediate protocol ACK.
///
/// Implementations run outside the webhook response path and must not assume the
/// protocol can retry when delivery fails.
#[async_trait]
pub trait ImmediateAckWorkflowObserver: Send + Sync {
    async fn observe_workflow_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck);

    /// Called when the asynchronous workflow returns an error after the protocol
    /// ACK has already been sent. Implementations must treat follow-up feedback
    /// as best-effort because the transport cannot retry this event.
    async fn observe_workflow_error(
        &self,
        _envelope: ProductInboundEnvelope,
        _error: ProductAdapterError,
    ) {
    }
}

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
        self.process_verified_webhook_immediate_ack_with_observer(body, evidence, None)
            .await
    }

    /// Same as [`Self::process_verified_webhook_immediate_ack`], but notifies a
    /// host-owned observer after the asynchronous workflow dispatch returns an
    /// ack or error. This lets product hosts trigger follow-up delivery (for
    /// example a final reply push) without delaying the protocol-level ACK.
    pub async fn process_verified_webhook_immediate_ack_with_observer(
        &self,
        body: &[u8],
        evidence: &ProtocolAuthEvidence,
        observer: Option<Arc<dyn ImmediateAckWorkflowObserver>>,
    ) -> Result<WebhookProcessOutcome, RunnerError> {
        let (envelope, permit) = self.prepare_inbound_envelope(body, evidence).await?;
        let workflow_envelope = envelope.clone();
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
            // The admission permit gates only fast intake (auth/parse/stamp/
            // submit), bounded by `workflow_timeout`. It must NOT be held across
            // the post-ACK observer delivery poll, which can wait for the run's
            // final reply for far longer (Slack's observer polls up to ~120s).
            // Holding admission across that unbounded wait would let
            // `max_in_flight` slow turns exhaust every intake slot and silently
            // reject new inbound webhooks under load. The permit is therefore
            // released the moment the workflow durably accepts the inbound
            // (`is_durable_outcome`), before observer follow-up runs. On error
            // or timeout (no durable acceptance) the permit drops at scope end,
            // which keeps correct backpressure over the bounded intake window.
            //
            // Timeout drops the in-flight workflow future. That is the intended
            // cancellation boundary for this generic async trait call: the
            // runner does not hold a separate task handle or protocol-specific
            // resource owner to abort. Workflows that open DB/network resources
            // must make their own futures cancellation-safe at await points.
            let mut permit = Some(permit);
            let workflow_result =
                tokio::time::timeout(workflow_timeout, workflow.submit_inbound(workflow_envelope))
                    .await;
            match workflow_result {
                Ok(Ok(ack)) => {
                    if ack.retry_disposition() == InboundRetryDisposition::Retry {
                        tracing::warn!(
                            target = "ironclaw::product_adapter::runner",
                            "async webhook workflow dispatch requested retry after protocol ack; event was not retried by protocol transport"
                        );
                    }
                    // Release admission before the (potentially long) delivery
                    // poll once the run is durably submitted. A non-durable ack
                    // (e.g. retryable rejection) keeps the permit until scope
                    // end so admission still backpressures un-accepted intake.
                    if ack.is_durable_outcome() {
                        drop(permit.take());
                    }
                    if let Some(observer) = observer {
                        observer.observe_workflow_ack(envelope, ack).await;
                    }
                }
                Ok(Err(error)) => {
                    tracing::debug!(
                        target = "ironclaw::product_adapter::runner",
                        error = %error,
                        "async webhook workflow dispatch failed after protocol ack"
                    );
                    if let Some(observer) = observer {
                        observer.observe_workflow_error(envelope, error).await;
                    }
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use super::ImmediateAckWorkflowObserver;
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
    use tokio::sync::Notify;

    use crate::auth_verifier::SharedSecretHeaderAuth;
    use crate::runner::{
        NativeProductAdapterRunner, NativeProductAdapterRunnerConfig, RunnerError, WebhookAuth,
        WebhookProcessOutcome, evidence_from_bearer_subject,
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
        async fn submit_inbound(
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

    struct RejectingWorkflow;

    #[async_trait]
    impl ironclaw_product_adapters::ProductWorkflow for RejectingWorkflow {
        async fn submit_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            Err(ProductAdapterError::WorkflowRejected {
                kind: ironclaw_product_adapters::ProductWorkflowRejectionKind::ScopeNotFound,
                status_code: 404,
                retryable: false,
                reason: ironclaw_product_adapters::RedactedString::new("missing binding"),
            })
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

    /// Workflow that returns a *non-durable* retryable rejection via the
    /// `Ok(ack)` arm (distinct from `RejectingWorkflow`, which returns `Err`).
    /// `ProductInboundAck::Rejected(Retryable)` reports `is_durable_outcome() ==
    /// false`, so the admission permit must be retained across the observer.
    struct NonDurableRejectWorkflow;

    #[async_trait]
    impl ironclaw_product_adapters::ProductWorkflow for NonDurableRejectWorkflow {
        async fn submit_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            Ok(ProductInboundAck::Rejected(
                ironclaw_product_adapters::ProductRejection::retryable(
                    ironclaw_product_adapters::ProductRejectionKind::PolicyDenied,
                    "policy temporarily unavailable",
                ),
            ))
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

    struct RecordingObserver {
        ack_count: Arc<AtomicUsize>,
        error_count: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ImmediateAckWorkflowObserver for RecordingObserver {
        async fn observe_workflow_ack(
            &self,
            _envelope: ProductInboundEnvelope,
            _ack: ProductInboundAck,
        ) {
            self.ack_count.fetch_add(1, Ordering::SeqCst);
        }

        async fn observe_workflow_error(
            &self,
            _envelope: ProductInboundEnvelope,
            _error: ProductAdapterError,
        ) {
            self.error_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Observer that blocks inside `observe_workflow_ack` until released. Models
    /// the real `SlackFinalReplyDeliveryObserver`, which polls the run for its
    /// final reply for up to `max_wait` (default 120s) inside this callback.
    ///
    /// Release uses a latched `AtomicBool` polled with `yield_now`, not a
    /// `Notify`: a single shared observer is invoked by two tasks, and the
    /// second invocation can register *after* the release fires, which
    /// `Notify::notify_waiters` would not wake. The latch wakes current and
    /// future waiters deterministically.
    struct BlockingObserver {
        entered: Arc<AtomicUsize>,
        released: Arc<AtomicBool>,
    }

    #[async_trait]
    impl ImmediateAckWorkflowObserver for BlockingObserver {
        async fn observe_workflow_ack(
            &self,
            _envelope: ProductInboundEnvelope,
            _ack: ProductInboundAck,
        ) {
            self.entered.fetch_add(1, Ordering::SeqCst);
            while !self.released.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        }
    }

    struct BlockingWorkflow {
        entered: Arc<AtomicUsize>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl ironclaw_product_adapters::ProductWorkflow for BlockingWorkflow {
        async fn submit_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            self.release.notified().await;
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
        runner_with_workflow(parse_count, Arc::new(AckWorkflow), 1)
    }

    fn runner_with_workflow(
        parse_count: Arc<AtomicUsize>,
        workflow: Arc<dyn ironclaw_product_adapters::ProductWorkflow>,
        max_in_flight: usize,
    ) -> NativeProductAdapterRunner {
        NativeProductAdapterRunner::with_config(
            Arc::new(StaticAdapter::new(parse_count)),
            workflow,
            WebhookAuth::SharedSecretHeader(SharedSecretHeaderAuth {
                header_name: "X-Test-Secret".into(),
                expected_secret: "topsecret".into(),
                subject: "slack_install_alpha".into(),
            }),
            NativeProductAdapterRunnerConfig::new(
                Duration::from_secs(1),
                std::num::NonZeroUsize::new(max_in_flight).expect("nonzero"),
            ),
        )
    }

    fn verified_evidence(runner: &NativeProductAdapterRunner) -> ProtocolAuthEvidence {
        let mut headers = http::HeaderMap::new();
        headers.insert("X-Test-Secret", "topsecret".parse().expect("header value"));
        runner
            .verify_webhook_auth(&headers, b"{}")
            .expect("webhook auth should verify")
    }

    #[tokio::test]
    async fn process_verified_webhook_immediate_ack_dispatches_workflow() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let runner = runner(Arc::clone(&parse_count));
        let evidence = verified_evidence(&runner);
        let outcome = runner
            .process_verified_webhook_immediate_ack(b"{}", &evidence)
            .await
            .expect("verified webhook should dispatch");

        assert_eq!(outcome, WebhookProcessOutcome::AcceptedForAsyncDispatch);
        runner.drain_immediate_ack_tasks().await;
        assert_eq!(parse_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn immediate_ack_observer_receives_post_ack_workflow_errors() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let ack_count = Arc::new(AtomicUsize::new(0));
        let error_count = Arc::new(AtomicUsize::new(0));
        let runner = runner_with_workflow(Arc::clone(&parse_count), Arc::new(RejectingWorkflow), 1);
        let observer = Arc::new(RecordingObserver {
            ack_count: Arc::clone(&ack_count),
            error_count: Arc::clone(&error_count),
        });
        let evidence = verified_evidence(&runner);

        let outcome = runner
            .process_verified_webhook_immediate_ack_with_observer(b"{}", &evidence, Some(observer))
            .await
            .expect("verified webhook should dispatch");

        assert_eq!(outcome, WebhookProcessOutcome::AcceptedForAsyncDispatch);
        runner.drain_immediate_ack_tasks().await;
        assert_eq!(parse_count.load(Ordering::SeqCst), 1);
        assert_eq!(ack_count.load(Ordering::SeqCst), 0);
        assert_eq!(error_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn process_verified_webhook_immediate_ack_rejects_when_at_capacity() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let runner = runner_with_workflow(
            Arc::clone(&parse_count),
            Arc::new(BlockingWorkflow {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }),
            1,
        );
        let evidence = verified_evidence(&runner);
        let first = runner
            .process_verified_webhook_immediate_ack(b"{}", &evidence)
            .await
            .expect("first webhook should dispatch");
        assert_eq!(first, WebhookProcessOutcome::AcceptedForAsyncDispatch);
        while entered.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }

        let err = runner
            .process_verified_webhook_immediate_ack(b"{}", &evidence)
            .await
            .expect_err("second webhook should be rejected while permit is held");
        assert!(matches!(
            err,
            RunnerError::TooManyInFlight { max_in_flight: 1 }
        ));
        release.notify_waiters();
        runner.drain_immediate_ack_tasks().await;
        assert_eq!(parse_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn drain_immediate_ack_tasks_waits_for_all_tasks() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let runner = Arc::new(runner_with_workflow(
            Arc::clone(&parse_count),
            Arc::new(BlockingWorkflow {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }),
            2,
        ));
        let evidence = verified_evidence(&runner);
        runner
            .process_verified_webhook_immediate_ack(b"{}", &evidence)
            .await
            .expect("first webhook should dispatch");
        runner
            .process_verified_webhook_immediate_ack(b"{}", &evidence)
            .await
            .expect("second webhook should dispatch");
        while entered.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }

        let drain_runner = Arc::clone(&runner);
        let drain = tokio::spawn(async move {
            drain_runner.drain_immediate_ack_tasks().await;
        });
        tokio::task::yield_now().await;
        assert!(!drain.is_finished());
        release.notify_waiters();
        drain.await.expect("drain task should finish");
        assert_eq!(parse_count.load(Ordering::SeqCst), 2);
    }

    /// Regression: the admission permit must be released once the workflow has
    /// durably accepted the inbound (its `submit_inbound` returned), NOT held
    /// across the post-ACK observer delivery poll. The real Slack observer polls
    /// the submitted run for its final reply for up to 120s inside
    /// `observe_workflow_ack`; if admission stayed pinned for that whole window,
    /// only `SLACK_MAX_IN_FLIGHT_WEBHOOKS` slow turns would exhaust all admission
    /// slots and new inbound webhooks would be rejected `TooManyInFlight` for up
    /// to two minutes — silent backpressure-induced message loss under load.
    ///
    /// With `max_in_flight = 1`: webhook #1 durably accepts, then its observer
    /// blocks (modelling the long delivery poll). Webhook #2 must still be
    /// admitted because admission is freed after durable acceptance.
    #[tokio::test]
    async fn admission_released_after_durable_accept_not_held_across_delivery() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let observer_entered = Arc::new(AtomicUsize::new(0));
        let observer_released = Arc::new(AtomicBool::new(false));
        // AckWorkflow returns a durable NoOp ack immediately; the observer then
        // blocks, standing in for the up-to-120s final-reply delivery poll.
        let runner = runner_with_workflow(Arc::clone(&parse_count), Arc::new(AckWorkflow), 1);
        let observer = Arc::new(BlockingObserver {
            entered: Arc::clone(&observer_entered),
            released: Arc::clone(&observer_released),
        });
        let evidence = verified_evidence(&runner);

        let first = runner
            .process_verified_webhook_immediate_ack_with_observer(
                b"{}",
                &evidence,
                Some(Arc::clone(&observer) as Arc<dyn ImmediateAckWorkflowObserver>),
            )
            .await
            .expect("first webhook should be accepted for async dispatch");
        assert_eq!(first, WebhookProcessOutcome::AcceptedForAsyncDispatch);

        // Wait until the observer is actively blocking — i.e. the workflow has
        // durably accepted and we are now in the delivery phase.
        while observer_entered.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }

        // The second webhook must be admitted: admission only gates intake, not
        // the unbounded delivery wait the observer is currently sitting in.
        let second = runner
            .process_verified_webhook_immediate_ack_with_observer(
                b"{}",
                &evidence,
                Some(Arc::clone(&observer) as Arc<dyn ImmediateAckWorkflowObserver>),
            )
            .await
            .expect("second webhook must be admitted while first is still in delivery");
        assert_eq!(second, WebhookProcessOutcome::AcceptedForAsyncDispatch);

        observer_released.store(true, Ordering::SeqCst);
        runner.drain_immediate_ack_tasks().await;
        assert_eq!(parse_count.load(Ordering::SeqCst), 2);
    }

    /// Counterpart to the durable-accept test: a *non-durable* ack
    /// (`Rejected(Retryable)`) must KEEP the admission permit across the observer
    /// so admission still backpressures un-accepted intake. This pins the
    /// `is_durable_outcome()` guard so a future edit that drops the permit
    /// unconditionally would be caught.
    #[tokio::test]
    async fn admission_retained_across_observer_for_non_durable_ack() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let observer_entered = Arc::new(AtomicUsize::new(0));
        let observer_released = Arc::new(AtomicBool::new(false));
        let runner = runner_with_workflow(
            Arc::clone(&parse_count),
            Arc::new(NonDurableRejectWorkflow),
            1,
        );
        let observer = Arc::new(BlockingObserver {
            entered: Arc::clone(&observer_entered),
            released: Arc::clone(&observer_released),
        });
        let evidence = verified_evidence(&runner);

        let first = runner
            .process_verified_webhook_immediate_ack_with_observer(
                b"{}",
                &evidence,
                Some(Arc::clone(&observer) as Arc<dyn ImmediateAckWorkflowObserver>),
            )
            .await
            .expect("first webhook should be accepted for async dispatch");
        assert_eq!(first, WebhookProcessOutcome::AcceptedForAsyncDispatch);

        // Wait until the observer is blocking — the workflow returned a
        // non-durable ack, so the permit must still be held at this point.
        while observer_entered.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }

        // The permit is retained across the observer for a non-durable ack, so a
        // second intake must be rejected while the first is still in delivery.
        let err = runner
            .process_verified_webhook_immediate_ack_with_observer(
                b"{}",
                &evidence,
                Some(Arc::clone(&observer) as Arc<dyn ImmediateAckWorkflowObserver>),
            )
            .await
            .expect_err("second webhook should be rejected while non-durable permit is held");
        assert!(matches!(
            err,
            RunnerError::TooManyInFlight { max_in_flight: 1 }
        ));

        observer_released.store(true, Ordering::SeqCst);
        runner.drain_immediate_ack_tasks().await;
        assert_eq!(parse_count.load(Ordering::SeqCst), 1);
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
