//! The generic outbound delivery coordinator (extension-runtime §5.4,
//! OUT-1..7).
//!
//! Sending a message decomposes into two halves: **semantics and
//! reliability** (target resolution, authorization, attempt persistence,
//! retry, crash recovery — identical for every channel, owned here,
//! once) and **vendor mechanics** (rendering, splitting, API selection,
//! error mapping — owned by each extension's
//! [`ChannelAdapter::deliver`](ironclaw_extension_contracts::channel_adapter::ChannelAdapter)).
//!
//! Rules this module owns:
//! - Every user-visible channel output is a semantic [`DeliveryIntent`];
//!   emitters never know what channel the user is on (OUT-1).
//! - An attempt is persisted (`Prepared`→`Sending`) **before** any vendor
//!   egress (OUT-3); the coordinator is the sole delivery-state writer —
//!   adapters get no store and cannot mark anything delivered (OUT-4).
//! - Target/channel/context resolution and attachment materialization run as a
//!   read-only preflight while the attempt remains `Prepared`. The coordinator
//!   claims `Prepared -> Sending` immediately before calling the adapter, so
//!   transient preflight failures remain safely retryable while crash recovery
//!   still treats every `Sending` attempt as possibly delivered (OUT-6).
//! - Once any part of a multipart delivery is sent, a later retryable part
//!   failure is terminal — a whole-envelope retry would duplicate the parts
//!   the vendor already accepted (OUT-7).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{
    ExternalConversationRef, OutboundEnvelope, OutboundPart, OutboundTarget, PartDeliveryOutcome,
};
use async_trait::async_trait;
use ironclaw_attachments::DEFAULT_ATTACHMENT_BUDGETS;
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::path::ScopedPath;
use ironclaw_host_api::product_adapter::AdapterInstallationId;
use ironclaw_outbound::{
    ClaimDeliveryAttemptForSendOutcome, ClaimDeliveryAttemptForSendRequest,
    CommunicationPreferenceRepository, DeliveryFailureKind, FailPreparedDeliveryAttemptOutcome,
    FailPreparedDeliveryAttemptRequest, OutboundDeliveryAttempt, OutboundDeliveryDecision,
    OutboundDeliveryStatus, OutboundPolicyService, OutboundPushCandidate, OutboundPushKind,
    OutboundStateStorePort, PrepareCommunicationDeliveryRequest, RecoverInterruptedDeliveryRequest,
    ReplyAttachmentIntent, UpdateDeliveryStatusRequest, ValidatedReplyTargetBinding,
};
use ironclaw_product_contracts::delivery::{
    ChannelDeliveryResolver, DeliveryReplyContextSource, ResolvedChannelDelivery,
};
use ironclaw_threads::{AttachmentRef, ThreadScope};
use ironclaw_turns::{TurnRunId, TurnScope};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::debug;

use crate::ProductSurfaceFailure;
use crate::outbound_delivery::{
    ProductOutboundTargetResolver, VerifiedProductOutboundTargetMetadata,
};
use crate::{ProjectFilesystemReader, ProjectFsEntryKind, ProjectFsError};

/// The semantic intents (§5.4). Emitters express *what* is being
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
    /// Immediate result or user-correctable rejection of a product command.
    CommandFeedback,
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
            Self::FinalReply | Self::GatePrompt | Self::AuthPrompt | Self::TriggeredDelivery
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
            Self::CommandFeedback => "command-feedback",
            Self::Working => "working",
            Self::Cleanup => "cleanup",
            Self::TriggeredDelivery => "triggered-delivery",
        }
    }
}

/// A no-context source for channels/tests without stored contexts.
pub struct NoReplyContext;

