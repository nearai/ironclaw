//! Production implementation of the model-initiated channel delivery port
//! (`ironclaw_outbound::ModelChannelDelivery`): drives an explicit
//! `builtin.outbound_deliver`-style tool call through the same
//! `DeliveryCoordinator` / `OutboundPolicyService` pipeline every other
//! ModelDelivery-class push uses.
//!
//! Mirrors two existing seams rather than inventing new ones:
//! - `crate::run_delivery::observer`'s FinalReply delivery
//!   construction (`OutboundPolicyService::new` plus a per-call reply-target
//!   authority, `CoordinatedDeliveryRequest` shape).
//! - the same-origin check asks the durable conversation-binding store which
//!   thread a reply-target ref is bound to (`resolve_stored_reply_target`) —
//!   runs themselves carry no reply route.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use crate::ProjectFilesystemReader;
use crate::{
    CoordinatedDeliveryError, CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest,
    DeliveryCoordinator, DeliveryIntent, ProductOutboundTargetResolver, ProductSurfaceFailure,
    VerifiedProductOutboundTargetMetadata,
};
use async_trait::async_trait;
use chrono::Utc;
use ironclaw_conversations::{
    ConversationBindingService, InboundTurnError, ResolveStoredReplyTargetRequest,
    StoredReplyTargetAccess,
};
use ironclaw_extension_contracts::channel_adapter::OutboundPart;
use ironclaw_extension_contracts::preference_target::PreferenceTargetCodec;
use ironclaw_host_api::ids::{AgentId, RunId};
use ironclaw_host_api::turn::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    DeliveryFailureKind, MODEL_DELIVERY_MAX_CONTENT_BYTES, MODEL_DELIVERY_PER_RUN_CAP,
    ModelChannelDelivery, ModelChannelDeliveryError, ModelChannelDeliveryEvidence,
    ModelChannelDeliveryRequest, OutboundDeliveryTargetProvider, OutboundDeliveryTargetRegistry,
    OutboundDeliveryTargetScope, OutboundError, OutboundPolicyService, OutboundStateStorePort,
    PrepareCommunicationDeliveryRequest, ProjectionUpdateRef, ReplyTargetBindingClaim,
    ReplyTargetBindingValidator, ReplyTargetValidationRequest, RunNotificationContext,
    RunNotificationEventKind, RunNotificationOrigin, ThreadProjectionAccessClaim,
    ThreadProjectionAccessPolicy, ThreadProjectionAccessRequest, ValidatedReplyTargetBinding,
};
use ironclaw_threads::ThreadScope;

/// Tracing target shared by every log line in this module.
const TRACE_TARGET: &str = "ironclaw::reborn::model_channel_delivery";

/// FIFO bound on the number of distinct runs the per-run cap ledger tracks
/// (mirrors `HINT_SEEN_CAP` / `DELIVERED_RUNS_CAP` in
/// `crate::run_delivery`). An evicted run's counter resets to
/// zero if it somehow reappears — harmless: it just means one run, already
/// long finished, gets a fresh budget.
const MODEL_DELIVERY_TRACKED_RUNS_CAP: usize = 256;

