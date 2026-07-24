//! The generic outbound delivery coordinator (extension-runtime §5.4,
//! OUT-1..7).
//!
//! Sending a message decomposes into two halves: **semantics and
//! reliability** (target resolution, authorization, attempt persistence,
//! retry, crash recovery — identical for every channel, owned here,
//! once) and **vendor mechanics** (rendering, splitting, API selection,
//! error mapping — owned by each extension's
//! [`ChannelAdapter::deliver`](crate::ChannelAdapter)).
//!
//! Rules this module owns:
//! - Every user-visible channel output is a semantic [`DeliveryIntent`];
//!   emitters never know what channel the user is on (OUT-1).
//! - An attempt is persisted (`Prepared`→`Sending`) **before** any vendor
//!   egress (OUT-3); the coordinator is the sole delivery-state writer —
//!   adapters get no store and cannot mark anything delivered (OUT-4).
//! - A crash after possible vendor success leaves `Sending`; recovery marks
//!   it `Unknown` and never blindly resends (OUT-6).
//! - Once any part of a multipart delivery is sent, a later retryable part
//!   failure is terminal — a whole-envelope retry would duplicate the parts
//!   the vendor already accepted (OUT-7).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{
    ChannelAdapter, ExternalConversationRef, OutboundEnvelope, OutboundPart, OutboundTarget,
    PartDeliveryOutcome,
};
use async_trait::async_trait;
use ironclaw_host_api::RestrictedEgress;
use ironclaw_outbound::{
    ClaimDeliveryAttemptForSendRequest, CommunicationPreferenceRepository, DeliveryFailureKind,
    OutboundDeliveryAttempt, OutboundDeliveryDecision, OutboundDeliveryStatus,
    OutboundPolicyService, OutboundPushCandidate, OutboundPushKind, OutboundStateStore,
    PrepareCommunicationDeliveryRequest, RecoverInterruptedDeliveryRequest,
    UpdateDeliveryStatusRequest, ValidatedReplyTargetBinding,
};
use ironclaw_turns::{TurnRunId, TurnScope};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::debug;

use crate::ProductWorkflowError;
use crate::outbound_delivery::{
    ProductOutboundTargetResolver, VerifiedProductOutboundTargetMetadata,
};

/// Semantic delivery intents (§5.4). Emitters express *what* is being
/// communicated; the coordinator decides targeting, persistence, and retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryIntent {
    /// The assistant's final reply for a run.
    FinalReply,
    /// An approval gate needs the user.
    GatePrompt,
    /// An auth gate needs the user (authorization URL — DM-only).
    AuthPrompt,
    /// The run failed, timed out, or a message was dropped.
    FailureNotice,
    /// The user must connect an account before the channel works.
    ConnectRequired,
    /// Pairing or account-connection status feedback.
    ConnectionStatus,
    /// A transient "working on it" indicator.
    Working,
    /// A run-scoped working indicator delivered only after the sealed reply
    /// target is revalidated.
    RunProgress,
    /// A run-scoped failure/auth-unavailable notice delivered only after the
    /// sealed reply target is revalidated.
    RunFailureNotice,
    /// Remove an earlier delivery (e.g. delete the working indicator).
    Cleanup,
    /// A routine/heartbeat-initiated delivery to a preference target.
    TriggeredDelivery,
}

impl DeliveryIntent {
    /// Policy-class intents run the outbound-policy pipeline (validated
    /// reply-target bindings + preference targets). Notice-class intents are
    /// source-routed system notices on the originating conversation.
    pub fn runs_outbound_policy(self) -> bool {
        matches!(
            self,
            Self::FinalReply
                | Self::GatePrompt
                | Self::AuthPrompt
                | Self::RunProgress
                | Self::RunFailureNotice
                | Self::TriggeredDelivery
        )
    }

    /// Notice-class intents (`deliver_notice`): still persisted and driven by
    /// the coordinator, but targeted at the originating conversation instead
    /// of a policy-resolved binding.
    pub fn is_notice_class(self) -> bool {
        !self.runs_outbound_policy()
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::FinalReply => "final-reply",
            Self::GatePrompt => "gate-prompt",
            Self::AuthPrompt => "auth-prompt",
            Self::FailureNotice => "failure-notice",
            Self::ConnectRequired => "connect-required",
            Self::ConnectionStatus => "connection-status",
            Self::Working => "working",
            Self::RunProgress => "run-progress",
            Self::RunFailureNotice => "run-failure-notice",
            Self::Cleanup => "cleanup",
            Self::TriggeredDelivery => "triggered-delivery",
        }
    }
}

