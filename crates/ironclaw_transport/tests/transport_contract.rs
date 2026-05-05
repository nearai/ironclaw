use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
use ironclaw_transport::{
    AttachmentKind, TransportAdapter, TransportAdapterId, TransportAttachment,
    TransportDeliveryAck, TransportEgress, TransportError, TransportErrorKind, TransportHealth,
    TransportIngress, TransportIngressSink, TransportMessage, TransportMessageId,
    TransportMetadata, TransportRegistry, TransportReply, TransportRoute, TransportSubmission,
    TransportThreadId,
};
use serde_json::json;
use tokio::sync::Mutex;

fn scope(user: &str) -> ResourceScope {
    ResourceScope::local_default(
        UserId::new(user).expect("valid user id"),
        InvocationId::new(),
    )
    .expect("valid local scope")
}

fn adapter_id(name: &str) -> TransportAdapterId {
    TransportAdapterId::new(name).expect("valid adapter id")
}

fn route(adapter: &str, user: &str) -> TransportRoute {
    TransportRoute {
        adapter_id: adapter_id(adapter),
        scope: scope(user),
        recipient: Some("external-user-1".to_string()),
        conversation_id: Some("conversation-1".to_string()),
        thread_id: Some(TransportThreadId::new("thread-1").expect("valid thread id")),
        metadata: TransportMetadata::from([("channel_id".to_string(), json!("C123"))]),
    }
}

#[derive(Default)]
struct RecordingSink {
    submissions: Mutex<Vec<TransportIngress>>,
}

#[async_trait]
impl TransportIngressSink for RecordingSink {
    async fn submit_ingress(
        &self,
        ingress: TransportIngress,
    ) -> Result<TransportSubmission, TransportError> {
        self.submissions.lock().await.push(ingress);
        Ok(TransportSubmission {
            accepted_at: chrono::Utc::now(),
            correlation_id: None,
        })
    }
}

struct StartupAdapter {
    id: TransportAdapterId,
    ingress: TransportIngress,
}

#[async_trait]
impl TransportAdapter for StartupAdapter {
    fn adapter_id(&self) -> &TransportAdapterId {
        &self.id
    }

    async fn start(&self, sink: Arc<dyn TransportIngressSink>) -> Result<(), TransportError> {
        sink.submit_ingress(self.ingress.clone()).await?;
        Ok(())
    }

    async fn deliver(
        &self,
        _egress: TransportEgress,
    ) -> Result<TransportDeliveryAck, TransportError> {
        unreachable!("startup adapter is not used for delivery in this test")
    }

    async fn health_check(&self) -> Result<TransportHealth, TransportError> {
        Ok(TransportHealth::healthy())
    }
}

#[tokio::test]
async fn adapter_normalizes_ingress_and_does_not_let_metadata_override_scope() {
    let mut metadata = TransportMetadata::new();
    metadata.insert("user_id".to_string(), json!("mallory"));
    metadata.insert("thread_id".to_string(), json!("spoofed-thread"));

    let ingress = TransportIngress {
        message_id: TransportMessageId::new("web-message-1").expect("valid message id"),
        route: route("gateway", "alice"),
        message: TransportMessage {
            text: "hello from the browser".to_string(),
            attachments: vec![TransportAttachment {
                id: "upload-1".to_string(),
                kind: AttachmentKind::Document,
                mime_type: Some("text/plain".to_string()),
                filename: Some("notes.txt".to_string()),
                size_bytes: Some(24),
                data: b"hello attachment".to_vec(),
                storage_ref: Some("workspace://uploads/upload-1".to_string()),
                source_url: None,
                metadata: TransportMetadata::new(),
            }],
        },
        sender_display_name: Some("Alice".to_string()),
        timezone: Some("America/Los_Angeles".to_string()),
        received_at: chrono::Utc::now(),
        metadata,
    };

    let sink = Arc::new(RecordingSink::default());
    let adapter = StartupAdapter {
        id: adapter_id("gateway"),
        ingress,
    };

    adapter
        .start(sink.clone())
        .await
        .expect("adapter should submit normalized ingress");

    let submissions = sink.submissions.lock().await;
    assert_eq!(submissions.len(), 1);
    let submitted = &submissions[0];
    assert_eq!(submitted.route.scope.user_id, UserId::new("alice").unwrap());
    assert_eq!(
        submitted.route.thread_id,
        Some(TransportThreadId::new("thread-1").unwrap())
    );
    assert_eq!(submitted.metadata["user_id"], json!("mallory"));
    assert_eq!(
        submitted.message.attachments[0].storage_ref.as_deref(),
        Some("workspace://uploads/upload-1")
    );
    assert_eq!(
        submitted.message.attachments[0].data,
        b"hello attachment".to_vec()
    );
}

