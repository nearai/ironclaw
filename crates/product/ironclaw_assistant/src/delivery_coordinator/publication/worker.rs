//! The per-target reconcile loop.
//!
//! Desired state is the projection's newest document; the worker converges
//! the sink toward it, coalescing naturally (it always publishes the *latest*
//! snapshot, so intermediate replaceable revisions collapse) while never
//! skipping a control-critical or terminal point.
//!
//! The durable order of one reconcile is fixed:
//! 1. load the publication row;
//! 2. prepare everything provider-independent — the channel and sink, the
//!    stored reply context, the disclosed/enriched document copy, and (at the
//!    terminal point) the materialized attachments — all *before* ownership
//!    is taken, so unbounded work never burns lease time;
//! 3. acquire the atomic publication claim (lease + fence) immediately
//!    before provider access;
//! 4. persist the newest desired revision under that fence, so the store
//!    knows what was being published before the provider is touched;
//! 5. call the bound sink, bounded by a timeout clamped to the lease TTL so
//!    the claim stays valid for the entire provider operation;
//! 6. persist the checkpoint, published revision, evidence, and outcome —
//!    every write carries the fence, so a worker that lost its claim to
//!    another node is rejected at the store, not trusted.

use std::sync::Arc;
use std::time::Duration;

use ironclaw_extension_contracts::channel_adapter::ChannelError;
use ironclaw_extension_contracts::reply::{
    ReplyAttentionKind, ReplyContextBytes, ReplyDocument, ReplyOutcomeReason, ReplyReconcilePoint,
    ReplyReconcileRequest, ReplyRevision, ReplySinkCheckpoint, ReplySinkOutcome, ReplySinkReport,
    ReplyTarget,
};
use ironclaw_host_api::attachment::WorkspaceFile;
use ironclaw_outbound::{
    AdvanceReplyPublicationRequest, DeliveryFailureKind, ReplyPublicationClaim,
    ReplyPublicationEvidence, ReplyPublicationRecord, ReplyPublicationSettlement,
};
use ironclaw_product_contracts::delivery::ResolvedChannelDelivery;
use ironclaw_threads::ThreadScope;

use super::{ReplyPublication, RunKey, TargetState, kernel_ports};
use crate::delivery_coordinator::{
    CoordinatedDeliveryError, DeliveryCoordinator, materialize_workspace_files,
    workspace_materialization_failure_kind,
};
use crate::projection::reply::{ReplySnapshot, disclose_for_audience};

use super::LOG_TARGET;

/// What the last successful reconcile looked like, to tell a control-critical
/// transition from ordinary progress.
#[derive(Default)]
struct Published {
    attention: Option<(ReplyAttentionKind, Option<String>)>,
    finalized: bool,
    has_text: bool,
    at: Option<tokio::time::Instant>,
    /// The store's published revision as this worker last saw it — `None`
    /// until the first row read. Every wake is planned against this mirror;
    /// the row itself is read only when a reconcile is due.
    revision: Option<u64>,
}

