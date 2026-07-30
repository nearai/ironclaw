use super::*;

/// delivery target registry: the id must resolve for the trigger creator (the
/// same ownership check the delivery layer applies at fire time). Fails
/// closed when no provider is registered or the id is unknown/foreign.
pub(super) async fn validate_trigger_delivery_target_against_registry(
    registry: &crate::outbound::MutableOutboundDeliveryTargetRegistry,
    scope: &ironclaw_host_api::resource::ResourceScope,
    target: &ironclaw_triggers::TriggerDeliveryTargetId,
) -> Result<(), TriggerError> {
    let invalid = |reason: String| TriggerError::InvalidRecord {
        kind: ironclaw_triggers::TriggerRecordValidationKind::DeliveryTargetInvalid,
        reason,
    };
    let target_id =
        crate::outbound::OutboundDeliveryTargetId::new(target.as_str()).map_err(|error| {
            tracing::debug!(
                target = "ironclaw::reborn::trigger_create",
                %error,
                "per-trigger delivery target id failed outbound target id validation"
            );
            invalid("delivery target id is not a valid outbound target id".to_string())
        })?;
    let caller = crate::outbound::OutboundDeliveryTargetScope::new(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
    );
    use crate::outbound::OutboundDeliveryTargetProvider as _;
    match registry
        .resolve_outbound_delivery_target(&caller, &target_id)
        .await
    {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(invalid(
            "delivery target is not available to this caller".to_string(),
        )),
        Err(error) => {
            tracing::warn!(
                target = "ironclaw::reborn::trigger_create",
                %error,
                "outbound delivery target lookup failed during trigger create validation"
            );
            Err(TriggerError::Backend {
                reason: "outbound delivery target lookup unavailable".to_string(),
            })
        }
    }
}

#[async_trait::async_trait]
pub(crate) trait TriggerSourceReplyTarget: Send + Sync {
    async fn reply_target(
        &self,
        request: ironclaw_turns::GetRunStateRequest,
    ) -> Result<ironclaw_turns::ReplyTargetBindingRef, ironclaw_turns::TurnError>;
}

pub(crate) struct TurnStateTriggerSourceReplyTarget {
    turn_state: Arc<dyn ironclaw_turns::AgentTurnRuntimePort>,
}

impl TurnStateTriggerSourceReplyTarget {
    pub(crate) fn new(turn_state: Arc<dyn ironclaw_turns::AgentTurnRuntimePort>) -> Self {
        Self { turn_state }
    }
}

#[async_trait::async_trait]
impl TriggerSourceReplyTarget for TurnStateTriggerSourceReplyTarget {
    async fn reply_target(
        &self,
        request: GetRunStateRequest,
    ) -> Result<ironclaw_turns::ReplyTargetBindingRef, ironclaw_turns::TurnError> {
        self.turn_state
            .get_run_state(request)
            .await
            .map(|state| state.reply_target_binding_ref)
    }
}

pub(super) struct TriggerCreatorPairingHook {
    pub(super) outbound_delivery_targets:
        Arc<crate::outbound::MutableOutboundDeliveryTargetRegistry>,
    pub(super) source_reply_target: Arc<std::sync::RwLock<Arc<dyn TriggerSourceReplyTarget>>>,
    pub(super) scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(super) conversations: tokio::sync::OnceCell<RebornFilesystemConversationServices>,
}

#[async_trait::async_trait]
impl TriggerCreateHook for TriggerCreatorPairingHook {
    async fn resolve_implicit_delivery_target(
        &self,
        scope: &ironclaw_host_api::resource::ResourceScope,
        run_id: Option<RunId>,
    ) -> Result<Option<ironclaw_triggers::TriggerDeliveryTargetId>, TriggerError> {
        resolve_current_run_delivery_target(
            &self.source_reply_target,
            &self.outbound_delivery_targets,
            scope,
            run_id,
        )
        .await
    }

    async fn validate_delivery_target(
        &self,
        scope: &ironclaw_host_api::resource::ResourceScope,
        target: &ironclaw_triggers::TriggerDeliveryTargetId,
    ) -> Result<(), TriggerError> {
        validate_trigger_delivery_target_against_registry(
            &self.outbound_delivery_targets,
            scope,
            target,
        )
        .await
    }

    async fn after_trigger_persisted(&self, record: &TriggerRecord) -> Result<(), TriggerError> {
        let filesystem = Arc::clone(&self.scoped_filesystem);
        let conversations = self
            .conversations
            .get_or_try_init(|| async move {
                RebornFilesystemConversationServices::new(filesystem).await
            })
            .await
            .map_err(|error| {
                trigger_pairing_error(TriggerPairingFailureSource::ConversationInit, error)
            })?;
        pair_trigger_creator(conversations, record).await
    }
}

