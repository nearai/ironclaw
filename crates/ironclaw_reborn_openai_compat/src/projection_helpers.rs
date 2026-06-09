use ironclaw_host_api::UserId;
use ironclaw_product_adapters::{ProjectionReadRequest, ProjectionSubscriptionRequest};
use ironclaw_turns::TurnScope;

use crate::{OpenAiCompatAuthenticatedCaller, OpenAiCompatErrorKind, OpenAiCompatHttpError};

pub(crate) fn ensure_projection_read_matches_caller(
    caller: &OpenAiCompatAuthenticatedCaller,
    projection_read: &ProjectionReadRequest,
) -> Result<(), OpenAiCompatHttpError> {
    ensure_projection_scope_matches_caller(
        caller,
        &projection_read.actor.user_id,
        &projection_read.scope,
    )
}

pub(crate) fn ensure_projection_subscription_matches_caller(
    caller: &OpenAiCompatAuthenticatedCaller,
    projection_subscription: &ProjectionSubscriptionRequest,
) -> Result<(), OpenAiCompatHttpError> {
    ensure_projection_scope_matches_caller(
        caller,
        &projection_subscription.actor.user_id,
        &projection_subscription.scope,
    )
}

fn ensure_projection_scope_matches_caller(
    caller: &OpenAiCompatAuthenticatedCaller,
    actor_user_id: &UserId,
    scope: &TurnScope,
) -> Result<(), OpenAiCompatHttpError> {
    let caller_scope = caller.scope();
    let matches_caller = actor_user_id == caller_scope.user_id()
        && &scope.tenant_id == caller_scope.tenant_id()
        && scope.agent_id.as_ref() == caller_scope.agent_id()
        && scope.project_id.as_ref() == caller_scope.project_id()
        && scope
            .explicit_owner_user_id()
            .is_none_or(|owner| owner == caller_scope.user_id());
    if matches_caller {
        Ok(())
    } else {
        Err(OpenAiCompatHttpError::from_kind(
            403,
            false,
            OpenAiCompatErrorKind::PermissionDenied,
            None,
        ))
    }
}