pub(super) async fn run_target(publication: Arc<ReplyPublication>, target: Arc<TargetState>) {
    let run_key = RunKey::new(&target.registration.scope, target.registration.run_id);
    let mut published = Published::default();
    let mut attempts: u32 = 0;
    let mut attempted_revision: u64 = 0;
    loop {
        let Some(coordinator) = publication.coordinator() else {
            return;
        };
        // 1. Desired state.
        let Some(snapshot) = publication
            .projection
            .snapshot(&run_key.scope, run_key.run_id)
        else {
            target.wake.notified().await;
            continue;
        };
        if snapshot.terminal_pending && !snapshot.document.is_terminal() {
            let fetch_publication = Arc::clone(&publication);
            let fetch_key = run_key.clone();
            tokio::spawn(async move { fetch_publication.ensure_terminal_facts(&fetch_key).await });
            target.wake.notified().await;
            continue;
        }
        // 2. Published state — from the local mirror. Every streamed token
        // wakes this loop and most wakes end inside the pacing window or on a
        // transport that only hears the terminal, so the row is read only
        // once a reconcile is actually due (step 3b); the first pass seeds
        // the mirror from the store.
        let terminal = snapshot.document.is_terminal();
        let published_revision = match published.revision {
            Some(revision) => revision,
            None => match load_row(&coordinator, &run_key, &target).await {
                Ok(record) => {
                    if !record.publication.status.is_active() {
                        return;
                    }
                    published.revision = Some(record.publication.published_revision);
                    record.publication.published_revision
                }
                Err(LoadFailure::Vanished) => return,
                Err(LoadFailure::Retry) => {
                    tokio::time::sleep(publication.settings.retry_backoff).await;
                    continue;
                }
            },
        };
        let behind = snapshot.revision > published_revision;
        let mut heartbeat = false;
        if !behind {
            let can_heartbeat = !terminal
                && published_revision > 0
                && target
                    .transport
                    .reconciles_at(ReplyReconcilePoint::Heartbeat);
            if !can_heartbeat {
                target.wake.notified().await;
                continue;
            }
            let idle_for = published
                .at
                .map(|at| at.elapsed())
                .unwrap_or(Duration::ZERO);
            if idle_for < publication.settings.heartbeat_interval {
                tokio::select! {
                    _ = tokio::time::sleep(publication.settings.heartbeat_interval - idle_for) => {}
                    _ = target.wake.notified() => {}
                }
                continue;
            }
            heartbeat = true;
        }
        if snapshot.revision != attempted_revision {
            attempted_revision = snapshot.revision;
            attempts = 0;
        }
        let point = if heartbeat {
            ReplyReconcilePoint::Heartbeat
        } else {
            reconcile_point(&snapshot, published_revision, &published, terminal)
        };
        if !target.transport.reconciles_at(point) {
            // A `message` channel waits for the terminal revision.
            target.wake.notified().await;
            continue;
        }
        if point == ReplyReconcilePoint::Progress
            && let Some(last) = published.at
        {
            let elapsed = last.elapsed();
            if elapsed < publication.settings.min_progress_interval {
                // Stay wake-responsive inside the pacing window: a revision
                // arriving mid-window may be control-critical (the answer's
                // first text, an attention transition, the terminal) and must
                // re-evaluate immediately; ordinary progress falls back into
                // the remainder of the window on the next pass.
                tokio::select! {
                    _ = tokio::time::sleep(publication.settings.min_progress_interval - elapsed) => {}
                    _ = target.wake.notified() => {}
                }
                continue;
            }
        }
        // 3. Prepare everything provider-independent before ownership is
        // taken: channel resolution, the stored reply context, disclosure and
        // gate-prompt enrichment, and terminal attachment materialization.
        let prep = prepare(
            &publication,
            &coordinator,
            &target,
            &run_key,
            &snapshot,
            point,
        )
        .await;
        if let Err(Step::Later { delay }) = prep {
            // Nothing was attempted and nothing needs a durable write — but a
            // channel that never comes back must not keep the row `Active`
            // forever: once the document is terminal the wait counts toward
            // the terminal attempt budget and then fails closed.
            if terminal {
                attempts = attempts.saturating_add(1);
                if attempts >= publication.settings.terminal_attempt_budget {
                    if let Ok(ReplyPublicationClaim::Acquired(claimed)) = coordinator
                        .claim_reply_publication(
                            run_key.scope.clone(),
                            target.delivery_id,
                            publication.publisher_id.clone(),
                            publication.settings.lease_ttl,
                        )
                        .await
                    {
                        settle(
                            &coordinator,
                            &target,
                            &run_key,
                            claimed.publication.fence,
                            ReplyPublicationSettlement::Failed(DeliveryFailureKind::Rejected),
                        )
                        .await;
                    }
                    return;
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = target.wake.notified() => {}
            }
            continue;
        }
        // 3b. The row, read now that a reconcile is due. A published
        // revision the mirror did not know about means another owner moved
        // this publication: re-plan against the fresh row.
        let record = match load_row(&coordinator, &run_key, &target).await {
            Ok(record) => record,
            Err(LoadFailure::Vanished) => return,
            Err(LoadFailure::Retry) => {
                tokio::time::sleep(publication.settings.retry_backoff).await;
                continue;
            }
        };
        if !record.publication.status.is_active() {
            return;
        }
        if record.publication.published_revision != published_revision {
            published.revision = Some(record.publication.published_revision);
            continue;
        }
        // 4. Own the publication — the atomic claim immediately before
        // provider egress. Re-entry doubles as the heartbeat. Failure paths
        // discovered during preparation also settle under this claim.
        let record = match coordinator
            .claim_reply_publication(
                run_key.scope.clone(),
                target.delivery_id,
                publication.publisher_id.clone(),
                publication.settings.lease_ttl,
            )
            .await
        {
            Ok(ReplyPublicationClaim::Acquired(claimed)) => {
                if claimed.publication.published_revision != record.publication.published_revision {
                    // Another owner advanced this publication between the
                    // load and the claim; re-plan against the fresh row.
                    published.revision = Some(claimed.publication.published_revision);
                    continue;
                }
                claimed
            }
            Ok(ReplyPublicationClaim::Held { expires_at, .. }) => {
                let wait = (expires_at - chrono::Utc::now())
                    .to_std()
                    .unwrap_or(Duration::from_millis(50))
                    .min(publication.settings.lease_ttl)
                    .max(Duration::from_millis(20));
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = target.wake.notified() => {}
                }
                continue;
            }
            Ok(ReplyPublicationClaim::Settled(_)) => return,
            Err(error) => {
                tracing::debug!(target: LOG_TARGET, %error, "reply publication claim failed; retrying");
                tokio::time::sleep(publication.settings.retry_backoff).await;
                continue;
            }
        };
        let fence = record.publication.fence;
        let (record, step) = match prep {
            Ok(prepared) => {
                // 5. The newest desired revision is durable before any
                // provider access, under the fence the claim handed back.
                let desired = snapshot.revision.max(record.publication.desired_revision);
                let record = if record.publication.desired_revision < desired
                    || (terminal && record.publication.terminal_revision.is_none())
                {
                    let request = AdvanceScope {
                        target: &target,
                        run_key: &run_key,
                        snapshot: &snapshot,
                        record: &record,
                        fence,
                        terminal,
                    }
                    .request(
                        record.publication.published_revision,
                        record.publication.generation,
                        None,
                        record.publication.evidence.clone(),
                    );
                    match coordinator.advance_reply_publication(request).await {
                        Ok(record) => record,
                        Err(error) => {
                            tracing::debug!(target: LOG_TARGET, %error, "desired reply revision could not be persisted");
                            if is_fenced_out(&error) {
                                return;
                            }
                            tokio::time::sleep(publication.settings.retry_backoff).await;
                            continue;
                        }
                    }
                } else {
                    record
                };
                // 6. Reconcile, bounded within the lease just claimed.
                let step = reconcile(&publication, &snapshot, &record, point, prepared).await;
                (record, step)
            }
            Err(step) => (record, step),
        };
        let outcome = handle_step(
            StepContext {
                publication: &publication,
                coordinator: &coordinator,
                target: &target,
                run_key: &run_key,
                snapshot: &snapshot,
                record: &record,
                fence,
                terminal,
                published: &mut published,
                attempts: &mut attempts,
            },
            step,
        )
        .await;
        match outcome {
            LoopStep::Continue => {}
            LoopStep::Exit => return,
        }
    }
}