/// One channel's delivery half, resolved from a single active-snapshot read
/// (generation-pinned: an in-flight delivery keeps these `Arc`s across an
/// upgrade).
#[derive(Clone)]
pub struct ResolvedChannelDelivery {
    pub extension_id: String,
    pub installation_id: String,
    pub adapter: Arc<dyn ChannelAdapter>,
    /// Policy-enforced egress built from the same snapshot read.
    pub egress: Arc<dyn RestrictedEgress>,
}

/// Resolver port: the coordinator's view of the active extension set.
/// Defined here (the coordinator is the consumer); implemented over the
/// extension host's snapshot by composition.
pub trait ChannelDeliveryResolver: Send + Sync {
    fn resolve_channel_delivery(&self, extension_id: &str) -> Option<ResolvedChannelDelivery>;
}

/// Read half of the host-side `reply_context` storage (ING-11): the opaque
/// vendor context an adapter attached to the originating inbound message,
/// handed back at delivery time.
#[async_trait]
pub trait DeliveryReplyContextSource: Send + Sync {
    async fn reply_context(
        &self,
        extension_id: &str,
        installation_id: &str,
        conversation_fingerprint: &str,
    ) -> Option<Vec<u8>>;
}

/// A no-context source for channels/tests without stored contexts.
pub struct NoReplyContext;

#[async_trait]
impl DeliveryReplyContextSource for NoReplyContext {
    async fn reply_context(&self, _: &str, _: &str, _: &str) -> Option<Vec<u8>> {
        None
    }
}

/// One coordinated delivery request: a policy-approved attempt driven
/// through a channel adapter.
pub struct CoordinatedDeliveryRequest<'a> {
    pub intent: DeliveryIntent,
    /// Policy inputs (resolution request, run id, projection ref).
    pub delivery: PrepareCommunicationDeliveryRequest,
    /// Channel-neutral content parts; the adapter owns rendering.
    pub parts: Vec<OutboundPart>,
    /// Optional vendor thread anchor (e.g. a thread timestamp).
    pub thread_anchor: Option<String>,
    /// AuthPrompt-style payloads must never land in shared conversations.
    pub require_direct_message_target: bool,
    /// The extension whose channel carries this delivery.
    pub extension_id: &'a str,
}

struct AuthorizedDeliveryTarget {
    binding: ValidatedReplyTargetBinding,
    require_direct_message: bool,
}

/// One notice-class delivery request (§5.4: `Working`, `Cleanup`,
/// `FailureNotice`, `ConnectRequired`, `ConnectionStatus`): a source-routed
/// system notice on the originating conversation. There is no policy
/// resolution — the target IS the conversation the triggering inbound event
/// arrived on — but the attempt is persisted and driven under the same
/// sole-writer rules.
pub struct NoticeDeliveryRequest<'a> {
    pub intent: DeliveryIntent,
    pub scope: TurnScope,
    pub turn_run_id: Option<TurnRunId>,
    /// The originating conversation (the source route). Requests without a
    /// source conversation cannot be constructed — that is the fail-closed
    /// rule for notices.
    pub conversation: ExternalConversationRef,
    pub thread_anchor: Option<String>,
    pub parts: Vec<OutboundPart>,
    pub extension_id: &'a str,
    /// Audit discriminator recorded in the attempt's projection ref
    /// (e.g. a run id or event id), so repeated notices stay distinguishable.
    pub notice_ref: String,
}

