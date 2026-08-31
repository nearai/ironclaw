//! The coordinator's reply-publication operations (design doc §5).
//!
//! Reply publication keeps its state on the outbound delivery attempt
//! aggregate, and the coordinator stays that aggregate's sole writer: every
//! guarded store operation is reached through the methods here, so the
//! publication worker never holds the store. The methods are thin on purpose
//! — the guard semantics (fence, lease, monotonic revisions, one-way
//! settlement) live in `ironclaw_outbound` and are proved by its conformance
//! suite; what the coordinator adds is the attempt row itself (one per run and
//! exact target, `Prepared`, keyed like every other projection fact) and the
//! channel resolution reply publication reads through it.

use std::time::Duration;

use chrono::Utc;
use ironclaw_extension_contracts::external::ExternalConversationRef;
use ironclaw_host_api::turn::{ReplyTargetBindingRef, TurnRunId, TurnScope};
use ironclaw_outbound::{
    AdvanceReplyPublicationRequest, ClaimReplyPublicationLeaseRequest, OpenReplyPublicationRequest,
    OutboundDeliveryAttempt, OutboundDeliveryId, OutboundDeliveryStatus, OutboundPushCandidate,
    OutboundPushKind, ProjectionUpdateRef, PublisherId, ReleaseReplyPublicationLeaseRequest,
    ReplyPublicationClaim, ReplyPublicationRecord, ReplyPublicationSettlement,
    ReplyPublicationTarget, ReplyPublicationTargetDescriptor, ReplyPublicationTargetKey,
    SettleReplyPublicationRequest,
};
use ironclaw_product_contracts::delivery::ResolvedChannelDelivery;

use super::{CoordinatedDeliveryError, DeliveryCoordinator};

/// Everything needed to open one publication: the run, the authorized
/// reply-target binding the attempt is keyed on, the exact target key, and
/// the descriptor any node needs to address the target again.
#[derive(Debug, Clone)]
pub(crate) struct OpenReplyPublication {
    pub scope: TurnScope,
    pub run_id: TurnRunId,
    pub reply_target: ReplyTargetBindingRef,
    pub key: ReplyPublicationTargetKey,
    pub descriptor: ReplyPublicationTargetDescriptor,
}

impl DeliveryCoordinator {
    /// The channel a publication addresses, from one active-snapshot read.
    pub(crate) fn resolve_reply_channel(
        &self,
        extension_id: &str,
    ) -> Option<ResolvedChannelDelivery> {
        self.resolver.resolve_channel_delivery(extension_id)
    }

    /// The stored opaque reply context (ING-11) for a publication target, or
    /// `None` for a target without a vendor conversation (the projection
    /// stream). A storage failure is an error, never a silent `None`: a
    /// reply that must thread under its trigger is not sent unthreaded.
    pub(crate) async fn reply_context_for_publication(
        &self,
        channel: &ResolvedChannelDelivery,
        conversation: Option<&ExternalConversationRef>,
    ) -> Result<Option<Vec<u8>>, CoordinatedDeliveryError> {
        let Some(conversation) = conversation else {
            return Ok(None);
        };
        self.reply_context
            .reply_context(
                &channel.extension_id,
                &channel.installation_id,
                &conversation.conversation_fingerprint(),
            )
            .await
            .map_err(|error| {
                tracing::debug!(
                    target: "ironclaw::reborn::delivery",
                    extension_id = %channel.extension_id,
                    %error,
                    "reply publication: reply-context read failed"
                );
                CoordinatedDeliveryError::ReplyContextUnavailable
            })
    }

