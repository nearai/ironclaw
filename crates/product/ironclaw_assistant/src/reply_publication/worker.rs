//! The per-target reconcile loop.
//!
//! Desired state is the projection's newest document; the worker converges
//! the sink toward it under a lease, coalescing naturally (it always
//! publishes the *latest* snapshot, so intermediate replaceable revisions
//! collapse) while never skipping a control-critical or terminal point.
//! Every store write goes through the coordinator with the fence the lease
//! claim handed back, so a worker that lost its lease to another node is
//! rejected at the store, not trusted.

use std::sync::Arc;
use std::time::Duration;

use ironclaw_extension_contracts::channel::ReplyTransport;
use ironclaw_extension_contracts::channel_adapter::ChannelError;
use ironclaw_extension_contracts::reply::{
    ReplyAttentionKind, ReplyContextBytes, ReplyId, ReplyReconcilePoint, ReplyReconcileRequest,
    ReplyRevision, ReplySinkOutcome, ReplySinkReport, ReplyTarget,
};
use ironclaw_host_api::attachment::WorkspaceFile;
use ironclaw_outbound::{
    AdvanceReplyPublicationRequest, DeliveryFailureKind, ReplyPublicationClaim,
    ReplyPublicationEvidence, ReplyPublicationRecord, ReplyPublicationSettlement,
};
use ironclaw_product_contracts::delivery::ResolvedChannelDelivery;
use ironclaw_threads::ThreadScope;

use super::{Inner, RunKey, TargetState};
use crate::delivery_coordinator::{
    CoordinatedDeliveryError, materialize_workspace_files, workspace_materialization_failure_kind,
};
use crate::reply_projection::{ReplySnapshot, disclose_for_audience};

const LOG_TARGET: &str = "ironclaw::reborn::reply_publication";

/// What the last successful reconcile looked like, to tell a control-critical
/// transition from ordinary progress.
#[derive(Default)]
struct Published {
    attention: Option<(ReplyAttentionKind, Option<String>)>,
    finalized: bool,
    at: Option<tokio::time::Instant>,
}

