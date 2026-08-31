//! Reply publication: the evolved delivery coordinator's progressive lane.
//! One worker per (run, exact target) reconciles the projection's document
//! through the channel's bound `ReplySink`, keeps its state on the outbound
//! attempt aggregate (lease, fence, revisions, checkpoint, evidence), and
//! settles truthfully. These tests drive the production service over the
//! real coordinator and the real in-memory outbound store; only the sink,
//! the durable-fact source, and the run-stop requester are doubles.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_contracts::channel::ReplyTransport;
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::reply::{
    ReplyAudience, ReplyOutcomeReason, ReplyPhase, ReplyReconcilePoint, ReplySinkOutcome,
};
use ironclaw_extension_contracts::test_support::fakes::RecordingReplySink;
use ironclaw_extension_contracts::tool_adapter::{
    RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest, RestrictedEgressResponse,
};
use ironclaw_host_api::ids::{AgentId, ExtensionId, TenantId, ThreadId, UserId};
use ironclaw_host_api::product_adapter::AdapterInstallationId;
use ironclaw_host_api::turn::{
    ReplyTargetBindingRef, TurnActor, TurnId, TurnRunId, TurnScope, TurnStatus,
};
use ironclaw_loop_contracts::{
    LoopCompletionKind, LoopDriverId, LoopHostMilestone, LoopHostMilestoneKind,
};
use ironclaw_outbound::{
    OutboundDeliveryStatus, OutboundStateStorePort, ReplyPublicationSettlement,
    ReplyPublicationStatus,
};
use ironclaw_product_contracts::delivery::{
    ChannelDeliveryResolver, DeliveryReplyContextError, DeliveryReplyContextSource,
    ResolvedChannelDelivery,
};

use super::{
    ReplyPublicationDeps, ReplyPublicationError, ReplyPublicationService, ReplyPublicationSettings,
    ReplyStopRequester, ReplyTargetRegistration, TerminalReplyFactSource,
};
use crate::delivery_coordinator::{
    DeliveryCoordinator, DeliveryRetryPolicy, NoDeliveryRegistrations,
};
use crate::reply_projection::{ReplyProjection, TerminalReplyFacts};

struct DenyAllEgress;

#[async_trait]
impl RestrictedEgress for DenyAllEgress {
    async fn send(
        &self,
        _request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        Err(RestrictedEgressError::PolicyDenied)
    }
}

/// Resolves every extension id to one recording sink with the configured
/// transport; `None` when marked unavailable.
struct SinkResolver {
    sink: Arc<RecordingReplySink>,
    transport: Mutex<ReplyTransport>,
    generation: Mutex<u64>,
}

impl ChannelDeliveryResolver for SinkResolver {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery> {
        Some(ResolvedChannelDelivery {
            extension_id: ExtensionId::new(extension_id).ok()?,
            installation_id: AdapterInstallationId::new("inst-1").ok()?,
            reply: Some(
                Arc::clone(&self.sink) as Arc<dyn ironclaw_extension_contracts::reply::ReplySink>
            ),
            delivery: None,
            egress: Arc::new(DenyAllEgress),
            reply_transport: Some(*self.transport.lock().unwrap()),
            generation: *self.generation.lock().unwrap(),
            requires_enrollment: false,
            declared_egress_hosts: Vec::new(),
        })
    }
}

struct FixedReplyContext(Option<Vec<u8>>);