/// Handles [`CoordinatedModelChannelDelivery`] needs, mirroring exactly the
/// subset of `crate::run_delivery::RunDeliveryServices` fields its
/// `OutboundPolicyService` construction threads for FinalReply deliveries,
/// plus the Task 2 port's own registry/run-state/target-resolver
/// dependencies.
pub struct ModelChannelDeliveryDeps {
    /// The outbound target catalog. Typed as the PROVIDER port, not the
    /// concrete [`OutboundDeliveryTargetRegistry`], because composition's
    /// registry is the mutable one channel activation registers providers
    /// into after this service is constructed — a fixed snapshot taken at
    /// build time would never see an activated channel's targets.
    ///
    /// Owner scoping is NOT assumed of whatever is injected here: the port's
    /// default `resolve_outbound_delivery_target` filters on `final_replies`
    /// only. [`CoordinatedModelChannelDelivery::new`] wraps this provider in
    /// an owner-filtering registry view, so cross-user access is a property of
    /// this service rather than of the caller's wiring.
    pub registry: Arc<dyn OutboundDeliveryTargetProvider>,
    pub coordinator: Arc<DeliveryCoordinator>,
    pub outbound_store: Arc<dyn OutboundStateStorePort>,
    pub target_resolver: Arc<dyn ProductOutboundTargetResolver>,
    /// Durable conversation-binding authority for the same-origin check:
    /// resolves which thread a sealed reply-target ref is bound to, so a
    /// target bound to the calling run's own thread is rejected as the
    /// origin conversation.
    pub binding_service: Arc<dyn ConversationBindingService>,
    /// Canonical project-scoped reader the coordinator materializes
    /// `/workspace/...` references through. Model deliveries carry no
    /// attachments, so this is contract plumbing, not a data path.
    pub project_filesystem: Arc<dyn ProjectFilesystemReader>,
    /// Fallback agent used to derive the project-filesystem authority scope
    /// when the calling run's scope carries no agent id.
    pub fallback_agent_id: AgentId,
}

/// Per-run delivery counters, FIFO-bounded to [`MODEL_DELIVERY_TRACKED_RUNS_CAP`]
/// distinct runs.
type RunDeliveryLedger = Mutex<(VecDeque<RunId>, HashMap<RunId, u32>)>;

/// Production [`ModelChannelDelivery`]: the extension-host-owned generic
/// channel-delivery-tool implementation of the port Task 2 defined.
pub struct CoordinatedModelChannelDelivery {
    deps: ModelChannelDeliveryDeps,
    /// Owner-scoping view over `deps.registry`. See [`Self::new`].
    owner_scoped_registry: OutboundDeliveryTargetRegistry,
    run_delivery_counts: RunDeliveryLedger,
}

impl CoordinatedModelChannelDelivery {
    pub fn new(deps: ModelChannelDeliveryDeps) -> Self {
        // Owner scoping must be a guarantee of THIS service, not a property
        // of whatever provider the caller injected: the
        // `OutboundDeliveryTargetProvider` trait's DEFAULT
        // `resolve_outbound_delivery_target` filters on `final_replies`
        // alone, so a bare provider would happily hand back another user's
        // target. `OutboundDeliveryTargetRegistry`'s own impl is the one that
        // applies `owner.matches_scope`, so wrap the injected provider in it
        // exactly once here. The wrapper delegates per call, so a mutable
        // registry behind it keeps its live view of providers registered
        // after this service was constructed.
        let owner_scoped_registry =
            OutboundDeliveryTargetRegistry::new(vec![Arc::clone(&deps.registry)]);
        Self {
            deps,
            owner_scoped_registry,
            run_delivery_counts: Mutex::new((VecDeque::new(), HashMap::new())),
        }
    }

    /// Reserve one delivery slot for `run_id` against the per-run cap
    /// (spec §5): FIFO-bounded to [`MODEL_DELIVERY_TRACKED_RUNS_CAP`] distinct
    /// runs, mirroring `HintSeenSet`'s eviction pattern
    /// (`crate::run_delivery`). Callers must only call this after
    /// content, target, and same-origin checks already passed.
    fn reserve_delivery_slot(&self, run_id: RunId) -> Result<(), ModelChannelDeliveryError> {
        let mut guard = self
            .run_delivery_counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (queue, counts) = &mut *guard;
        let is_new = !counts.contains_key(&run_id);
        let count = counts.entry(run_id).or_insert(0);
        if *count >= MODEL_DELIVERY_PER_RUN_CAP {
            return Err(ModelChannelDeliveryError::DeliveryCapExceeded);
        }
        *count += 1;
        if is_new {
            if queue.len() >= MODEL_DELIVERY_TRACKED_RUNS_CAP
                && let Some(oldest) = queue.pop_front()
            {
                counts.remove(&oldest);
            }
            queue.push_back(run_id);
        }
        Ok(())
    }
}