pub(super) async fn run_target(inner: Arc<Inner>, target: Arc<TargetState>) {
    let run_key = RunKey::new(&target.registration.scope, target.registration.run_id);
    let mut published = Published::default();
    let mut attempts: u32 = 0;
    let mut attempted_revision: u64 = 0;
    loop {
        // 1. Desired state.
        let Some(snapshot) = inner.projection.snapshot(&run_key.scope, run_key.run_id) else {
            target.wake.notified().await;
            continue;
        };
        if snapshot.terminal_pending && !snapshot.document.is_terminal() {
            let fetch_inner = Arc::clone(&inner);
            let fetch_key = run_key.clone();
            tokio::spawn(async move { fetch_inner.ensure_terminal_facts(&fetch_key).await });
            target.wake.notified().await;
            continue;
        }
        // 2. Published state.
        let record = match inner
            .coordinator
            .load_reply_publication(run_key.scope.clone(), target.delivery_id)
            .await
        {
            Ok(Some(record)) => record,
            Ok(None) => {
                tracing::debug!(target: LOG_TARGET, delivery_id = %target.delivery_id, "reply publication row vanished; worker exits");
                return;
            }
            Err(error) => {
                tracing::debug!(target: LOG_TARGET, %error, "reply publication load failed; retrying");
                tokio::time::sleep(inner.settings.retry_backoff).await;
                continue;
            }
        };
        if !record.publication.status.is_active() {
            return;
        }
        let terminal = snapshot.document.is_terminal();
        let behind = snapshot.revision > record.publication.published_revision;
        let mut heartbeat = false;
        if !behind {
            let can_heartbeat = !terminal
                && record.publication.published_revision > 0
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
            if idle_for < inner.settings.heartbeat_interval {
                tokio::select! {
                    _ = tokio::time::sleep(inner.settings.heartbeat_interval - idle_for) => {}
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
            reconcile_point(&snapshot, &record, &published, terminal)
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
            if elapsed < inner.settings.min_progress_interval {
                tokio::time::sleep(inner.settings.min_progress_interval - elapsed).await;
                continue;
            }
        }
        // 3. Own the publication.
        let record = match inner
            .coordinator
            .claim_reply_publication(
                run_key.scope.clone(),
                target.delivery_id,
                inner.publisher_id.clone(),
                inner.settings.lease_ttl,
            )
            .await
        {
            Ok(ReplyPublicationClaim::Acquired(record)) => record,
            Ok(ReplyPublicationClaim::Held { expires_at, .. }) => {
                let wait = (expires_at - chrono::Utc::now())
                    .to_std()
                    .unwrap_or(Duration::from_millis(50))
                    .min(inner.settings.lease_ttl)
                    .max(Duration::from_millis(20));
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = target.wake.notified() => {}
                }
                continue;
            }
            Ok(ReplyPublicationClaim::Settled(_)) => return,
            Err(error) => {
                tracing::debug!(target: LOG_TARGET, %error, "reply publication lease claim failed; retrying");
                tokio::time::sleep(inner.settings.retry_backoff).await;
                continue;
            }
        };
        let fence = record.publication.fence;
        // 4. Reconcile.
        let step = reconcile_once(&inner, &target, &run_key, &snapshot, &record, point).await;
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
                let advanced = inner
                    .coordinator
                    .advance_reply_publication(AdvanceReplyPublicationRequest {
                        delivery_id: target.delivery_id,
                        scope: run_key.scope.clone(),
                        fence,
                        desired_revision: snapshot.revision,
                        published_revision: snapshot.revision,
                        terminal_revision: terminal.then_some(snapshot.revision),
                        generation: Some(generation),
                        checkpoint: report.checkpoint.clone(),
                        evidence,
                        now: chrono::Utc::now(),
                    })
                    .await;
                if let Err(error) = advanced {
                    tracing::debug!(target: LOG_TARGET, %error, "reply publication advance refused; the lease was lost or the publication settled");
                    if is_fenced_out(&error) {
                        return;
                    }
                    tokio::time::sleep(inner.settings.retry_backoff).await;
                    continue;
                }
                published.attention = attention_signature(&snapshot);
                published.finalized = snapshot.document.answer.finalized;
                published.at = Some(tokio::time::Instant::now());
                attempts = 0;
                if terminal {
                    settle(
                        &inner,
                        &target,
                        &run_key,
                        fence,
                        ReplyPublicationSettlement::Delivered,
                    )
                    .await;
                    return;
                }
            }
            Step::Retry {
                reason,
                retry_after,
                generation,
                checkpoint,
            } => {
                attempts = attempts.saturating_add(1);
                record_outcome(
                    &inner,
                    &target,
                    &run_key,
                    &snapshot,
                    &record,
                    fence,
                    reason.clone(),
                    generation,
                    checkpoint,
                )
                .await;
                if terminal && attempts >= inner.settings.terminal_attempt_budget {
                    tracing::debug!(target: LOG_TARGET, delivery_id = %target.delivery_id, ?reason, "terminal reply reconcile exhausted its retry budget; failing closed");
                    settle(
                        &inner,
                        &target,
                        &run_key,
                        fence,
                        ReplyPublicationSettlement::Failed(
                            DeliveryFailureKind::TransportUnavailable,
                        ),
                    )
                    .await;
                    return;
                }
                let delay = retry_after.unwrap_or_else(|| backoff(&inner, attempts));
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = target.wake.notified(), if !terminal => {}
                }
            }
            Step::Ambiguous {
                reason,
                generation,
                checkpoint,
            } => {
                attempts = attempts.saturating_add(1);
                record_outcome(
                    &inner,
                    &target,
                    &run_key,
                    &snapshot,
                    &record,
                    fence,
                    Some(reason),
                    generation,
                    checkpoint,
                )
                .await;
                if terminal && attempts >= inner.settings.terminal_attempt_budget {
                    settle(
                        &inner,
                        &target,
                        &run_key,
                        fence,
                        ReplyPublicationSettlement::Unknown,
                    )
                    .await;
                    return;
                }
                tokio::time::sleep(backoff(&inner, attempts)).await;
            }
            Step::Permanent {
                reason,
                kind,
                generation,
            } => {
                record_outcome(
                    &inner,
                    &target,
                    &run_key,
                    &snapshot,
                    &record,
                    fence,
                    Some(reason),
                    generation,
                    None,
                )
                .await;
                settle(
                    &inner,
                    &target,
                    &run_key,
                    fence,
                    ReplyPublicationSettlement::Failed(kind),
                )
                .await;
                return;
            }
            Step::Stopped { generation } => {
                record_outcome(
                    &inner,
                    &target,
                    &run_key,
                    &snapshot,
                    &record,
                    fence,
                    Some(
                        ironclaw_extension_contracts::reply::ReplyOutcomeReason::new(
                            "stopped by user",
                        ),
                    ),
                    generation,
                    None,
                )
                .await;
                inner
                    .stop_requests
                    .request_stop(&run_key.scope, &target.registration.actor, run_key.run_id)
                    .await;
                // The run's terminal commit brings the terminal revision.
                target.wake.notified().await;
            }
            Step::Later { delay } => {
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = target.wake.notified() => {}
                }
            }
        }
    }
}