#[async_trait]
impl DeliveryReplyContextSource for FixedReplyContext {
    async fn reply_context(
        &self,
        _extension_id: &ExtensionId,
        _installation_id: &AdapterInstallationId,
        _conversation_fingerprint: &str,
    ) -> Result<Option<Vec<u8>>, DeliveryReplyContextError> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
struct FakeTerminalFacts {
    facts: Mutex<Option<TerminalReplyFacts>>,
    reads: Mutex<u32>,
}

#[async_trait]
impl TerminalReplyFactSource for FakeTerminalFacts {
    async fn terminal_reply_facts(
        &self,
        _scope: &TurnScope,
        _actor: &TurnActor,
        _run_id: TurnRunId,
    ) -> Result<TerminalReplyFacts, ReplyPublicationError> {
        *self.reads.lock().unwrap() += 1;
        Ok(self
            .facts
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(TerminalReplyFacts {
                actor: None,
                status: TurnStatus::Running,
                nothing_to_report: false,
                answer: None,
                attachments: Vec::new(),
                failure_summary: None,
            }))
    }
}

#[derive(Default)]
struct FakeStopRequester {
    stops: Mutex<Vec<TurnRunId>>,
}

#[async_trait]
impl ReplyStopRequester for FakeStopRequester {
    async fn request_stop(&self, _scope: &TurnScope, _actor: &TurnActor, run_id: TurnRunId) {
        self.stops.lock().unwrap().push(run_id);
    }
}

struct Harness {
    store: Arc<dyn OutboundStateStorePort>,
    coordinator: Arc<DeliveryCoordinator>,
    projection: Arc<ReplyProjection>,
    sink: Arc<RecordingReplySink>,
    resolver: Arc<SinkResolver>,
    facts: Arc<FakeTerminalFacts>,
    stops: Arc<FakeStopRequester>,
    service: Arc<ReplyPublicationService>,
    scope: TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
}

fn settings() -> ReplyPublicationSettings {
    ReplyPublicationSettings {
        lease_ttl: Duration::from_millis(400),
        min_progress_interval: Duration::ZERO,
        retry_backoff: Duration::from_millis(5),
        max_retry_backoff: Duration::from_millis(20),
        terminal_attempt_budget: 3,
        reconcile_timeout: Duration::from_secs(2),
        terminal_fact_attempts: 20,
        heartbeat_interval: Duration::from_secs(60),
    }
}

fn harness(label: &str, transport: ReplyTransport, session_channel: Option<&str>) -> Harness {
    harness_with(label, transport, session_channel, settings())
}

fn harness_with_settings(
    label: &str,
    transport: ReplyTransport,
    settings: ReplyPublicationSettings,
) -> Harness {
    harness_with(label, transport, None, settings)
}

fn harness_with(
    label: &str,
    transport: ReplyTransport,
    session_channel: Option<&str>,
    settings: ReplyPublicationSettings,
) -> Harness {
    harness_over_files(
        label,
        transport,
        session_channel,
        settings,
        Arc::new(crate::NoProjectFilesystem),
    )
}

fn harness_over_files(
    label: &str,
    transport: ReplyTransport,
    session_channel: Option<&str>,
    settings: ReplyPublicationSettings,
    project_filesystem: Arc<dyn crate::ProjectFilesystemReader>,
) -> Harness {
    let tenant_id = TenantId::new(format!("{label}-tenant")).unwrap();
    let agent_id = AgentId::new(format!("{label}-agent")).unwrap();
    let thread_id = ThreadId::new(format!("{label}-thread")).unwrap();
    let user_id = UserId::new(format!("{label}-user")).unwrap();
    let store: Arc<dyn OutboundStateStorePort> =
        Arc::new(ironclaw_outbound::test_support::in_memory_backed_outbound_state_store());
    let sink = Arc::new(RecordingReplySink::new(label));
    let resolver = Arc::new(SinkResolver {
        sink: Arc::clone(&sink),
        transport: Mutex::new(transport),
        generation: Mutex::new(3),
    });
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&store),
        Arc::clone(&resolver) as Arc<dyn ChannelDeliveryResolver>,
        Arc::new(FixedReplyContext(Some(b"vendor-ctx".to_vec()))),
        Arc::new(NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    let projection = Arc::new(ReplyProjection::new());
    let facts = Arc::new(FakeTerminalFacts::default());
    let stops = Arc::new(FakeStopRequester::default());
    let service = ReplyPublicationService::start(ReplyPublicationDeps {
        coordinator: Arc::clone(&coordinator),
        projection: Arc::clone(&projection),
        terminal_facts: Arc::clone(&facts) as Arc<dyn TerminalReplyFactSource>,
        stop_requests: Arc::clone(&stops) as Arc<dyn ReplyStopRequester>,
        attention: None,
        project_filesystem,
        session_channel: session_channel.map(|id| ExtensionId::new(id).unwrap()),
        settings,
    });
    Harness {
        store,
        coordinator,
        projection,
        sink,
        resolver,
        facts,
        stops,
        service,
        scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
        actor: TurnActor::new(user_id),
        run_id: TurnRunId::new(),
    }
}

impl Harness {
    fn registration(&self, audience: ReplyAudience) -> ReplyTargetRegistration {
        ReplyTargetRegistration {
            scope: self.scope.clone(),
            actor: self.actor.clone(),
            run_id: self.run_id,
            extension_id: ExtensionId::new("acme").unwrap(),
            reply_target: ReplyTargetBindingRef::new("reply:acme-chat-1").unwrap(),
            conversation: Some(
                ExternalConversationRef::new(Some("T1"), "C1", None, Some("1712.0001")).unwrap(),
            ),
            thread_anchor: None,
            audience,
        }
    }

    fn milestone(&self, kind: LoopHostMilestoneKind) -> LoopHostMilestone {
        LoopHostMilestone {
            scope: self.scope.clone(),
            actor: Some(self.actor.clone()),
            turn_id: TurnId::new(),
            run_id: self.run_id,
            loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
            kind,
        }
    }

    fn text(&self, text: &str) {
        self.projection
            .observe_milestone(&self.milestone(LoopHostMilestoneKind::ModelTextDelta {
                safe_text: text.to_string(),
            }));
    }

    fn complete_with(&self, answer: &str) {
        *self.facts.facts.lock().unwrap() = Some(TerminalReplyFacts {
            actor: Some(self.actor.clone()),
            status: TurnStatus::Completed,
            nothing_to_report: false,
            answer: Some(answer.to_string()),
            attachments: Vec::new(),
            failure_summary: None,
        });
        self.projection
            .observe_milestone(&self.milestone(LoopHostMilestoneKind::Completed {
                completion_kind: LoopCompletionKind::FinalReply,
                exit_id: ironclaw_host_api::turn::LoopExitId::new("exit:test").unwrap(),
            }));
    }