#[async_trait]
impl ModelChannelDelivery for CoordinatedModelChannelDelivery {
    async fn deliver_for_model(
        &self,
        request: ModelChannelDeliveryRequest,
    ) -> Result<ModelChannelDeliveryEvidence, ModelChannelDeliveryError> {
        // 1. Content size (spec §5): byte length, no slicing.
        if request.content.len() > MODEL_DELIVERY_MAX_CONTENT_BYTES {
            return Err(ModelChannelDeliveryError::ContentTooLarge);
        }

        // 2. Target resolution through the owner-scoping registry view built
        // in `new` — filters by (tenant, user) ownership AND by the
        // `final_replies` capability, so an unknown, foreign-owned, or
        // non-final-reply target is indistinguishable here (all `None`).
        let caller = OutboundDeliveryTargetScope::new(
            request.scope.tenant_id.clone(),
            // The AUTHENTICATED ACTOR owns the catalog this call may reach —
            // deliberately not `scope.user_id`.
            //
            // Owner and actor now coincide on every binding created under the
            // run-acts-as-invoker rule, but the actor stays authoritative for
            // runs persisted before it: resolving a legacy thread OWNER's
            // catalog would let a shared-channel participant enumerate and
            // deliver into that owner's personal destinations — their DM
            // included — from a conversation the owner may never read.
            //
            // Scoping to the actor keeps a caller inside their own connected
            // surfaces on every path, so an unfamiliar target simply does not
            // resolve (`TargetUnavailable`) instead of resolving to someone
            // else's.
            request.authenticated_actor_user_id.clone(),
        );
        let entry = self
            .owner_scoped_registry
            .resolve_outbound_delivery_target(&caller, &request.target_id)
            .await
            .map_err(|error| {
                tracing::debug!(
                    target: TRACE_TARGET,
                    run_id = %request.run_id,
                    %error,
                    "model channel delivery target lookup failed"
                );
                ModelChannelDeliveryError::Unavailable
            })?;
        let Some(entry) = entry else {
            return Err(ModelChannelDeliveryError::TargetUnavailable);
        };
        let target_binding_ref = entry.destination.clone();

        // 3. Same-origin check: runs carry no reply route, so the durable
        // conversation-binding store is the authority on which thread a
        // reply-target ref is bound to. `resolve_stored_reply_target`
        // succeeds only when the ref is bound to the probed thread — an `Ok`
        // against the run's own thread IS the origin conversation. A failure
        // that cannot rule the target out (backend unavailability) fails
        // closed as `Unavailable`.
        let Some(thread_id) = request.scope.thread_id.clone() else {
            tracing::debug!(
                target: TRACE_TARGET,
                run_id = %request.run_id,
                "model channel delivery unavailable: invocation scope has no thread id for the same-origin lookup"
            );
            return Err(ModelChannelDeliveryError::Unavailable);
        };
        let turn_scope = TurnScope::new_with_owner(
            request.scope.tenant_id.clone(),
            request.scope.agent_id.clone(),
            request.scope.project_id.clone(),
            thread_id.clone(),
            Some(request.scope.user_id.clone()),
        );
        let turn_run_id = TurnRunId::from_uuid(request.run_id.as_uuid());
        match self
            .deps
            .binding_service
            .resolve_stored_reply_target(ResolveStoredReplyTargetRequest {
                tenant_id: request.scope.tenant_id.clone(),
                actor_user_id: request.authenticated_actor_user_id.clone(),
                current_thread_id: thread_id.clone(),
                reply_target_binding_ref: target_binding_ref.clone(),
                access: StoredReplyTargetAccess::OrdinaryReply,
            })
            .await
        {
            Ok(_) => return Err(ModelChannelDeliveryError::OriginConversationTarget),
            // Bound to this thread but the actor's route authority failed:
            // still the origin conversation — reject rather than letting an
            // authority edge case re-open the origin lane.
            Err(InboundTurnError::AccessDenied {
                thread_id: denied, ..
            }) if denied == thread_id.as_str() => {
                return Err(ModelChannelDeliveryError::OriginConversationTarget);
            }
            // Definitive non-origin reads: the ref is not a stored
            // conversation route at all, or it is bound to a different
            // thread.
            Err(
                InboundTurnError::ThreadNotFound { .. } | InboundTurnError::AccessDenied { .. },
            ) => {}
            Err(error) => {
                tracing::debug!(
                    target: TRACE_TARGET,
                    run_id = %request.run_id,
                    %error,
                    "model channel delivery same-origin binding probe failed"
                );
                return Err(ModelChannelDeliveryError::Unavailable);
            }
        }

        // 4. Per-run cap. Only reserved after every check above passed.
        self.reserve_delivery_slot(request.run_id)?;

        // 5. Happy path: drive the coordinator exactly the way the observer
        // drives FinalReply deliveries, but with an explicit run-scoped
        // target and the ModelDelivery intent/kind.
        let actor = TurnActor::new(request.authenticated_actor_user_id.clone());
        let projection_ref = ProjectionUpdateRef::new(format!(
            "model-delivery:{run_id}:{invocation_id}",
            run_id = request.run_id,
            invocation_id = request.scope.invocation_id,
        ))
        .map_err(|reason| {
            tracing::error!(
                target: TRACE_TARGET,
                run_id = %request.run_id,
                reason,
                "model channel delivery projection ref rejected"
            );
            ModelChannelDeliveryError::Internal
        })?;

        let authority = ExpectedTargetAuthority {
            scope: turn_scope.clone(),
            actor: actor.clone(),
            expected_target: target_binding_ref.clone(),
        };
        let projection_access = DenyProjectionAccess;
        let outbound_policy = OutboundPolicyService::new(
            self.deps.outbound_store.as_ref(),
            &projection_access,
            &authority,
        );

        let delivery = PrepareCommunicationDeliveryRequest {
            resolution_request: CommunicationDeliveryResolutionRequest {
                scope: turn_scope,
                actor,
                modality: CommunicationModality::Text,
                intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                    event_kind: RunNotificationEventKind::ModelDelivery,
                    origin: RunNotificationOrigin::RunScopedTarget {
                        target: target_binding_ref,
                    },
                }),
            },
            turn_run_id: Some(turn_run_id),
            projection_ref,
            attempted_at: Utc::now(),
        };

        // Canonical project-filesystem authority scope. Model deliveries
        // never carry attachments, so nothing materializes through it; the
        // coordinator contract requires the scope on every request.
        let thread_scope = ThreadScope {
            tenant_id: request.scope.tenant_id.clone(),
            agent_id: request
                .scope
                .agent_id
                .clone()
                .unwrap_or_else(|| self.deps.fallback_agent_id.clone()),
            project_id: request.scope.project_id.clone(),
            owner_user_id: Some(request.authenticated_actor_user_id.clone()),
            mission_id: None,
        };
        let outcome = self
            .deps
            .coordinator
            .deliver(
                &outbound_policy,
                self.deps.target_resolver.as_ref(),
                self.deps.project_filesystem.as_ref(),
                CoordinatedDeliveryRequest {
                    intent: DeliveryIntent::ModelDelivery,
                    delivery,
                    parts: vec![OutboundPart::Text(request.content)],
                    attachments: Vec::new(),
                    thread_anchor: None,
                    require_direct_message_target: false,
                    extension_id: entry.summary.channel.as_str(),
                    thread_scope: &thread_scope,
                },
            )
            .await;

        // 6. Outcome mapping.
        match outcome {
            Ok(outcome) => {
                if matches!(outcome, CoordinatedDeliveryOutcome::NoDelivery) {
                    // Cannot happen with an explicit resolved candidate —
                    // log loud, this is a real invariant violation.
                    tracing::error!(
                        target: TRACE_TARGET,
                        run_id = %request.run_id,
                        "model channel delivery produced NoDelivery despite an explicit resolved candidate; this should be unreachable"
                    );
                }
                classify_delivery_outcome(entry.summary, outcome)
            }
            Err(error) => {
                match &error {
                    CoordinatedDeliveryError::ChannelUnavailable { extension_id } => {
                        tracing::debug!(
                            target: TRACE_TARGET,
                            run_id = %request.run_id,
                            extension_id,
                            "model channel delivery: resolved channel is not currently active"
                        );
                    }
                    other => {
                        tracing::warn!(
                            target: TRACE_TARGET,
                            run_id = %request.run_id,
                            error = %other,
                            "model channel delivery coordinator call failed"
                        );
                    }
                }
                Err(classify_coordinator_error(error))
            }
        }
    }
}

