use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{ResourceScope, RunId};
use ironclaw_outbound::{
    OutboundError, OutboundStateStore, RouteCurrentRunFinalReply, RouteCurrentRunFinalReplyError,
    RouteCurrentRunFinalReplyRequest, RunFinalReplyDestination, RunFinalReplyTargetRecord,
    RunFinalReplyTargetRequest, WEB_APP_OUTBOUND_DELIVERY_TARGET_ID,
};
use ironclaw_triggers::{
    TriggerDeliveryTargetId, TriggerError, TriggerRecordValidationKind,
    parse_trigger_delivery_target_id,
};
use ironclaw_turns::{
    GetRunStateRequest, ReplyTargetBindingRef, TurnActor, TurnOriginKind, TurnRunId, TurnScope,
    TurnStateStore,
};

use crate::{CurrentDeliveryTargetResolver, ProductWorkflowError};

/// Resolves the final-reply target sealed into a newly-created trigger.
///
/// Explicit model input must still resolve through current caller authority.
/// When the input is omitted, the service reads only host-owned run state: an
/// exact per-run destination wins, then an existing external source route is
/// inherited. WebUI runs resolve to the host-owned WebApp destination. No
/// prompt text, display label, provider name, or model-authored conversation id
/// participates in the decision.
pub struct TriggerFinalReplyTargetService {
    source_turn_state: Arc<dyn TurnStateStore>,
    run_targets: Arc<dyn OutboundStateStore>,
    current_targets: Arc<dyn CurrentDeliveryTargetResolver>,
}

impl TriggerFinalReplyTargetService {
    pub fn new(
        source_turn_state: Arc<dyn TurnStateStore>,
        run_targets: Arc<dyn OutboundStateStore>,
        current_targets: Arc<dyn CurrentDeliveryTargetResolver>,
    ) -> Self {
        Self {
            source_turn_state,
            run_targets,
            current_targets,
        }
    }

    pub async fn validate_explicit_target(
        &self,
        scope: &ResourceScope,
        target: &TriggerDeliveryTargetId,
    ) -> Result<(), TriggerError> {
        match self
            .current_targets
            .resolve_current_destination(scope, target)
            .await
        {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(invalid_target(
                "delivery target is not available to this caller",
            )),
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::product_workflow::trigger_final_reply_target",
                    %error,
                    "outbound delivery target lookup failed during trigger creation"
                );
                Err(target_backend_error(
                    "outbound delivery target lookup unavailable",
                ))
            }
        }
    }

    pub async fn resolve_source_run_target(
        &self,
        scope: &ResourceScope,
        run_id: Option<RunId>,
    ) -> Result<Option<TriggerDeliveryTargetId>, TriggerError> {
        let (Some(run_id), Some(thread_id)) = (run_id, scope.thread_id.clone()) else {
            return Ok(None);
        };
        let turn_scope = TurnScope::new_with_owner(
            scope.tenant_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            thread_id,
            Some(scope.user_id.clone()),
        );
        let turn_run_id = TurnRunId::from_uuid(run_id.as_uuid());
        let state = self
            .source_turn_state
            .get_run_state(GetRunStateRequest {
                scope: turn_scope.clone(),
                run_id: turn_run_id,
            })
            .await
            .map_err(|error| {
                tracing::warn!(
                    target = "ironclaw::product_workflow::trigger_final_reply_target",
                    error_kind = ?error.category(),
                    "source run lookup failed during trigger target inheritance"
                );
                target_backend_error("source run reply target lookup unavailable")
            })?;
        let actor = TurnActor::new(scope.user_id.clone());
        if state.actor.as_ref() != Some(&actor) {
            return Err(invalid_target(
                "source run is not owned by the trigger creator",
            ));
        }
        let sealed_destination = self
            .run_targets
            .load_run_final_reply_target(RunFinalReplyTargetRequest {
                run_id: turn_run_id,
                scope: turn_scope,
                actor,
            })
            .await
            .map_err(map_outbound_error)?
            .map(|record| record.destination);

        match sealed_destination {
            Some(RunFinalReplyDestination::WebApp) => web_app_trigger_target().map(Some),
            Some(RunFinalReplyDestination::External {
                reply_target_binding_ref,
            }) => self
                .resolve_current_binding(scope, &reply_target_binding_ref)
                .await
                .map(Some),
            None if state
                .product_context
                .as_ref()
                .is_some_and(|context| context.origin == TurnOriginKind::WebUi) =>
            {
                web_app_trigger_target().map(Some)
            }
            // Implicit inference only: a source run whose reply binding maps
            // to no current outbound target (e.g. a WebUI-less synthetic or
            // retired binding) simply seals no inherited target. Explicit
            // model-supplied targets still fail closed in
            // `validate_explicit_target`.
            None => match self
                .current_targets
                .resolve_current_target_id(scope, &state.reply_target_binding_ref)
                .await
            {
                Ok(Some(target_id)) => Ok(Some(target_id)),
                Ok(None) => Ok(None),
                Err(error) => {
                    tracing::warn!(
                        target = "ironclaw::product_workflow::trigger_final_reply_target",
                        %error,
                        "current delivery target lookup failed during trigger target inheritance"
                    );
                    Err(target_backend_error(
                        "outbound delivery target lookup unavailable",
                    ))
                }
            },
        }
    }

    async fn resolve_current_binding(
        &self,
        scope: &ResourceScope,
        target: &ReplyTargetBindingRef,
    ) -> Result<TriggerDeliveryTargetId, TriggerError> {
        match self
            .current_targets
            .resolve_current_target_id(scope, target)
            .await
        {
            Ok(Some(target_id)) => Ok(target_id),
            Ok(None) => Err(invalid_target(
                "source run delivery target is not available to this caller",
            )),
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::product_workflow::trigger_final_reply_target",
                    %error,
                    "source reply target lookup failed during trigger creation"
                );
                Err(target_backend_error(
                    "source reply target lookup unavailable",
                ))
            }
        }
    }
}

