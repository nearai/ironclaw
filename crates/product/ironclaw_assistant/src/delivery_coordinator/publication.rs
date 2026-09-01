//! Reply publication: the delivery coordinator's progressive lane (design doc
//! §5–§6).
//!
//! Every run's reply is published from one place — the [`DeliveryCoordinator`]
//! itself. The reply projection ([`crate::projection::reply`]) holds the
//! desired document; this private module owns *targets* — the exact places a
//! run answers to (its originating vendor conversation, the deployment's
//! session channel) — and, per target, one worker task that reconciles the
//! channel's bound [`ReplySink`] toward the newest revision. Cadence is the
//! channel's declared `[channel.reply]` transport: a `stream` sink hears
//! every reconcile point, a `message` sink hears the terminal one only.
//! Audience disclosure is applied to the copy each target receives.
//!
//! There is no separately constructed publication service: composition calls
//! [`DeliveryCoordinator::start_reply_publication`] once, and every caller —
//! the run-delivery observer, the process-journal hook, shutdown — reaches
//! publication through the coordinator's own methods. Workers, the target
//! map, and the projection subscription are implementation details behind
//! that surface.
//!
//! Publication state never lives here. It lives on the outbound delivery
//! attempt aggregate (one attempt row per run and exact target, written only
//! through the coordinator's guarded store operations): the atomic
//! publication claim (a lease and fence) a worker must hold before provider
//! egress, monotonic desired/published revisions (the desired revision is
//! made durable before the provider is touched), the sink's
//! generation-pinned checkpoint, bounded provider evidence, and a one-way
//! settlement that says `Delivered` only once the terminal revision was
//! applied, `Unknown` when read-back could not resolve an ambiguous provider
//! answer, and `Failed` otherwise. A publisher on any node can resume an
//! open publication from that row: the target descriptor is persisted at
//! open, the terminal revision is rebuilt from durable history, and the
//! fence rejects the stale worker. Two durable wake-ups exist — the
//! process-journal terminal commit (acknowledged only after the run's open
//! publications have workers again) and the boot-time
//! [`DeliveryCoordinator::resume_reply_publications`] sweep over the
//! existing outbound attempt index.
//!
//! [`ReplySink`]: ironclaw_extension_contracts::reply::ReplySink

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_contracts::channel::ReplyTransport;
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_extension_contracts::reply::{ReplyAudience, ReplyContextBytes, ReplyThreadAnchor};
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::turn::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope, TurnStatus};
use ironclaw_outbound::{
    OutboundDeliveryId, PublisherId, ReplyPublicationRecord, ReplyPublicationSettlement,
    ReplyPublicationTargetDescriptor, ReplyPublicationTargetKey,
};
use ironclaw_processes::{ProcessJournalCommit, ProcessJournalCommitObserver, ProcessKind};
use ironclaw_product_contracts::prompt_source::{
    ApprovalPromptContextSource, BlockedAuthPromptSource,
};
use ironclaw_threads::{AttachmentRef, SessionThreadService};
use ironclaw_turns::TurnCoordinator;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::{CoordinatedDeliveryError, DeliveryCoordinator, OpenReplyPublication};
use crate::ProjectFilesystemReader;
use crate::projection::reply::{ReplyProjection, ReplyProjectionEvent, ReplyProjectionObserver};

mod kernel_ports;
mod worker;

#[cfg(test)]
mod tests;