/// Reply-target authority for one delivery attempt: trusts only the exact
/// target this port already resolved and same-origin-checked. Mirrors
/// `ObservedReplyTargetAuthority`'s snapshot-equality validation
/// (`crate::run_delivery::observer`) rather than reusing it
/// directly (that type is private to its owning module).
struct ExpectedTargetAuthority {
    scope: TurnScope,
    actor: TurnActor,
    expected_target: ReplyTargetBindingRef,
}

#[async_trait]
impl ReplyTargetBindingValidator for ExpectedTargetAuthority {
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        if request.scope != self.scope
            || request.actor != self.actor
            || request.candidate.target != self.expected_target
        {
            return Err(OutboundError::AccessDenied);
        }
        Ok(ReplyTargetBindingClaim::new(request.candidate.target))
    }
}

/// Late-bound [`ModelChannelDelivery`] for one composition pass.
///
/// The capability handler must be inserted while the first-party registry is
/// assembled, but the [`DeliveryCoordinator`] that backs it is only built
/// afterwards in the same pass — it needs the generic extension host, whose
/// tool binder in turn needs the finished first-party registry. Rather than
/// break that cycle by reordering composition, the handler holds this slot
/// and composition fills it once the coordinator exists (first write wins,
/// mirroring the `channel_disconnect_slot` `OnceLock` beside it).
///
/// An unfilled slot fails closed as `Unavailable` — identical to the
/// host-runtime default handler, so a composition path that never binds a
/// delivery service is no worse off than one that never registered.
#[derive(Default)]
pub struct DeferredModelChannelDelivery {
    delivery: OnceLock<Arc<dyn ModelChannelDelivery>>,
}

