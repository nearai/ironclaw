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
    DenyAllEgress, FakeTurnKernel, FixedReplyContext, SinkResolver, harness, harness_with_settings,
    settings, wait_until,
};
use crate::delivery_coordinator::publication::{ReplyPublicationSettings, ReplyPublicationWiring};
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
    // The mutable per-conversation store moved on after the crash; the
    // resume must publish with the snapshot the registration persisted.
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&first.store),
        Arc::new(AnySinkResolver {
            sink: Arc::clone(&second_sink)
                as Arc<dyn ironclaw_extension_contracts::reply::ReplySink>,
            transport: ReplyTransport::Stream,
        }),
        Arc::new(FixedReplyContext(Some(b"stale-after-crash".to_vec()))),
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
    assert_eq!(
        requests[0]
            .reply_context
            .as_ref()
            .map(|context| context.as_bytes()),
        Some(b"vendor-ctx".as_slice()),
        "the resume publishes with the registration-time snapshot, not a fresh store read"
    );
}

/// A run's reply context is captured when its target is registered and rides
/// the durable descriptor: a newer message in the same conversation that
/// overwrites the latest-wins per-conversation store must not re-thread an
/// older run's reply.
struct MutableReplyContext(Mutex<Vec<u8>>);

#[async_trait]
impl DeliveryReplyContextSource for MutableReplyContext {
    async fn reply_context(
        &self,
        _extension_id: &ExtensionId,
        _installation_id: &AdapterInstallationId,
        _conversation_fingerprint: &str,
    ) -> Result<Option<Vec<u8>>, DeliveryReplyContextError> {
        Ok(Some(self.0.lock().unwrap().clone()))
    }
}

#[tokio::test]
async fn a_newer_dm_cannot_rethread_an_older_runs_reply() {
    let base = harness("ctx-snapshot", ReplyTransport::Message, None);
    let context = Arc::new(MutableReplyContext(Mutex::new(b"session-root-A".to_vec())));
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
    // DM A registers while its own context is the stored one.
    coordinator
        .register_reply_target(base.registration(ReplyAudience::Private))
        .await
        .unwrap();
    // DM B arrives in the same conversation and overwrites the store before
    // A's reply ever reaches the provider.
    *context.0.lock().unwrap() = b"session-root-B".to_vec();
    base.complete_with("the answer to A").await;
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
    let requests = base.sink.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]
            .reply_context
            .as_ref()
            .map(|context| context.as_bytes()),
        Some(b"session-root-A".as_slice()),
        "run A publishes with run A's own session root, not the newer DM's"
    );
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
            available: std::sync::atomic::AtomicBool::new(true),
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
    let commit = terminal_commit(
        &first,
        ironclaw_processes::ProcessLifecycleStatus::Completed,
    );
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

/// An ambiguous outcome with NO checkpoint anywhere — none handed back, none
/// previously persisted — cannot be reconciled by read-back: retrying would
/// blindly repeat the exact provider side effect, so the publication settles
/// `Unknown` after the single attempt (design doc §5).
struct CheckpointlessAmbiguousSink {
    calls: Mutex<u32>,
}

#[async_trait]
impl ironclaw_extension_contracts::reply::ReplySink for CheckpointlessAmbiguousSink {
    async fn reconcile(
        &self,
        _request: ironclaw_extension_contracts::reply::ReplyReconcileRequest,
        _egress: &dyn RestrictedEgress,
    ) -> Result<
        ironclaw_extension_contracts::reply::ReplySinkReport,
        ironclaw_extension_contracts::channel_adapter::ChannelError,
    > {
        *self.calls.lock().unwrap() += 1;
        Ok(ironclaw_extension_contracts::reply::ReplySinkReport {
            outcome: ironclaw_extension_contracts::reply::ReplySinkOutcome::Ambiguous {
                reason: ironclaw_extension_contracts::reply::ReplyOutcomeReason::new(
                    "transport failed after the send with nothing to read back",
                ),
            },
            checkpoint: None,
            evidence: ironclaw_extension_contracts::reply::ReplySinkEvidence::default(),
        })
    }
}