/// Why a target could not be registered or a publication could not proceed.
#[derive(Debug, thiserror::Error)]
pub enum ReplyPublicationError {
    /// The channel binds no reply sink (or declares no `[channel.reply]`):
    /// it cannot answer a run, and there is deliberately no fallback send.
    #[error("channel `{extension_id}` cannot answer a run: no bound reply sink")]
    ChannelCannotReply { extension_id: String },
    #[error("reply publication target is invalid: {reason}")]
    InvalidTarget { reason: String },
    /// Reply publication was never started on this coordinator
    /// ([`DeliveryCoordinator::start_reply_publication`]).
    #[error("reply publication is not started on this delivery coordinator")]
    NotStarted,
    #[error(transparent)]
    Coordinator(#[from] CoordinatedDeliveryError),
    /// The durable terminal facts could not be read; the publication stays
    /// open and is retried on the next terminal signal.
    #[error("terminal reply facts unavailable: {reason}")]
    TerminalFactsUnavailable { reason: String },
}

/// Pacing and budgets. Defaults suit a production deployment; tests shrink
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplyPublicationSettings {
    /// How long one worker owns a publication between claim re-entries. The
    /// claim is taken immediately before provider egress, and the sink call
    /// is bounded by `reconcile_timeout` (clamped to this), so the lease is
    /// valid for the entire provider operation.
    pub lease_ttl: Duration,
    /// Minimum gap between two `Progress` reconciles of one target; control
    /// -critical and terminal reconciles are never delayed by it.
    pub min_progress_interval: Duration,
    /// First retry delay when a sink asks for a retry without a hint; doubles
    /// up to `max_retry_backoff`.
    pub retry_backoff: Duration,
    pub max_retry_backoff: Duration,
    /// Consecutive non-applied reconciles of the terminal revision before the
    /// publication settles (`Unknown` after ambiguity, `Failed` otherwise).
    pub terminal_attempt_budget: u32,
    /// Upper bound on one sink reconcile call. Clamped to `lease_ttl` at use
    /// so a slow provider call can never outlive the claim that covers it.
    pub reconcile_timeout: Duration,
    /// How many times the durable terminal facts are re-read while the run's
    /// terminal commit catches up with its final milestone.
    pub terminal_fact_attempts: u32,
    /// How long a live `stream` target may sit idle before it is reconciled
    /// at the `Heartbeat` point (a provider session that expires unless the
    /// host re-asserts it). `message` targets never hear heartbeats.
    pub heartbeat_interval: Duration,
}

impl Default for ReplyPublicationSettings {
    fn default() -> Self {
        Self {
            lease_ttl: Duration::from_secs(60),
            min_progress_interval: Duration::from_millis(250),
            retry_backoff: Duration::from_millis(500),
            max_retry_backoff: Duration::from_secs(30),
            terminal_attempt_budget: 8,
            reconcile_timeout: Duration::from_secs(30),
            terminal_fact_attempts: 20,
            heartbeat_interval: Duration::from_secs(20 * 60),
        }
    }
}

/// One place a run answers to.
#[derive(Debug, Clone)]
pub struct ReplyTargetRegistration {
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub run_id: TurnRunId,
    pub extension_id: ExtensionId,
    /// The authorized reply-target binding the attempt row is keyed on.
    pub reply_target: ReplyTargetBindingRef,
    /// The vendor conversation; `None` for the session channel.
    pub conversation: Option<ExternalConversationRef>,
    pub thread_anchor: Option<ReplyThreadAnchor>,
    pub audience: ReplyAudience,
}

/// Everything reply publication needs beyond the coordinator's own
/// dependencies — the wiring argument of
/// [`DeliveryCoordinator::start_reply_publication`]. The durable-fact and
/// gate-prompt reads go straight to their owners — the turn kernel, the
/// thread service, and the same prompt sources the delivery observer
/// consults — rather than through publication-local port traits.
pub struct ReplyPublicationWiring {
    pub projection: Arc<ReplyProjection>,
    pub turn_coordinator: Arc<dyn TurnCoordinator>,
    pub thread_service: Arc<dyn SessionThreadService>,
    /// Approval prompt copy for an approval-gate attention facet.
    pub approval_context: Option<Arc<dyn ApprovalPromptContextSource>>,
    /// Auth challenge copy (and private-audience setup link) for an
    /// auth-gate attention facet.
    pub blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    pub project_filesystem: Arc<dyn ProjectFilesystemReader>,
    /// The deployment's authenticated-session channel, registered as a
    /// target for every run at its first revision. `None` for a deployment
    /// without one.
    pub session_channel: Option<ExtensionId>,
    pub settings: ReplyPublicationSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RunKey {
    pub(crate) scope: TurnScope,
    pub(crate) run_id: TurnRunId,
}

impl RunKey {
    fn new(scope: &TurnScope, run_id: TurnRunId) -> Self {
        Self {
            scope: scope.clone(),
            run_id,
        }
    }
}

/// One registered target and the worker driving it.
pub(crate) struct TargetState {
    pub(crate) registration: ReplyTargetRegistration,
    pub(crate) transport: ReplyTransport,
    /// The stored ingress reply context as snapshotted when this target was
    /// registered (and persisted on the durable descriptor). Per-run on
    /// purpose: the per-conversation store is latest-wins, and a newer
    /// top-level DM must never re-thread an older run's reply.
    pub(crate) reply_context: Option<ReplyContextBytes>,
    pub(crate) delivery_id: OutboundDeliveryId,
    pub(crate) key: ReplyPublicationTargetKey,
    pub(crate) wake: Notify,
    task: Mutex<Option<JoinHandle<()>>>,
}

/// The coordinator's reply-publication state: the projection subscription,
/// the live target map, and the per-run bookkeeping. Private to the
/// coordinator — constructed once by
/// [`DeliveryCoordinator::start_reply_publication`] and reached only through
/// coordinator methods.
pub(crate) struct ReplyPublication {
    /// The owning coordinator. Weak because the coordinator holds this state:
    /// a worker that outlives the coordinator (process teardown) exits
    /// instead of keeping it alive.
    coordinator: Weak<DeliveryCoordinator>,
    pub(crate) projection: Arc<ReplyProjection>,
    pub(crate) turn_coordinator: Arc<dyn TurnCoordinator>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) approval_context: Option<Arc<dyn ApprovalPromptContextSource>>,
    pub(crate) blocked_auth_prompts: Option<Arc<dyn BlockedAuthPromptSource>>,
    pub(crate) project_filesystem: Arc<dyn ProjectFilesystemReader>,
    pub(crate) session_channel: Option<ExtensionId>,
    pub(crate) settings: ReplyPublicationSettings,
    pub(crate) publisher_id: PublisherId,
    targets: Mutex<HashMap<RunKey, Vec<Arc<TargetState>>>>,
    /// Attachment sources of a run's terminal facts, kept until the run's
    /// last target settles so the terminal reconcile can materialize them.
    terminal_attachments: Mutex<HashMap<RunKey, Vec<AttachmentRef>>>,
    /// Runs whose terminal facts are being fetched right now (single-flight).
    fact_fetches: Mutex<HashSet<RunKey>>,
    /// Runs whose session-channel target registration is in flight.
    session_registrations: Mutex<HashSet<RunKey>>,
}

struct ProjectionListener {
    publication: Weak<ReplyPublication>,
}

impl ReplyProjectionObserver for ProjectionListener {
    fn reply_projection_event(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        event: ReplyProjectionEvent,
    ) {
        let Some(publication) = self.publication.upgrade() else {
            return;
        };
        let key = RunKey::new(scope, run_id);
        match event {
            ReplyProjectionEvent::Revised(_) => {
                publication.ensure_session_target(&key);
                publication.wake_run(&key);
            }
            ReplyProjectionEvent::TerminalPending => {
                publication.ensure_session_target(&key);
                let publication = Arc::clone(&publication);
                tokio::spawn(async move {
                    publication.ensure_terminal_facts(&key).await;
                });
            }
        }
    }
}

// ── The coordinator's publication surface ────────────────────────────────

impl DeliveryCoordinator {
    /// Start reply publication on this coordinator: build the private state
    /// and subscribe it to the projection. One-time — composition calls it
    /// right after wiring the turn and thread services (the coordinator
    /// itself is built earlier, before those exist). Returns `false` when
    /// publication was already started.
    pub fn start_reply_publication(self: &Arc<Self>, wiring: ReplyPublicationWiring) -> bool {
        let publisher_id = PublisherId::new(format!("publisher-{}", uuid::Uuid::new_v4().simple()))
            .unwrap_or_else(|_| {
                // A v4 uuid in simple form is 32 hex characters: always within the
                // identifier grammar. Kept total for the type system's sake.
                PublisherId::new("publisher").unwrap_or_else(|_| unreachable!("static id"))
            });
        let publication = Arc::new(ReplyPublication {
            coordinator: Arc::downgrade(self),
            projection: Arc::clone(&wiring.projection),
            turn_coordinator: wiring.turn_coordinator,
            thread_service: wiring.thread_service,
            approval_context: wiring.approval_context,
            blocked_auth_prompts: wiring.blocked_auth_prompts,
            project_filesystem: wiring.project_filesystem,
            session_channel: wiring.session_channel,
            settings: wiring.settings,
            publisher_id,
            targets: Mutex::new(HashMap::new()),
            terminal_attachments: Mutex::new(HashMap::new()),
            fact_fetches: Mutex::new(HashSet::new()),
            session_registrations: Mutex::new(HashSet::new()),
        });
        if self
            .reply_publication
            .set(Arc::clone(&publication))
            .is_err()
        {
            return false;
        }
        wiring.projection.add_observer(Arc::new(ProjectionListener {
            publication: Arc::downgrade(&publication),
        }));
        true
    }