impl DeferredModelChannelDelivery {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the backing service. Returns `false` when the slot was already
    /// filled (the earlier binding is kept).
    pub fn bind(&self, delivery: Arc<dyn ModelChannelDelivery>) -> bool {
        self.delivery.set(delivery).is_ok()
    }
}

#[async_trait]
impl ModelChannelDelivery for DeferredModelChannelDelivery {
    async fn deliver_for_model(
        &self,
        request: ModelChannelDeliveryRequest,
    ) -> Result<ModelChannelDeliveryEvidence, ModelChannelDeliveryError> {
        let Some(delivery) = self.delivery.get() else {
            tracing::debug!(
                target: TRACE_TARGET,
                run_id = %request.run_id,
                "model channel delivery unavailable: composition never bound a delivery service"
            );
            return Err(ModelChannelDeliveryError::Unavailable);
        };
        delivery.deliver_for_model(request).await
    }
}

/// Conversation resolver for explicit model deliveries.
///
/// An explicit delivery names an opaque catalog target, so — unlike the
/// live final-reply path — there is no inbound envelope carrying the
/// destination conversation. The conversation is decoded from the target's
/// vendor binding ref through the same [`PreferenceTargetCodec`] port the
/// triggered-delivery driver uses (`crate::run_delivery::triggered`),
/// which is the only place vendor knowledge enters this path. Codecs are
/// tried in registration order; the first that decodes the ref owns it.
///
/// Fails closed: an undecodable ref resolves to nothing rather than to some
/// other channel's conversation. Whether that channel is currently ACTIVE is
/// not this type's business — the coordinator's channel resolution is the
/// single enforcement point for that, and reports `ChannelUnavailable`.
pub struct CodecChannelTargetResolver {
    codecs: Vec<Arc<dyn PreferenceTargetCodec>>,
    /// Names the delivery path in the resolution-failure reason, so one shared
    /// implementation still produces a diagnostic that says which caller could
    /// not resolve its target.
    context_label: &'static str,
}

