//! Native ProductAdapter runner.
//!
//! `NativeProductAdapterRunner` is the integration point that turns a single
//! webhook request into the full Reborn pipeline:
//!
//! 1. Authenticate the protocol payload with a [`WebhookAuthVerifier`].
//! 2. On success, mint a `Verified` evidence via the public `mark_*_verified`
//!    helpers in `ironclaw_product_adapters::auth`.
//! 3. Hand the verified evidence + raw payload to the adapter's
//!    [`ironclaw_product_adapters::ProductAdapter::parse_inbound`].
//! 4. Forward the resulting envelope to the [`ironclaw_product_adapters::ProductWorkflow`]
//!    facade and return the structured outcome.
//!
//! The runner is deliberately not wasmtime-bound — the v2 component-model
//! plumbing lands in a follow-up. Telegram v2 today implements
//! `ProductAdapter` natively in Rust; the runner enforces the same auth /
//! dedupe / facade-only contract a wasmtime instance would.

use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use ironclaw_product_adapters::auth::{
    mark_bearer_token_verified, mark_request_signature_verified, mark_session_verified,
    mark_shared_secret_header_verified,
};
use ironclaw_product_adapters::redaction::RedactedString;
use ironclaw_product_adapters::{
    AuthRequirement, InboundRetryDisposition, ProductAdapter, ProductAdapterError,
    ProductInboundAck, ProductInboundEnvelope, ProductWorkflow, ProtocolAuthEvidence,
    ProtocolAuthFailure, TrustedInboundContext,
};
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::auth_verifier::{
    HmacWebhookAuth, SharedSecretHeaderAuth, VerificationOutcome, WebhookAuthVerifier,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RunnerError {
    #[error("webhook authentication failed: {failure}")]
    AuthenticationFailed { failure: ProtocolAuthFailure },
    #[error("native adapter panicked while parsing inbound payload")]
    AdapterPanicked,
    #[error("product workflow panicked while accepting inbound payload")]
    WorkflowPanicked,
    #[error("product workflow timed out after {timeout:?}")]
    WorkflowTimeout { timeout: Duration },
    #[error("too many in-flight webhook requests ({max_in_flight})")]
    TooManyInFlight { max_in_flight: usize },
    #[error("product workflow task failed before producing an outcome")]
    WorkflowJoinFailed,
    #[error(transparent)]
    Adapter(#[from] ProductAdapterError),
}

impl RunnerError {
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, RunnerError::AuthenticationFailed { .. })
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            RunnerError::AuthenticationFailed { .. } | RunnerError::AdapterPanicked => false,
            RunnerError::WorkflowPanicked
            | RunnerError::WorkflowTimeout { .. }
            | RunnerError::TooManyInFlight { .. }
            | RunnerError::WorkflowJoinFailed => true,
            RunnerError::Adapter(err) => err.is_retryable(),
        }
    }
}

/// What the protocol layer should do with the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebhookProcessOutcome {
    /// Auth succeeded, adapter parsed an envelope, workflow accepted it.
    /// Parsed `ProductInboundPayload::NoOp` events still flow through
    /// `ProductWorkflow` and return `Acknowledged` with a no-op ack.
    Acknowledged { ack: ProductInboundAck },
}

/// Webhook auth strategy.
pub enum WebhookAuth {
    Hmac(HmacWebhookAuth),
    SharedSecretHeader(SharedSecretHeaderAuth),
}

impl WebhookAuth {
    pub fn matches_requirement(&self, requirement: &AuthRequirement) -> bool {
        match (self, requirement) {
            (
                WebhookAuth::Hmac(v),
                AuthRequirement::RequestSignature {
                    header_name,
                    timestamp_header_name: Some(timestamp_header_name),
                },
            ) => {
                header_name_matches(&v.signature_header, header_name)
                    && header_name_matches(&v.timestamp_header, timestamp_header_name)
            }
            (
                WebhookAuth::SharedSecretHeader(v),
                AuthRequirement::SharedSecretHeader { header_name },
            ) => header_name_matches(&v.header_name, header_name),
            (
                WebhookAuth::Hmac(_),
                AuthRequirement::RequestSignature {
                    timestamp_header_name: None,
                    ..
                },
            ) => {
                // Intentionally unsupported: current webhook callers must bind
                // HMAC verification to both signature and timestamp headers.
                false
            }
            (WebhookAuth::Hmac(_), AuthRequirement::SharedSecretHeader { .. })
            | (WebhookAuth::Hmac(_), AuthRequirement::SessionCookie { .. })
            | (WebhookAuth::Hmac(_), AuthRequirement::BearerToken)
            | (WebhookAuth::SharedSecretHeader(_), AuthRequirement::RequestSignature { .. })
            | (WebhookAuth::SharedSecretHeader(_), AuthRequirement::SessionCookie { .. })
            | (WebhookAuth::SharedSecretHeader(_), AuthRequirement::BearerToken) => false,
        }
    }