/// Why one row read gave the worker nothing to plan against.
enum LoadFailure {
    /// The row is gone; the worker exits.
    Vanished,
    /// A transient store failure; retry after the backoff.
    Retry,
}

async fn load_row(
    coordinator: &Arc<DeliveryCoordinator>,
    run_key: &RunKey,
    target: &TargetState,
) -> Result<ReplyPublicationRecord, LoadFailure> {
    match coordinator
        .load_reply_publication(run_key.scope.clone(), target.delivery_id)
        .await
    {
        Ok(Some(record)) => Ok(record),
        Ok(None) => {
            tracing::debug!(target: LOG_TARGET, delivery_id = %target.delivery_id, "reply publication row vanished; worker exits");
            Err(LoadFailure::Vanished)
        }
        Err(error) => {
            tracing::debug!(target: LOG_TARGET, %error, "reply publication load failed; retrying");
            Err(LoadFailure::Retry)
        }
    }
}

enum Step {
    Applied {
        report: ReplySinkReport,
        generation: u64,
    },
    Retry {
        reason: Option<ReplyOutcomeReason>,
        retry_after: Option<Duration>,
        generation: u64,
        /// A sink may hand back its checkpoint with a retry (it opened a
        /// provider stream before the rate limit hit); it is persisted so the
        /// retry resumes that presentation instead of opening another.
        checkpoint: Option<ReplySinkCheckpoint>,
    },
    Ambiguous {
        reason: ReplyOutcomeReason,
        generation: u64,
        checkpoint: Option<ReplySinkCheckpoint>,
    },
    Permanent {
        reason: ReplyOutcomeReason,
        kind: DeliveryFailureKind,
        generation: u64,
    },
    Stopped {
        generation: u64,
    },
    /// Nothing was attempted (a dependency was unavailable); try again later.
    Later {
        delay: Duration,
    },
}

