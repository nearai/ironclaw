//! The corrected publication order and crash recovery: desired revision
//! durable before every provider call, provider-independent preparation
//! before the claim, the sink timeout clamped to the lease TTL, the boot
//! sweep over the outbound attempt index, and the journal acknowledgement
//! awaited behind a stable observer id. Split from the parent cadence suite
//! by theme; it drives the same harness.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_contracts::channel::ReplyTransport;
use ironclaw_extension_contracts::reply::{ReplyAudience, ReplyReconcilePoint};
use ironclaw_extension_contracts::test_support::fakes::RecordingReplySink;
use ironclaw_extension_contracts::tool_adapter::RestrictedEgress;
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::product_adapter::AdapterInstallationId;
use ironclaw_host_api::turn::{TurnRunId, TurnScope, TurnStatus};
use ironclaw_outbound::{
    OutboundStateStorePort, ReplyPublicationSettlement, ReplyPublicationStatus,
};
use ironclaw_product_contracts::delivery::{
    ChannelDeliveryResolver, DeliveryReplyContextError, DeliveryReplyContextSource,
    ResolvedChannelDelivery,
};
use ironclaw_threads::{MessageContent, SessionThreadService};
use ironclaw_turns::TurnCoordinator;

use super::{
    DenyAllEgress, FakeTurnKernel, FixedReplyContext, SinkResolver, harness, settings, wait_until,
};
use crate::delivery_coordinator::publication::ReplyPublicationWiring;
use crate::delivery_coordinator::{
    DeliveryCoordinator, DeliveryRetryPolicy, NoDeliveryRegistrations,
};
use crate::projection::reply::ReplyProjection;

/// A sink that checks, on every reconcile, that the store already carries a
/// desired revision at least as new as the one being published. Violations
/// are collected rather than panicking: the worker runs on its own task, and
/// a panic there would abort the worker instead of failing the test.
struct DesiredBeforeEgressSink {
    store: Arc<dyn OutboundStateStorePort>,
    scope: TurnScope,
    run_id: TurnRunId,
    violations: Mutex<Vec<String>>,
    calls: Mutex<u32>,
}

#[async_trait]
impl ironclaw_extension_contracts::reply::ReplySink for DesiredBeforeEgressSink {
    async fn reconcile(
        &self,
        request: ironclaw_extension_contracts::reply::ReplyReconcileRequest,
        _egress: &dyn RestrictedEgress,
    ) -> Result<
        ironclaw_extension_contracts::reply::ReplySinkReport,
        ironclaw_extension_contracts::channel_adapter::ChannelError,
    > {
        *self.calls.lock().unwrap() += 1;
        let records = self
            .store
            .list_reply_publications(self.scope.clone(), self.run_id)
            .await
            .unwrap_or_default();
        let durable_desired = records
            .iter()
            .map(|record| record.publication.desired_revision)
            .max()
            .unwrap_or(0);
        if durable_desired < request.revision.revision {
            self.violations.lock().unwrap().push(format!(
                "provider was called for revision {} while the store's desired revision was {}",
                request.revision.revision, durable_desired
            ));
        }
        Ok(
            ironclaw_extension_contracts::reply::ReplySinkReport::applied(
                None,
                ironclaw_extension_contracts::reply::ReplySinkEvidence::default(),
            ),
        )
    }
}

/// A resolver over an arbitrary sink (the harness's is fixed to the
/// recording fake).
struct AnySinkResolver {
    sink: Arc<dyn ironclaw_extension_contracts::reply::ReplySink>,
    transport: ReplyTransport,
}

impl ChannelDeliveryResolver for AnySinkResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        Some(ResolvedChannelDelivery {
            extension_id: ExtensionId::new(extension_id).ok()?,
            installation_id: AdapterInstallationId::new("inst-1").ok()?,
            reply: Some(Arc::clone(&self.sink)),
            delivery: None,
            egress: Arc::new(DenyAllEgress),
            reply_transport: Some(self.transport),
            generation: 3,
            requires_enrollment: false,
            declared_egress_hosts: Vec::new(),
        })
    }
}