impl CodecChannelTargetResolver {
    pub fn new(codecs: Vec<Arc<dyn PreferenceTargetCodec>>) -> Self {
        Self::with_context_label(codecs, "explicit delivery")
    }

    pub fn with_context_label(
        codecs: Vec<Arc<dyn PreferenceTargetCodec>>,
        context_label: &'static str,
    ) -> Self {
        Self {
            codecs,
            context_label,
        }
    }
}

#[async_trait]
impl ProductOutboundTargetResolver for CodecChannelTargetResolver {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductSurfaceFailure> {
        for codec in &self.codecs {
            let Some(external_conversation_ref) = codec.conversation_for_target(target.target())
            else {
                continue;
            };
            // Single enforcement point for the DM rule, checked against the
            // binding resolved NOW (mirrors `TriggeredReplyTargetAuthority`).
            if require_direct_message && !codec.is_personal_direct_message(target.target()) {
                return Err(ProductSurfaceFailure::OutboundTargetNotDirectMessage);
            }
            return Ok(VerifiedProductOutboundTargetMetadata {
                external_conversation_ref,
                external_actor_ref: None,
            });
        }
        Err(ProductSurfaceFailure::BindingResolutionFailed {
            reason: format!(
                "{}: no registered channel codec decodes the binding ref '{}'",
                self.context_label,
                target.target().as_str()
            ),
        })
    }
}

/// This port never subscribes to projection replay; deny unconditionally,
/// mirroring `AllowNoProjectionAccess` in `crate::run_delivery`.
struct DenyProjectionAccess;

#[async_trait]
impl ThreadProjectionAccessPolicy for DenyProjectionAccess {
    async fn authorize_projection_access(
        &self,
        _request: ThreadProjectionAccessRequest,
    ) -> Result<ThreadProjectionAccessClaim, OutboundError> {
        Err(OutboundError::AccessDenied)
    }
}