/// Product-owned current-run routing mutation. Capability code supplies only
/// trusted run/scope/actor context plus an opaque registry id; this service
/// owns target resolution and durable route persistence.
pub struct RunFinalReplyRoutingService {
    current_targets: Arc<dyn CurrentDeliveryTargetResolver>,
    store: Arc<dyn OutboundStateStore>,
}

impl RunFinalReplyRoutingService {
    pub fn new(
        current_targets: Arc<dyn CurrentDeliveryTargetResolver>,
        store: Arc<dyn OutboundStateStore>,
    ) -> Self {
        Self {
            current_targets,
            store,
        }
    }
}

#[async_trait]
impl RouteCurrentRunFinalReply for RunFinalReplyRoutingService {
    async fn route_current_run_final_reply(
        &self,
        request: RouteCurrentRunFinalReplyRequest,
    ) -> Result<(), RouteCurrentRunFinalReplyError> {
        let RouteCurrentRunFinalReplyRequest {
            run_id,
            scope,
            authenticated_actor_user_id,
            target_id,
        } = request;
        if scope.user_id != authenticated_actor_user_id {
            return Err(RouteCurrentRunFinalReplyError::AccessDenied);
        }
        let Some(thread_id) = scope.thread_id.clone() else {
            return Err(RouteCurrentRunFinalReplyError::InvalidRequest);
        };
        let destination = self
            .current_targets
            .resolve_current_destination(&scope, &target_id)
            .await
            .map_err(map_route_service_error)?
            .ok_or(RouteCurrentRunFinalReplyError::TargetUnavailable)?;
        self.store
            .put_run_final_reply_target(RunFinalReplyTargetRecord {
                run_id: TurnRunId::from_uuid(run_id.as_uuid()),
                scope: TurnScope::new_with_owner(
                    scope.tenant_id,
                    scope.agent_id,
                    scope.project_id,
                    thread_id,
                    Some(scope.user_id.clone()),
                ),
                actor: TurnActor::new(scope.user_id),
                destination,
            })
            .await
            .map_err(map_route_outbound_error)
    }
}

fn web_app_trigger_target() -> Result<TriggerDeliveryTargetId, TriggerError> {
    parse_trigger_delivery_target_id(WEB_APP_OUTBOUND_DELIVERY_TARGET_ID).map_err(|error| {
        tracing::error!(
            target = "ironclaw::product_workflow::trigger_final_reply_target",
            %error,
            "host-owned WebApp target id violated the trigger target contract"
        );
        target_backend_error("host-owned WebApp target is unavailable")
    })
}

fn invalid_target(reason: impl Into<String>) -> TriggerError {
    TriggerError::InvalidRecord {
        kind: TriggerRecordValidationKind::DeliveryTargetInvalid,
        reason: reason.into(),
    }
}

fn target_backend_error(reason: impl Into<String>) -> TriggerError {
    TriggerError::Backend {
        reason: reason.into(),
    }
}

fn map_outbound_error(error: OutboundError) -> TriggerError {
    tracing::warn!(
        target = "ironclaw::product_workflow::trigger_final_reply_target",
        %error,
        "sealed source-run final-reply target lookup failed"
    );
    target_backend_error("source run final-reply target lookup unavailable")
}

fn map_route_service_error(error: ProductWorkflowError) -> RouteCurrentRunFinalReplyError {
    match error {
        ProductWorkflowError::BindingAccessDenied
        | ProductWorkflowError::BindingRequired { .. }
        | ProductWorkflowError::UnknownInstallation
        | ProductWorkflowError::InvalidBindingRequest { .. } => {
            RouteCurrentRunFinalReplyError::AccessDenied
        }
        ProductWorkflowError::Transient { .. } => RouteCurrentRunFinalReplyError::Unavailable,
        _ => RouteCurrentRunFinalReplyError::Internal,
    }
}

fn map_route_outbound_error(error: OutboundError) -> RouteCurrentRunFinalReplyError {
    match error {
        OutboundError::InvalidRequest { .. }
        | OutboundError::PreferenceTargetMissing { .. }
        | OutboundError::SubscriptionScopeMismatch
        | OutboundError::DeliveryNotFound => RouteCurrentRunFinalReplyError::InvalidRequest,
        OutboundError::AccessDenied => RouteCurrentRunFinalReplyError::AccessDenied,
        OutboundError::CasConflict | OutboundError::Backend | OutboundError::Serialization => {
            RouteCurrentRunFinalReplyError::Unavailable
        }
    }
}