    async fn publications(&self) -> Vec<ironclaw_outbound::ReplyPublicationRecord> {
        self.store
            .list_reply_publications(self.scope.clone(), self.run_id)
            .await
            .unwrap()
    }

    async fn wait_settled(&self) -> ironclaw_outbound::ReplyPublicationRecord {
        wait_until(|| async {
            self.publications()
                .await
                .into_iter()
                .find(|record| !record.publication.status.is_active())
        })
        .await
    }
}

async fn wait_until<F, Fut, T>(mut probe: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = probe().await {
            return value;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not reached within the wait budget"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn a_stream_target_receives_opened_progress_and_terminal_revisions_and_settles_delivered() {
    let harness = harness("stream", ReplyTransport::Stream, None);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    // Registration opens the publication on the attempt aggregate before any
    // revision exists: the row is the durable proof a reply was owed here.
    let opened = harness.publications().await;
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].publication.published_revision, 0);
    assert!(opened[0].publication.status.is_active());
    assert_eq!(opened[0].attempt.status, OutboundDeliveryStatus::Prepared);
    assert!(
        opened[0].publication.descriptor.is_some(),
        "the descriptor lets any node resume the target"
    );

    harness.text("Here is ");
    let first = wait_until(|| async {
        let requests = harness.sink.requests();
        (!requests.is_empty()).then_some(requests)
    })
    .await;
    assert_eq!(first[0].point, ReplyReconcilePoint::Opened);
    assert_eq!(first[0].revision.document.answer.text.as_str(), "Here is ");
    assert_eq!(first[0].target.audience, ReplyAudience::Private);
    assert_eq!(
        first[0]
            .reply_context
            .as_ref()
            .map(|c| c.as_bytes().to_vec()),
        Some(b"vendor-ctx".to_vec()),
        "the stored vendor reply context rides every reconcile"
    );
    assert_eq!(first[0].extension_generation, 3);
    assert!(first[0].checkpoint.is_none(), "nothing applied yet");

    harness.text("Here is what I found.");
    let progressed = wait_until(|| async {
        let requests = harness.sink.requests();
        (requests.len() >= 2).then_some(requests)
    })
    .await;
    assert_eq!(progressed[1].point, ReplyReconcilePoint::Progress);
    assert_eq!(
        progressed[1].checkpoint.as_ref().map(|c| c.payload()),
        Some("applied:1"),
        "the sink's own checkpoint from the previous apply is handed back"
    );

    harness.complete_with("Here is what I found, finalized.");
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    assert_eq!(settled.attempt.status, OutboundDeliveryStatus::Delivered);
    let requests = harness.sink.requests();
    let terminal = requests.last().unwrap();
    assert_eq!(terminal.point, ReplyReconcilePoint::Terminal);
    assert_eq!(terminal.revision.document.phase, ReplyPhase::Completed);
    assert_eq!(
        terminal.revision.document.answer.text.as_str(),
        "Here is what I found, finalized.",
        "the terminal revision carries the durable transcript text, not the stream"
    );
    assert!(terminal.revision.document.answer.finalized);
    assert_eq!(
        settled.publication.published_revision,
        terminal.revision.revision
    );
    assert_eq!(
        settled.publication.terminal_revision,
        Some(terminal.revision.revision)
    );
    assert!(
        settled
            .publication
            .evidence
            .provider_refs
            .iter()
            .any(|r| r.as_str() == format!("stream:{}", terminal.revision.revision)),
        "provider evidence from the terminal apply is recorded: {:?}",
        settled.publication.evidence
    );
    assert!(settled.publication.evidence.read_back_verified);
    assert!(
        settled.publication.lease.is_none(),
        "a settled publication holds no lease"
    );
    // The run's live document is evicted once every target settled; the
    // durable facts it was built from are untouched.
    wait_until(|| async {
        harness
            .projection
            .snapshot(&harness.scope, harness.run_id)
            .is_none()
            .then_some(())
    })
    .await;
}

#[tokio::test]
async fn a_message_target_receives_only_the_terminal_revision() {
    let harness = harness("message", ReplyTransport::Message, None);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Shared))
        .await
        .unwrap();
    harness.text("progress the channel never sees");
    harness.text("progress the channel never sees … still not");
    // Give a progressive publisher every chance to misbehave.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        harness.sink.requests().is_empty(),
        "a message-transport channel is reconciled at the terminal point only"
    );

    harness.complete_with("The answer.");
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    let requests = harness.sink.requests();
    assert_eq!(requests.len(), 1, "exactly one reconcile: the terminal one");
    assert_eq!(requests[0].point, ReplyReconcilePoint::Terminal);
    assert_eq!(
        requests[0].revision.document.answer.text.as_str(),
        "The answer."
    );
    assert_eq!(requests[0].target.audience, ReplyAudience::Shared);
}

