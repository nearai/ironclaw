//! The generic outbound delivery coordinator (extension-runtime §5.4,
//! OUT-1..7).
//!
//! Sending a message decomposes into two halves: **semantics and
//! reliability** (target resolution, authorization, attempt persistence,
//! retry, crash recovery, drain — identical for every channel, owned here,
//! once) and **vendor mechanics** (rendering, splitting, API selection,
//! error mapping — owned by each extension's
//! [`ChannelAdapter::deliver`](ironclaw_product_adapters::ChannelAdapter)).
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

use async_trait::async_trait;
use ironclaw_host_api::RestrictedEgress;
use ironclaw_outbound::{
    CommunicationPreferenceRepository, DeliveryFailureKind, OutboundDeliveryAttempt,
    OutboundDeliveryDecision, OutboundDeliveryStatus, OutboundPolicyService, OutboundStateStore,
    PrepareCommunicationDeliveryRequest, UpdateDeliveryStatusRequest, ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::{
    ChannelAdapter, OutboundEnvelope, OutboundPart, OutboundTarget, PartDeliveryOutcome,
};
use thiserror::Error;
use tracing::debug;

use crate::ProductWorkflowError;
use crate::outbound_delivery::{
    ProductOutboundTargetResolver, VerifiedProductOutboundTargetMetadata,
};

/// The nine semantic intents (§5.4). Emitters express *what* is being
/// communicated; the coordinator decides targeting, persistence, and retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryIntent {
    /// The assistant's final reply for a run.
    FinalReply,
    /// Incremental progress for a running turn.
    Progress,
    /// An approval gate needs the user.
    GatePrompt,
    /// An auth gate needs the user (authorization URL — DM-only).
    AuthPrompt,
    /// The run failed, timed out, or a message was dropped.
    FailureNotice,
    /// The user must connect an account before the channel works.
    ConnectRequired,
    /// A transient "working on it" indicator.
    Working,
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
                | Self::Progress
                | Self::GatePrompt
                | Self::AuthPrompt
                | Self::TriggeredDelivery
        )
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

/// Coordinator outcome for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatedDeliveryOutcome {
    /// No target resolved (policy said no delivery) — nothing sent.
    NoDelivery,
    /// Policy rejected the candidate; the attempt records the rejection.
    Rejected { attempt: OutboundDeliveryAttempt },
    /// The adapter reported every part sent.
    Delivered {
        attempt: OutboundDeliveryAttempt,
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
    #[error("delivery is already in flight for this attempt")]
    AlreadyInFlight,
    #[error("the coordinator is draining; new deliveries are rejected")]
    Draining,
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
    /// Per-delivery single-flight: a delivery id enters once.
    in_flight: Mutex<HashSet<ironclaw_outbound::OutboundDeliveryId>>,
    draining: std::sync::atomic::AtomicBool,
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
            in_flight: Mutex::new(HashSet::new()),
            draining: std::sync::atomic::AtomicBool::new(false),
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
            if attempt.status != OutboundDeliveryStatus::Sending {
                continue;
            }
            self.store
                .update_delivery_status(UpdateDeliveryStatusRequest {
                    delivery_id: attempt.delivery_id,
                    scope: scope.clone(),
                    status: OutboundDeliveryStatus::Unknown,
                    updated_at: chrono::Utc::now(),
                    failure_kind: None,
                })
                .await?;
            recovered += 1;
        }
        if recovered > 0 {
            debug!(
                recovered,
                "delivery coordinator: interrupted deliveries marked Unknown (never resent)"
            );
        }
        Ok(recovered)
    }

    /// Stop accepting new deliveries; in-flight sends finish on their own
    /// futures (the caller awaits them).
    pub fn begin_drain(&self) {
        self.draining
            .store(true, std::sync::atomic::Ordering::SeqCst);
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
        if self.draining.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(CoordinatedDeliveryError::Draining);
        }

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

        // Single-flight per delivery id.
        let delivery_id = attempt.delivery_id;
        {
            let mut in_flight = self
                .in_flight
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !in_flight.insert(delivery_id) {
                return Err(CoordinatedDeliveryError::AlreadyInFlight);
            }
        }
        let result = self
            .drive_authorized(
                outbound_policy,
                target_resolver,
                request.intent,
                attempt,
                target,
                request.parts,
                request.thread_anchor,
                request.require_direct_message_target,
                request.extension_id,
            )
            .await;
        self.in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&delivery_id);
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn drive_authorized(
        &self,
        outbound_policy: &OutboundPolicyService<'_>,
        target_resolver: &dyn ProductOutboundTargetResolver,
        intent: DeliveryIntent,
        attempt: OutboundDeliveryAttempt,
        target: ValidatedReplyTargetBinding,
        parts: Vec<OutboundPart>,
        thread_anchor: Option<String>,
        require_direct_message: bool,
        extension_id: &str,
    ) -> Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError> {
        let _ = intent;
        // 2. Resolve the trusted conversation metadata for the sealed target.
        let metadata: VerifiedProductOutboundTargetMetadata = match target_resolver
            .resolve_product_outbound_target_metadata(&target, require_direct_message)
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
                &metadata
                    .external_conversation_ref
                    .conversation_fingerprint(),
            )
            .await;

        let envelope = OutboundEnvelope {
            extension_id: channel.extension_id.clone(),
            installation_id: channel.installation_id.clone(),
            delivery_attempt_id: attempt.delivery_id.to_string(),
            target: OutboundTarget {
                conversation: metadata.external_conversation_ref.clone(),
                thread_anchor,
            },
            parts,
            reply_context,
        };

        // 5. Persist Sending BEFORE any vendor egress (OUT-3).
        if let Err(error) = outbound_policy
            .update_delivery_status(UpdateDeliveryStatusRequest {
                delivery_id: attempt.delivery_id,
                scope: attempt.scope.clone(),
                status: OutboundDeliveryStatus::Sending,
                updated_at: chrono::Utc::now(),
                failure_kind: None,
            })
            .await
        {
            return Err(CoordinatedDeliveryError::Outbound(error));
        }

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