/// Coordinator outcome for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatedDeliveryOutcome {
    /// No target resolved (policy said no delivery) — nothing sent.
    NoDelivery,
    /// Policy rejected the candidate; the attempt records the rejection.
    Rejected { attempt: OutboundDeliveryAttempt },
    /// The same durable delivery fact was already claimed or settled. No
    /// vendor egress occurred for this replay.
    DuplicateSuppressed {
        delivery_id: ironclaw_outbound::OutboundDeliveryId,
    },
    /// The adapter reported every part sent.
    Delivered {
        attempt: OutboundDeliveryAttempt,
        /// The resolved target conversation, so emitters can record follow-up
        /// state (gate routes, cleanup targets) without vendor knowledge.
        conversation: ExternalConversationRef,
        vendor_message_refs: Vec<String>,
    },
    /// Terminal failure (permanent, retries exhausted, or partial-multipart).
    Failed {
        attempt: OutboundDeliveryAttempt,
        failure_kind: DeliveryFailureKind,
    },
}

/// Coordinator-level failures raised before or around the adapter call.
#[derive(Debug, Error)]
pub enum CoordinatedDeliveryError {
    #[error("outbound policy failed: {0}")]
    Outbound(#[from] ironclaw_outbound::OutboundError),
    #[error("product workflow failed: {0}")]
    Workflow(#[from] ProductWorkflowError),
    #[error("no active channel for extension `{extension_id}`")]
    ChannelUnavailable { extension_id: String },
    #[error("intent {intent:?} does not belong to this delivery path")]
    IntentClassMismatch { intent: DeliveryIntent },
    #[error("notice request is invalid: {reason}")]
    InvalidNotice { reason: String },
}

/// Retry policy for retryable per-part outcomes (bounded, jitter-free by
/// default — tests inject zero delays).
#[derive(Debug, Clone)]
pub struct DeliveryRetryPolicy {
    pub max_attempts: u32,
    pub backoff: Duration,
}

impl Default for DeliveryRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff: Duration::from_millis(500),
        }
    }
}

/// The delivery coordinator. Sole writer of delivery state; one instance per
/// composition (§5.4: "no direct product send path").
pub struct DeliveryCoordinator {
    store: Arc<dyn OutboundStateStore>,
    resolver: Arc<dyn ChannelDeliveryResolver>,
    reply_context: Arc<dyn DeliveryReplyContextSource>,
    retry: DeliveryRetryPolicy,
    /// Scopes whose interrupted (`Sending`) attempts from prior lifetimes
    /// have been reconciled this lifetime. The store enumerates attempts per
    /// scope only, so recovery runs lazily before a scope's first delivery.
    recovered_scopes: Mutex<HashSet<TurnScope>>,
}

impl DeliveryCoordinator {
    /// Production construction requires a real store, resolver, and reply
    /// context source — there is deliberately no no-op-sink constructor
    /// (OUT-4): a composition that cannot persist attempts must not deliver.
    pub fn new(
        store: Arc<dyn OutboundStateStore>,
        resolver: Arc<dyn ChannelDeliveryResolver>,
        reply_context: Arc<dyn DeliveryReplyContextSource>,
        retry: DeliveryRetryPolicy,
    ) -> Self {
        Self {
            store,
            resolver,
            reply_context,
            retry,
            recovered_scopes: Mutex::new(HashSet::new()),
        }
    }

    /// Run crash recovery for `scope` exactly once per coordinator lifetime,
    /// before the scope's first delivery. Recovery failures are logged and
    /// do not block the new delivery: the stray attempt stays `Sending` and
    /// the next lifetime reconciles it.
    async fn ensure_scope_recovered(&self, scope: &TurnScope) {
        {
            let mut recovered = self
                .recovered_scopes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !recovered.insert(scope.clone()) {
                return;
            }
        }
        if let Err(error) = self.recover_interrupted_deliveries(scope.clone()).await {
            debug!(
                error = %error,
                "delivery coordinator: lazy interrupted-delivery recovery failed"
            );
        }
    }

    /// Crash recovery (OUT-6): every attempt still `Sending` in this scope
    /// crashed between vendor egress and the result write. Mark each
    /// `Unknown`; never blindly resend.
    pub async fn recover_interrupted_deliveries(
        &self,
        scope: ironclaw_turns::TurnScope,
    ) -> Result<usize, ironclaw_outbound::OutboundError> {
        let attempts = self.store.list_delivery_attempts(scope.clone()).await?;
        let mut recovered = 0usize;
        for attempt in attempts {
            // The list is a point-in-time snapshot; between reading it and
            // acting, another worker may have completed egress and written a
            // terminal result. The guarded store transition re-verifies
            // `Sending` under CAS and no-ops otherwise, so a stale snapshot can
            // never clobber a durable `Delivered`/`Failed` back to `Unknown`.
            if attempt.status != OutboundDeliveryStatus::Sending {
                continue;
            }
            if self
                .store
                .recover_interrupted_delivery_attempt(RecoverInterruptedDeliveryRequest {
                    delivery_id: attempt.delivery_id,
                    scope: scope.clone(),
                })
                .await?
            {
                recovered += 1;
            }
        }
        if recovered > 0 {
            debug!(
                recovered,
                "delivery coordinator: interrupted deliveries marked Unknown (never resent)"
            );
        }
        Ok(recovered)
    }