    fn started_reply_publication(&self) -> Option<&Arc<ReplyPublication>> {
        self.reply_publication.get()
    }

    /// Register one target for a run: resolves the channel (which must bind
    /// a reply sink — there is no fallback), opens the publication on the
    /// attempt aggregate, and starts the worker. Idempotent per exact target.
    pub async fn register_reply_target(
        &self,
        registration: ReplyTargetRegistration,
    ) -> Result<(), ReplyPublicationError> {
        let publication = self
            .started_reply_publication()
            .ok_or(ReplyPublicationError::NotStarted)?;
        publication.register_target(registration).await.map(|_| ())
    }

    /// The run reached a terminal commit (or a publisher learned so): resume
    /// any open publication for it from the store and make sure the terminal
    /// revision is built from durable facts.
    pub async fn reply_run_terminal(&self, scope: &TurnScope, run_id: TurnRunId) {
        let Some(publication) = self.started_reply_publication() else {
            return;
        };
        let key = RunKey::new(scope, run_id);
        if let Err(error) = publication.recover_run(&key).await {
            tracing::debug!(
                target: "ironclaw::reborn::reply_publication",
                %run_id,
                %error,
                "reply publication recovery failed; the next terminal signal retries"
            );
        }
        publication.ensure_terminal_facts(&key).await;
    }