/// What the outer loop does after a step was handled.
enum LoopStep {
    Continue,
    Exit,
}

/// What every guarded advance of one reconcile derives from: the row as
/// loaded, the snapshot being published, the claim's fence, and whether the
/// snapshot is terminal. The derived fields — a desired revision that never
/// moves backwards and a terminal revision set once — live here alone.
struct AdvanceScope<'a> {
    target: &'a TargetState,
    run_key: &'a RunKey,
    snapshot: &'a ReplySnapshot,
    record: &'a ReplyPublicationRecord,
    fence: u64,
    terminal: bool,
}

impl AdvanceScope<'_> {
    fn request(
        &self,
        published_revision: u64,
        generation: Option<u64>,
        checkpoint: Option<ReplySinkCheckpoint>,
        evidence: ReplyPublicationEvidence,
    ) -> AdvanceReplyPublicationRequest {
        AdvanceReplyPublicationRequest {
            delivery_id: self.target.delivery_id,
            scope: self.run_key.scope.clone(),
            fence: self.fence,
            desired_revision: self
                .snapshot
                .revision
                .max(self.record.publication.desired_revision),
            published_revision,
            terminal_revision: self
                .record
                .publication
                .terminal_revision
                .or(self.terminal.then_some(self.snapshot.revision)),
            generation,
            checkpoint,
            evidence,
            now: chrono::Utc::now(),
        }
    }
}

struct StepContext<'a> {
    publication: &'a Arc<ReplyPublication>,
    coordinator: &'a Arc<DeliveryCoordinator>,
    target: &'a TargetState,
    run_key: &'a RunKey,
    snapshot: &'a ReplySnapshot,
    record: &'a ReplyPublicationRecord,
    fence: u64,
    terminal: bool,
    published: &'a mut Published,
    attempts: &'a mut u32,
}