#[tokio::test]
async fn the_session_channel_is_registered_for_every_run_with_a_disclosed_private_document() {
    let harness = harness("session", ReplyTransport::Stream, Some("web-app"));
    // No explicit registration: the first revision of any run registers the
    // deployment's session channel as a target (the WebUI edge).
    harness.projection.observe_milestone(&harness.milestone(
        LoopHostMilestoneKind::ModelReasoningDelta {
            safe_delta: "thinking".to_string(),
        },
    ));
    harness.text("hello");
    let requests = wait_until(|| async {
        let requests = harness.sink.requests();
        requests
            .iter()
            .any(|r| r.revision.document.answer.text.as_str() == "hello")
            .then_some(requests)
    })
    .await;
    let request = requests
        .iter()
        .find(|r| r.revision.document.answer.text.as_str() == "hello")
        .unwrap();
    assert_eq!(request.target.audience, ReplyAudience::Private);
    assert!(request.target.conversation.is_none());
    assert!(
        !request.revision.document.reasoning.is_empty(),
        "a private target sees the reasoning summary"
    );
    let publications = harness.publications().await;
    assert_eq!(publications.len(), 1);
    assert_eq!(
        publications[0]
            .publication
            .descriptor
            .as_ref()
            .map(|d| d.extension_id.as_str()),
        Some("web-app")
    );
}

#[tokio::test]
async fn shared_audience_targets_never_receive_reasoning() {
    let harness = harness("shared", ReplyTransport::Stream, None);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Shared))
        .await
        .unwrap();
    harness.projection.observe_milestone(&harness.milestone(
        LoopHostMilestoneKind::ModelReasoningDelta {
            safe_delta: "private thinking".to_string(),
        },
    ));
    harness.text("public answer");
    let requests = wait_until(|| async {
        let requests = harness.sink.requests();
        requests
            .iter()
            .any(|r| r.revision.document.answer.text.as_str() == "public answer")
            .then_some(requests)
    })
    .await;
    assert!(
        requests
            .iter()
            .all(|r| r.revision.document.reasoning.is_empty()),
        "disclosure policy strips reasoning for a shared conversation: {requests:?}"
    );
}

#[tokio::test]
async fn retryable_outcomes_back_off_and_the_terminal_budget_fails_closed() {
    let harness = harness("retry", ReplyTransport::Message, None);
    harness.sink.script([
        ReplySinkOutcome::Retryable {
            reason: ReplyOutcomeReason::new("rate limited"),
            retry_after: Some(Duration::from_millis(30)),
        },
        ReplySinkOutcome::Applied,
    ]);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    harness.complete_with("eventually");
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    assert_eq!(harness.sink.requests().len(), 2, "one retry after the hint");
    assert!(
        started.elapsed() >= Duration::from_millis(30),
        "the provider's retry-after hint is honored"
    );
    let last_outcome = settled.publication.evidence.last_outcome.clone();
    assert!(
        last_outcome.is_none() || last_outcome.as_ref().map(|r| r.as_str()) != Some("rate limited"),
        "the final apply clears the transient reason from the evidence"
    );

    // A terminal reconcile that never stops asking for retries ends in a
    // truthful failure, not an endless loop and not a fake Delivered.
    let stuck = harness_with_prefix("retry-stuck", ReplyTransport::Message);
    stuck.sink.script(std::iter::repeat_n(
        ReplySinkOutcome::Retryable {
            reason: ReplyOutcomeReason::new("still rate limited"),
            retry_after: None,
        },
        10,
    ));
    stuck
        .service
        .register_target(stuck.registration(ReplyAudience::Private))
        .await
        .unwrap();
    stuck.complete_with("never lands");
    let settled = stuck.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Failed(
            ironclaw_outbound::DeliveryFailureKind::TransportUnavailable
        ))
    );
    assert_eq!(settled.attempt.status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        stuck.sink.requests().len(),
        settings().terminal_attempt_budget as usize,
        "the budget bounds the attempts"
    );
    assert_eq!(
        settled
            .publication
            .evidence
            .last_outcome
            .as_ref()
            .map(|r| r.as_str()),
        Some("still rate limited")
    );
}

fn harness_with_prefix(label: &str, transport: ReplyTransport) -> Harness {
    harness(label, transport, None)
}

#[tokio::test]
async fn permanent_and_unauthorized_outcomes_settle_failed_without_another_attempt() {
    let harness = harness("permanent", ReplyTransport::Message, None);
    harness.sink.script([ReplySinkOutcome::Permanent {
        reason: ReplyOutcomeReason::new("channel archived"),
    }]);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    harness.complete_with("gone");
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Failed(
            ironclaw_outbound::DeliveryFailureKind::Rejected
        ))
    );
    assert_eq!(harness.sink.requests().len(), 1);

    let unauthorized = harness_with_prefix("unauthorized", ReplyTransport::Message);
    unauthorized.sink.script([ReplySinkOutcome::Unauthorized {
        reason: ReplyOutcomeReason::new("token revoked"),
    }]);
    unauthorized
        .service
        .register_target(unauthorized.registration(ReplyAudience::Private))
        .await
        .unwrap();
    unauthorized.complete_with("gone");
    let settled = unauthorized.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Failed(
            ironclaw_outbound::DeliveryFailureKind::AuthorizationRevoked
        ))
    );
    assert_eq!(unauthorized.sink.requests().len(), 1);
}