#[tokio::test]
async fn the_desired_revision_is_durable_before_every_provider_call() {
    let base = harness("desired-first", ReplyTransport::Stream, None);
    let sink = Arc::new(DesiredBeforeEgressSink {
        store: Arc::clone(&base.store),
        scope: base.scope.clone(),
        run_id: base.run_id,
        violations: Mutex::new(Vec::new()),
        calls: Mutex::new(0),
    });
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&base.store),
        Arc::new(AnySinkResolver {
            sink: Arc::clone(&sink) as Arc<dyn ironclaw_extension_contracts::reply::ReplySink>,
            transport: ReplyTransport::Stream,
        }),
        Arc::new(FixedReplyContext(Some(b"vendor-ctx".to_vec()))),
        Arc::new(NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    assert!(coordinator.start_reply_publication(ReplyPublicationWiring {
        projection: Arc::clone(&base.projection),
        turn_coordinator: Arc::clone(&base.kernel) as Arc<dyn TurnCoordinator>,
        thread_service: Arc::clone(&base.threads) as Arc<dyn SessionThreadService>,
        approval_context: None,
        blocked_auth_prompts: None,
        project_filesystem: Arc::new(crate::NoProjectFilesystem),
        session_channel: None,
        settings: settings(),
    }));
    coordinator
        .register_reply_target(base.registration(ReplyAudience::Private))
        .await
        .unwrap();
    base.text("progressive text");
    wait_until(|| async { (*sink.calls.lock().unwrap() >= 1).then_some(()) }).await;
    base.complete_with("the final answer").await;
    coordinator
        .reply_run_terminal(&base.scope, base.run_id)
        .await;
    let settled = wait_until(|| async {
        base.publications()
            .await
            .into_iter()
            .find(|record| !record.publication.status.is_active())
    })
    .await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    assert!(
        *sink.calls.lock().unwrap() >= 2,
        "a progressive and a terminal reconcile both reached the provider"
    );
    let violations = sink.violations.lock().unwrap().clone();
    assert!(
        violations.is_empty(),
        "the desired revision must be durable before provider access: {violations:?}"
    );
}

/// The reply-context read is provider-independent preparation and happens
/// before ownership is taken: on the first reconcile of a publication no
/// lease may exist yet when the context is read.
struct LeaseObservingReplyContext {
    store: Arc<dyn OutboundStateStorePort>,
    scope: TurnScope,
    run_id: TurnRunId,
    lease_seen_on_first_read: Mutex<Option<bool>>,
}

#[async_trait]
impl DeliveryReplyContextSource for LeaseObservingReplyContext {
    async fn reply_context(
        &self,
        _extension_id: &ExtensionId,
        _installation_id: &AdapterInstallationId,
        _conversation_fingerprint: &str,
    ) -> Result<Option<Vec<u8>>, DeliveryReplyContextError> {
        let unseen = self.lease_seen_on_first_read.lock().unwrap().is_none();
        if unseen {
            let records = self
                .store
                .list_reply_publications(self.scope.clone(), self.run_id)
                .await
                .unwrap_or_default();
            let lease_present = records
                .iter()
                .any(|record| record.publication.lease.is_some());
            let mut first = self.lease_seen_on_first_read.lock().unwrap();
            if first.is_none() {
                *first = Some(lease_present);
            }
        }
        Ok(Some(b"vendor-ctx".to_vec()))
    }
}