async fn handle_step(context: StepContext<'_>, step: Step) -> LoopStep {
    let StepContext {
        publication,
        coordinator,
        target,
        run_key,
        snapshot,
        record,
        fence,
        terminal,
        published,
        attempts,
    } = context;
    let scope = AdvanceScope {
        target,
        run_key,
        snapshot,
        record,
        fence,
        terminal,
    };
    match step {
        Step::Applied { report, generation } => {
            let evidence = ReplyPublicationEvidence {
                provider_refs: report.evidence.provider_refs.clone(),
                read_back_verified: report.evidence.read_back_verified,
                last_outcome: None,
                generation_changed: record
                    .publication
                    .generation
                    .is_some_and(|previous| previous != generation),
            };
            let advanced = coordinator
                .advance_reply_publication(scope.request(
                    snapshot.revision,
                    Some(generation),
                    report.checkpoint.clone(),
                    evidence,
                ))
                .await;
            if let Err(error) = advanced {
                tracing::debug!(target: LOG_TARGET, %error, "reply publication advance refused; the claim was lost or the publication settled");
                if is_fenced_out(&error) {
                    return LoopStep::Exit;
                }
                tokio::time::sleep(publication.settings.retry_backoff).await;
                return LoopStep::Continue;
            }
            published.attention = attention_signature(snapshot);
            published.finalized = snapshot.document.answer.finalized;
            published.has_text = !snapshot.document.answer.text.as_str().is_empty();
            published.at = Some(tokio::time::Instant::now());
            published.revision = Some(snapshot.revision);
            *attempts = 0;
            if terminal {
                settle(
                    coordinator,
                    target,
                    run_key,
                    fence,
                    ReplyPublicationSettlement::Delivered,
                )
                .await;
                return LoopStep::Exit;
            }
            LoopStep::Continue
        }
        Step::Retry {
            reason,
            retry_after,
            generation,
            checkpoint,
        } => {
            *attempts = attempts.saturating_add(1);
            record_outcome(coordinator, &scope, reason.clone(), generation, checkpoint).await;
            if terminal && *attempts >= publication.settings.terminal_attempt_budget {
                tracing::debug!(target: LOG_TARGET, delivery_id = %target.delivery_id, ?reason, "terminal reply reconcile exhausted its retry budget; failing closed");
                settle(
                    coordinator,
                    target,
                    run_key,
                    fence,
                    ReplyPublicationSettlement::Failed(DeliveryFailureKind::TransportUnavailable),
                )
                .await;
                return LoopStep::Exit;
            }
            let delay = retry_after.unwrap_or_else(|| backoff(publication, *attempts));
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = target.wake.notified(), if !terminal => {}
            }
            LoopStep::Continue
        }
        Step::Ambiguous {
            reason,
            generation,
            checkpoint,
        } => {
            *attempts = attempts.saturating_add(1);
            // A retry is safe only when SOME checkpoint exists for the sink
            // to reconcile from — the one it just handed back, or one a
            // previous applied reconcile persisted. With neither, repeating
            // the call would blindly repeat the exact provider side effect
            // (a first message-transport send, a first stream open), so the
            // publication fails closed as `Unknown` instead.
            let has_read_back_state =
                checkpoint.is_some() || record.publication.checkpoint.is_some();
            record_outcome(coordinator, &scope, Some(reason), generation, checkpoint).await;
            if !has_read_back_state
                || (terminal && *attempts >= publication.settings.terminal_attempt_budget)
            {
                settle(
                    coordinator,
                    target,
                    run_key,
                    fence,
                    ReplyPublicationSettlement::Unknown,
                )
                .await;
                return LoopStep::Exit;
            }
            tokio::time::sleep(backoff(publication, *attempts)).await;
            LoopStep::Continue
        }
        Step::Permanent {
            reason,
            kind,
            generation,
        } => {
            record_outcome(coordinator, &scope, Some(reason), generation, None).await;
            settle(
                coordinator,
                target,
                run_key,
                fence,
                ReplyPublicationSettlement::Failed(kind),
            )
            .await;
            LoopStep::Exit
        }
        Step::Stopped { generation } => {
            record_outcome(
                coordinator,
                &scope,
                Some(ReplyOutcomeReason::new("stopped by user")),
                generation,
                None,
            )
            .await;
            kernel_ports::request_stop(
                publication.turn_coordinator.as_ref(),
                &run_key.scope,
                &target.registration.actor,
                run_key.run_id,
            )
            .await;
            // The run's terminal commit brings the terminal revision.
            target.wake.notified().await;
            LoopStep::Continue
        }
        Step::Later { delay } => {
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = target.wake.notified() => {}
            }
            LoopStep::Continue
        }
    }
}

