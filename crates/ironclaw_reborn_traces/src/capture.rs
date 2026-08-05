//! Autonomous capture pipeline: standing-policy gate, envelope build, queue,
//! immediate flush, and the periodic queue-flush worker.
//!
//! This is the half of turn-end trace capture that is pure Trace Commons —
//! consent policy, envelope scoring/redaction, queue/hold semantics, retry
//! cadence. It is keyed on a **scope string** and this crate's own
//! [`ConversationMessage`], so it names no turn, thread, or runtime type; the
//! observer that turns a terminal turn lifecycle event into those two inputs
//! is `ironclaw_runner::trace_capture` (PROPOSAL §6.10.1, WS6 — the eviction's
//! two named destinations are "`trace_commons` + the turn-runner observer
//! seam", and this is the `trace_commons` half).
//!
//! Capture must never block or fail whatever produced the turn: every entry
//! point here is infallible from the caller's perspective and logs at
//! `debug!` only (`info!`/`warn!` corrupt the REPL).
//!
//! Credit-notice delivery (v1 broadcasts via `ChannelManager`) is
//! intentionally not wired here yet: there is no outbound notification
//! surface at this tier. The notice outbox still accumulates on disk and is
//! delivered when the same scope runs under the v1 binary; a Reborn-native
//! delivery path is a follow-up.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::ConversationMessage;
use crate::client::{
    TraceClientAutonomousCaptureOutcome, TraceClientAutonomousCaptureRequest, TraceClientHost,
    TraceClientScope,
};
use crate::contribution::{self as trace, resolve_effective_capture_policy};

/// Recent-transcript bound, mirroring v1 (last 24 messages, max 5 turns).
pub const CAPTURE_MESSAGE_LIMIT: usize = 24;
/// Per-envelope turn bound, mirroring v1.
pub const CAPTURE_MAX_TURNS: usize = 5;
/// Immediate flush limit after queueing one envelope (v1 parity).
const CAPTURE_FLUSH_LIMIT: usize = 10;
/// Periodic queue-flush cadence and per-scope limit (v1 parity).
const TRACE_QUEUE_WORKER_INTERVAL: Duration = Duration::from_secs(300);
const TRACE_QUEUE_WORKER_FLUSH_LIMIT: usize = 25;

/// Scopes whose queues the periodic worker flushes. Seeded with the runtime
/// owner and extended with every scope seen at capture time. Queued items for
/// scopes not seen since boot only flush on that scope's next turn — this tier
/// has no user directory to enumerate (v1 lists active users from its
/// database).
pub type ObservedTraceScopes = Arc<Mutex<BTreeSet<String>>>;

/// Record `scope` as having produced capturable work, so the periodic worker
/// retries its queue.
pub fn record_observed_scope(observed_scopes: &ObservedTraceScopes, scope: &str) {
    let mut scopes = match observed_scopes.lock() {
        Ok(scopes) => scopes,
        Err(poisoned) => poisoned.into_inner(),
    };
    scopes.insert(scope.to_string());
}

/// One capture's best-effort pipeline: resolve the effective standing policy,
/// build the envelope, then queue (and immediately try to flush) it.
///
/// Errors never propagate — every exit is a `debug!` line keyed by the
/// pseudonymous contributor ref, never raw content.
///
/// `scope` is the tenant-scoped trace state key (see
/// [`trace::trace_scope_key`]); `task_failed` marks the transcript's terminal
/// outcome as a failure, which the caller knows and the transcript does not.
pub async fn capture_conversation_trace(
    scope: &str,
    messages: &[ConversationMessage],
    task_failed: bool,
) {
    if messages.is_empty() {
        return;
    }
    let scope_ref = trace::local_pseudonymous_contributor_id(scope);
    // Gate on the EFFECTIVE enrollment (personal-invite OR admin-provisioned
    // instance), mirroring the flush gate: an instance-only-enrolled user has no
    // enabled per-user policy, so a per-user-only check would drop their turns
    // before queueing — leaving the instance-aware flush nothing to submit. The
    // resolver returns the governing (and always-enabled) policy, or None when
    // the scope is enrolled in neither.
    let policy = match resolve_effective_capture_policy(Some(scope)) {
        Ok(Some(policy)) => policy,
        Ok(None) => return,
        Err(error) => {
            tracing::debug!(%error, %scope_ref, "Reborn trace capture could not resolve policy");
            return;
        }
    };

    let outcome = TraceClientHost
        .prepare_autonomous_envelope_from_messages(TraceClientAutonomousCaptureRequest {
            scope: TraceClientScope::user(scope.to_string()),
            // The lifecycle event does not identify the product surface
            // (REPL/WebUI/channel) behind the turn, so the channel is the
            // honest catch-all rather than a guess.
            channel: trace::TraceChannel::Other,
            messages,
            policy: &policy,
            max_turns: CAPTURE_MAX_TURNS,
            // Reborn thread transcripts carry no structured outcome payload;
            // the lifecycle event's terminal status is authoritative.
            outcome_override: task_failed.then_some(trace::TaskSuccess::Failure),
        })
        .await;
    match outcome {
        Ok(TraceClientAutonomousCaptureOutcome::Submit(envelope)) => {
            let trace_scope = TraceClientScope::user(scope.to_string());
            if let Err(error) = TraceClientHost.queue_envelope_for_scope(&trace_scope, &envelope) {
                tracing::debug!(%error, %scope_ref, "Reborn trace capture failed to queue envelope");
                return;
            }
            if let Err(error) = TraceClientHost
                .flush_scope_queue(&trace_scope, CAPTURE_FLUSH_LIMIT)
                .await
            {
                tracing::debug!(%error, %scope_ref, "Reborn trace queue flush failed; worker retries");
            }
        }
        Ok(TraceClientAutonomousCaptureOutcome::Held {
            kind,
            reason,
            envelope,
        }) => {
            let submission_id = envelope.submission_id;
            // Only manual-review holds (e.g. High residual-PII-risk) are
            // retained for the user to authorize. Policy/value gates (low
            // score, disallowed tools) are not review-worthy and are dropped
            // as before — just logged for diagnostics.
            if !matches!(kind, trace::TraceQueueHoldKind::ManualReview) {
                tracing::debug!(
                    %submission_id,
                    %reason,
                    %scope_ref,
                    "Reborn trace capture held by policy gate (dropped)"
                );
                return;
            }
            // Retain: queue with a ManualReview hold sidecar so the flush
            // worker skips it until it is authorized.
            let trace_scope = TraceClientScope::user(scope.to_string());
            if let Err(error) =
                TraceClientHost.queue_held_envelope_for_scope(&trace_scope, &envelope, &reason)
            {
                tracing::debug!(%error, %scope_ref, "Reborn trace capture failed to retain held envelope");
                return;
            }
            tracing::debug!(
                %submission_id,
                %reason,
                %scope_ref,
                "Reborn trace capture held for manual review (retained)"
            );
        }
        Ok(TraceClientAutonomousCaptureOutcome::Skipped) => {}
        Err(error) => {
            tracing::debug!(%error, %scope_ref, "Reborn trace capture failed to build envelope");
        }
    }
}