#[tokio::test]
async fn a_checkpointless_ambiguous_outcome_settles_unknown_without_a_retry() {
    let base = harness("ambiguous-no-ckpt", ReplyTransport::Message, None);
    let sink = Arc::new(CheckpointlessAmbiguousSink {
        calls: Mutex::new(0),
    });
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&base.store),
        Arc::new(AnySinkResolver {
            sink: Arc::clone(&sink) as Arc<dyn ironclaw_extension_contracts::reply::ReplySink>,
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
        "no read-back state anywhere means fail closed, not retry"
    );
    assert_eq!(
        *sink.calls.lock().unwrap(),
        1,
        "the ambiguous side effect is never blindly repeated"
    );
}

/// One top-level agent-turn terminal journal commit for the harness's run.
fn terminal_commit(
    harness: &super::Harness,
    status: ironclaw_processes::ProcessLifecycleStatus,
) -> ironclaw_processes::ProcessJournalCommit {
    let scope = &harness.scope;
    ironclaw_processes::ProcessJournalCommit {
        state: ironclaw_processes::JournaledProcessSnapshot {
            process_id: ironclaw_host_api::ids::ProcessId::from_uuid(harness.run_id.as_uuid()),
            process_kind: ironclaw_processes::ProcessKind::AgentTurn,
            scope: ironclaw_host_api::resource::ResourceScope {
                tenant_id: scope.tenant_id.clone(),
                user_id: harness.actor.user_id.clone(),
                agent_id: scope.agent_id.clone(),
                project_id: scope.project_id.clone(),
                mission_id: None,
                thread_id: Some(scope.thread_id.clone()),
                invocation_id: ironclaw_host_api::ids::InvocationId::new(),
            },
            status,
            suspension: None,
            checkpoint_ref: None,
            checkpoint_kind: None,
            input_ref: None,
            failure: None,
            journal_cursor: ironclaw_processes::ProcessJournalCursor(1),
            lease: None,
            crash_reclaim_count: 0,
            created_at: chrono::Utc::now(),
            owner_user_id: Some(harness.actor.user_id.clone()),
            parent_process_id: None,
            concurrency_class: None,
            root_process_id: None,
            metadata: serde_json::Value::Null,
        },
        kind: ironclaw_processes::ProcessJournalKind::Failed,
        occurred_at: Some(chrono::Utc::now()),
        sanitized_reason: None,
    }
}

/// `RecoveryRequired` is terminal in the process contract: its commit must
/// resume an orphaned publication like any other terminal status, rendered
/// as a failed reply — skipping it would strand the reply until the next
/// boot sweep.
#[tokio::test]
async fn a_recovery_required_terminal_commit_also_resumes_the_publication() {
    use ironclaw_processes::ProcessJournalCommitObserver as _;
    let first = harness("recovery-required", ReplyTransport::Stream, None);
    first
        .coordinator
        .register_reply_target(first.registration(ReplyAudience::Private))
        .await
        .unwrap();
    first.text("half an answer");
    wait_until(|| async { (!first.sink.requests().is_empty()).then_some(()) }).await;
    first.coordinator.shutdown_reply_publication().await;
    // The lost run ends RecoveryRequired (an expired lease) — no finalized
    // transcript row exists.
    first.kernel.set_status(TurnStatus::RecoveryRequired);

    let second_sink = Arc::new(RecordingReplySink::new("recovery-required-2"));
    let second_kernel = Arc::new(FakeTurnKernel::default());
    second_kernel.set_status(TurnStatus::RecoveryRequired);
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
    let commit = terminal_commit(
        &first,
        ironclaw_processes::ProcessLifecycleStatus::RecoveryRequired,
    );
    coordinator
        .observe_process_commit(commit)
        .await
        .expect("recovery ran before the acknowledgement");
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
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered),
        "the failed-reply terminal document was published and the publication settled"
    );
    let requests = second_sink.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        matches!(
            requests[0].revision.document.outcome,
            Some(ironclaw_extension_contracts::reply::ReplyOutcome::Failed { .. })
        ),
        "a RecoveryRequired run publishes as a failed reply: {:?}",
        requests[0].revision.document.outcome
    );
}