/// Everything a reconcile needs that does not require publication ownership.
struct PreparedReconcile {
    channel: ResolvedChannelDelivery,
    reply_context: Option<ReplyContextBytes>,
    document: ReplyDocument,
    reply_target: ReplyTarget,
    materialized_attachments: Vec<WorkspaceFile>,
}

async fn prepare(
    publication: &Arc<ReplyPublication>,
    coordinator: &Arc<DeliveryCoordinator>,
    target: &TargetState,
    run_key: &RunKey,
    snapshot: &ReplySnapshot,
    point: ReplyReconcilePoint,
) -> Result<PreparedReconcile, Step> {
    let registration = &target.registration;
    let Some(channel) = coordinator.resolve_reply_channel(registration.extension_id.as_str())
    else {
        tracing::debug!(target: LOG_TARGET, extension_id = %registration.extension_id, "reply channel is not active; retrying later");
        return Err(Step::Later {
            delay: publication
                .settings
                .max_retry_backoff
                .min(Duration::from_secs(5)),
        });
    };
    if channel.reply.is_none() {
        return Err(Step::Permanent {
            reason: ReplyOutcomeReason::new("channel no longer binds a reply sink"),
            kind: DeliveryFailureKind::Rejected,
            generation: channel.generation,
        });
    }
    // The reply context is the per-run SNAPSHOT taken at registration (and
    // persisted on the durable descriptor for resumes) — never a fresh read
    // of the latest-wins per-conversation store, which a newer message in
    // the same conversation may have overwritten.
    let reply_context = target.reply_context.clone();
    let reply_target = ReplyTarget {
        scope: registration.scope.clone(),
        actor: registration.actor.clone(),
        run_id: registration.run_id,
        conversation: registration.conversation.clone(),
        thread_anchor: registration.thread_anchor.clone(),
        audience: registration.audience,
    };
    let mut document = disclose_for_audience(&snapshot.document, registration.audience);
    if document.attention.is_some() {
        kernel_ports::enrich_attention(
            publication.approval_context.as_ref(),
            publication.blocked_auth_prompts.as_ref(),
            publication.turn_coordinator.as_ref(),
            &reply_target,
            &mut document,
        )
        .await;
        // Disclosure has the last word even over enrichment.
        document = disclose_for_audience(&document, registration.audience);
    }
    let materialized_attachments =
        if point == ReplyReconcilePoint::Terminal && !document.attachments.is_empty() {
            materialize(publication, run_key, &channel, &reply_target).await?
        } else {
            Vec::new()
        };
    Ok(PreparedReconcile {
        channel,
        reply_context,
        document,
        reply_target,
        materialized_attachments,
    })
}