/// Handle for the periodic queue-flush worker.
pub struct TraceQueueFlushWorkerHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

impl TraceQueueFlushWorkerHandle {
    pub async fn shutdown(self) {
        self.cancel.cancel();
        if let Err(error) = self.handle.await {
            tracing::debug!(%error, "Reborn trace queue flush worker did not shut down cleanly");
        }
    }
}

/// Periodic queue flush, mirroring v1's 300s worker: retries envelopes whose
/// immediate flush failed (network blips, endpoint downtime) for every scope
/// observed since boot.
pub fn spawn_trace_queue_flush_worker(
    observed_scopes: ObservedTraceScopes,
) -> TraceQueueFlushWorkerHandle {
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TRACE_QUEUE_WORKER_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick fires immediately; consume it so the first flush
        // happens one full interval after boot.
        interval.tick().await;
        loop {
            tokio::select! {
                _ = worker_cancel.cancelled() => break,
                _ = interval.tick() => {}
            }
            let scopes: Vec<String> = {
                let scopes = match observed_scopes.lock() {
                    Ok(scopes) => scopes,
                    Err(poisoned) => poisoned.into_inner(),
                };
                scopes.iter().cloned().collect()
            };
            if scopes.is_empty() {
                continue;
            }
            if let Err(error) = TraceClientHost
                .flush_queue_worker_tick(scopes.clone(), TRACE_QUEUE_WORKER_FLUSH_LIMIT)
                .await
            {
                tracing::debug!(%error, "Reborn trace queue worker tick failed");
            }
            // Prune drained scopes so the observed set stays bounded by actual
            // pending backlog, not by every caller ever seen on this runtime. A
            // scope with no flushable queue entries is dropped; its next turn
            // re-adds it via `record_observed_scope`. Scopes that still hold
            // pending work (e.g. a flush that hit the per-tick limit, or an
            // endpoint that's down) are retained so the next tick retries them.
            //
            // `trace_scope_has_pending_queue` is a synchronous `read_dir` per
            // scope, so it runs on the blocking pool against a *snapshot* of the
            // set rather than under the guard: `record_observed_scope` takes the
            // same lock from the capture path, and a slow or stalled filesystem
            // would otherwise block capture-time scope recording and this
            // runtime worker thread. The set is re-acquired only to apply the
            // result, and scopes recorded during the probe are left alone — they
            // were not in the snapshot, so a concurrent first capture is never
            // pruned by a decision made before it happened.
            let probe_scopes = scopes;
            let drained = match tokio::task::spawn_blocking(move || {
                probe_scopes
                    .into_iter()
                    .filter(|scope| !trace::trace_scope_has_pending_queue(scope.as_str()))
                    .collect::<Vec<String>>()
            })
            .await
            {
                Ok(drained) => drained,
                Err(error) => {
                    tracing::debug!(%error, "Reborn trace queue prune probe failed");
                    continue;
                }
            };
            if !drained.is_empty() {
                let mut observed = match observed_scopes.lock() {
                    Ok(observed) => observed,
                    Err(poisoned) => poisoned.into_inner(),
                };
                for scope in drained {
                    observed.remove(&scope);
                }
            }
        }
    });
    TraceQueueFlushWorkerHandle { cancel, handle }
}