struct RecordingAdapter {
    id: TransportAdapterId,
    delivered: Mutex<Vec<TransportEgress>>,
}

impl RecordingAdapter {
    fn new(id: &str) -> Self {
        Self {
            id: adapter_id(id),
            delivered: Mutex::new(Vec::new()),
        }
    }
}

struct StartResultAdapter {
    id: TransportAdapterId,
    fail_start: bool,
    started_count: Mutex<usize>,
}

impl StartResultAdapter {
    fn new(id: &str, fail_start: bool) -> Self {
        Self {
            id: adapter_id(id),
            fail_start,
            started_count: Mutex::new(0),
        }
    }

    async fn started_count(&self) -> usize {
        *self.started_count.lock().await
    }
}

#[async_trait]
impl TransportAdapter for StartResultAdapter {
    fn adapter_id(&self) -> &TransportAdapterId {
        &self.id
    }

    async fn start(&self, _sink: Arc<dyn TransportIngressSink>) -> Result<(), TransportError> {
        *self.started_count.lock().await += 1;
        if self.fail_start {
            return Err(TransportError::new(
                TransportErrorKind::StartupFailed,
                format!("{} unavailable", self.id),
            ));
        }
        Ok(())
    }

    async fn deliver(
        &self,
        _egress: TransportEgress,
    ) -> Result<TransportDeliveryAck, TransportError> {
        unreachable!("start-result adapter is not used for delivery in this test")
    }

    async fn health_check(&self) -> Result<TransportHealth, TransportError> {
        Ok(TransportHealth::healthy())
    }
}

#[async_trait]
impl TransportAdapter for RecordingAdapter {
    fn adapter_id(&self) -> &TransportAdapterId {
        &self.id
    }

    async fn start(&self, _sink: Arc<dyn TransportIngressSink>) -> Result<(), TransportError> {
        Ok(())
    }

    async fn deliver(
        &self,
        egress: TransportEgress,
    ) -> Result<TransportDeliveryAck, TransportError> {
        self.delivered.lock().await.push(egress);
        Ok(TransportDeliveryAck {
            adapter_id: self.id.clone(),
            delivered_at: chrono::Utc::now(),
        })
    }

    async fn health_check(&self) -> Result<TransportHealth, TransportError> {
        Ok(TransportHealth::healthy())
    }
}

#[tokio::test]
async fn registry_start_all_keeps_starting_after_adapter_failures() {
    let failing = Arc::new(StartResultAdapter::new("failing", true));
    let healthy = Arc::new(StartResultAdapter::new("healthy", false));
    let registry = TransportRegistry::new();
    registry
        .register(failing.clone())
        .expect("register failing");
    registry
        .register(healthy.clone())
        .expect("register healthy");

    let reports = registry
        .start_all(Arc::new(RecordingSink::default()))
        .await
        .expect("start_all should not fail at the registry level");
    assert_eq!(reports.len(), 2);
    let failed: Vec<_> = reports.iter().filter(|r| r.result.is_err()).collect();
    let succeeded: Vec<_> = reports.iter().filter(|r| r.result.is_ok()).collect();
    assert_eq!(failed.len(), 1, "one adapter should report failure");
    assert_eq!(succeeded.len(), 1, "one adapter should report success");

    assert_eq!(failing.started_count().await, 1);
    assert_eq!(healthy.started_count().await, 1);
}

#[tokio::test]
async fn registry_start_all_reports_per_adapter_failures() {
    let failing = Arc::new(StartResultAdapter::new("failing", true));
    let registry = TransportRegistry::new();
    registry.register(failing).expect("register failing");

    let reports = registry
        .start_all(Arc::new(RecordingSink::default()))
        .await
        .expect("start_all should report per-adapter results, not aggregate-error");
    assert_eq!(reports.len(), 1);
    assert!(reports[0].result.is_err());
}