#[tokio::test]
async fn an_ambiguous_terminal_outcome_settles_unknown_after_read_back_attempts() {
    let harness = harness("ambiguous", ReplyTransport::Message, None);
    harness.sink.script(std::iter::repeat_n(
        ReplySinkOutcome::Ambiguous {
            reason: ReplyOutcomeReason::new("provider timed out after accepting"),
        },
        10,
    ));
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    harness.complete_with("maybe landed");
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Unknown),
        "ambiguity that read-back cannot resolve is reported as Unknown, never Delivered"
    );
    assert_eq!(settled.attempt.status, OutboundDeliveryStatus::Unknown);
    assert_eq!(
        harness.sink.requests().len(),
        settings().terminal_attempt_budget as usize
    );
    assert_eq!(
        settled
            .publication
            .evidence
            .last_outcome
            .as_ref()
            .map(|r| r.as_str()),
        Some("provider timed out after accepting")
    );
}

#[tokio::test]
async fn a_stop_from_the_channel_requests_a_run_cancel_and_the_terminal_revision_still_lands() {
    let harness = harness("stop", ReplyTransport::Stream, None);
    harness.sink.script([ReplySinkOutcome::StoppedByUser]);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    harness.text("working…");
    wait_until(|| async {
        let stops = harness.stops.stops.lock().unwrap().clone();
        stops.contains(&harness.run_id).then_some(())
    })
    .await;
    // The kernel cancels the run; its terminal commit yields Cancelled facts.
    *harness.facts.facts.lock().unwrap() = Some(TerminalReplyFacts {
        actor: Some(harness.actor.clone()),
        status: TurnStatus::Cancelled,
        nothing_to_report: false,
        answer: None,
        attachments: Vec::new(),
        failure_summary: None,
    });
    harness
        .service
        .run_terminal(&harness.scope, harness.run_id)
        .await;
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    let last = harness.sink.requests().last().cloned().unwrap();
    assert_eq!(last.point, ReplyReconcilePoint::Terminal);
    assert_eq!(last.revision.document.phase, ReplyPhase::Cancelled);
}

#[tokio::test]
async fn a_publisher_on_another_node_resumes_an_open_publication_from_the_store() {
    let first = harness("resume", ReplyTransport::Stream, None);
    first
        .service
        .register_target(first.registration(ReplyAudience::Private))
        .await
        .unwrap();
    first.text("half an answer");
    wait_until(|| async { (!first.sink.requests().is_empty()).then_some(()) }).await;
    let before = first.publications().await;
    assert_eq!(before[0].publication.published_revision, 1);
    // The first process dies: its workers stop, its projection is gone.
    first.service.shutdown().await;

    // A fresh process over the same store observes the run's terminal
    // commit. It has no in-memory document, so it rebuilds the terminal
    // revision from durable facts and resumes the publication with the
    // persisted checkpoint — publishing the terminal revision exactly once.
    let second_sink = Arc::new(RecordingReplySink::new("resume-2"));
    let resolver = Arc::new(SinkResolver {
        sink: Arc::clone(&second_sink),
        transport: Mutex::new(ReplyTransport::Stream),
        generation: Mutex::new(4),
    });
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&first.store),
        Arc::clone(&resolver) as Arc<dyn ChannelDeliveryResolver>,
        Arc::new(FixedReplyContext(Some(b"vendor-ctx".to_vec()))),
        Arc::new(NoDeliveryRegistrations),
        DeliveryRetryPolicy {
            max_attempts: 1,
            backoff: Duration::ZERO,
        },
    ));
    let facts = Arc::new(FakeTerminalFacts::default());
    *facts.facts.lock().unwrap() = Some(TerminalReplyFacts {
        actor: Some(first.actor.clone()),
        status: TurnStatus::Completed,
        nothing_to_report: false,
        answer: Some("the whole answer".to_string()),
        attachments: Vec::new(),
        failure_summary: None,
    });
    let second = ReplyPublicationService::start(ReplyPublicationDeps {
        coordinator,
        projection: Arc::new(ReplyProjection::new()),
        terminal_facts: Arc::clone(&facts) as Arc<dyn TerminalReplyFactSource>,
        stop_requests: Arc::new(FakeStopRequester::default()),
        attention: None,
        project_filesystem: Arc::new(crate::NoProjectFilesystem),
        session_channel: None,
        settings: settings(),
    });
    second.run_terminal(&first.scope, first.run_id).await;
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
        requests[0].checkpoint.as_ref().map(|c| c.payload()),
        Some("applied:1"),
        "the checkpoint the first node persisted is handed to the sink"
    );
    assert_eq!(requests[0].extension_generation, 4);
    assert!(
        settled.publication.evidence.generation_changed,
        "the extension generation moved under the checkpoint and the evidence says so"
    );
    assert_eq!(
        requests[0].revision.document.answer.text.as_str(),
        "the whole answer"
    );
    assert!(
        settled.publication.fence > before[0].publication.fence,
        "the takeover bumped the fence"
    );
    assert!(
        first.sink.requests().len() == 1,
        "the dead node published nothing more"
    );
}