#[tokio::test]
async fn provider_independent_preparation_runs_before_the_claim() {
    let base = harness("prep-first", ReplyTransport::Message, None);
    let context = Arc::new(LeaseObservingReplyContext {
        store: Arc::clone(&base.store),
        scope: base.scope.clone(),
        run_id: base.run_id,
        lease_seen_on_first_read: Mutex::new(None),
    });
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&base.store),
        Arc::new(AnySinkResolver {
            sink: Arc::clone(&base.sink) as Arc<dyn ironclaw_extension_contracts::reply::ReplySink>,
            transport: ReplyTransport::Message,
        }),
        Arc::clone(&context) as Arc<dyn DeliveryReplyContextSource>,
        Arc::new(NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    assert!(coordinator.start_reply_publication(ReplyPublicationWiring {
        projection: Arc::clone(&base.projection),
        turn_coordinator: Arc::clone(&base.kernel) as Arc<dyn TurnCoordinator>,
        thread_service: Arc::clone(&base.threads) as Arc<dyn SessionThreadService>,
        approval_context: None,
        blocked_auth_prompts: None,
        project_filesystem: Arc::new(crate::NoProjectFilesystem),
        session_channel: None,
        settings: settings(),
    }));
    coordinator
        .register_reply_target(base.registration(ReplyAudience::Private))
        .await
        .unwrap();
    base.complete_with("answer").await;
    coordinator
        .reply_run_terminal(&base.scope, base.run_id)
        .await;
    wait_until(|| async {
        base.publications()
            .await
            .into_iter()
            .find(|record| !record.publication.status.is_active())
    })
    .await;
    assert_eq!(
        *context.lease_seen_on_first_read.lock().unwrap(),
        Some(false),
        "the stored reply context is read before the publication claim is taken"
    );
}

/// The sink call is bounded by the lease: a provider call slower than the
/// lease TTL is cut off at the TTL (the timeout is clamped), recorded as
/// ambiguous, and the terminal budget settles `Unknown` — well before the
/// slow provider call would have returned on its own.
struct StallingSink {
    delay: Duration,
}

#[async_trait]
impl ironclaw_extension_contracts::reply::ReplySink for StallingSink {
    async fn reconcile(
        &self,
        _request: ironclaw_extension_contracts::reply::ReplyReconcileRequest,
        _egress: &dyn RestrictedEgress,
    ) -> Result<
        ironclaw_extension_contracts::reply::ReplySinkReport,
        ironclaw_extension_contracts::channel_adapter::ChannelError,
    > {
        tokio::time::sleep(self.delay).await;
        Ok(
            ironclaw_extension_contracts::reply::ReplySinkReport::applied(
                None,
                ironclaw_extension_contracts::reply::ReplySinkEvidence::default(),
            ),
        )
    }
}

#[tokio::test]
async fn a_sink_call_never_outlives_the_lease_that_covers_it() {
    let base = harness("lease-bound", ReplyTransport::Message, None);
    let mut bounded = settings();
    bounded.lease_ttl = Duration::from_millis(100);
    bounded.reconcile_timeout = Duration::from_secs(30);
    bounded.terminal_attempt_budget = 2;
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&base.store),
        Arc::new(AnySinkResolver {
            sink: Arc::new(StallingSink {
                delay: Duration::from_secs(20),
            }),
            transport: ReplyTransport::Message,
        }),
        Arc::new(FixedReplyContext(Some(b"vendor-ctx".to_vec()))),
        Arc::new(NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    assert!(coordinator.start_reply_publication(ReplyPublicationWiring {
        projection: Arc::clone(&base.projection),
        turn_coordinator: Arc::clone(&base.kernel) as Arc<dyn TurnCoordinator>,
        thread_service: Arc::clone(&base.threads) as Arc<dyn SessionThreadService>,
        approval_context: None,
        blocked_auth_prompts: None,
        project_filesystem: Arc::new(crate::NoProjectFilesystem),
        session_channel: None,
        settings: bounded,
    }));
    let started = tokio::time::Instant::now();
    coordinator
        .register_reply_target(base.registration(ReplyAudience::Private))
        .await
        .unwrap();
    base.complete_with("slow answer").await;
    coordinator
        .reply_run_terminal(&base.scope, base.run_id)
        .await;
    let settled = wait_until(|| async {
        base.publications()
            .await
            .into_iter()
            .find(|record| !record.publication.status.is_active())
    })
    .await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Unknown),
        "a provider call cut off by the lease bound is ambiguous, never claimed applied"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the sink timeout was clamped to the lease TTL, not the configured 30s"
    );
}