// ─── Cadence exemption: the answer's first text is control-critical ────────

/// A fast run can reach its terminal commit well inside the progress pacing
/// window. If the answer's first text were an ordinary `Progress` reconcile,
/// the pacing sleep would swallow it and the user's stream would jump from
/// "working" straight to the finalized answer. The first visible text is a
/// control-critical transition and publishes immediately; the window paces
/// only text-to-text growth after that.
#[tokio::test]
async fn the_answers_first_text_is_not_delayed_by_the_progress_pacing_window() {
    let mut paced = settings();
    paced.min_progress_interval = Duration::from_secs(120);
    let harness = harness_with_settings(
        "first-text-immediate",
        ironclaw_extension_contracts::channel::ReplyTransport::Stream,
        paced,
    );
    harness
        .coordinator
        .register_reply_target(
            harness.registration(ironclaw_extension_contracts::reply::ReplyAudience::Private),
        )
        .await
        .unwrap();
    // A pre-text revision consumes the run's un-throttled `Opened` publish.
    harness.projection.observe_milestone(&harness.milestone(
        ironclaw_loop_contracts::LoopHostMilestoneKind::IterationStarted { iteration: 1 },
    ));
    wait_until(|| async { (!harness.sink.requests().is_empty()).then_some(()) }).await;
    // A second pre-text revision wakes the worker into the pacing window —
    // the shape a real run produces (model-started activity right after the
    // opening publish). The worker must stay wake-responsive inside that
    // window, or the text below sits until the window elapses.
    harness.projection.observe_milestone(&harness.milestone(
        ironclaw_loop_contracts::LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id: None,
        },
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    harness.text("partial answer");
    let requests = wait_until(|| async {
        let requests = harness.sink.requests();
        requests
            .iter()
            .any(|request| request.revision.document.answer.text.as_str() == "partial answer")
            .then_some(requests)
    })
    .await;
    let first_text = requests
        .iter()
        .find(|request| request.revision.document.answer.text.as_str() == "partial answer")
        .unwrap();
    assert_eq!(
        first_text.point,
        ReplyReconcilePoint::ControlCritical,
        "the answer's first text publishes as a control-critical point, exempt from progress pacing"
    );
}

// ─── Session-channel registration resilience ───────────────────────────────

/// A first revision whose milestones never carried the actor skips
/// registration — but must retry on the next revision instead of latching
/// the run out of the session channel for its whole life.
#[tokio::test]
async fn an_actorless_first_revision_does_not_lock_the_session_channel_out() {
    let harness = harness("session-actorless", ReplyTransport::Stream, Some("web-app"));
    harness
        .projection
        .observe_milestone(&ironclaw_loop_contracts::LoopHostMilestone {
            scope: harness.scope.clone(),
            actor: None,
            turn_id: ironclaw_host_api::turn::TurnId::new(),
            run_id: harness.run_id,
            loop_driver_id: ironclaw_loop_contracts::LoopDriverId::new("test_loop").unwrap(),
            kind: ironclaw_loop_contracts::LoopHostMilestoneKind::IterationStarted { iteration: 1 },
        });
    // The next revision carries the actor: registration must succeed now.
    harness.text("hello after the actor is known");
    let publications = wait_until(|| async {
        let publications = harness.publications().await;
        (!publications.is_empty()).then_some(publications)
    })
    .await;
    assert_eq!(
        publications[0]
            .publication
            .descriptor
            .as_ref()
            .map(|descriptor| descriptor.extension_id.as_str()),
        Some("web-app"),
        "the session channel registers once the actor is known"
    );
}

// ── Store probe ──────────────────────────────────────────────────────────

/// Delegates every store operation to the real in-memory store while
/// counting the reads the publication lane issues and, when asked, slowing
/// the desired-revision write so a claim's remaining lease can be measured.
struct ProbeStore {
    inner: Arc<dyn OutboundStateStorePort>,
    loads: std::sync::atomic::AtomicUsize,
    lists: std::sync::atomic::AtomicUsize,
    advance_delay: Mutex<Duration>,
}

impl ProbeStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(
                ironclaw_outbound::test_support::in_memory_backed_outbound_state_store(),
            ),
            loads: std::sync::atomic::AtomicUsize::new(0),
            lists: std::sync::atomic::AtomicUsize::new(0),
            advance_delay: Mutex::new(Duration::ZERO),
        })
    }

    fn loads(&self) -> usize {
        self.loads.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn lists(&self) -> usize {
        self.lists.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl OutboundStateStorePort for ProbeStore {
    async fn put_run_delivery_cleanup(
        &self,
        record: ironclaw_outbound::RunDeliveryCleanupRecord,
    ) -> Result<(), ironclaw_outbound::OutboundError> {
        self.inner.put_run_delivery_cleanup(record).await
    }
    async fn load_run_delivery_cleanup(
        &self,
        request: ironclaw_outbound::RunDeliveryCleanupRequest,
    ) -> Result<Vec<ironclaw_outbound::RunDeliveryCleanupRecord>, ironclaw_outbound::OutboundError>
    {
        self.inner.load_run_delivery_cleanup(request).await
    }
    async fn complete_run_delivery_cleanup(
        &self,
        record: &ironclaw_outbound::RunDeliveryCleanupRecord,
    ) -> Result<(), ironclaw_outbound::OutboundError> {
        self.inner.complete_run_delivery_cleanup(record).await
    }
    async fn put_thread_notification_policy(
        &self,
        policy: ironclaw_outbound::ThreadNotificationPolicy,
    ) -> Result<(), ironclaw_outbound::OutboundError> {
        self.inner.put_thread_notification_policy(policy).await
    }
    async fn load_thread_notification_policy(
        &self,
        scope: TurnScope,
    ) -> Result<ironclaw_outbound::ThreadNotificationPolicy, ironclaw_outbound::OutboundError> {
        self.inner.load_thread_notification_policy(scope).await
    }
    async fn upsert_subscription(
        &self,
        record: ironclaw_outbound::ProjectionSubscriptionRecord,
    ) -> Result<(), ironclaw_outbound::OutboundError> {
        self.inner.upsert_subscription(record).await
    }
    async fn load_subscription_cursor(
        &self,
        request: ironclaw_outbound::LoadSubscriptionCursorRequest,
    ) -> Result<
        Option<ironclaw_event_projections::ProjectionCursor>,
        ironclaw_outbound::OutboundError,
    > {
        self.inner.load_subscription_cursor(request).await
    }
    async fn record_delivery_attempt(
        &self,
        attempt: ironclaw_outbound::OutboundDeliveryAttempt,
    ) -> Result<(), ironclaw_outbound::OutboundError> {
        self.inner.record_delivery_attempt(attempt).await
    }
    async fn claim_delivery_attempt_for_send(
        &self,
        request: ironclaw_outbound::ClaimDeliveryAttemptForSendRequest,
    ) -> Result<bool, ironclaw_outbound::OutboundError> {
        self.inner.claim_delivery_attempt_for_send(request).await
    }
    async fn recover_interrupted_delivery_attempt(
        &self,
        request: ironclaw_outbound::RecoverInterruptedDeliveryRequest,
    ) -> Result<bool, ironclaw_outbound::OutboundError> {
        self.inner
            .recover_interrupted_delivery_attempt(request)
            .await
    }
    async fn update_delivery_status(
        &self,
        request: ironclaw_outbound::UpdateDeliveryStatusRequest,
    ) -> Result<(), ironclaw_outbound::OutboundError> {
        self.inner.update_delivery_status(request).await
    }
    async fn list_delivery_attempts(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<ironclaw_outbound::OutboundDeliveryAttempt>, ironclaw_outbound::OutboundError>
    {
        self.inner.list_delivery_attempts(scope).await
    }
    async fn open_reply_publication(
        &self,
        request: ironclaw_outbound::OpenReplyPublicationRequest,
    ) -> Result<ironclaw_outbound::ReplyPublicationRecord, ironclaw_outbound::OutboundError> {
        self.inner.open_reply_publication(request).await
    }
    async fn claim_reply_publication_lease(
        &self,
        request: ironclaw_outbound::ClaimReplyPublicationLeaseRequest,
    ) -> Result<ironclaw_outbound::ReplyPublicationClaim, ironclaw_outbound::OutboundError> {
        self.inner.claim_reply_publication_lease(request).await
    }
    async fn advance_reply_publication(
        &self,
        request: ironclaw_outbound::AdvanceReplyPublicationRequest,
    ) -> Result<ironclaw_outbound::ReplyPublicationRecord, ironclaw_outbound::OutboundError> {
        let delay = *self.advance_delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        self.inner.advance_reply_publication(request).await
    }
    async fn settle_reply_publication(
        &self,
        request: ironclaw_outbound::SettleReplyPublicationRequest,
    ) -> Result<ironclaw_outbound::ReplyPublicationRecord, ironclaw_outbound::OutboundError> {
        self.inner.settle_reply_publication(request).await
    }
    async fn release_reply_publication_lease(
        &self,
        request: ironclaw_outbound::ReleaseReplyPublicationLeaseRequest,
    ) -> Result<(), ironclaw_outbound::OutboundError> {
        self.inner.release_reply_publication_lease(request).await
    }
    async fn load_reply_publication(
        &self,
        scope: TurnScope,
        delivery_id: ironclaw_outbound::OutboundDeliveryId,
    ) -> Result<Option<ironclaw_outbound::ReplyPublicationRecord>, ironclaw_outbound::OutboundError>
    {
        self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.load_reply_publication(scope, delivery_id).await
    }
    async fn list_reply_publications(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<ironclaw_outbound::ReplyPublicationRecord>, ironclaw_outbound::OutboundError>
    {
        self.lists.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.list_reply_publications(scope, run_id).await
    }
    async fn list_open_reply_publications(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<ironclaw_outbound::ReplyPublicationRecord>, ironclaw_outbound::OutboundError>
    {
        self.inner.list_open_reply_publications(scope).await
    }
}

fn probed_harness(
    label: &str,
    transport: ReplyTransport,
    settings: ReplyPublicationSettings,
) -> (super::Harness, Arc<ProbeStore>) {
    let probe = ProbeStore::new();
    let harness = super::harness_over_store(
        label,
        transport,
        None,
        settings,
        Arc::new(crate::NoProjectFilesystem),
        Arc::clone(&probe) as Arc<dyn OutboundStateStorePort>,
    );
    (harness, probe)
}

/// A wake inside the progress-pacing window is decided from local facts —
/// the transport's cadence and the pacing clock — before the store is read.
/// Every streamed token chunk wakes the worker; a store read per wake would
/// be tens of reads per second per streaming run, discarded unread.
#[tokio::test]
async fn progress_wakes_inside_the_pacing_window_do_not_read_the_store() {
    let mut paced = settings();
    paced.min_progress_interval = Duration::from_millis(400);
    let (harness, probe) = probed_harness("paced-reads", ReplyTransport::Stream, paced);
    harness
        .coordinator
        .register_reply_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    harness.text("a");
    wait_until(|| async {
        harness
            .publications()
            .await
            .first()
            .filter(|record| record.publication.published_revision >= 1)
            .cloned()
    })
    .await;
    let loads_before = probe.loads();

    for index in 0..40 {
        harness.text(&format!("a{index}"));
        // Provider chunks arrive spaced out; each one is its own wake.
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    tokio::time::sleep(Duration::from_millis(80)).await;

    let loads = probe.loads() - loads_before;
    assert!(
        loads <= 2,
        "{loads} store reads for 40 paced wakes; the pacing window must be checked first"
    );
}

/// Waiting for settlement watches the worker this process is running; the
/// store is read only when no local worker remains (a publisher on another
/// node), never in a 25 ms polling loop over the whole thread's rows.
#[tokio::test]
async fn awaiting_settlement_does_not_scan_the_store_while_a_local_worker_is_live() {
    let base = harness("settle-wait", ReplyTransport::Message, None);
    let probe = ProbeStore::new();
    let mut stalling = settings();
    // The stall must outlast the wait: a short lease would cut the sink call
    // and settle Unknown right at the deadline.
    stalling.lease_ttl = Duration::from_secs(5);
    stalling.reconcile_timeout = Duration::from_secs(30);
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&probe) as Arc<dyn OutboundStateStorePort>,
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
        settings: stalling,
    }));
    coordinator
        .register_reply_target(base.registration(ReplyAudience::Private))
        .await
        .unwrap();
    base.complete_with("slow answer").await;
    coordinator
        .reply_run_terminal(&base.scope, base.run_id)
        .await;
    let lists_before = probe.lists();

    let settled = coordinator
        .await_reply_settled(&base.scope, base.run_id, Duration::from_millis(400))
        .await;

    assert!(!settled, "the sink is still stalling");
    let scans = probe.lists() - lists_before;
    assert!(
        scans <= 2,
        "{scans} thread scans while waiting 400 ms on a live local worker"
    );
    coordinator.shutdown_reply_publication().await;
}

/// The sink call is bounded by what is LEFT of the claim, not by the full
/// TTL: the desired-revision write between the claim and the call consumes
/// lease time, and a call budgeted with the full TTL could outlive the claim
/// and overlap a takeover's provider call.
#[tokio::test]
async fn the_sink_timeout_is_bounded_by_the_remaining_lease_not_the_full_ttl() {
    let mut bounded = settings();
    bounded.lease_ttl = Duration::from_millis(1_000);
    bounded.reconcile_timeout = Duration::from_secs(30);
    bounded.terminal_attempt_budget = 1;
    let base = harness("remaining-lease", ReplyTransport::Message, None);
    let probe = ProbeStore::new();
    *probe.advance_delay.lock().unwrap() = Duration::from_millis(700);
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&probe) as Arc<dyn OutboundStateStorePort>,
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
    coordinator
        .register_reply_target(base.registration(ReplyAudience::Private))
        .await
        .unwrap();
    base.complete_with("slow answer").await;
    let started = tokio::time::Instant::now();
    coordinator
        .reply_run_terminal(&base.scope, base.run_id)
        .await;
    let settled = wait_until(|| async {
        probe
            .list_reply_publications(base.scope.clone(), base.run_id)
            .await
            .unwrap()
            .into_iter()
            .find(|record| !record.publication.status.is_active())
    })
    .await;
    let elapsed = started.elapsed();

    assert_eq!(
        settled.publication.status,
        ironclaw_outbound::ReplyPublicationStatus::Settled(
            ironclaw_outbound::ReplyPublicationSettlement::Unknown
        )
    );
    // Claim → 700 ms desired write → sink cut at the lease's remaining
    // ~300 ms → 700 ms evidence write ≈ 1.7 s. A call budgeted with the full
    // 1 s TTL lands at ≈ 2.4 s.
    assert!(
        elapsed < Duration::from_millis(2_100),
        "the sink ran past the claim it was covered by: settled after {elapsed:?}"
    );
    coordinator.shutdown_reply_publication().await;
}

/// The stored ingress context is snapshotted at registration and persisted
/// on the descriptor for resumes. What is persisted is the SEAM-BOUNDED
/// value the worker publishes with — an oversized stored context is dropped
/// once, at registration, not stored raw and re-validated on every read.
#[tokio::test]
async fn the_descriptor_persists_the_validated_reply_context_not_the_raw_bytes() {
    let base = harness("bounded-context", ReplyTransport::Stream, None);
    let oversized = vec![b'x'; ironclaw_extension_contracts::reply::REPLY_CONTEXT_MAX_BYTES + 1];
    let sink = Arc::new(RecordingReplySink::new("bounded-context"));
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&base.store),
        Arc::new(AnySinkResolver {
            sink: Arc::clone(&sink) as Arc<dyn ironclaw_extension_contracts::reply::ReplySink>,
            transport: ReplyTransport::Stream,
        }),
        Arc::new(FixedReplyContext(Some(oversized))),
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

    let record = base
        .store
        .list_reply_publications(base.scope.clone(), base.run_id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("the registration opened a row");
    assert_eq!(
        record
            .publication
            .descriptor
            .expect("descriptor")
            .reply_context,
        None,
        "an over-bound context is dropped at registration, never persisted raw"
    );
    coordinator.shutdown_reply_publication().await;
}