#[tokio::test]
async fn a_live_lease_held_elsewhere_is_respected_until_it_lapses() {
    let harness = harness("held", ReplyTransport::Message, None);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    let record = harness.publications().await.remove(0);
    // Another publisher holds the lease for a short while.
    let held = harness
        .store
        .claim_reply_publication_lease(ironclaw_outbound::ClaimReplyPublicationLeaseRequest {
            delivery_id: record.attempt.delivery_id,
            scope: harness.scope.clone(),
            owner: ironclaw_outbound::PublisherId::new("other-node").unwrap(),
            ttl: Duration::from_millis(150),
            now: chrono::Utc::now(),
        })
        .await
        .unwrap();
    assert!(matches!(
        held,
        ironclaw_outbound::ReplyPublicationClaim::Acquired(_)
    ));

    harness.complete_with("after the lease lapses");
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(
        harness.sink.requests().is_empty(),
        "no reconcile while another publisher holds a live lease"
    );
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    assert_eq!(harness.sink.requests().len(), 1);
}

#[tokio::test]
async fn a_channel_that_cannot_reply_is_refused_at_registration() {
    let harness = harness("no-reply", ReplyTransport::Stream, None);
    struct NoChannel;
    impl ChannelDeliveryResolver for NoChannel {
        fn resolve_channel_delivery(&self, _: &str) -> Option<ResolvedChannelDelivery> {
            None
        }
    }
    let coordinator = Arc::new(DeliveryCoordinator::new(
        Arc::clone(&harness.store),
        Arc::new(NoChannel),
        Arc::new(FixedReplyContext(None)),
        Arc::new(NoDeliveryRegistrations),
        DeliveryRetryPolicy::default(),
    ));
    let service = ReplyPublicationService::start(ReplyPublicationDeps {
        coordinator,
        projection: Arc::clone(&harness.projection),
        terminal_facts: Arc::clone(&harness.facts) as Arc<dyn TerminalReplyFactSource>,
        stop_requests: Arc::clone(&harness.stops) as Arc<dyn ReplyStopRequester>,
        attention: None,
        project_filesystem: Arc::new(crate::NoProjectFilesystem),
        session_channel: None,
        settings: settings(),
    });
    let error = service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap_err();
    assert!(
        matches!(error, ReplyPublicationError::ChannelCannotReply { .. }),
        "no silent fallback: a channel without a bound reply sink is a registration error, got {error:?}"
    );
    assert!(
        harness.publications().await.is_empty(),
        "nothing was opened"
    );
    let _ = &harness.coordinator;
    let _ = &harness.resolver;
}

#[tokio::test]
async fn an_idle_stream_target_is_reconciled_at_the_heartbeat_point_but_a_message_target_is_not() {
    let mut fast = settings();
    fast.heartbeat_interval = Duration::from_millis(30);
    let harness = harness_with_settings("heartbeat", ReplyTransport::Stream, fast);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    harness.text("working on it");
    let heartbeat = wait_until(|| async {
        harness
            .sink
            .points()
            .into_iter()
            .find(|point| *point == ReplyReconcilePoint::Heartbeat)
    })
    .await;
    assert_eq!(heartbeat, ReplyReconcilePoint::Heartbeat);
    let requests = harness.sink.requests();
    let last = requests.last().unwrap();
    assert_eq!(
        last.revision.revision, requests[0].revision.revision,
        "a heartbeat re-presents the already published revision"
    );
    assert_eq!(
        last.checkpoint.as_ref().map(|c| c.payload()),
        Some("applied:1"),
        "with the sink's own checkpoint so it can decide nothing changed"
    );
    let record = harness.publications().await.remove(0);
    assert!(record.publication.status.is_active());
    assert_eq!(record.publication.published_revision, 1);

    let quiet = harness_with_settings("heartbeat-message", ReplyTransport::Message, fast);
    quiet
        .service
        .register_target(quiet.registration(ReplyAudience::Private))
        .await
        .unwrap();
    quiet.text("working on it");
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        quiet.sink.requests().is_empty(),
        "a message channel hears nothing before the terminal revision, heartbeats included"
    );
}

#[tokio::test]
async fn a_checkpoint_returned_with_a_retryable_outcome_is_persisted_for_the_retry() {
    // A sink that opened a provider stream and then hit a rate limit hands
    // back the checkpoint that names the stream. The host must keep it, or
    // the retry opens a second stream.
    let harness = harness("retry-checkpoint", ReplyTransport::Stream, None);
    harness.sink.script([
        ReplySinkOutcome::Retryable {
            reason: ReplyOutcomeReason::new("rate limited after opening"),
            retry_after: Some(Duration::from_millis(10)),
        },
        ReplySinkOutcome::Applied,
    ]);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    harness.text("first");
    let requests = wait_until(|| async {
        let requests = harness.sink.requests();
        (requests.len() >= 2).then_some(requests)
    })
    .await;
    assert_eq!(
        requests[1].checkpoint.as_ref().map(|c| c.payload()),
        Some("retryable:1"),
        "the retry carries the checkpoint the sink returned with its Retryable outcome"
    );
}