    /// Wait (bounded) until every publication opened for the run has
    /// settled. `true` when none is still active — including when the run
    /// never had a publication. Lets a caller order its own side effects
    /// (retracting a working indicator, swapping a reaction) after the
    /// answer landed, without owning the answer.
    pub async fn await_reply_settled(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        timeout: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.list_reply_publications(scope.clone(), run_id).await {
                Ok(records) if records.iter().all(|r| !r.publication.status.is_active()) => {
                    return true;
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::debug!(
                        target: "ironclaw::reborn::reply_publication",
                        %run_id,
                        %error,
                        "could not read the run's publications while waiting for settlement"
                    );
                    return false;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Resume every open publication in the tenant from the outbound attempt
    /// index — the boot-time half of crash recovery. The journal's terminal
    /// commit is acknowledged only after recovery ran, so a crash *after*
    /// that acknowledgement leaves rows this sweep finds; a crash *before*
    /// it is redelivered by the journal. Returns how many workers were
    /// given a target again.
    pub async fn resume_reply_publications(
        &self,
        scope: &TurnScope,
    ) -> Result<usize, ReplyPublicationError> {
        let publication = self
            .started_reply_publication()
            .ok_or(ReplyPublicationError::NotStarted)?;
        let records = self.list_open_reply_publications(scope.clone()).await?;
        let resumed = publication.resume_records(records).await?;
        for key in resumed.iter() {
            let publication = Arc::clone(publication);
            let key = key.clone();
            tokio::spawn(async move {
                publication.ensure_terminal_facts(&key).await;
            });
        }
        Ok(resumed.len())
    }

    /// Stop every publication worker without settling anything, handing each
    /// held lease back so another publisher can resume at once. (A crash
    /// skips the hand-back; the lease then lapses on its own and the same
    /// resume path applies.)
    pub async fn shutdown_reply_publication(&self) {
        let Some(publication) = self.started_reply_publication() else {
            return;
        };
        let targets: Vec<Arc<TargetState>> = publication
            .lock_targets()
            .drain()
            .flat_map(|(_, targets)| targets)
            .collect();
        for target in targets {
            if let Some(task) = target.task.lock().unwrap_or_else(|p| p.into_inner()).take() {
                task.abort();
            }
            let scope = target.registration.scope.clone();
            let held = match self
                .load_reply_publication(scope.clone(), target.delivery_id)
                .await
            {
                Ok(Some(record))
                    if record
                        .publication
                        .lease
                        .as_ref()
                        .is_some_and(|lease| lease.owner == publication.publisher_id) =>
                {
                    Some(record.publication.fence)
                }
                Ok(_) => None,
                Err(error) => {
                    tracing::debug!(
                        target: "ironclaw::reborn::reply_publication",
                        delivery_id = %target.delivery_id,
                        %error,
                        "could not read a publication during shutdown; its lease will lapse"
                    );
                    None
                }
            };
            if let Some(fence) = held
                && let Err(error) = self
                    .release_reply_publication(scope, target.delivery_id, fence)
                    .await
            {
                tracing::debug!(
                    target: "ironclaw::reborn::reply_publication",
                    delivery_id = %target.delivery_id,
                    %error,
                    "could not release a publication lease during shutdown; it will lapse"
                );
            }
        }
    }
}

/// The durable wake-up: every terminal commit of a top-level user run
/// resumes that run's open publications. On the node that ran the run this
/// is the fast path to the terminal facts; on any other node it is how an
/// orphaned publication (a crashed publisher, a lapsed lease) gets a worker
/// again. Recovery is awaited before the commit is acknowledged, so the
/// journal's durable observer cursor never advances past a terminal commit
/// whose publications have no worker — a crash after the acknowledgement is
/// covered by [`DeliveryCoordinator::resume_reply_publications`] at boot,
/// and a crash before it is redelivered by the journal.
#[async_trait]
impl ProcessJournalCommitObserver for DeliveryCoordinator {
    fn process_observer_id(&self) -> &'static str {
        "reply-publication-commit-observer-v1"
    }

    async fn observe_process_commit(&self, commit: ProcessJournalCommit) -> Result<(), String> {
        let Some(publication) = self.started_reply_publication() else {
            return Ok(());
        };
        let snapshot = &commit.state;
        // Every terminal status counts — `RecoveryRequired` included: it is
        // terminal in the process contract, the terminal reducer renders it
        // as a failed reply, and skipping it would strand a lost publisher's
        // open publication until the next boot sweep.
        if snapshot.process_kind != ProcessKind::AgentTurn
            || snapshot.parent_process_id.is_some()
            || !snapshot.status.is_terminal()
        {
            return Ok(());
        }
        let (Some(thread_id), Some(agent_id)) = (
            snapshot.scope.thread_id.clone(),
            snapshot.scope.agent_id.clone(),
        ) else {
            return Ok(());
        };
        let scope = TurnScope::new(
            snapshot.scope.tenant_id.clone(),
            Some(agent_id),
            snapshot.scope.project_id.clone(),
            thread_id,
        );
        let run_id = TurnRunId::from_uuid(snapshot.process_id.as_uuid());
        let key = RunKey::new(&scope, run_id);
        // Await recovery so the acknowledgement means what it says; an error
        // leaves the cursor unacknowledged and the journal redelivers.
        publication
            .recover_run(&key)
            .await
            .map_err(|error| format!("reply publication recovery failed: {error}"))?;
        // The terminal-fact fetch polls the kernel while the terminal commit
        // settles; it never holds the journal. A crash before it lands is
        // covered by the boot resume over the still-open rows.
        let publication = Arc::clone(publication);
        tokio::spawn(async move {
            publication.ensure_terminal_facts(&key).await;
        });
        Ok(())
    }
}

// ── Publication internals ────────────────────────────────────────────────

impl ReplyPublication {
    /// The owning coordinator, while it is alive. `None` only during process
    /// teardown; callers treat it as "stop quietly".
    pub(crate) fn coordinator(&self) -> Option<Arc<DeliveryCoordinator>> {
        self.coordinator.upgrade()
    }

    fn lock_targets(&self) -> std::sync::MutexGuard<'_, HashMap<RunKey, Vec<Arc<TargetState>>>> {
        self.targets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wake_run(&self, key: &RunKey) {
        let targets = self.lock_targets().get(key).cloned().unwrap_or_default();
        for target in targets {
            target.wake.notify_one();
        }
    }

    pub(crate) fn terminal_attachments(&self, key: &RunKey) -> Vec<AttachmentRef> {
        self.terminal_attachments
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    /// The session channel answers every run: register it at the run's first
    /// revision (once per run, in the background).
    fn ensure_session_target(self: &Arc<Self>, key: &RunKey) {
        let Some(extension_id) = self.session_channel.clone() else {
            return;
        };
        {
            let targets = self.lock_targets();
            if targets.get(key).is_some_and(|targets| {
                targets
                    .iter()
                    .any(|t| t.registration.extension_id == extension_id)
            }) {
                return;
            }
        }
        if !self
            .session_registrations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key.clone())
        {
            return;
        }
        let Some(snapshot) = self.projection.snapshot(&key.scope, key.run_id) else {
            self.session_registrations
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(key);
            return;
        };
        let Some(actor) = snapshot.actor else {
            tracing::debug!(
                target: "ironclaw::reborn::reply_publication",
                run_id = %key.run_id,
                "run has no actor; the session channel target is not registered"
            );
            return;
        };
        let reply_target = match ReplyTargetBindingRef::new(format!(
            "reply-sink:{}:{}",
            extension_id.as_str(),
            key.scope.thread_id
        )) {
            Ok(reply_target) => reply_target,
            Err(error) => {
                tracing::debug!(
                    target: "ironclaw::reborn::reply_publication",
                    %error,
                    "session channel reply target ref is invalid"
                );
                return;
            }
        };
        let publication = Arc::clone(self);
        let registration = ReplyTargetRegistration {
            scope: key.scope.clone(),
            actor,
            run_id: key.run_id,
            extension_id,
            reply_target,
            conversation: None,
            thread_anchor: None,
            audience: ReplyAudience::Private,
        };
        let key = key.clone();
        tokio::spawn(async move {
            if let Err(error) = publication.register_target(registration).await {
                tracing::debug!(
                    target: "ironclaw::reborn::reply_publication",
                    run_id = %key.run_id,
                    %error,
                    "session channel target registration failed"
                );
                publication
                    .session_registrations
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&key);
            }
        });
    }

    async fn register_target(
        self: &Arc<Self>,
        registration: ReplyTargetRegistration,
    ) -> Result<Arc<TargetState>, ReplyPublicationError> {
        let coordinator = self
            .coordinator()
            .ok_or(ReplyPublicationError::NotStarted)?;
        let channel = coordinator
            .resolve_reply_channel(registration.extension_id.as_str())
            .ok_or_else(|| ReplyPublicationError::ChannelCannotReply {
                extension_id: registration.extension_id.as_str().to_string(),
            })?;
        let (Some(_sink), Some(transport)) = (&channel.reply, channel.reply_transport) else {
            return Err(ReplyPublicationError::ChannelCannotReply {
                extension_id: registration.extension_id.as_str().to_string(),
            });
        };
        let key = target_key(&registration)?;
        let run_key = RunKey::new(&registration.scope, registration.run_id);
        if let Some(existing) = self
            .lock_targets()
            .get(&run_key)
            .and_then(|targets| targets.iter().find(|t| t.key == key).cloned())
        {
            existing.wake.notify_one();
            return Ok(existing);
        }
        // Snapshot the stored ingress reply context NOW, at registration —
        // this run's message just stored it — and persist it with the
        // descriptor. The per-conversation store is latest-wins, so reading
        // it again at reconcile time would let a newer message in the same
        // conversation re-thread this run's reply.
        let reply_context_bytes = coordinator
            .reply_context_for_publication(&channel, registration.conversation.as_ref())
            .await?;
        let reply_context = validate_reply_context(reply_context_bytes.clone());
        let record = coordinator
            .open_reply_publication(OpenReplyPublication {
                scope: registration.scope.clone(),
                run_id: registration.run_id,
                reply_target: registration.reply_target.clone(),
                key: key.clone(),
                descriptor: ReplyPublicationTargetDescriptor {
                    extension_id: registration.extension_id.clone(),
                    actor: registration.actor.clone(),
                    reply_target: registration.reply_target.clone(),
                    conversation: registration.conversation.clone(),
                    thread_anchor: registration.thread_anchor.clone(),
                    audience: registration.audience,
                    transport,
                    reply_context: reply_context_bytes,
                },
            })
            .await?;
        Ok(self.spawn_target(
            run_key,
            registration,
            transport,
            reply_context,
            record.attempt.delivery_id,
            key,
        ))
    }

    fn spawn_target(
        self: &Arc<Self>,
        run_key: RunKey,
        registration: ReplyTargetRegistration,
        transport: ReplyTransport,
        reply_context: Option<ReplyContextBytes>,
        delivery_id: OutboundDeliveryId,
        key: ReplyPublicationTargetKey,
    ) -> Arc<TargetState> {
        let target = Arc::new(TargetState {
            registration,
            transport,
            reply_context,
            delivery_id,
            key,
            wake: Notify::new(),
            task: Mutex::new(None),
        });
        {
            let mut targets = self.lock_targets();
            let entry = targets.entry(run_key.clone()).or_default();
            if let Some(existing) = entry.iter().find(|t| t.key == target.key) {
                existing.wake.notify_one();
                return Arc::clone(existing);
            }
            entry.push(Arc::clone(&target));
        }
        let publication = Arc::clone(self);
        let worker_target = Arc::clone(&target);
        let handle = tokio::spawn(async move {
            worker::run_target(Arc::clone(&publication), Arc::clone(&worker_target)).await;
            publication.forget_target(&run_key, &worker_target.key);
        });
        *target.task.lock().unwrap_or_else(|p| p.into_inner()) = Some(handle);
        target.wake.notify_one();
        target
    }

    /// A target settled (or its worker ended): drop it, and once the run has
    /// no target left, evict the run's live document (cache only).
    fn forget_target(&self, run_key: &RunKey, key: &ReplyPublicationTargetKey) {
        let run_done = {
            let mut targets = self.lock_targets();
            let Some(entry) = targets.get_mut(run_key) else {
                return;
            };
            entry.retain(|t| &t.key != key);
            if entry.is_empty() {
                targets.remove(run_key);
                true
            } else {
                false
            }
        };
        if run_done {
            self.projection.evict(&run_key.scope, run_key.run_id);
            self.terminal_attachments
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(run_key);
            self.session_registrations
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(run_key);
        }
    }

    /// Resume every open publication of the run from the store: a worker per
    /// Active record that has none here.
    async fn recover_run(self: &Arc<Self>, key: &RunKey) -> Result<usize, ReplyPublicationError> {
        let coordinator = self
            .coordinator()
            .ok_or(ReplyPublicationError::NotStarted)?;
        let records = coordinator
            .list_reply_publications(key.scope.clone(), key.run_id)
            .await?;
        Ok(self.resume_records(records).await?.len())
    }

    /// Give each Active record a worker on this process (idempotent per
    /// delivery id). A record without a descriptor cannot be addressed and
    /// settles `Unknown` (the reply may or may not have reached it; nothing
    /// pretends either way). Returns the distinct runs that were resumed.
    async fn resume_records(
        self: &Arc<Self>,
        records: Vec<ReplyPublicationRecord>,
    ) -> Result<Vec<RunKey>, ReplyPublicationError> {
        let coordinator = self
            .coordinator()
            .ok_or(ReplyPublicationError::NotStarted)?;
        let mut resumed: Vec<RunKey> = Vec::new();
        for record in records {
            if !record.publication.status.is_active() {
                continue;
            }
            let key = RunKey::new(&record.attempt.scope, record.publication.target.run_id);
            let already = self.lock_targets().get(&key).is_some_and(|targets| {
                targets
                    .iter()
                    .any(|t| t.delivery_id == record.attempt.delivery_id)
            });
            if already {
                continue;
            }
            let Some(descriptor) = record.publication.descriptor.clone() else {
                tracing::debug!(
                    target: "ironclaw::reborn::reply_publication",
                    run_id = %key.run_id,
                    delivery_id = %record.attempt.delivery_id,
                    "open reply publication carries no target descriptor; settling Unknown"
                );
                settle_unaddressable(&coordinator, &key, &record).await;
                continue;
            };
            let registration = ReplyTargetRegistration {
                scope: key.scope.clone(),
                actor: descriptor.actor,
                run_id: key.run_id,
                extension_id: descriptor.extension_id,
                reply_target: descriptor.reply_target,
                conversation: descriptor.conversation,
                thread_anchor: descriptor.thread_anchor,
                audience: descriptor.audience,
            };
            // Resume with the SNAPSHOT the registration persisted — never a
            // fresh read of the mutable per-conversation store.
            let reply_context = validate_reply_context(descriptor.reply_context);
            self.spawn_target(
                key.clone(),
                registration,
                descriptor.transport,
                reply_context,
                record.attempt.delivery_id,
                record.publication.target.key.clone(),
            );
            if !resumed.contains(&key) {
                resumed.push(key);
            }
        }
        Ok(resumed)
    }

    /// Fetch the durable terminal facts and apply them (single-flight per
    /// run). The run's terminal commit can trail its final milestone by a
    /// little, so a not-yet-terminal read is retried with backoff.
    pub(crate) async fn ensure_terminal_facts(self: &Arc<Self>, key: &RunKey) {
        if !self
            .fact_fetches
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key.clone())
        {
            return;
        }
        let outcome = self.fetch_and_apply_terminal_facts(key).await;
        self.fact_fetches
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
        if let Err(error) = outcome {
            tracing::debug!(
                target: "ironclaw::reborn::reply_publication",
                run_id = %key.run_id,
                %error,
                "terminal reply facts could not be applied; the next terminal signal retries"
            );
        }
    }