async fn resolve_current_run_delivery_target(
    source_reply_target: &std::sync::RwLock<Arc<dyn TriggerSourceReplyTarget>>,
    registry: &crate::outbound::MutableOutboundDeliveryTargetRegistry,
    scope: &ResourceScope,
    run_id: Option<RunId>,
) -> Result<Option<ironclaw_triggers::TriggerDeliveryTargetId>, TriggerError> {
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    let Some(thread_id) = scope.thread_id.clone() else {
        return Ok(None);
    };
    let turn_scope = TurnScope::new_with_owner(
        scope.tenant_id.clone(),
        scope.agent_id.clone(),
        scope.project_id.clone(),
        thread_id,
        Some(scope.user_id.clone()),
    );
    let source_reply_target = source_reply_target
        .read()
        .map(|source| Arc::clone(&*source))
        .map_err(|error| {
            tracing::warn!(
                target = "ironclaw::reborn::trigger_create",
                error = ?error,
                "source reply-target resolver lock is unavailable"
            );
            TriggerError::Backend {
                reason: "source reply-target resolver unavailable".to_string(),
            }
        })?;
    let reply_target = source_reply_target
        .reply_target(GetRunStateRequest {
            scope: turn_scope,
            run_id: ironclaw_turns::TurnRunId::from_uuid(run_id.as_uuid()),
        })
        .await
        .map_err(|error| {
            tracing::warn!(
                target = "ironclaw::reborn::trigger_create",
                %error,
                %run_id,
                "source run lookup failed during implicit trigger delivery-target resolution"
            );
            TriggerError::Backend {
                reason: "source run lookup unavailable".to_string(),
            }
        })?;
    let caller = crate::outbound::OutboundDeliveryTargetScope::new(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
    );
    use crate::outbound::OutboundDeliveryTargetProvider as _;
    let entry = registry
        .resolve_reply_target_binding(&caller, &reply_target)
        .await
        .map_err(|error| {
            tracing::warn!(
                target = "ironclaw::reborn::trigger_create",
                %error,
                %run_id,
                "outbound target lookup failed during implicit trigger delivery-target resolution"
            );
            TriggerError::Backend {
                reason: "outbound delivery target lookup unavailable".to_string(),
            }
        })?;
    entry
        .map(|entry| {
            ironclaw_triggers::TriggerDeliveryTargetId::new(
                entry.summary.target_id.as_str().to_string(),
            )
            .map_err(|reason| TriggerError::InvalidRecord {
                kind: ironclaw_triggers::TriggerRecordValidationKind::DeliveryTargetInvalid,
                reason,
            })
        })
        .transpose()
}

pub(super) async fn pair_trigger_creator(
    pairing: &dyn ConversationActorPairingService,
    record: &TriggerRecord,
) -> Result<(), TriggerError> {
    let adapter_kind = AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).map_err(|error| {
        trigger_pairing_error(TriggerPairingFailureSource::TypedIdentity, error)
    })?;
    let adapter_installation_id =
        AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID).map_err(|error| {
            trigger_pairing_error(TriggerPairingFailureSource::TypedIdentity, error)
        })?;
    let external_actor_ref = ExternalActorRef::new(
        TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE,
        record.creator_user_id.as_str(),
    )
    .map_err(|error| trigger_pairing_error(TriggerPairingFailureSource::TypedIdentity, error))?;
    pairing
        .pair_external_actor(
            record.tenant_id.clone(),
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            record.creator_user_id.clone(),
        )
        .await
        .map_err(|error| trigger_pairing_error(TriggerPairingFailureSource::ActorPairing, error))
}

enum TriggerPairingFailureSource {
    TypedIdentity,
    ConversationInit,
    ActorPairing,
}

impl TriggerPairingFailureSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::TypedIdentity => "typed_identity",
            Self::ConversationInit => "conversation_init",
            Self::ActorPairing => "actor_pairing",
        }
    }
}

fn trigger_pairing_error(
    source: TriggerPairingFailureSource,
    _error: impl std::fmt::Display,
) -> TriggerError {
    tracing::debug!(
        error_kind = "pairing_failure",
        error_source = source.as_str(),
        "trigger creator actor pairing failed"
    );
    TriggerError::Backend {
        reason: "trigger creator actor pairing failed".to_string(),
    }
}