#[tokio::test]
async fn registry_start_all_errors_when_no_adapters_registered() {
    let registry = TransportRegistry::new();
    let error = registry
        .start_all(Arc::new(RecordingSink::default()))
        .await
        .expect_err("empty registry must error");
    assert_eq!(error.kind(), TransportErrorKind::StartupFailed);
}

#[tokio::test]
async fn registry_routes_egress_to_named_adapter_and_fails_closed_for_unknown_adapter() {
    let gateway = Arc::new(RecordingAdapter::new("gateway"));
    let slack = Arc::new(RecordingAdapter::new("slack"));
    let registry = TransportRegistry::new();
    registry
        .register(gateway.clone())
        .expect("register gateway");
    registry.register(slack.clone()).expect("register slack");

    let egress = TransportEgress::Reply(TransportReply {
        route: route("slack", "alice"),
        content: "done".to_string(),
        attachments: Vec::new(),
        metadata: TransportMetadata::new(),
    });

    registry
        .deliver(&adapter_id("slack"), egress)
        .await
        .expect("delivery should be routed to slack");

    assert!(gateway.delivered.lock().await.is_empty());
    assert_eq!(slack.delivered.lock().await.len(), 1);

    let error = registry
        .deliver(
            &adapter_id("missing"),
            TransportEgress::Reply(TransportReply {
                route: route("missing", "alice"),
                content: "hello".to_string(),
                attachments: Vec::new(),
                metadata: TransportMetadata::new(),
            }),
        )
        .await
        .expect_err("unknown adapter must fail closed");
    assert_eq!(error.kind(), TransportErrorKind::AdapterNotFound);
}

#[tokio::test]
async fn registry_rejects_egress_with_mismatched_route_adapter() {
    let gateway = Arc::new(RecordingAdapter::new("gateway"));
    let registry = TransportRegistry::new();
    registry
        .register(gateway.clone())
        .expect("register gateway");

    let error = registry
        .deliver(
            &adapter_id("gateway"),
            TransportEgress::Reply(TransportReply {
                route: route("slack", "alice"),
                content: "wrong route".to_string(),
                attachments: Vec::new(),
                metadata: TransportMetadata::new(),
            }),
        )
        .await
        .expect_err("mismatched route adapter must be rejected");

    assert_eq!(error.kind(), TransportErrorKind::InvalidRequest);
    assert!(gateway.delivered.lock().await.is_empty());
}

#[tokio::test]
async fn duplicate_adapter_registration_is_rejected() {
    let registry = TransportRegistry::new();
    registry
        .register(Arc::new(RecordingAdapter::new("gateway")))
        .expect("first registration succeeds");

    let error = registry
        .register(Arc::new(RecordingAdapter::new("gateway")))
        .expect_err("duplicate adapter must be rejected");

    assert_eq!(error.kind(), TransportErrorKind::AdapterAlreadyExists);
}

#[tokio::test]
async fn registry_replace_and_unregister_update_delivery_target() {
    let first = Arc::new(RecordingAdapter::new("gateway"));
    let second = Arc::new(RecordingAdapter::new("gateway"));
    let registry = TransportRegistry::new();
    registry.register(first.clone()).expect("register first");

    let replaced = registry.replace(second.clone()).expect("replace adapter");
    assert!(replaced.is_some());

    registry
        .deliver(
            &adapter_id("gateway"),
            TransportEgress::Reply(TransportReply {
                route: route("gateway", "alice"),
                content: "replacement".to_string(),
                attachments: Vec::new(),
                metadata: TransportMetadata::new(),
            }),
        )
        .await
        .expect("delivery should reach replacement");

    assert!(first.delivered.lock().await.is_empty());
    assert_eq!(second.delivered.lock().await.len(), 1);

    let removed = registry
        .unregister(&adapter_id("gateway"))
        .expect("unregister adapter");
    assert!(removed.is_some());

    let error = registry
        .deliver(
            &adapter_id("gateway"),
            TransportEgress::Reply(TransportReply {
                route: route("gateway", "alice"),
                content: "gone".to_string(),
                attachments: Vec::new(),
                metadata: TransportMetadata::new(),
            }),
        )
        .await
        .expect_err("unregistered adapter must fail closed");
    assert_eq!(error.kind(), TransportErrorKind::AdapterNotFound);
}