async fn reconcile(
    publication: &Arc<ReplyPublication>,
    snapshot: &ReplySnapshot,
    record: &ReplyPublicationRecord,
    point: ReplyReconcilePoint,
    prepared: PreparedReconcile,
) -> Step {
    let PreparedReconcile {
        channel,
        reply_context,
        document,
        reply_target,
        materialized_attachments,
    } = prepared;
    let generation = channel.generation;
    let Some(sink) = channel.reply.clone() else {
        // Checked during preparation; the channel snapshot is immutable.
        return Step::Permanent {
            reason: ReplyOutcomeReason::new("channel no longer binds a reply sink"),
            kind: DeliveryFailureKind::Rejected,
            generation,
        };
    };
    // The pre-claim point classification stands: the caller re-plans (and
    // re-classifies) whenever the claimed row's published revision moved
    // between the load and the claim.
    let request = ReplyReconcileRequest {
        revision: ReplyRevision {
            revision: snapshot.revision,
            document,
        },
        point,
        target: reply_target,
        reply_context,
        checkpoint: record.publication.checkpoint.clone(),
        extension_generation: generation,
        materialized_attachments,
    };
    // The sink call is bounded by what is LEFT of the claim — the desired
    // revision write between the claim and this call consumed lease time —
    // so the claim always outlives the provider operation and lease expiry
    // can never produce two simultaneous provider calls.
    let remaining = record
        .publication
        .lease
        .as_ref()
        .map(|lease| {
            (lease.expires_at - chrono::Utc::now())
                .to_std()
                .unwrap_or(Duration::ZERO)
        })
        .unwrap_or(publication.settings.lease_ttl);
    let timeout = publication.settings.reconcile_timeout.min(remaining);
    let reconcile =
        tokio::time::timeout(timeout, sink.reconcile(request, channel.egress.as_ref())).await;
    match reconcile {
        Ok(Ok(report)) => match &report.outcome {
            ReplySinkOutcome::Applied => Step::Applied { report, generation },
            ReplySinkOutcome::Retryable {
                reason,
                retry_after,
            } => Step::Retry {
                reason: Some(reason.clone()),
                retry_after: *retry_after,
                generation,
                checkpoint: report.checkpoint.clone(),
            },
            ReplySinkOutcome::Ambiguous { reason } => Step::Ambiguous {
                reason: reason.clone(),
                generation,
                checkpoint: report.checkpoint.clone(),
            },
            ReplySinkOutcome::Permanent { reason } => Step::Permanent {
                reason: reason.clone(),
                kind: DeliveryFailureKind::Rejected,
                generation,
            },
            // Fail-closed per `.claude/rules/lifecycle.md`: an authentication
            // rejection is terminal and never retried until the credential
            // changes. The failure kind, reason, and the sink's checkpoint
            // stay on the settled row; restoring credentials goes through the
            // extension's ordinary reconnect flow, and the settled reply is
            // not republished.
            ReplySinkOutcome::Unauthorized { reason } => Step::Permanent {
                reason: reason.clone(),
                kind: DeliveryFailureKind::AuthorizationRevoked,
                generation,
            },
            ReplySinkOutcome::StoppedByUser => Step::Stopped { generation },
        },
        Ok(Err(error)) => channel_error_step(error, generation),
        Err(_elapsed) => Step::Ambiguous {
            reason: ReplyOutcomeReason::new("reply sink reconcile timed out"),
            generation,
            checkpoint: None,
        },
    }
}

/// A sink error is the adapter's own failure. Rendering/configuration faults
/// do not heal by retrying; transfer faults say whether they might.
fn channel_error_step(error: ChannelError, generation: u64) -> Step {
    let reason = ReplyOutcomeReason::new(error.to_string());
    match error {
        ChannelError::AttachmentTransfer {
            retryable: true, ..
        } => Step::Retry {
            reason: Some(reason),
            retry_after: None,
            generation,
            checkpoint: None,
        },
        ChannelError::Parse { .. }
        | ChannelError::Configuration { .. }
        | ChannelError::Render { .. }
        | ChannelError::VendorWiring { .. }
        | ChannelError::AttachmentTransfer { .. }
        | ChannelError::Unsupported => Step::Permanent {
            reason,
            kind: DeliveryFailureKind::Rejected,
            generation,
        },
    }
}

async fn materialize(
    publication: &Arc<ReplyPublication>,
    run_key: &RunKey,
    channel: &ResolvedChannelDelivery,
    reply_target: &ReplyTarget,
) -> Result<Vec<WorkspaceFile>, Step> {
    let Some(agent_id) = run_key.scope.agent_id.clone() else {
        return Err(Step::Permanent {
            reason: ReplyOutcomeReason::new("reply attachments need an agent-scoped run"),
            kind: DeliveryFailureKind::Rejected,
            generation: channel.generation,
        });
    };
    let thread_scope = ThreadScope {
        tenant_id: run_key.scope.tenant_id.clone(),
        agent_id,
        project_id: run_key.scope.project_id.clone(),
        owner_user_id: Some(reply_target.actor.user_id.clone()),
        mission_id: None,
    };
    let sources = publication.terminal_attachments(run_key);
    match materialize_workspace_files(
        publication.project_filesystem.as_ref(),
        &thread_scope,
        sources,
    )
    .await
    {
        Ok(files) => Ok(files),
        Err(error) => {
            let kind = workspace_materialization_failure_kind(&error);
            let reason = ReplyOutcomeReason::new(error.to_string());
            Err(match kind {
                DeliveryFailureKind::TransportUnavailable => Step::Retry {
                    reason: Some(reason),
                    retry_after: None,
                    generation: channel.generation,
                    checkpoint: None,
                },
                kind => Step::Permanent {
                    reason,
                    kind,
                    generation: channel.generation,
                },
            })
        }
    }
}