/// Pure outcome-mapping (contract bullet 6). Runs strictly after the
/// coordinator call already happened, so unit-testing it directly does not
/// violate "test through the caller, not just the helper" (there is no
/// side effect left to gate here — only classification of one that already
/// ran).
fn classify_delivery_outcome(
    target: ironclaw_outbound::OutboundDeliveryTargetSummary,
    outcome: CoordinatedDeliveryOutcome,
) -> Result<ModelChannelDeliveryEvidence, ModelChannelDeliveryError> {
    match outcome {
        CoordinatedDeliveryOutcome::Delivered {
            vendor_message_refs,
            ..
        } => Ok(ModelChannelDeliveryEvidence {
            target,
            provider_message_refs: vendor_message_refs,
            durably_recorded: true,
            already_delivered: false,
        }),
        // A stream reply is delivered by the projection pipeline, and the
        // cursor is the durable proof the user can see it. There is no vendor
        // message ref because no vendor was involved — reporting the cursor
        // as a ref would fabricate a provider identifier, so the evidence is
        // honestly ref-free. Reachable only if a model-requested delivery
        // ever resolves to a stream channel; the coordinator routes model
        // deliveries on the delivery axis, so today it does not.
        CoordinatedDeliveryOutcome::StreamDelivered { .. } => Ok(ModelChannelDeliveryEvidence {
            target,
            provider_message_refs: Vec::new(),
            durably_recorded: true,
            already_delivered: false,
        }),
        CoordinatedDeliveryOutcome::StreamDeliveredUnconfirmed { .. } => {
            Ok(ModelChannelDeliveryEvidence {
                target,
                provider_message_refs: Vec::new(),
                durably_recorded: false,
                already_delivered: false,
            })
        }
        // The send happened (the refs are real) but the durable confirmation
        // write failed. Success-shaped so the model never resends; the
        // evidence carries the honest weaker claim.
        CoordinatedDeliveryOutcome::DeliveredUnconfirmed {
            vendor_message_refs,
            ..
        } => Ok(ModelChannelDeliveryEvidence {
            target,
            provider_message_refs: vendor_message_refs,
            durably_recorded: false,
            already_delivered: false,
        }),
        // The durable row confirms that this exact delivery fact completed in
        // an earlier invocation. Provider refs are not retained in the row, so
        // report only the evidence we actually have — flagged as a replay, so
        // the empty ref list cannot be mistaken for an unverified send and
        // answered with a duplicate.
        CoordinatedDeliveryOutcome::AlreadyDelivered { .. } => Ok(ModelChannelDeliveryEvidence {
            target,
            provider_message_refs: Vec::new(),
            durably_recorded: true,
            already_delivered: true,
        }),
        CoordinatedDeliveryOutcome::Rejected { .. } => Err(ModelChannelDeliveryError::Rejected),
        CoordinatedDeliveryOutcome::Failed { failure_kind, .. } => {
            Err(ModelChannelDeliveryError::Failed { kind: failure_kind })
        }
        CoordinatedDeliveryOutcome::NoDelivery => Err(ModelChannelDeliveryError::Internal),
    }
}

/// Pure coordinator-error classification (contract bullet 6).
fn classify_coordinator_error(error: CoordinatedDeliveryError) -> ModelChannelDeliveryError {
    match error {
        CoordinatedDeliveryError::ChannelUnavailable { .. }
        | CoordinatedDeliveryError::ReplyContextUnavailable => ModelChannelDeliveryError::Failed {
            kind: DeliveryFailureKind::TransportUnavailable,
        },
        // Model-correctable classes stay model-visible (rules/tools.md): a
        // concurrent duplicate, a policy/target-validation rejection, and
        // model-supplied attachment misuse can all be retried or repaired by
        // the model. `Internal` is reserved for host faults (backend errors,
        // unreadable workspace, intent-classification bugs).
        CoordinatedDeliveryError::AlreadyInFlight => ModelChannelDeliveryError::Failed {
            kind: DeliveryFailureKind::Rejected,
        },
        // Workflow wraps both model-correctable target-validation rejections
        // and host-side failures; only the former may surface as Rejected.
        CoordinatedDeliveryError::Workflow(
            ProductSurfaceFailure::OutboundTargetNotDirectMessage,
        ) => ModelChannelDeliveryError::Rejected,
        CoordinatedDeliveryError::Workflow(_) => ModelChannelDeliveryError::Internal,
        CoordinatedDeliveryError::WorkspaceAttachmentBudgetExceeded
        | CoordinatedDeliveryError::WorkspaceAttachmentRefInvalid { .. }
        | CoordinatedDeliveryError::PreMaterializedWorkspaceAttachment => {
            ModelChannelDeliveryError::Rejected
        }
        CoordinatedDeliveryError::Outbound(_)
        | CoordinatedDeliveryError::IntentClassMismatch { .. }
        | CoordinatedDeliveryError::InvalidNotice { .. }
        | CoordinatedDeliveryError::WorkspaceAttachmentRead(_) => {
            ModelChannelDeliveryError::Internal
        }
    }
}

#[cfg(test)]
mod tests;