#[tokio::test]
async fn a_text_microburst_coalesces_and_the_latest_text_precedes_tool_activity() {
    let mut paced = settings();
    paced.min_progress_interval = Duration::from_millis(15);
    let harness = harness_with_settings("microburst", ReplyTransport::Stream, paced);
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    let activity_id = ironclaw_host_api::turn::CapabilityActivityId::new();
    for index in 0..64 {
        harness.text(&format!("partial answer {index}"));
    }
    harness.projection.observe_milestone(&harness.milestone(
        LoopHostMilestoneKind::CapabilityInvoked {
            activity_id,
            capability_id: ironclaw_host_api::ids::CapabilityId::new("builtin.http").unwrap(),
        },
    ));
    let requests = wait_until(|| async {
        let requests = harness.sink.requests();
        requests
            .iter()
            .any(|r| !r.revision.document.activities.is_empty())
            .then_some(requests)
    })
    .await;
    assert!(
        requests.len() <= 6,
        "a 64-update burst collapses onto a few reconciles under the pacing window, got {}",
        requests.len()
    );
    let with_tool = requests
        .iter()
        .find(|r| !r.revision.document.activities.is_empty())
        .unwrap();
    assert_eq!(
        with_tool.revision.document.answer.text.as_str(),
        "partial answer 63",
        "the reconcile that shows the tool already carries the latest text: desired state, never a stale interleaving"
    );
}

// ─── Terminal attachments: workspace bytes ride the terminal reconcile ─────
//
// The coordinator no longer materializes `/workspace/...` references for a
// discrete send; the run's files travel with the answer, read from the
// project filesystem at the terminal reconcile only, under the same budgets
// and fail-closed rules the coordinator used to apply.

struct ScriptedProjectFilesystem {
    results: Mutex<
        std::collections::HashMap<
            String,
            Result<ironclaw_host_api::attachment::WorkspaceFile, crate::ProjectFsError>,
        >,
    >,
    stats: Mutex<
        std::collections::HashMap<String, Result<crate::ProjectFsStat, crate::ProjectFsError>>,
    >,
    reads: Mutex<Vec<String>>,
}

impl ScriptedProjectFilesystem {
    fn new() -> Self {
        Self {
            results: Mutex::new(std::collections::HashMap::new()),
            stats: Mutex::new(std::collections::HashMap::new()),
            reads: Mutex::new(Vec::new()),
        }
    }

    fn insert_file(&self, path: &str, bytes: &[u8]) {
        self.stats.lock().unwrap().insert(
            path.to_string(),
            Ok(crate::ProjectFsStat {
                path: path.to_string(),
                kind: crate::ProjectFsEntryKind::File,
                size_bytes: bytes.len() as u64,
                mime_type: "text/plain".to_string(),
            }),
        );
        self.results.lock().unwrap().insert(
            path.to_string(),
            Ok(ironclaw_host_api::attachment::WorkspaceFile {
                path: ironclaw_host_api::path::ScopedPath::new(path).unwrap(),
                filename: path.rsplit('/').next().map(str::to_string),
                mime_type: "text/plain".to_string(),
                bytes: bytes.to_vec(),
            }),
        );
    }

    fn insert_error(&self, path: &str, error: crate::ProjectFsError) {
        self.stats
            .lock()
            .unwrap()
            .insert(path.to_string(), Err(error.clone()));
        self.results
            .lock()
            .unwrap()
            .insert(path.to_string(), Err(error));
    }

    fn read_count(&self) -> usize {
        self.reads.lock().unwrap().len()
    }
}

#[async_trait]
impl crate::ProjectFilesystemReader for ScriptedProjectFilesystem {
    async fn list_dir(
        &self,
        _thread_scope: &ironclaw_threads::ThreadScope,
        _path: &str,
    ) -> Result<Vec<crate::ProjectFsEntry>, crate::ProjectFsError> {
        Err(crate::ProjectFsError::NotADirectory)
    }

    async fn read_file(
        &self,
        _thread_scope: &ironclaw_threads::ThreadScope,
        path: &str,
    ) -> Result<ironclaw_host_api::attachment::WorkspaceFile, crate::ProjectFsError> {
        self.reads.lock().unwrap().push(path.to_string());
        self.results
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or(Err(crate::ProjectFsError::NotFound))
    }

    async fn stat(
        &self,
        _thread_scope: &ironclaw_threads::ThreadScope,
        path: &str,
    ) -> Result<crate::ProjectFsStat, crate::ProjectFsError> {
        self.stats
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .unwrap_or(Err(crate::ProjectFsError::NotFound))
    }
}

fn attachment_ref(
    id: &str,
    path: &str,
    filename: &str,
    size: u64,
) -> ironclaw_threads::AttachmentRef {
    ironclaw_threads::AttachmentRef {
        id: id.to_string(),
        kind: ironclaw_threads::AttachmentKind::from_mime_type("text/plain"),
        mime_type: "text/plain".to_string(),
        filename: Some(filename.to_string()),
        size_bytes: Some(size),
        storage_key: Some(path.to_string()),
        extracted_text: None,
    }
}

impl Harness {
    fn complete_with_attachments(
        &self,
        answer: &str,
        attachments: Vec<ironclaw_threads::AttachmentRef>,
    ) {
        *self.facts.facts.lock().unwrap() = Some(TerminalReplyFacts {
            actor: Some(self.actor.clone()),
            status: TurnStatus::Completed,
            nothing_to_report: false,
            answer: Some(answer.to_string()),
            attachments,
            failure_summary: None,
        });
        self.projection
            .observe_milestone(&self.milestone(LoopHostMilestoneKind::Completed {
                completion_kind: LoopCompletionKind::FinalReply,
                exit_id: ironclaw_host_api::turn::LoopExitId::new("exit:test").unwrap(),
            }));
    }
}