/// Crash after the journal acknowledged the terminal commit: nothing will
/// ever redeliver the event, and only the boot sweep over the outbound
/// attempt index can give the still-open publication a worker again.
#[tokio::test]
async fn the_boot_sweep_resumes_an_open_publication_without_any_journal_signal() {
    let first = harness("boot-sweep", ReplyTransport::Stream, None);
    first
        .coordinator
        .register_reply_target(first.registration(ReplyAudience::Private))
        .await
        .unwrap();
    first.text("half an answer");
    wait_until(|| async { (!first.sink.requests().is_empty()).then_some(()) }).await;
    // The first process dies after the journal cursor already advanced: its
    // workers stop, and no further terminal signal will arrive.
    first.coordinator.shutdown_reply_publication().await;
    first
        .commit_answer(MessageContent::text("the whole answer"))
        .await;

    let second_sink = Arc::new(RecordingReplySink::new("boot-sweep-2"));
    let second_kernel = Arc::new(FakeTurnKernel::default());
    second_kernel.set_status(TurnStatus::Completed);
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&first.store),
        Arc::new(AnySinkResolver {
            sink: Arc::clone(&second_sink)
                as Arc<dyn ironclaw_extension_contracts::reply::ReplySink>,
            transport: ReplyTransport::Stream,
        }),
        Arc::new(FixedReplyContext(Some(b"vendor-ctx".to_vec()))),
        Arc::new(NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    assert!(coordinator.start_reply_publication(ReplyPublicationWiring {
        projection: Arc::new(ReplyProjection::new()),
        turn_coordinator: Arc::clone(&second_kernel) as Arc<dyn TurnCoordinator>,
        thread_service: Arc::clone(&first.threads) as Arc<dyn SessionThreadService>,
        approval_context: None,
        blocked_auth_prompts: None,
        project_filesystem: Arc::new(crate::NoProjectFilesystem),
        session_channel: None,
        settings: settings(),
    }));
    let resumed = coordinator
        .resume_reply_publications(&first.scope)
        .await
        .unwrap();
    assert_eq!(resumed, 1, "the sweep found the one open publication");
    let settled = wait_until(|| async {
        first
            .publications()
            .await
            .into_iter()
            .find(|record| !record.publication.status.is_active())
    })
    .await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    let requests = second_sink.requests();
    assert_eq!(
        requests.len(),
        1,
        "the resumed publication publishes the terminal revision once"
    );
    assert_eq!(requests[0].point, ReplyReconcilePoint::Terminal);
}

/// The journal observer id is the durable cursor key: renaming it would
/// replay or orphan the cursor a deployed journal already holds.
#[test]
fn the_journal_observer_identity_is_stable() {
    use ironclaw_processes::ProcessJournalCommitObserver as _;
    let store: Arc<dyn OutboundStateStorePort> =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let coordinator = DeliveryCoordinator::new(
        store,
        Arc::new(SinkResolver {
            sink: Arc::new(RecordingReplySink::new("id")),
            transport: Mutex::new(ReplyTransport::Stream),
            generation: Mutex::new(1),
        }),
        Arc::new(FixedReplyContext(None)),
        Arc::new(NoDeliveryRegistrations),
        DeliveryRetryPolicy::default(),
    );
    assert_eq!(
        coordinator.process_observer_id(),
        "reply-publication-commit-observer-v1"
    );
}