fn reconcile_point(
    snapshot: &ReplySnapshot,
    published_revision: u64,
    published: &Published,
    terminal: bool,
) -> ReplyReconcilePoint {
    if terminal {
        ReplyReconcilePoint::Terminal
    } else if published_revision == 0 {
        ReplyReconcilePoint::Opened
    } else if attention_signature(snapshot) != published.attention
        || snapshot.document.answer.finalized != published.finalized
        // The answer's first visible text: a fast run reaches its terminal
        // commit inside the progress pacing window, and pacing the first
        // text away would jump the stream from "working" straight to the
        // finalized answer. Pacing applies to text-to-text growth only.
        || (!published.has_text && !snapshot.document.answer.text.as_str().is_empty())
    {
        ReplyReconcilePoint::ControlCritical
    } else {
        ReplyReconcilePoint::Progress
    }
}

fn attention_signature(snapshot: &ReplySnapshot) -> Option<(ReplyAttentionKind, Option<String>)> {
    snapshot.document.attention.as_ref().map(|attention| {
        (
            attention.kind,
            attention.gate_ref.as_ref().map(|g| g.as_str().to_string()),
        )
    })
}

/// One non-applied outcome's durable write: evidence and (when handed back)
/// the checkpoint, under the worker's fence.
async fn record_outcome(
    coordinator: &Arc<DeliveryCoordinator>,
    scope: &AdvanceScope<'_>,
    last_outcome: Option<ReplyOutcomeReason>,
    generation: u64,
    checkpoint: Option<ReplySinkCheckpoint>,
) {
    let evidence = ReplyPublicationEvidence {
        provider_refs: scope.record.publication.evidence.provider_refs.clone(),
        read_back_verified: scope.record.publication.evidence.read_back_verified,
        last_outcome,
        generation_changed: scope
            .record
            .publication
            .generation
            .is_some_and(|previous| previous != generation),
    };
    if let Err(error) = coordinator
        .advance_reply_publication(scope.request(
            scope.record.publication.published_revision,
            Some(generation),
            checkpoint,
            evidence,
        ))
        .await
    {
        tracing::debug!(target: LOG_TARGET, %error, "reply publication evidence write refused");
    }
}

async fn settle(
    coordinator: &Arc<DeliveryCoordinator>,
    target: &TargetState,
    run_key: &RunKey,
    fence: u64,
    settlement: ReplyPublicationSettlement,
) {
    if let Err(error) = coordinator
        .settle_reply_publication(run_key.scope.clone(), target.delivery_id, fence, settlement)
        .await
    {
        tracing::debug!(target: LOG_TARGET, %error, ?settlement, "reply publication settlement refused");
    }
}

fn backoff(publication: &Arc<ReplyPublication>, attempts: u32) -> Duration {
    let base = publication
        .settings
        .retry_backoff
        .max(Duration::from_millis(1));
    let factor = 2u32.saturating_pow(attempts.saturating_sub(1).min(16));
    (base.saturating_mul(factor)).min(publication.settings.max_retry_backoff)
}

fn is_fenced_out(error: &CoordinatedDeliveryError) -> bool {
    matches!(
        error,
        CoordinatedDeliveryError::Outbound(
            ironclaw_outbound::OutboundError::StaleReplyPublisher { .. }
                | ironclaw_outbound::OutboundError::ReplyPublicationSettled
                | ironclaw_outbound::OutboundError::ReplyPublicationNotFound
        )
    )
}