    fn verify(&self, headers: &http::HeaderMap, body: &[u8]) -> VerificationOutcome {
        match self {
            WebhookAuth::Hmac(v) => v.verify(headers, body),
            WebhookAuth::SharedSecretHeader(v) => v.verify(headers, body),
        }
    }

    fn mint_evidence(&self, subject: String) -> ProtocolAuthEvidence {
        match self {
            WebhookAuth::Hmac(v) => mark_request_signature_verified(
                v.signature_header.clone(),
                Some(v.timestamp_header.clone()),
                subject,
            ),
            WebhookAuth::SharedSecretHeader(v) => {
                mark_shared_secret_header_verified(v.header_name.clone(), subject)
            }
        }
    }
}

/// Convenience constructor for synchronous-API or CLI auth bridges.
pub fn evidence_from_session_subject(subject: impl Into<String>) -> ProtocolAuthEvidence {
    mark_session_verified("ironclaw_session", subject)
}

pub fn evidence_from_bearer_subject(subject: impl Into<String>) -> ProtocolAuthEvidence {
    mark_bearer_token_verified(subject)
}

fn header_name_matches(configured: &str, required: &str) -> bool {
    configured.eq_ignore_ascii_case(required)
}

pub const DEFAULT_WEBHOOK_WORKFLOW_TIMEOUT: Duration = Duration::from_secs(55);
pub const DEFAULT_MAX_IN_FLIGHT_WEBHOOKS: usize = 64;
const DEFAULT_MAX_IN_FLIGHT_WEBHOOKS_NONZERO: NonZeroUsize =
    match NonZeroUsize::new(DEFAULT_MAX_IN_FLIGHT_WEBHOOKS) {
        Some(value) => value,
        None => NonZeroUsize::MIN,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeProductAdapterRunnerConfig {
    pub workflow_timeout: Duration,
    pub max_in_flight: NonZeroUsize,
}

impl NativeProductAdapterRunnerConfig {
    pub fn new(workflow_timeout: Duration, max_in_flight: NonZeroUsize) -> Self {
        Self {
            workflow_timeout,
            max_in_flight,
        }
    }

    pub fn with_workflow_timeout(mut self, workflow_timeout: Duration) -> Self {
        self.workflow_timeout = workflow_timeout;
        self
    }

    pub fn with_max_in_flight(mut self, max_in_flight: NonZeroUsize) -> Self {
        self.max_in_flight = max_in_flight;
        self
    }

    pub fn max_in_flight(&self) -> usize {
        self.max_in_flight.get()
    }
}

impl Default for NativeProductAdapterRunnerConfig {
    fn default() -> Self {
        Self {
            workflow_timeout: DEFAULT_WEBHOOK_WORKFLOW_TIMEOUT,
            max_in_flight: DEFAULT_MAX_IN_FLIGHT_WEBHOOKS_NONZERO,
        }
    }
}

pub struct NativeProductAdapterRunner {
    adapter: Arc<dyn ProductAdapter>,
    workflow: Arc<dyn ProductWorkflow>,
    auth: WebhookAuth,
    config: NativeProductAdapterRunnerConfig,
    admission: Arc<Semaphore>,
}

impl NativeProductAdapterRunner {
    pub fn new(
        adapter: Arc<dyn ProductAdapter>,
        workflow: Arc<dyn ProductWorkflow>,
        auth: WebhookAuth,
    ) -> Self {
        Self::with_config(
            adapter,
            workflow,
            auth,
            NativeProductAdapterRunnerConfig::default(),
        )
    }

    pub fn with_config(
        adapter: Arc<dyn ProductAdapter>,
        workflow: Arc<dyn ProductWorkflow>,
        auth: WebhookAuth,
        config: NativeProductAdapterRunnerConfig,
    ) -> Self {
        Self {
            adapter,
            workflow,
            auth,
            admission: Arc::new(Semaphore::new(config.max_in_flight())),
            config,
        }
    }

    pub fn config(&self) -> NativeProductAdapterRunnerConfig {
        self.config
    }

    pub async fn process_webhook(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<WebhookProcessOutcome, RunnerError> {
        if !self
            .auth
            .matches_requirement(self.adapter.auth_requirement())
        {
            return Err(RunnerError::AuthenticationFailed {
                failure: ProtocolAuthFailure::Other {
                    detail: RedactedString::new(
                        "configured webhook auth strategy does not match adapter auth requirement",
                    ),
                },
            });
        }
        let evidence = match self.auth.verify(headers, body) {
            VerificationOutcome::Verified { subject } => self.auth.mint_evidence(subject),
            VerificationOutcome::Failed { failure } => {
                return Err(RunnerError::AuthenticationFailed { failure });
            }
        };
        let _permit = self.admission.clone().try_acquire_owned().map_err(|_| {
            RunnerError::TooManyInFlight {
                max_in_flight: self.config.max_in_flight(),
            }
        })?;
        let parse_result = catch_unwind(AssertUnwindSafe(|| {
            self.adapter.parse_inbound(body, &evidence)
        }));
        let parsed = match parse_result {
            Ok(result) => result?,
            Err(_) => return Err(RunnerError::AdapterPanicked),
        };
        // Host stamps the trusted context (adapter id, installation id,
        // verified auth claim, received-at timestamp) before the workflow
        // ever sees the envelope. Adapters can't fabricate this surface.
        let context = TrustedInboundContext::from_verified_evidence(
            self.adapter.adapter_id().clone(),
            self.adapter.installation_id().clone(),
            chrono::Utc::now(),
            &evidence,
        )?;
        let envelope = ProductInboundEnvelope::from_trusted_parse(context, parsed)?;
        let workflow = Arc::clone(&self.workflow);
        let mut workflow_task =
            tokio::spawn(async move { workflow.accept_inbound(envelope).await });
        let ack = match tokio::time::timeout(self.config.workflow_timeout, &mut workflow_task).await
        {
            Ok(Ok(result)) => result?,
            Ok(Err(join_error)) if join_error.is_panic() => {
                return Err(RunnerError::WorkflowPanicked);
            }
            Ok(Err(_)) => return Err(RunnerError::WorkflowJoinFailed),
            Err(_) => {
                workflow_task.abort();
                return Err(RunnerError::WorkflowTimeout {
                    timeout: self.config.workflow_timeout,
                });
            }
        };
        if ack.retry_disposition() == InboundRetryDisposition::Retry {
            return Err(ProductAdapterError::WorkflowTransient {
                reason: RedactedString::new("workflow requested inbound retry"),
            }
            .into());
        }
        Ok(WebhookProcessOutcome::Acknowledged { ack })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use http::HeaderMap;
    use http::header::HeaderValue;
    use ironclaw_product_adapters::capabilities::ProductAdapterCapabilities;
    use ironclaw_product_adapters::external::{
        ExternalActorRef, ExternalConversationRef, ExternalEventId,
    };
    use ironclaw_product_adapters::identity::{
        AdapterInstallationId, ProductAdapterId, ProductSurfaceKind,
    };
    use ironclaw_product_adapters::{
        AuthRequirement, OutboundDeliverySink, ParsedProductInbound, ProductInboundPayload,
        ProductOutboundEnvelope, ProductRejection, ProductRejectionKind, ProductRenderOutcome,
        ProductTriggerReason, ProjectionSubscriptionRequest, ProtocolHttpEgress,
        UserMessagePayload,
    };
    use tokio::sync::Notify;

    use super::*;

    /// Shared `AuthRequirement` for the stub adapters. Matches the
    /// `SharedSecretHeader` strategy used by `shared_secret_auth()` below so
    /// the host-side seal is satisfiable for the success-path tests.
    fn stub_auth_requirement() -> AuthRequirement {
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Test-Secret".into(),
        }
    }

    struct StaticAdapter {
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        capabilities: ProductAdapterCapabilities,
        auth_requirement: AuthRequirement,
        parsed: ParsedProductInbound,
        parse_count: Option<Arc<AtomicUsize>>,
    }

    impl StaticAdapter {
        fn new(parsed: ParsedProductInbound) -> Self {
            Self {
                adapter_id: ProductAdapterId::new("telegram_v2").expect("valid"),
                installation_id: AdapterInstallationId::new("install_alpha").expect("valid"),
                capabilities: ProductAdapterCapabilities::empty(),
                auth_requirement: stub_auth_requirement(),
                parsed,
                parse_count: None,
            }
        }

        fn with_auth_requirement(mut self, auth_requirement: AuthRequirement) -> Self {
            self.auth_requirement = auth_requirement;
            self
        }

        fn with_parse_count(mut self, parse_count: Arc<AtomicUsize>) -> Self {
            self.parse_count = Some(parse_count);
            self
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
            &self.auth_requirement
        }

        fn parse_inbound(
            &self,
            _raw_payload: &[u8],
            _auth_evidence: &ProtocolAuthEvidence,
        ) -> Result<ParsedProductInbound, ProductAdapterError> {
            if let Some(parse_count) = &self.parse_count {
                parse_count.fetch_add(1, Ordering::SeqCst);
            }
            Ok(self.parsed.clone())
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

    struct PanicAdapter {
        adapter_id: ProductAdapterId,
        installation_id: AdapterInstallationId,
        capabilities: ProductAdapterCapabilities,
        auth_requirement: AuthRequirement,
    }

    impl PanicAdapter {
        fn new() -> Self {
            Self {
                adapter_id: ProductAdapterId::new("telegram_v2").expect("valid"),
                installation_id: AdapterInstallationId::new("install_alpha").expect("valid"),
                capabilities: ProductAdapterCapabilities::empty(),
                auth_requirement: stub_auth_requirement(),
            }
        }
    }

    #[async_trait]
    impl ProductAdapter for PanicAdapter {
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
            _raw_payload: &[u8],
            _auth_evidence: &ProtocolAuthEvidence,
        ) -> Result<ParsedProductInbound, ProductAdapterError> {
            panic!("adapter parse panic must be contained")
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

    /// Helper for workflow stubs: `resolve_projection_subscription` is never
    /// exercised by the runner tests (the runner only invokes `accept_inbound`),
    /// but the trait requires it. Return a deterministic adapter-shape error
    /// so accidental calls fail loudly.
    fn projection_subscription_unimplemented() -> ProductAdapterError {
        ProductAdapterError::Internal {
            detail: ironclaw_product_adapters::redaction::RedactedString::new(
                "test stub: resolve_projection_subscription not supported",
            ),
        }
    }

    struct AckWorkflow;

    #[async_trait]
    impl ProductWorkflow for AckWorkflow {
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
            Err(projection_subscription_unimplemented())
        }
    }

    struct RetryableRejectedWorkflow;

    #[async_trait]
    impl ProductWorkflow for RetryableRejectedWorkflow {
        async fn accept_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            Ok(ProductInboundAck::Rejected(ProductRejection::retryable(
                ProductRejectionKind::PolicyDenied,
                "policy temporarily unavailable",
            )))
        }

        async fn resolve_projection_subscription(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
            Err(projection_subscription_unimplemented())
        }
    }

    struct RecordingWorkflow {
        seen_payloads: Arc<Mutex<Vec<ProductInboundPayload>>>,
    }

    #[async_trait]
    impl ProductWorkflow for RecordingWorkflow {
        async fn accept_inbound(
            &self,
            envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            self.seen_payloads
                .lock()
                .expect("seen payloads mutex should not be poisoned")
                .push(envelope.payload().clone());
            Ok(ProductInboundAck::NoOp)
        }

        async fn resolve_projection_subscription(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
            Err(projection_subscription_unimplemented())
        }
    }

    struct PendingWorkflow;

    #[async_trait]
    impl ProductWorkflow for PendingWorkflow {
        async fn accept_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            std::future::pending().await
        }

        async fn resolve_projection_subscription(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
            Err(projection_subscription_unimplemented())
        }
    }

    struct PanicWorkflow;

    #[async_trait]
    impl ProductWorkflow for PanicWorkflow {
        async fn accept_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            panic!("workflow panic must be contained")
        }

        async fn resolve_projection_subscription(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
            Err(projection_subscription_unimplemented())
        }
    }

    struct BlockingWorkflow {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl ProductWorkflow for BlockingWorkflow {
        async fn accept_inbound(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProductInboundAck, ProductAdapterError> {
            self.entered.notify_waiters();
            self.release.notified().await;
            Ok(ProductInboundAck::NoOp)
        }

        async fn resolve_projection_subscription(
            &self,
            _envelope: ProductInboundEnvelope,
        ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
            Err(projection_subscription_unimplemented())
        }
    }

    /// Sample `ParsedProductInbound` with a non-NoOp payload.
    fn sample_parsed() -> ParsedProductInbound {
        ParsedProductInbound::new(
            ExternalEventId::new("update:42").expect("valid"),
            ExternalActorRef::new("telegram_user", "777", None::<String>).expect("valid"),
            ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-100"))
                .expect("valid"),
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello", Vec::new(), ProductTriggerReason::DirectChat)
                    .expect("valid"),
            ),
        )
        .expect("valid parsed")
    }

    fn sample_noop_parsed() -> ParsedProductInbound {
        ParsedProductInbound::new(
            ExternalEventId::new("update:noop").expect("valid"),
            ExternalActorRef::new("telegram_user", "777", None::<String>).expect("valid"),
            ExternalConversationRef::new(None, "12345", Some("topic-7"), Some("msg-100"))
                .expect("valid"),
            ProductInboundPayload::NoOp,
        )
        .expect("valid parsed")
    }

    fn shared_secret_auth() -> WebhookAuth {
        WebhookAuth::SharedSecretHeader(SharedSecretHeaderAuth {
            header_name: "X-Test-Secret".into(),
            expected_secret: "topsecret".into(),
            subject: "telegram_install_alpha".into(),
        })
    }

    fn auth_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("X-Test-Secret", HeaderValue::from_static("topsecret"));
        headers
    }

    fn test_config(max_in_flight: usize, timeout: Duration) -> NativeProductAdapterRunnerConfig {
        NativeProductAdapterRunnerConfig::new(
            timeout,
            std::num::NonZeroUsize::new(max_in_flight).expect("nonzero"),
        )
    }

    #[tokio::test]
    async fn process_webhook_routes_noop_payload_through_workflow() {
        let seen_payloads = Arc::new(Mutex::new(Vec::new()));
        let runner = NativeProductAdapterRunner::with_config(
            Arc::new(StaticAdapter::new(sample_noop_parsed())),
            Arc::new(RecordingWorkflow {
                seen_payloads: Arc::clone(&seen_payloads),
            }),
            shared_secret_auth(),
            test_config(1, Duration::from_secs(1)),
        );

        let outcome = runner
            .process_webhook(&auth_headers(), b"{}")
            .await
            .expect("NoOp payload should still be acknowledged by workflow");

        assert_eq!(
            outcome,
            WebhookProcessOutcome::Acknowledged {
                ack: ProductInboundAck::NoOp
            }
        );
        assert_eq!(
            seen_payloads
                .lock()
                .expect("seen payloads mutex should not be poisoned")
                .as_slice(),
            &[ProductInboundPayload::NoOp]
        );
    }

    #[tokio::test]
    async fn process_webhook_accepts_case_insensitive_shared_secret_header_match() {
        let adapter = StaticAdapter::new(sample_parsed()).with_auth_requirement(
            AuthRequirement::SharedSecretHeader {
                header_name: "x-test-secret".into(),
            },
        );
        let runner = NativeProductAdapterRunner::with_config(
            Arc::new(adapter),
            Arc::new(AckWorkflow),
            shared_secret_auth(),
            test_config(1, Duration::from_secs(1)),
        );

        let outcome = runner
            .process_webhook(&auth_headers(), b"{}")
            .await
            .expect("case-only header-name differences should not reject matching auth");

        assert!(matches!(
            outcome,
            WebhookProcessOutcome::Acknowledged {
                ack: ProductInboundAck::NoOp
            }
        ));
    }

    #[tokio::test]
    async fn process_webhook_surfaces_retryable_policy_rejection_as_retryable_error() {
        let runner = NativeProductAdapterRunner::with_config(
            Arc::new(StaticAdapter::new(sample_parsed())),
            Arc::new(RetryableRejectedWorkflow),
            shared_secret_auth(),
            test_config(1, Duration::from_secs(1)),
        );

        let err = runner
            .process_webhook(&auth_headers(), b"{}")
            .await
            .expect_err("retryable workflow ack should ask protocol to redeliver");

        assert!(err.is_retryable());
        assert!(matches!(
            err,
            RunnerError::Adapter(ProductAdapterError::WorkflowTransient { .. })
        ));
    }

    #[test]
    fn webhook_auth_matches_hmac_requirement_header_names_case_insensitively() {
        let auth = WebhookAuth::Hmac(HmacWebhookAuth::new(
            "X-Signature",
            "X-Timestamp",
            b"topsecret".to_vec(),
            "telegram_install_alpha",
        ));
        let requirement = AuthRequirement::RequestSignature {
            header_name: "x-signature".into(),
            timestamp_header_name: Some("x-timestamp".into()),
        };

        assert!(auth.matches_requirement(&requirement));
    }

    #[tokio::test]
    async fn process_webhook_rejects_auth_strategy_mismatch_before_parse() {
        let parse_count = Arc::new(AtomicUsize::new(0));
        let adapter = StaticAdapter::new(sample_parsed())
            .with_auth_requirement(AuthRequirement::RequestSignature {
                header_name: "X-Signature".into(),
                timestamp_header_name: Some("X-Timestamp".into()),
            })
            .with_parse_count(Arc::clone(&parse_count));
        let runner = NativeProductAdapterRunner::with_config(
            Arc::new(adapter),
            Arc::new(AckWorkflow),
            shared_secret_auth(),
            test_config(1, Duration::from_secs(1)),
        );

        let err = runner
            .process_webhook(&auth_headers(), b"{}")
            .await
            .expect_err("auth strategy mismatch should fail closed");

        assert!(err.is_auth_failure());
        assert!(!err.is_retryable());
        assert!(matches!(
            err,
            RunnerError::AuthenticationFailed {
                failure: ProtocolAuthFailure::Other { .. }
            }
        ));
        assert_eq!(parse_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn process_webhook_times_out_slow_workflow() {
        let runner = NativeProductAdapterRunner::with_config(
            Arc::new(StaticAdapter::new(sample_parsed())),
            Arc::new(PendingWorkflow),
            shared_secret_auth(),
            test_config(1, Duration::from_millis(5)),
        );
        let err = runner
            .process_webhook(&auth_headers(), b"{}")
            .await
            .expect_err("slow workflow should time out");
        assert!(matches!(err, RunnerError::WorkflowTimeout { .. }));
    }

    #[tokio::test]
    async fn process_webhook_rejects_when_max_in_flight_reached() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let runner = Arc::new(NativeProductAdapterRunner::with_config(
            Arc::new(StaticAdapter::new(sample_parsed())),
            Arc::new(BlockingWorkflow {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }),
            shared_secret_auth(),
            test_config(1, Duration::from_secs(1)),
        ));
        let first_runner = Arc::clone(&runner);
        let first_headers = auth_headers();
        let first =
            tokio::spawn(async move { first_runner.process_webhook(&first_headers, b"{}").await });
        entered.notified().await;

        let err = runner
            .process_webhook(&auth_headers(), b"{}")
            .await
            .expect_err("second request should be rejected by admission control");
        assert_eq!(err, RunnerError::TooManyInFlight { max_in_flight: 1 });

        release.notify_waiters();
        first.await.expect("join").expect("first request succeeds");
    }

    #[tokio::test]
    async fn process_webhook_contains_adapter_panics() {
        let runner = NativeProductAdapterRunner::with_config(
            Arc::new(PanicAdapter::new()),
            Arc::new(AckWorkflow),
            shared_secret_auth(),
            test_config(1, Duration::from_secs(1)),
        );
        let err = runner
            .process_webhook(&auth_headers(), b"{}")
            .await
            .expect_err("adapter panic should become runner error");
        assert_eq!(err, RunnerError::AdapterPanicked);
    }

    #[tokio::test]
    async fn process_webhook_contains_workflow_panics() {
        let runner = NativeProductAdapterRunner::with_config(
            Arc::new(StaticAdapter::new(sample_parsed())),
            Arc::new(PanicWorkflow),
            shared_secret_auth(),
            test_config(1, Duration::from_secs(1)),
        );
        let err = runner
            .process_webhook(&auth_headers(), b"{}")
            .await
            .expect_err("workflow panic should become runner error");
        assert_eq!(err, RunnerError::WorkflowPanicked);
    }
}