/// The journal's terminal commit is acknowledged only after the run's open
/// publications have workers again: `observe_process_commit` returns `Ok`
/// only once recovery ran, so a crash before the acknowledgement is
/// redelivered and a crash after it leaves rows the boot sweep resumes.
#[tokio::test]
async fn the_terminal_commit_is_acknowledged_only_after_recovery_ran() {
    use ironclaw_processes::ProcessJournalCommitObserver as _;
    let first = harness("commit-ack", ReplyTransport::Stream, None);
    first
        .coordinator
        .register_reply_target(first.registration(ReplyAudience::Private))
        .await
        .unwrap();
    first.text("half an answer");
    wait_until(|| async { (!first.sink.requests().is_empty()).then_some(()) }).await;
    first.coordinator.shutdown_reply_publication().await;
    first
        .commit_answer(MessageContent::text("the whole answer"))
        .await;

    let second_sink = Arc::new(RecordingReplySink::new("commit-ack-2"));
    let second_kernel = Arc::new(FakeTurnKernel::default());
    second_kernel.set_status(TurnStatus::Completed);
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&first.store),
        Arc::new(AnySinkResolver {
            sink: Arc::clone(&second_sink)
                as Arc<dyn ironclaw_extension_contracts::reply::ReplySink>,
            transport: ReplyTransport::Stream,
        }),
        Arc::new(FixedReplyContext(Some(b"vendor-ctx".to_vec()))),
        Arc::new(NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    assert!(coordinator.start_reply_publication(ReplyPublicationWiring {
        projection: Arc::new(ReplyProjection::new()),
        turn_coordinator: Arc::clone(&second_kernel) as Arc<dyn TurnCoordinator>,
        thread_service: Arc::clone(&first.threads) as Arc<dyn SessionThreadService>,
        approval_context: None,
        blocked_auth_prompts: None,
        project_filesystem: Arc::new(crate::NoProjectFilesystem),
        session_channel: None,
        settings: settings(),
    }));
    let scope = &first.scope;
    let commit = ironclaw_processes::ProcessJournalCommit {
        state: ironclaw_processes::JournaledProcessSnapshot {
            process_id: ironclaw_host_api::ids::ProcessId::from_uuid(first.run_id.as_uuid()),
            process_kind: ironclaw_processes::ProcessKind::AgentTurn,
            scope: ironclaw_host_api::resource::ResourceScope {
                tenant_id: scope.tenant_id.clone(),
                user_id: first.actor.user_id.clone(),
                agent_id: scope.agent_id.clone(),
                project_id: scope.project_id.clone(),
                mission_id: None,
                thread_id: Some(scope.thread_id.clone()),
                invocation_id: ironclaw_host_api::ids::InvocationId::new(),
            },
            status: ironclaw_processes::ProcessLifecycleStatus::Completed,
            suspension: None,
            checkpoint_ref: None,
            checkpoint_kind: None,
            input_ref: None,
            failure: None,
            journal_cursor: ironclaw_processes::ProcessJournalCursor(1),
            lease: None,
            crash_reclaim_count: 0,
            created_at: chrono::Utc::now(),
            owner_user_id: Some(first.actor.user_id.clone()),
            parent_process_id: None,
            concurrency_class: None,
            root_process_id: None,
            metadata: serde_json::Value::Null,
        },
        kind: ironclaw_processes::ProcessJournalKind::Completed,
        occurred_at: Some(chrono::Utc::now()),
        sanitized_reason: None,
    };
    coordinator
        .observe_process_commit(commit)
        .await
        .expect("recovery ran before the acknowledgement");
    // The acknowledgement implies the worker exists; only the terminal-fact
    // fetch (re-derivable from durable state) may still be in flight.
    let settled = wait_until(|| async {
        first
            .publications()
            .await
            .into_iter()
            .find(|record| !record.publication.status.is_active())
    })
    .await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    assert_eq!(second_sink.requests().len(), 1);
}