#[test]
fn transport_errors_are_stable_and_redacted() {
    let error = TransportError::new(
        TransportErrorKind::DeliveryFailed,
        "provider failed with token sk-test-secret at /Users/alice/.ironclaw/config",
    );

    assert_eq!(error.kind(), TransportErrorKind::DeliveryFailed);
    assert_eq!(error.safe_reason(), "redacted");
    let display = error.to_string();
    assert!(display.contains("delivery_failed"));
    assert!(!display.contains("sk-test-secret"));
    assert!(!display.contains("/Users/alice"));
}

/// Adapter whose health and shutdown selectively fail; used to exercise the
/// best-effort policy in [`TransportRegistry::health_check_all`] and
/// [`TransportRegistry::shutdown_all`].
struct FlakyAdapter {
    id: TransportAdapterId,
    fail_health: bool,
    fail_shutdown: bool,
    shutdown_count: Mutex<usize>,
}

impl FlakyAdapter {
    fn new(id: &str, fail_health: bool, fail_shutdown: bool) -> Self {
        Self {
            id: adapter_id(id),
            fail_health,
            fail_shutdown,
            shutdown_count: Mutex::new(0),
        }
    }

    async fn shutdown_count(&self) -> usize {
        *self.shutdown_count.lock().await
    }
}

#[async_trait]
impl TransportAdapter for FlakyAdapter {
    fn adapter_id(&self) -> &TransportAdapterId {
        &self.id
    }

    async fn start(&self, _sink: Arc<dyn TransportIngressSink>) -> Result<(), TransportError> {
        Ok(())
    }

    async fn deliver(
        &self,
        _egress: TransportEgress,
    ) -> Result<TransportDeliveryAck, TransportError> {
        unreachable!("flaky adapter is not used for delivery")
    }

    async fn health_check(&self) -> Result<TransportHealth, TransportError> {
        if self.fail_health {
            return Err(TransportError::new(
                TransportErrorKind::Unavailable,
                format!("{} unhealthy", self.id),
            ));
        }
        Ok(TransportHealth::healthy())
    }

    async fn shutdown(&self) -> Result<(), TransportError> {
        *self.shutdown_count.lock().await += 1;
        if self.fail_shutdown {
            return Err(TransportError::new(
                TransportErrorKind::Internal,
                format!("{} shutdown failed", self.id),
            ));
        }
        Ok(())
    }
}

#[tokio::test]
async fn registry_health_check_reports_per_adapter_and_does_not_short_circuit() {
    let healthy = Arc::new(FlakyAdapter::new("healthy", false, false));
    let unhealthy = Arc::new(FlakyAdapter::new("unhealthy", true, false));
    let registry = TransportRegistry::new();
    registry.register(healthy).expect("register healthy");
    registry.register(unhealthy).expect("register unhealthy");

    let reports = registry
        .health_check_all()
        .await
        .expect("registry-level error not expected");
    assert_eq!(reports.len(), 2);
    let healthy_ids: Vec<_> = reports
        .iter()
        .filter(|r| r.result.is_ok())
        .map(|r| r.adapter_id.as_str().to_string())
        .collect();
    let unhealthy_ids: Vec<_> = reports
        .iter()
        .filter(|r| r.result.is_err())
        .map(|r| r.adapter_id.as_str().to_string())
        .collect();
    assert_eq!(healthy_ids, vec!["healthy".to_string()]);
    assert_eq!(unhealthy_ids, vec!["unhealthy".to_string()]);
}

#[tokio::test]
async fn registry_shutdown_visits_all_adapters_even_when_one_fails() {
    let bad = Arc::new(FlakyAdapter::new("bad", false, true));
    let good = Arc::new(FlakyAdapter::new("good", false, false));
    let registry = TransportRegistry::new();
    registry.register(bad.clone()).expect("register bad");
    registry.register(good.clone()).expect("register good");

    let result = registry.shutdown_all().await;
    assert!(result.is_err(), "first failure must surface");
    assert_eq!(bad.shutdown_count().await, 1);
    assert_eq!(
        good.shutdown_count().await,
        1,
        "good adapter must still be shut down even though earlier adapter failed"
    );
}

#[tokio::test]
async fn validate_public_id_rejects_path_traversal_and_spaces() {
    assert!(TransportAdapterId::new("../etc/passwd").is_err());
    assert!(TransportAdapterId::new("has space").is_err());
    assert!(TransportAdapterId::new("").is_err());
    assert!(TransportAdapterId::new("valid-id_123").is_ok());
}