enum Step {
    Applied {
        report: ReplySinkReport,
        generation: u64,
    },
    Retry {
        reason: Option<ironclaw_extension_contracts::reply::ReplyOutcomeReason>,
        retry_after: Option<Duration>,
        generation: u64,
        /// A sink may hand back its checkpoint with a retry (it opened a
        /// provider stream before the rate limit hit); it is persisted so the
        /// retry resumes that presentation instead of opening another.
        checkpoint: Option<ironclaw_extension_contracts::reply::ReplySinkCheckpoint>,
    },
    Ambiguous {
        reason: ironclaw_extension_contracts::reply::ReplyOutcomeReason,
        generation: u64,
        checkpoint: Option<ironclaw_extension_contracts::reply::ReplySinkCheckpoint>,
    },
    Permanent {
        reason: ironclaw_extension_contracts::reply::ReplyOutcomeReason,
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

async fn reconcile_once(
    inner: &Arc<Inner>,
    target: &TargetState,
    run_key: &RunKey,
    snapshot: &ReplySnapshot,
    record: &ReplyPublicationRecord,
    point: ReplyReconcilePoint,
) -> Step {
    let registration = &target.registration;
    let Some(channel) = inner
        .coordinator
        .resolve_reply_channel(registration.extension_id.as_str())
    else {
        tracing::debug!(target: LOG_TARGET, extension_id = %registration.extension_id, "reply channel is not active; retrying later");
        return Step::Later {
            delay: inner.settings.max_retry_backoff.min(Duration::from_secs(5)),
        };
    };
    let Some(sink) = channel.reply.clone() else {
        return Step::Permanent {
            reason: ironclaw_extension_contracts::reply::ReplyOutcomeReason::new(
                "channel no longer binds a reply sink",
            ),
            kind: DeliveryFailureKind::Rejected,
            generation: channel.generation,
        };
    };
    let reply_context = match inner
        .coordinator
        .reply_context_for_publication(&channel, registration.conversation.as_ref())
        .await
    {
        Ok(bytes) => bytes.and_then(|bytes| match ReplyContextBytes::new(bytes) {
            Ok(context) => Some(context),
            Err(error) => {
                tracing::debug!(target: LOG_TARGET, %error, "stored reply context exceeds the seam bound; publishing without it");
                None
            }
        }),
        Err(error) => {
            tracing::debug!(target: LOG_TARGET, %error, "reply context unavailable; retrying later");
            return Step::Later {
                delay: inner.settings.retry_backoff,
            };
        }
    };
    let reply_target = ReplyTarget {
        scope: registration.scope.clone(),
        actor: registration.actor.clone(),
        run_id: registration.run_id,
        conversation: registration.conversation.clone(),
        thread_anchor: registration.thread_anchor.clone(),
        audience: registration.audience,
    };
    let mut document = disclose_for_audience(&snapshot.document, registration.audience);
    if document.attention.is_some()
        && let Some(enricher) = inner.attention.as_ref()
    {
        enricher.enrich(&reply_target, &mut document).await;
        // Disclosure has the last word even over enrichment.
        document = disclose_for_audience(&document, registration.audience);
    }
    let materialized_attachments =
        if point == ReplyReconcilePoint::Terminal && !document.attachments.is_empty() {
            match materialize(inner, run_key, &channel, &reply_target).await {
                Ok(files) => files,
                Err(step) => return step,
            }
        } else {
            Vec::new()
        };
    let request = ReplyReconcileRequest {
        revision: ReplyRevision {
            reply_id: ReplyId::for_run(&registration.run_id),
            revision: snapshot.revision,
            document,
        },
        point,
        target: reply_target,
        reply_context,
        checkpoint: record.publication.checkpoint.clone(),
        extension_generation: channel.generation,
        materialized_attachments,
    };
    let generation = channel.generation;
    let reconcile = tokio::time::timeout(
        inner.settings.reconcile_timeout,
        sink.reconcile(request, channel.egress.as_ref()),
    )
    .await;
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
            ReplySinkOutcome::Unauthorized { reason } => Step::Permanent {
                reason: reason.clone(),
                kind: DeliveryFailureKind::AuthorizationRevoked,
                generation,
            },
            ReplySinkOutcome::StoppedByUser => Step::Stopped { generation },
        },
        Ok(Err(error)) => channel_error_step(error, generation),
        Err(_elapsed) => Step::Ambiguous {
            reason: ironclaw_extension_contracts::reply::ReplyOutcomeReason::new(
                "reply sink reconcile timed out",
            ),
            generation,
            checkpoint: None,
        },
    }
}

