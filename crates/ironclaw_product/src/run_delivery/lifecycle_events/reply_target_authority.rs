use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_outbound::{
    OutboundError, ReplyTargetBindingClaim, ReplyTargetBindingValidator,
    ReplyTargetValidationRequest, ThreadProjectionAccessClaim, ThreadProjectionAccessPolicy,
    ThreadProjectionAccessRequest,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnScope};

use super::CurrentDeliveryTargetResolver;
use crate::{
    ConversationBindingService, ProductConversationRouteKind, ProductOutboundTargetResolver,
    ProductWorkflowError, ResolveStoredProductReplyTargetRequest, ResolvedStoredProductReplyTarget,
    StoredProductReplyTargetAccess, VerifiedProductOutboundTargetMetadata,
};

pub(super) struct StoredReplyTargetAuthority {
    pub(super) binding_service: Arc<dyn ConversationBindingService>,
    pub(super) scope: TurnScope,
    pub(super) actor: TurnActor,
    pub(super) expected_target: ReplyTargetBindingRef,
    pub(super) expected_adapter_id: String,
    pub(super) expected_installation_id: String,
    pub(super) access: StoredProductReplyTargetAccess,
}

pub(super) enum LiveReplyTargetAuthority {
    Source(StoredReplyTargetAuthority),
    Current {
        resolver: Arc<dyn CurrentDeliveryTargetResolver>,
        scope: TurnScope,
        actor: TurnActor,
        expected_target: ReplyTargetBindingRef,
        expected_extension_id: String,
    },
}

#[async_trait]
impl ReplyTargetBindingValidator for LiveReplyTargetAuthority {
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        match self {
            Self::Source(source) => source.validate_reply_target(request).await,
            Self::Current {
                resolver,
                scope,
                actor,
                expected_target,
                expected_extension_id,
            } => {
                if request.scope != *scope
                    || request.actor != *actor
                    || request.candidate.target != *expected_target
                {
                    return Err(OutboundError::AccessDenied);
                }
                let current = resolver
                    .resolve_current_target(scope, actor, expected_target)
                    .await
                    .map_err(|error| match error {
                        ProductWorkflowError::Transient { .. } => OutboundError::Backend,
                        _ => OutboundError::AccessDenied,
                    })?;
                if current
                    .as_ref()
                    .is_none_or(|current| current.extension_id != *expected_extension_id)
                {
                    return Err(OutboundError::AccessDenied);
                }
                Ok(ReplyTargetBindingClaim::new(request.candidate.target))
            }
        }
    }
}

#[async_trait]
impl ProductOutboundTargetResolver for LiveReplyTargetAuthority {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ironclaw_outbound::ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        match self {
            Self::Source(source) => {
                source
                    .resolve_product_outbound_target_metadata(target, require_direct_message)
                    .await
            }
            Self::Current {
                resolver,
                scope,
                actor,
                expected_target,
                expected_extension_id,
            } => {
                if target.target() != expected_target {
                    return Err(ProductWorkflowError::BindingAccessDenied);
                }
                let current = resolver
                    .resolve_current_target(scope, actor, expected_target)
                    .await?
                    .ok_or(ProductWorkflowError::BindingAccessDenied)?;
                if current.extension_id != *expected_extension_id {
                    return Err(ProductWorkflowError::BindingAccessDenied);
                }
                if require_direct_message && !current.personal_direct_message {
                    return Err(ProductWorkflowError::OutboundTargetNotDirectMessage);
                }
                Ok(VerifiedProductOutboundTargetMetadata {
                    external_conversation_ref: current.external_conversation_ref,
                    external_actor_ref: None,
                })
            }
        }
    }
}

impl StoredReplyTargetAuthority {
    async fn resolve(&self) -> Result<ResolvedStoredProductReplyTarget, ProductWorkflowError> {
        let target = self
            .binding_service
            .resolve_stored_reply_target(ResolveStoredProductReplyTargetRequest {
                scope: self.scope.clone(),
                actor: self.actor.clone(),
                reply_target_binding_ref: self.expected_target.clone(),
                access: self.access,
            })
            .await?;
        if target.adapter_id.as_str() != self.expected_adapter_id
            || target.installation_id.as_str() != self.expected_installation_id
        {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        Ok(target)
    }
}

#[async_trait]
impl ReplyTargetBindingValidator for StoredReplyTargetAuthority {
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
        self.resolve().await.map_err(|error| match error {
            ProductWorkflowError::Transient { .. } => OutboundError::Backend,
            _ => OutboundError::AccessDenied,
        })?;
        Ok(ReplyTargetBindingClaim::new(request.candidate.target))
    }
}

#[async_trait]
impl ProductOutboundTargetResolver for StoredReplyTargetAuthority {
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ironclaw_outbound::ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError> {
        if target.target() != &self.expected_target {
            return Err(ProductWorkflowError::BindingAccessDenied);
        }
        let resolved = self.resolve().await?;
        if require_direct_message && resolved.route_kind != ProductConversationRouteKind::Direct {
            return Err(ProductWorkflowError::OutboundTargetNotDirectMessage);
        }
        Ok(VerifiedProductOutboundTargetMetadata {
            external_conversation_ref: resolved.external_conversation_ref,
            external_actor_ref: None,
        })
    }
}

pub(crate) struct AllowNoProjectionAccess;

#[async_trait]
impl ThreadProjectionAccessPolicy for AllowNoProjectionAccess {
    async fn authorize_projection_access(
        &self,
        _request: ThreadProjectionAccessRequest,
    ) -> Result<ThreadProjectionAccessClaim, OutboundError> {
        Err(OutboundError::AccessDenied)
    }
}