    /// Open (or re-open) the publication for one run and target: one
    /// `Prepared` attempt row carrying the publication substate, keyed like
    /// every other projection fact. Idempotent per delivery id and target.
    pub(crate) async fn open_reply_publication(
        &self,
        request: OpenReplyPublication,
    ) -> Result<ReplyPublicationRecord, CoordinatedDeliveryError> {
        let projection_ref = ProjectionUpdateRef::new(format!(
            "reply-publication:{}:{}",
            request.run_id, request.key
        ))
        .map_err(|reason| CoordinatedDeliveryError::InvalidNotice { reason })?;
        let attempt = OutboundDeliveryAttempt {
            delivery_id: OutboundDeliveryId::for_projection_fact(
                &request.scope,
                &request.reply_target,
                &projection_ref,
            )?,
            scope: request.scope.clone(),
            candidate: OutboundPushCandidate {
                tenant_id: request.scope.tenant_id.clone(),
                agent_id: request.scope.agent_id.clone(),
                project_id: request.scope.project_id.clone(),
                thread_id: request.scope.thread_id.clone(),
                turn_run_id: Some(request.run_id),
                target: request.reply_target,
                kind: OutboundPushKind::FinalReply,
                projection_ref,
                requires_reply_target_revalidation: false,
            },
            status: OutboundDeliveryStatus::Prepared,
            attempted_at: Utc::now(),
            failure_kind: None,
        };
        // The store creates the attempt row *with* its publication substate
        // in one write (a plain row would read as a different aggregate) and
        // hands back the stored record on a re-open, so a crash between
        // registration and the first lease never resets a live publication.
        Ok(self
            .store
            .open_reply_publication(OpenReplyPublicationRequest {
                attempt,
                target: ReplyPublicationTarget {
                    run_id: request.run_id,
                    key: request.key,
                },
                descriptor: Some(request.descriptor),
                now: Utc::now(),
            })
            .await?)
    }

    pub(crate) async fn claim_reply_publication(
        &self,
        scope: TurnScope,
        delivery_id: OutboundDeliveryId,
        owner: PublisherId,
        ttl: Duration,
    ) -> Result<ReplyPublicationClaim, CoordinatedDeliveryError> {
        Ok(self
            .store
            .claim_reply_publication_lease(ClaimReplyPublicationLeaseRequest {
                delivery_id,
                scope,
                owner,
                ttl,
                now: Utc::now(),
            })
            .await?)
    }

    pub(crate) async fn advance_reply_publication(
        &self,
        request: AdvanceReplyPublicationRequest,
    ) -> Result<ReplyPublicationRecord, CoordinatedDeliveryError> {
        Ok(self.store.advance_reply_publication(request).await?)
    }

    pub(crate) async fn settle_reply_publication(
        &self,
        scope: TurnScope,
        delivery_id: OutboundDeliveryId,
        fence: u64,
        settlement: ReplyPublicationSettlement,
    ) -> Result<ReplyPublicationRecord, CoordinatedDeliveryError> {
        Ok(self
            .store
            .settle_reply_publication(SettleReplyPublicationRequest {
                delivery_id,
                scope,
                fence,
                settlement,
                now: Utc::now(),
            })
            .await?)
    }

    pub(crate) async fn release_reply_publication(
        &self,
        scope: TurnScope,
        delivery_id: OutboundDeliveryId,
        fence: u64,
    ) -> Result<(), CoordinatedDeliveryError> {
        Ok(self
            .store
            .release_reply_publication_lease(ReleaseReplyPublicationLeaseRequest {
                delivery_id,
                scope,
                fence,
            })
            .await?)
    }

    pub(crate) async fn load_reply_publication(
        &self,
        scope: TurnScope,
        delivery_id: OutboundDeliveryId,
    ) -> Result<Option<ReplyPublicationRecord>, CoordinatedDeliveryError> {
        Ok(self
            .store
            .load_reply_publication(scope, delivery_id)
            .await?)
    }

    /// Every publication opened for `run_id` in `scope`, settled or not.
    pub(crate) async fn list_reply_publications(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
    ) -> Result<Vec<ReplyPublicationRecord>, CoordinatedDeliveryError> {
        Ok(self.store.list_reply_publications(scope, run_id).await?)
    }

    /// Every publication still `Active` in the caller's tenant — the
    /// boot-time crash-recovery read behind
    /// [`DeliveryCoordinator::resume_reply_publications`].
    pub(crate) async fn list_open_reply_publications(
        &self,
        scope: TurnScope,
    ) -> Result<Vec<ReplyPublicationRecord>, CoordinatedDeliveryError> {
        Ok(self.store.list_open_reply_publications(scope).await?)
    }
}