/// A sink error is the adapter's own failure. Rendering/configuration faults
/// do not heal by retrying; transfer faults say whether they might.
fn channel_error_step(error: ChannelError, generation: u64) -> Step {
    let reason = ironclaw_extension_contracts::reply::ReplyOutcomeReason::new(error.to_string());
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
    inner: &Arc<Inner>,
    run_key: &RunKey,
    channel: &ResolvedChannelDelivery,
    reply_target: &ReplyTarget,
) -> Result<Vec<WorkspaceFile>, Step> {
    let Some(agent_id) = run_key.scope.agent_id.clone() else {
        return Err(Step::Permanent {
            reason: ironclaw_extension_contracts::reply::ReplyOutcomeReason::new(
                "reply attachments need an agent-scoped run",
            ),
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
    let sources = inner.terminal_attachments(run_key);
    match materialize_workspace_files(inner.project_filesystem.as_ref(), &thread_scope, sources)
        .await
    {
        Ok(files) => Ok(files),
        Err(error) => {
            let kind = workspace_materialization_failure_kind(&error);
            let reason =
                ironclaw_extension_contracts::reply::ReplyOutcomeReason::new(error.to_string());
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
    record: &ReplyPublicationRecord,
    published: &Published,
    terminal: bool,
) -> ReplyReconcilePoint {
    if terminal {
        ReplyReconcilePoint::Terminal
    } else if record.publication.published_revision == 0 {
        ReplyReconcilePoint::Opened
    } else if attention_signature(snapshot) != published.attention
        || snapshot.document.answer.finalized != published.finalized
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

#[allow(clippy::too_many_arguments)] // arch-exempt: too_many_args, worker bookkeeping bundle pending a `PublicationWrite` struct, plan #8007
async fn record_outcome(
    inner: &Arc<Inner>,
    target: &TargetState,
    run_key: &RunKey,
    snapshot: &ReplySnapshot,
    record: &ReplyPublicationRecord,
    fence: u64,
    last_outcome: Option<ironclaw_extension_contracts::reply::ReplyOutcomeReason>,
    generation: u64,
    checkpoint: Option<ironclaw_extension_contracts::reply::ReplySinkCheckpoint>,
) {
    let terminal = snapshot.document.is_terminal();
    let evidence = ReplyPublicationEvidence {
        provider_refs: record.publication.evidence.provider_refs.clone(),
        read_back_verified: record.publication.evidence.read_back_verified,
        last_outcome,
        generation_changed: record
            .publication
            .generation
            .is_some_and(|previous| previous != generation),
    };
    if let Err(error) = inner
        .coordinator
        .advance_reply_publication(AdvanceReplyPublicationRequest {
            delivery_id: target.delivery_id,
            scope: run_key.scope.clone(),
            fence,
            desired_revision: snapshot.revision.max(record.publication.desired_revision),
            published_revision: record.publication.published_revision,
            terminal_revision: record
                .publication
                .terminal_revision
                .or(terminal.then_some(snapshot.revision)),
            generation: Some(generation),
            checkpoint,
            evidence,
            now: chrono::Utc::now(),
        })
        .await
    {
        tracing::debug!(target: LOG_TARGET, %error, "reply publication evidence write refused");
    }
}

async fn settle(
    inner: &Arc<Inner>,
    target: &TargetState,
    run_key: &RunKey,
    fence: u64,
    settlement: ReplyPublicationSettlement,
) {
    if let Err(error) = inner
        .coordinator
        .settle_reply_publication(run_key.scope.clone(), target.delivery_id, fence, settlement)
        .await
    {
        tracing::debug!(target: LOG_TARGET, %error, ?settlement, "reply publication settlement refused");
    }
}

fn backoff(inner: &Arc<Inner>, attempts: u32) -> Duration {
    let base = inner.settings.retry_backoff.max(Duration::from_millis(1));
    let factor = 2u32.saturating_pow(attempts.saturating_sub(1).min(16));
    (base.saturating_mul(factor)).min(inner.settings.max_retry_backoff)
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

// Keep the transport vocabulary in the worker's signature space so a future
// cadence variant is a compile error here rather than a silent no-op.
#[allow(dead_code)]
fn cadence(transport: ReplyTransport) -> &'static str {
    match transport {
        ReplyTransport::Stream => "stream",
        ReplyTransport::Message => "message",
    }
}