#[tokio::test]
async fn terminal_attachments_ride_the_terminal_reconcile_with_their_workspace_bytes() {
    let files = Arc::new(ScriptedProjectFilesystem::new());
    files.insert_file("/workspace/report.txt", b"hello");
    let harness = harness_over_files(
        "files",
        ReplyTransport::Stream,
        None,
        settings(),
        Arc::clone(&files) as Arc<dyn crate::ProjectFilesystemReader>,
    );
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    harness.text("drafting");
    wait_until(|| async { (!harness.sink.requests().is_empty()).then_some(()) }).await;
    assert_eq!(
        files.read_count(),
        0,
        "no workspace bytes are read for progress"
    );

    harness.complete_with_attachments(
        "report attached",
        vec![attachment_ref(
            "att-1",
            "/workspace/report.txt",
            "renamed.txt",
            5,
        )],
    );
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Delivered)
    );
    let requests = harness.sink.requests();
    let terminal = requests.last().unwrap();
    assert_eq!(terminal.point, ReplyReconcilePoint::Terminal);
    assert_eq!(terminal.revision.document.attachments.len(), 1);
    assert_eq!(
        terminal.revision.document.attachments[0].filename.as_str(),
        "renamed.txt"
    );
    assert!(matches!(
        terminal.materialized_attachments.as_slice(),
        [file]
            if file.path.as_str() == "/workspace/report.txt"
                && file.filename.as_deref() == Some("renamed.txt")
                && file.bytes == b"hello"
    ));
    assert!(
        requests[..requests.len() - 1]
            .iter()
            .all(|r| r.materialized_attachments.is_empty()),
        "bytes travel only with the terminal revision"
    );
    assert_eq!(files.read_count(), 1);
}

#[tokio::test]
async fn a_missing_or_oversized_workspace_file_fails_the_publication_closed() {
    let files = Arc::new(ScriptedProjectFilesystem::new());
    let harness = harness_over_files(
        "files-missing",
        ReplyTransport::Message,
        None,
        settings(),
        Arc::clone(&files) as Arc<dyn crate::ProjectFilesystemReader>,
    );
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    harness.complete_with_attachments(
        "see attached",
        vec![attachment_ref(
            "att-1",
            "/workspace/missing.txt",
            "missing.txt",
            5,
        )],
    );
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Failed(
            ironclaw_outbound::DeliveryFailureKind::Rejected
        )),
        "a reply that names a file the workspace does not hold is not sent without it"
    );
    assert!(
        harness.sink.requests().is_empty(),
        "nothing reached the sink: the failure is closed before the provider"
    );
    assert_eq!(
        settled
            .publication
            .evidence
            .last_outcome
            .as_ref()
            .map(|r| r.as_str().is_empty()),
        Some(false)
    );

    // Too many declared files fail before a single byte is read.
    let files = Arc::new(ScriptedProjectFilesystem::new());
    let harness = harness_over_files(
        "files-budget",
        ReplyTransport::Message,
        None,
        settings(),
        Arc::clone(&files) as Arc<dyn crate::ProjectFilesystemReader>,
    );
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    let too_many = (0..=ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS.max_count)
        .map(|index| {
            let path = format!("/workspace/f{index}.txt");
            files.insert_file(&path, b"x");
            attachment_ref(&format!("att-{index}"), &path, &format!("f{index}.txt"), 1)
        })
        .collect();
    harness.complete_with_attachments("many", too_many);
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Failed(
            ironclaw_outbound::DeliveryFailureKind::Rejected
        ))
    );
    assert_eq!(
        files.read_count(),
        0,
        "budgets are enforced on metadata before any read"
    );
}

#[tokio::test]
async fn an_unavailable_workspace_reader_is_retried_then_fails_closed_as_transport_unavailable() {
    let files = Arc::new(ScriptedProjectFilesystem::new());
    files.insert_error("/workspace/report.txt", crate::ProjectFsError::Unavailable);
    let harness = harness_over_files(
        "files-unavailable",
        ReplyTransport::Message,
        None,
        settings(),
        Arc::clone(&files) as Arc<dyn crate::ProjectFilesystemReader>,
    );
    harness
        .service
        .register_target(harness.registration(ReplyAudience::Private))
        .await
        .unwrap();
    harness.complete_with_attachments(
        "report attached",
        vec![attachment_ref(
            "att-1",
            "/workspace/report.txt",
            "report.txt",
            5,
        )],
    );
    let settled = harness.wait_settled().await;
    assert_eq!(
        settled.publication.status,
        ReplyPublicationStatus::Settled(ReplyPublicationSettlement::Failed(
            ironclaw_outbound::DeliveryFailureKind::TransportUnavailable
        )),
        "an unavailable reader is a transient fault: retried within the budget, then reported as such"
    );
    assert!(harness.sink.requests().is_empty());
}