    /// Deliver one policy-class intent end to end.
    ///
    /// `outbound_policy` stays borrow-based per call (it wraps this
    /// coordinator's store plus the caller's validators); the coordinator
    /// owns everything after the policy decision.
    pub async fn deliver(
        &self,
        outbound_policy: &OutboundPolicyService<'_>,
        communication_preferences: &dyn CommunicationPreferenceRepository,
        target_resolver: &dyn ProductOutboundTargetResolver,
        request: CoordinatedDeliveryRequest<'_>,
    ) -> Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError> {
        if !request.intent.runs_outbound_policy() {
            return Err(CoordinatedDeliveryError::IntentClassMismatch {
                intent: request.intent,
            });
        }
        self.ensure_scope_recovered(&request.delivery.resolution_request.scope)
            .await;

        // 1. Policy: authorize the candidate and persist the attempt.
        let Some(decision) = outbound_policy
            .prepare_communication_delivery_attempt(request.delivery, communication_preferences)
            .await?
        else {
            return Ok(CoordinatedDeliveryOutcome::NoDelivery);
        };
        let (attempt, target) = match decision {
            OutboundDeliveryDecision::Authorized { attempt, target } => (attempt, target),
            OutboundDeliveryDecision::Rejected { attempt } => {
                return Ok(CoordinatedDeliveryOutcome::Rejected { attempt });
            }
        };

        if !self
            .store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id: attempt.delivery_id,
                scope: attempt.scope.clone(),
            })
            .await?
        {
            return Ok(CoordinatedDeliveryOutcome::DuplicateSuppressed {
                delivery_id: attempt.delivery_id,
            });
        }

        self.drive_authorized(
            target_resolver,
            attempt,
            AuthorizedDeliveryTarget {
                binding: target,
                require_direct_message: request.require_direct_message_target,
            },
            request.parts,
            request.thread_anchor,
            request.extension_id,
        )
        .await
    }

    /// Deliver one notice-class intent to its source conversation, under the
    /// same persistence and sole-writer rules as the policy path. The attempt
    /// is recorded `Prepared` before the channel resolves and moves to
    /// `Sending` before any vendor egress.
    pub async fn deliver_notice(
        &self,
        request: NoticeDeliveryRequest<'_>,
    ) -> Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError> {
        if !request.intent.is_notice_class() {
            return Err(CoordinatedDeliveryError::IntentClassMismatch {
                intent: request.intent,
            });
        }
        self.ensure_scope_recovered(&request.scope).await;

        // Persist the attempt before anything else. The synthetic reply
        // target names the source conversation (hashed: fingerprints can
        // exceed the ref bound); `requires_reply_target_revalidation` is
        // false because there is no policy binding to revalidate — the
        // source conversation is the target by construction.
        let target = notice_target_ref(&request.conversation)
            .map_err(|reason| CoordinatedDeliveryError::InvalidNotice { reason })?;
        let projection_ref = ironclaw_outbound::ProjectionUpdateRef::new(format!(
            "system-notice:{}:{}",
            request.intent.as_str(),
            request.notice_ref
        ))
        .map_err(|reason| CoordinatedDeliveryError::InvalidNotice { reason })?;
        let attempt = OutboundDeliveryAttempt {
            delivery_id: ironclaw_outbound::OutboundDeliveryId::new(),
            scope: request.scope.clone(),
            candidate: OutboundPushCandidate {
                tenant_id: request.scope.tenant_id.clone(),
                agent_id: request.scope.agent_id.clone(),
                project_id: request.scope.project_id.clone(),
                thread_id: request.scope.thread_id.clone(),
                turn_run_id: request.turn_run_id,
                target,
                kind: OutboundPushKind::DeliveryStatus,
                projection_ref,
                requires_reply_target_revalidation: false,
            },
            status: OutboundDeliveryStatus::Prepared,
            attempted_at: chrono::Utc::now(),
            failure_kind: None,
        };
        self.store.record_delivery_attempt(attempt.clone()).await?;
        if !self
            .store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id: attempt.delivery_id,
                scope: attempt.scope.clone(),
            })
            .await?
        {
            return Ok(CoordinatedDeliveryOutcome::DuplicateSuppressed {
                delivery_id: attempt.delivery_id,
            });
        }

        self.drive_resolved(
            attempt,
            request.extension_id,
            request.conversation,
            request.thread_anchor,
            request.parts,
        )
        .await
    }

    async fn drive_authorized(
        &self,
        target_resolver: &dyn ProductOutboundTargetResolver,
        attempt: OutboundDeliveryAttempt,
        target: AuthorizedDeliveryTarget,
        parts: Vec<OutboundPart>,
        thread_anchor: Option<String>,
        extension_id: &str,
    ) -> Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError> {
        // 2. Resolve the trusted conversation metadata for the sealed target.
        let metadata: VerifiedProductOutboundTargetMetadata = match target_resolver
            .resolve_product_outbound_target_metadata(
                &target.binding,
                target.require_direct_message,
            )
            .await
        {
            Ok(metadata) => metadata,
            Err(error) => {
                let kind =
                    crate::outbound_delivery::delivery_failure_kind_for_workflow_error(&error);
                self.mark_terminal(&attempt, OutboundDeliveryStatus::Failed, Some(kind))
                    .await;
                return Err(CoordinatedDeliveryError::Workflow(error));
            }
        };

        self.drive_resolved(
            attempt,
            extension_id,
            metadata.external_conversation_ref,
            thread_anchor,
            parts,
        )
        .await
    }

    /// Shared delivery drive: channel resolution (generation-pinned), reply
    /// context, `Sending` persisted before egress (OUT-3), bounded retries,
    /// and the partial-multipart terminal rule (OUT-7).
    async fn drive_resolved(
        &self,
        attempt: OutboundDeliveryAttempt,
        extension_id: &str,
        conversation: ExternalConversationRef,
        thread_anchor: Option<String>,
        parts: Vec<OutboundPart>,
    ) -> Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError> {
        // 3. Resolve the channel from ONE snapshot read (generation-pinned).
        let Some(channel) = self.resolver.resolve_channel_delivery(extension_id) else {
            self.mark_terminal(
                &attempt,
                OutboundDeliveryStatus::Failed,
                Some(DeliveryFailureKind::TransportUnavailable),
            )
            .await;
            return Err(CoordinatedDeliveryError::ChannelUnavailable {
                extension_id: extension_id.to_string(),
            });
        };

        // 4. Stored reply context for source-route replies (ING-11).
        let reply_context = self
            .reply_context
            .reply_context(
                &channel.extension_id,
                &channel.installation_id,
                &conversation.conversation_fingerprint(),
            )
            .await;

        let envelope = OutboundEnvelope {
            extension_id: channel.extension_id.clone(),
            installation_id: channel.installation_id.clone(),
            delivery_attempt_id: attempt.delivery_id.to_string(),
            target: OutboundTarget {
                conversation: conversation.clone(),
                thread_anchor,
            },
            parts,
            reply_context,
        };

        // 5. The caller atomically persisted `Prepared -> Sending` before
        // resolving this envelope, so only its durable claim can reach vendor
        // egress (OUT-3 and replay-safe at-most-once dispatch).

        // 6. Drive the adapter with bounded retries. Once any part has been
        //    sent, a later retryable failure is terminal (OUT-7).
        let mut attempts_used = 0u32;
        loop {
            attempts_used += 1;
            let report = channel
                .adapter
                .deliver(envelope.clone(), channel.egress.as_ref())
                .await;
            match report {
                Ok(report) => {
                    let mut sent_refs = Vec::new();
                    let mut retryable = false;
                    let mut permanent = false;
                    let mut unauthorized = false;
                    for part in &report.parts {
                        match part {
                            PartDeliveryOutcome::Sent { vendor_message_ref } => {
                                if let Some(reference) = vendor_message_ref {
                                    sent_refs.push(reference.clone());
                                }
                            }
                            PartDeliveryOutcome::Retryable { .. } => retryable = true,
                            PartDeliveryOutcome::Permanent { .. } => permanent = true,
                            PartDeliveryOutcome::Unauthorized { .. } => unauthorized = true,
                        }
                    }
                    let any_sent = report
                        .parts
                        .iter()
                        .any(|part| matches!(part, PartDeliveryOutcome::Sent { .. }));
                    let all_sent = report
                        .parts
                        .iter()
                        .all(|part| matches!(part, PartDeliveryOutcome::Sent { .. }));

                    if all_sent && !report.parts.is_empty() {
                        self.mark_terminal(&attempt, OutboundDeliveryStatus::Delivered, None)
                            .await;
                        return Ok(CoordinatedDeliveryOutcome::Delivered {
                            attempt,
                            conversation,
                            vendor_message_refs: sent_refs,
                        });
                    }
                    if unauthorized {
                        let kind = DeliveryFailureKind::AuthorizationRevoked;
                        self.mark_terminal(&attempt, OutboundDeliveryStatus::Failed, Some(kind))
                            .await;
                        return Ok(CoordinatedDeliveryOutcome::Failed {
                            attempt,
                            failure_kind: kind,
                        });
                    }
                    if permanent || (retryable && any_sent) {
                        // Partial multipart (OUT-7): retrying the whole
                        // envelope would duplicate already-accepted parts.
                        let kind = DeliveryFailureKind::Rejected;
                        self.mark_terminal(&attempt, OutboundDeliveryStatus::Failed, Some(kind))
                            .await;
                        return Ok(CoordinatedDeliveryOutcome::Failed {
                            attempt,
                            failure_kind: kind,
                        });
                    }
                    // Fully-retryable report (nothing sent).
                    if attempts_used >= self.retry.max_attempts {
                        let kind = DeliveryFailureKind::TransportUnavailable;
                        self.mark_terminal(&attempt, OutboundDeliveryStatus::Failed, Some(kind))
                            .await;
                        return Ok(CoordinatedDeliveryOutcome::Failed {
                            attempt,
                            failure_kind: kind,
                        });
                    }
                    tokio::time::sleep(self.retry.backoff).await;
                }
                Err(error) => {
                    debug!(
                        extension_id = %channel.extension_id,
                        error = %error,
                        "delivery coordinator: adapter deliver failed"
                    );
                    if attempts_used >= self.retry.max_attempts {
                        let kind = DeliveryFailureKind::TransportUnavailable;
                        self.mark_terminal(&attempt, OutboundDeliveryStatus::Failed, Some(kind))
                            .await;
                        return Ok(CoordinatedDeliveryOutcome::Failed {
                            attempt,
                            failure_kind: kind,
                        });
                    }
                    tokio::time::sleep(self.retry.backoff).await;
                }
            }
        }
    }

    async fn mark_terminal(
        &self,
        attempt: &OutboundDeliveryAttempt,
        status: OutboundDeliveryStatus,
        failure_kind: Option<DeliveryFailureKind>,
    ) {
        if let Err(error) = self
            .store
            .update_delivery_status(UpdateDeliveryStatusRequest {
                delivery_id: attempt.delivery_id,
                scope: attempt.scope.clone(),
                status,
                updated_at: chrono::Utc::now(),
                failure_kind,
            })
            .await
        {
            // silent-ok: terminal-status bookkeeping must not mask the
            // delivery outcome; the attempt stays in its prior durable state
            // and recovery reconciles it.
            debug!(
                delivery_id = %attempt.delivery_id,
                error = %error,
                "delivery coordinator: terminal status write failed"
            );
        }
    }
}

/// Synthetic reply-target ref naming a notice's source conversation. Hashed:
/// conversation fingerprints embed raw ids and can exceed the 256-byte ref
/// bound.
fn notice_target_ref(
    conversation: &ExternalConversationRef,
) -> Result<ironclaw_turns::ReplyTargetBindingRef, String> {
    let digest = Sha256::digest(conversation.conversation_fingerprint().as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    ironclaw_turns::ReplyTargetBindingRef::new(format!("system-notice:{hex}"))
}