#[async_trait]
impl DeliveryReplyContextSource for NoReplyContext {
    async fn reply_context(
        &self,
        _: &ExtensionId,
        _: &AdapterInstallationId,
        _: &str,
    ) -> Option<Vec<u8>> {
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
    /// Ordered durable references from the finalized assistant message.
    /// Bytes are loaded only after policy authorization and immediately before
    /// adapter dispatch.
    pub attachments: Vec<AttachmentRef>,
    /// Optional vendor thread anchor (e.g. a thread timestamp).
    pub thread_anchor: Option<String>,
    /// AuthPrompt-style payloads must never land in shared conversations.
    pub require_direct_message_target: bool,
    /// The extension whose channel carries this delivery.
    pub extension_id: &'a str,
    /// Canonical project-filesystem authority scope for resolving transient
    /// `/workspace/...` references in final/triggered assistant text.
    pub thread_scope: &'a ThreadScope,
}

struct AuthorizedDeliveryTarget {
    binding: ValidatedReplyTargetBinding,
    require_direct_message: bool,
    /// Optional threading anchor within the resolved target conversation.
    thread_anchor: Option<String>,
}

/// Inputs for resolving transient `/workspace/...` references in final or
/// triggered assistant text into materialized [`OutboundPart::File`] parts.
struct WorkspaceMaterialization<'a> {
    intent: DeliveryIntent,
    project_filesystem: &'a dyn ProjectFilesystemReader,
    thread_scope: &'a ThreadScope,
    attachments: Vec<AttachmentRef>,
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
    /// The durable delivery fact was already confirmed `Delivered`. No vendor
    /// egress occurred for this replay.
    DuplicateSuppressed {
        delivery_id: ironclaw_outbound::OutboundDeliveryId,
        /// Resolved conversation when the coordinator reached that boundary
        /// before observing the authoritative delivered row. This confirms a
        /// route, but does not fabricate a vendor message reference.
        conversation: Option<ExternalConversationRef>,
    },
    /// Another caller already advanced this durable delivery, but the
    /// authoritative state does not prove successful vendor delivery.
    ExistingDeliveryUnconfirmed {
        status: OutboundDeliveryStatus,
        failure_kind: Option<DeliveryFailureKind>,
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

enum ResolvedChannelContextOutcome {
    Resolved {
        channel: ResolvedChannelDelivery,
        reply_context: Option<Vec<u8>>,
    },
    ExistingDelivery(Box<CoordinatedDeliveryOutcome>),
}

/// Coordinator-level failures raised before or around the adapter call.
#[derive(Debug, Error)]
pub enum CoordinatedDeliveryError {
    #[error("outbound policy failed: {0}")]
    Outbound(#[from] ironclaw_outbound::OutboundError),
    #[error("product workflow failed: {0}")]
    Workflow(#[from] ProductSurfaceFailure),
    #[error("no active channel for extension `{extension_id}`")]
    ChannelUnavailable { extension_id: String },
    #[error("intent {intent:?} does not belong to this delivery path")]
    IntentClassMismatch { intent: DeliveryIntent },
    #[error("notice request is invalid: {reason}")]
    InvalidNotice { reason: String },
    #[error("workspace attachment could not be read")]
    WorkspaceAttachmentRead(#[source] ProjectFsError),
    #[error("workspace attachments exceed the delivery budget")]
    WorkspaceAttachmentBudgetExceeded,
    #[error("workspace attachment reference is invalid: {reason}")]
    WorkspaceAttachmentRefInvalid { reason: &'static str },
    #[error("caller-supplied materialized workspace attachments are not accepted")]
    PreMaterializedWorkspaceAttachment,
}

fn workspace_materialization_failure_kind(error: &CoordinatedDeliveryError) -> DeliveryFailureKind {
    match error {
        CoordinatedDeliveryError::WorkspaceAttachmentRead(ProjectFsError::Unavailable) => {
            DeliveryFailureKind::TransportUnavailable
        }
        _ => DeliveryFailureKind::Rejected,
    }
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
    store: Arc<dyn OutboundStateStorePort>,
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
        store: Arc<dyn OutboundStateStorePort>,
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
    /// held the durable egress claim when its coordinator stopped. The claim
    /// immediately precedes adapter delivery, so recovery still cannot tell
    /// whether the vendor was never contacted or may have accepted the
    /// message. Mark each `Unknown`; never blindly resend. A per-attempt
    /// recovery failure does not abandon the captured snapshot: recovery
    /// continues, then returns the first typed store error after all remaining
    /// attempts have been guarded.
    pub async fn recover_interrupted_deliveries(
        &self,
        scope: ironclaw_turns::TurnScope,
    ) -> Result<usize, ironclaw_outbound::OutboundError> {
        let attempts = self.store.list_delivery_attempts(scope.clone()).await?;
        let mut recovered = 0usize;
        let mut first_error = None;
        for attempt in attempts {
            if attempt.status != OutboundDeliveryStatus::Sending {
                continue;
            }
            match self
                .store
                .recover_interrupted_delivery_attempt(RecoverInterruptedDeliveryRequest {
                    delivery_id: attempt.delivery_id,
                    scope: scope.clone(),
                })
                .await
            {
                Ok(true) => recovered += 1,
                Ok(false) => {}
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        if recovered > 0 {
            debug!(
                recovered,
                "delivery coordinator: interrupted deliveries marked Unknown (never resent)"
            );
        }
        if let Some(error) = first_error {
            return Err(error);
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
        project_filesystem: &dyn ProjectFilesystemReader,
        request: CoordinatedDeliveryRequest<'_>,
    ) -> Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError> {
        if !request.intent.runs_outbound_policy() {
            return Err(CoordinatedDeliveryError::IntentClassMismatch {
                intent: request.intent,
            });
        }
        reject_caller_supplied_files(&request.parts)?;
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

        self.drive_authorized(
            target_resolver,
            attempt,
            AuthorizedDeliveryTarget {
                binding: target,
                require_direct_message: request.require_direct_message_target,
                thread_anchor: request.thread_anchor,
            },
            request.parts,
            request.extension_id,
            WorkspaceMaterialization {
                intent: request.intent,
                project_filesystem,
                thread_scope: request.thread_scope,
                attachments: request.attachments,
            },
        )
        .await
    }

    /// Deliver one notice-class intent to its source conversation, under the
    /// same persistence and sole-writer rules as the policy path. The attempt
    /// is recorded `Prepared` before the channel resolves and moves to
    /// `Sending` immediately before vendor egress.
    pub async fn deliver_notice(
        &self,
        request: NoticeDeliveryRequest<'_>,
    ) -> Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError> {
        if !request.intent.is_notice_class() {
            return Err(CoordinatedDeliveryError::IntentClassMismatch {
                intent: request.intent,
            });
        }
        reject_caller_supplied_files(&request.parts)?;
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
        extension_id: &str,
        materialization: WorkspaceMaterialization<'_>,
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
                    crate::outbound_delivery::delivery_failure_kind_for_surface_error(&error);
                if !kind.is_permanent_preflight() {
                    return Err(CoordinatedDeliveryError::Workflow(error));
                }
                return match self.fail_prepared(&attempt, kind).await? {
                    FailPreparedDeliveryAttemptOutcome::Settled => {
                        Err(CoordinatedDeliveryError::Workflow(error))
                    }
                    FailPreparedDeliveryAttemptOutcome::Existing(existing) => {
                        Ok(Self::outcome_for_existing_delivery(*existing, None))
                    }
                };
            }
        };

        // Resolve the generation-pinned adapter and stored reply context
        // before touching workspace bytes. A missing channel or failed
        // context lookup must not cause file materialization as a side effect.
        let (channel, reply_context) = match self
            .resolve_channel_context(&attempt, extension_id, &metadata.external_conversation_ref)
            .await?
        {
            ResolvedChannelContextOutcome::Resolved {
                channel,
                reply_context,
            } => (channel, reply_context),
            ResolvedChannelContextOutcome::ExistingDelivery(outcome) => return Ok(*outcome),
        };

        let parts = match materialize_workspace_file_parts(materialization, parts).await {
            Ok(parts) => parts,
            Err(error) => {
                let failure_kind = workspace_materialization_failure_kind(&error);
                if !failure_kind.is_permanent_preflight() {
                    return Err(error);
                }
                return match self.fail_prepared(&attempt, failure_kind).await? {
                    FailPreparedDeliveryAttemptOutcome::Settled => Err(error),
                    FailPreparedDeliveryAttemptOutcome::Existing(existing) => {
                        Ok(Self::outcome_for_existing_delivery(
                            *existing,
                            Some(metadata.external_conversation_ref.clone()),
                        ))
                    }
                };
            }
        };

        self.drive_prepared(
            attempt,
            channel,
            metadata.external_conversation_ref,
            target.thread_anchor,
            parts,
            reply_context,
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
        let (channel, reply_context) = match self
            .resolve_channel_context(&attempt, extension_id, &conversation)
            .await?
        {
            ResolvedChannelContextOutcome::Resolved {
                channel,
                reply_context,
            } => (channel, reply_context),
            ResolvedChannelContextOutcome::ExistingDelivery(outcome) => return Ok(*outcome),
        };
        self.drive_prepared(
            attempt,
            channel,
            conversation,
            thread_anchor,
            parts,
            reply_context,
        )
        .await
    }

    async fn resolve_channel_context(
        &self,
        attempt: &OutboundDeliveryAttempt,
        extension_id: &str,
        conversation: &ExternalConversationRef,
    ) -> Result<ResolvedChannelContextOutcome, CoordinatedDeliveryError> {
        // Resolve the channel from ONE snapshot read (generation-pinned).
        let Some(channel) = self.resolver.resolve_channel_delivery(extension_id) else {
            // The resolver currently has no typed permanent/transient
            // taxonomy. Preserve the existing fail-closed, no-retry behavior
            // and caller error, while recording only the sanitized permanent
            // `Rejected` kind accepted by the preflight-settlement contract.
            return match self
                .fail_prepared(attempt, DeliveryFailureKind::Rejected)
                .await?
            {
                FailPreparedDeliveryAttemptOutcome::Settled => {
                    Err(CoordinatedDeliveryError::ChannelUnavailable {
                        extension_id: extension_id.to_string(),
                    })
                }
                FailPreparedDeliveryAttemptOutcome::Existing(existing) => {
                    Ok(ResolvedChannelContextOutcome::ExistingDelivery(Box::new(
                        Self::outcome_for_existing_delivery(*existing, Some(conversation.clone())),
                    )))
                }
            };
        };

        // Stored reply context for source-route replies (ING-11).
        let reply_context = self
            .reply_context
            .reply_context(
                &channel.extension_id,
                &channel.installation_id,
                &conversation.conversation_fingerprint(),
            )
            .await;

        Ok(ResolvedChannelContextOutcome::Resolved {
            channel,
            reply_context,
        })
    }

    async fn drive_prepared(
        &self,
        attempt: OutboundDeliveryAttempt,
        channel: ResolvedChannelDelivery,
        conversation: ExternalConversationRef,
        thread_anchor: Option<String>,
        parts: Vec<OutboundPart>,
        reply_context: Option<Vec<u8>>,
    ) -> Result<CoordinatedDeliveryOutcome, CoordinatedDeliveryError> {
        let envelope = OutboundEnvelope {
            extension_id: channel.extension_id.as_str().to_string(),
            installation_id: channel.installation_id.as_str().to_string(),
            delivery_attempt_id: attempt.delivery_id.to_string(),
            target: OutboundTarget {
                conversation: conversation.clone(),
                thread_anchor,
            },
            parts,
            reply_context,
        };

        match self
            .store
            .claim_delivery_attempt_for_send(ClaimDeliveryAttemptForSendRequest {
                delivery_id: attempt.delivery_id,
                scope: attempt.scope.clone(),
            })
            .await?
        {
            ClaimDeliveryAttemptForSendOutcome::Claimed => {}
            ClaimDeliveryAttemptForSendOutcome::Existing(existing) => {
                return Ok(Self::outcome_for_existing_delivery(
                    *existing,
                    Some(conversation),
                ));
            }
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

    async fn fail_prepared(
        &self,
        attempt: &OutboundDeliveryAttempt,
        failure_kind: DeliveryFailureKind,
    ) -> Result<FailPreparedDeliveryAttemptOutcome, ironclaw_outbound::OutboundError> {
        self.store
            .fail_prepared_delivery_attempt(FailPreparedDeliveryAttemptRequest {
                delivery_id: attempt.delivery_id,
                scope: attempt.scope.clone(),
                updated_at: chrono::Utc::now(),
                failure_kind,
            })
            .await
    }

    fn duplicate_suppressed(
        attempt: &OutboundDeliveryAttempt,
        conversation: Option<ExternalConversationRef>,
    ) -> CoordinatedDeliveryOutcome {
        CoordinatedDeliveryOutcome::DuplicateSuppressed {
            delivery_id: attempt.delivery_id,
            conversation,
        }
    }

    fn outcome_for_existing_delivery(
        attempt: OutboundDeliveryAttempt,
        conversation: Option<ExternalConversationRef>,
    ) -> CoordinatedDeliveryOutcome {
        if attempt.status == OutboundDeliveryStatus::Delivered {
            Self::duplicate_suppressed(&attempt, conversation)
        } else {
            CoordinatedDeliveryOutcome::ExistingDeliveryUnconfirmed {
                status: attempt.status,
                failure_kind: attempt.failure_kind,
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

async fn materialize_workspace_file_parts(
    materialization: WorkspaceMaterialization<'_>,
    mut parts: Vec<OutboundPart>,
) -> Result<Vec<OutboundPart>, CoordinatedDeliveryError> {
    let WorkspaceMaterialization {
        intent,
        project_filesystem,
        thread_scope,
        attachments,
    } = materialization;
    reject_caller_supplied_files(&parts)?;
    if !matches!(
        intent,
        DeliveryIntent::FinalReply | DeliveryIntent::TriggeredDelivery
    ) {
        return if attachments.is_empty() {
            Ok(parts)
        } else {
            Err(CoordinatedDeliveryError::WorkspaceAttachmentRefInvalid {
                reason: "delivery intent does not accept attachments",
            })
        };
    }

    if attachments.is_empty() {
        return Ok(parts);
    }
    if attachments.len() > DEFAULT_ATTACHMENT_BUDGETS.max_count {
        return Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded);
    }

    let mut refs = Vec::with_capacity(attachments.len());
    let mut seen_ids = HashSet::with_capacity(attachments.len());
    let mut seen_paths = HashSet::with_capacity(attachments.len());
    for attachment in attachments {
        if !seen_ids.insert(attachment.id.clone()) {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentRefInvalid {
                reason: "duplicate attachment id",
            });
        }
        let path = attachment
            .storage_key
            .as_deref()
            .ok_or(CoordinatedDeliveryError::WorkspaceAttachmentRefInvalid {
                reason: "missing storage key",
            })
            .and_then(|path| {
                ScopedPath::new(path).map_err(|_| {
                    CoordinatedDeliveryError::WorkspaceAttachmentRefInvalid {
                        reason: "malformed storage key",
                    }
                })
            })?;
        if !seen_paths.insert(path.clone()) {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentRefInvalid {
                reason: "duplicate storage key",
            });
        }
        let intent = ReplyAttachmentIntent {
            path,
            filename: attachment.filename.ok_or(
                CoordinatedDeliveryError::WorkspaceAttachmentRefInvalid {
                    reason: "missing filename",
                },
            )?,
            mime_type: attachment.mime_type,
            size_bytes: attachment.size_bytes.ok_or(
                CoordinatedDeliveryError::WorkspaceAttachmentRefInvalid {
                    reason: "missing size",
                },
            )?,
        };
        intent.validate().map_err(|error| match error {
            ironclaw_outbound::OutboundError::ReplyAttachmentIntentLimitExceeded => {
                CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded
            }
            _ => CoordinatedDeliveryError::WorkspaceAttachmentRefInvalid {
                reason: "invalid attachment metadata",
            },
        })?;
        refs.push(intent);
    }
    // Preflight every file before reading any bytes. The production delivery
    // reader is independently capped at max_file_bytes, while this metadata
    // pass avoids even bounded allocations when the declared set already
    // violates per-file or aggregate budgets.
    let mut declared_total_bytes = 0u64;
    for attachment in &refs {
        let stat = project_filesystem
            .stat(thread_scope, attachment.path.as_str())
            .await
            .map_err(CoordinatedDeliveryError::WorkspaceAttachmentRead)?;
        if stat.path != attachment.path.as_str() {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentRead(
                ProjectFsError::Internal,
            ));
        }
        if stat.kind != ProjectFsEntryKind::File {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentRead(
                ProjectFsError::NotAFile,
            ));
        }
        if stat.size_bytes != attachment.size_bytes
            || stat.size_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes as u64
        {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded);
        }
        declared_total_bytes = declared_total_bytes
            .checked_add(stat.size_bytes)
            .ok_or(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded)?;
        if declared_total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes as u64 {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded);
        }
    }

    let mut total_bytes = 0usize;
    let mut files = Vec::with_capacity(refs.len());
    for attachment in refs {
        let mut file = project_filesystem
            .read_file(thread_scope, attachment.path.as_str())
            .await
            .map_err(CoordinatedDeliveryError::WorkspaceAttachmentRead)?;
        if file.path != attachment.path {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentRead(
                ProjectFsError::Internal,
            ));
        }
        let file_bytes = file.bytes.len();
        if u64::try_from(file_bytes).ok() != Some(attachment.size_bytes)
            || file_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes
        {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded);
        }
        total_bytes = total_bytes
            .checked_add(file_bytes)
            .ok_or(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded)?;
        if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded);
        }
        file.filename = Some(attachment.filename);
        file.mime_type = attachment.mime_type;
        files.push(OutboundPart::File(file));
    }
    parts.extend(files);
    validate_final_workspace_files(&parts)?;
    Ok(parts)
}

fn reject_caller_supplied_files(parts: &[OutboundPart]) -> Result<(), CoordinatedDeliveryError> {
    if parts
        .iter()
        .any(|part| matches!(part, OutboundPart::File(_)))
    {
        return Err(CoordinatedDeliveryError::PreMaterializedWorkspaceAttachment);
    }
    Ok(())
}

fn validate_final_workspace_files(parts: &[OutboundPart]) -> Result<(), CoordinatedDeliveryError> {
    let mut count = 0usize;
    let mut total_bytes = 0usize;
    for file in parts.iter().filter_map(|part| match part {
        OutboundPart::File(file) => Some(file),
        OutboundPart::Text(_) | OutboundPart::AuthPrompt { .. } | OutboundPart::Retract { .. } => {
            None
        }
    }) {
        count = count
            .checked_add(1)
            .ok_or(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded)?;
        if count > DEFAULT_ATTACHMENT_BUDGETS.max_count
            || file.bytes.len() > DEFAULT_ATTACHMENT_BUDGETS.max_file_bytes
        {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded);
        }
        total_bytes = total_bytes
            .checked_add(file.bytes.len())
            .ok_or(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded)?;
        if total_bytes > DEFAULT_ATTACHMENT_BUDGETS.max_total_bytes {
            return Err(CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded);
        }
    }
    Ok(())
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
