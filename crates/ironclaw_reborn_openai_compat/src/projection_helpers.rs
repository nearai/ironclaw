use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_adapters::{ProjectionReadRequest, ProjectionSubscriptionRequest};

use crate::{OpenAiCompatAuthenticatedCaller, OpenAiCompatErrorKind, OpenAiCompatHttpError};

pub(crate) fn ensure_projection_read_matches_caller(
    caller: &OpenAiCompatAuthenticatedCaller,
    projection_read: &ProjectionReadRequest,
) -> Result<(), OpenAiCompatHttpError> {
    ensure_projection_fields_match_caller(
        caller,
        &projection_read.actor.user_id,
        &projection_read.scope.tenant_id,
        projection_read.scope.agent_id.as_ref(),
        projection_read.scope.project_id.as_ref(),
        projection_read.scope.explicit_owner_user_id(),
    )
}

pub(crate) fn ensure_projection_subscription_matches_caller(
    caller: &OpenAiCompatAuthenticatedCaller,
    projection_subscription: &ProjectionSubscriptionRequest,
) -> Result<(), OpenAiCompatHttpError> {
    ensure_projection_fields_match_caller(
        caller,
        &projection_subscription.actor.user_id,
        &projection_subscription.scope.tenant_id,
        projection_subscription.scope.agent_id.as_ref(),
        projection_subscription.scope.project_id.as_ref(),
        projection_subscription.scope.explicit_owner_user_id(),
    )
}

fn ensure_projection_fields_match_caller(
    caller: &OpenAiCompatAuthenticatedCaller,
    actor_user_id: &UserId,
    tenant_id: &TenantId,
    agent_id: Option<&AgentId>,
    project_id: Option<&ProjectId>,
    owner_user_id: Option<&UserId>,
) -> Result<(), OpenAiCompatHttpError> {
    let caller_scope = caller.scope();
    let matches_caller = actor_user_id == caller_scope.user_id()
        && tenant_id == caller_scope.tenant_id()
        && agent_id == caller_scope.agent_id()
        && project_id == caller_scope.project_id()
        && owner_user_id.is_none_or(|owner| owner == caller_scope.user_id());
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