    async fn fetch_and_apply_terminal_facts(
        self: &Arc<Self>,
        key: &RunKey,
    ) -> Result<(), ReplyPublicationError> {
        let coordinator = self
            .coordinator()
            .ok_or(ReplyPublicationError::NotStarted)?;
        let snapshot = self.projection.snapshot(&key.scope, key.run_id);
        if snapshot.as_ref().is_some_and(|s| s.document.is_terminal()) {
            self.wake_run(key);
            return Ok(());
        }
        let actor = snapshot
            .and_then(|s| s.actor)
            .or_else(|| {
                self.lock_targets()
                    .get(key)
                    .and_then(|targets| targets.first().map(|t| t.registration.actor.clone()))
            })
            .ok_or_else(|| ReplyPublicationError::TerminalFactsUnavailable {
                reason: "the run's actor is unknown".to_string(),
            })?;
        // A run rebuilt on this process numbers its terminal revision above
        // whatever its targets already saw, or the resumed publications would
        // read as complete.
        let floor = coordinator
            .list_reply_publications(key.scope.clone(), key.run_id)
            .await?
            .iter()
            .map(|record| {
                record
                    .publication
                    .desired_revision
                    .max(record.publication.published_revision)
            })
            .max()
            .unwrap_or(0);
        if floor > 0 {
            self.projection
                .raise_revision_floor(&key.scope, key.run_id, floor);
        }
        let mut delay = Duration::from_millis(50);
        for _ in 0..self.settings.terminal_fact_attempts.max(1) {
            let facts = kernel_ports::terminal_reply_facts(
                self.turn_coordinator.as_ref(),
                self.thread_service.as_ref(),
                &key.scope,
                &actor,
                key.run_id,
            )
            .await?;
            if is_terminal(facts.status) {
                self.terminal_attachments
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(key.clone(), facts.attachments.clone());
                self.projection
                    .apply_terminal_facts(&key.scope, key.run_id, facts);
                self.wake_run(key);
                return Ok(());
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(2));
        }
        Err(ReplyPublicationError::TerminalFactsUnavailable {
            reason: "the run's terminal commit has not landed".to_string(),
        })
    }
}

/// The snapshot's seam-bounded form; an oversized or absent snapshot
/// publishes without a context (never a substituted one).
fn validate_reply_context(bytes: Option<Vec<u8>>) -> Option<ReplyContextBytes> {
    bytes.and_then(|bytes| match ReplyContextBytes::new(bytes) {
        Ok(context) => Some(context),
        Err(error) => {
            tracing::debug!(
                target: "ironclaw::reborn::reply_publication",
                %error,
                "stored reply context exceeds the seam bound; publishing without it"
            );
            None
        }
    })
}

fn is_terminal(status: TurnStatus) -> bool {
    matches!(
        status,
        TurnStatus::Completed
            | TurnStatus::Failed
            | TurnStatus::Cancelled
            | TurnStatus::RecoveryRequired
    )
}

async fn settle_unaddressable(
    coordinator: &DeliveryCoordinator,
    key: &RunKey,
    record: &ReplyPublicationRecord,
) {
    if let Err(error) = coordinator
        .settle_reply_publication(
            key.scope.clone(),
            record.attempt.delivery_id,
            record.publication.fence,
            ReplyPublicationSettlement::Unknown,
        )
        .await
    {
        tracing::debug!(
            target: "ironclaw::reborn::reply_publication",
            delivery_id = %record.attempt.delivery_id,
            %error,
            "could not settle an unaddressable reply publication"
        );
    }
}

/// The exact-target key: the channel plus a digest of the conversation
/// identity and thread anchor. Two registrations of one run to the same
/// place share it; a different thread is a different target.
fn target_key(
    registration: &ReplyTargetRegistration,
) -> Result<ReplyPublicationTargetKey, ReplyPublicationError> {
    let identity = format!(
        "{}\n{}\n{}",
        registration.extension_id.as_str(),
        registration
            .conversation
            .as_ref()
            .map(ExternalConversationRef::conversation_fingerprint)
            .unwrap_or_else(|| "session".to_string()),
        registration
            .thread_anchor
            .as_ref()
            .map(|anchor| anchor.as_str())
            .unwrap_or_default()
    );
    let digest = ironclaw_common::hashing::sha256_hex(identity.as_bytes());
    let short = digest.get(..32).unwrap_or(digest.as_str());
    ReplyPublicationTargetKey::new(format!("{}:{short}", registration.extension_id.as_str()))
        .map_err(|error| ReplyPublicationError::InvalidTarget {
            reason: error.to_string(),
        })
}
