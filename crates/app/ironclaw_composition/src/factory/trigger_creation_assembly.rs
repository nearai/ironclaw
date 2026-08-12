use super::*;

use ironclaw_turns::TurnError;

/// Late-bindable [`AgentTurnRuntimePort`] view over the runtime's own turn
/// state — the run-state source every caller-initiated "which run is this"
/// lookup reads (today `builtin.outbound_deliver`'s same-origin check).
/// Production installs the runtime's own turn state and never repoints it; a
/// `test-support` harness repoints it (see
/// `rebind_standalone_trigger_source_turn_state_for_test`) so those lookups
/// can see runs recorded in the harness's own store.
pub(crate) struct LateBoundAgentTurnRuntime {
    source: Arc<std::sync::RwLock<Arc<dyn ironclaw_turns::AgentTurnRuntimePort>>>,
}

impl LateBoundAgentTurnRuntime {
    pub(crate) fn new(
        source: Arc<std::sync::RwLock<Arc<dyn ironclaw_turns::AgentTurnRuntimePort>>>,
    ) -> Self {
        Self { source }
    }

    fn current(&self) -> Result<Arc<dyn ironclaw_turns::AgentTurnRuntimePort>, TurnError> {
        self.source
            .read()
            .map(|source| Arc::clone(&*source))
            .map_err(|_| TurnError::Unavailable {
                reason: "late-bound turn-state source lock is unavailable".to_string(),
            })
    }
}

#[async_trait::async_trait]
impl ironclaw_turns::AgentTurnRuntimePort for LateBoundAgentTurnRuntime {
    async fn submit_turn(
        &self,
        request: ironclaw_turns::SubmitTurnRequest,
        admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        run_profile_resolver: &dyn ironclaw_loop_contracts::RunProfileResolver,
    ) -> Result<ironclaw_turns::SubmitTurnResponse, TurnError> {
        self.current()?
            .submit_turn(request, admission_policy, run_profile_resolver)
            .await
    }

    async fn resume_turn(
        &self,
        request: ironclaw_turns::ResumeTurnRequest,
    ) -> Result<ironclaw_turns::ResumeTurnResponse, TurnError> {
        self.current()?.resume_turn(request).await
    }

    async fn retry_turn(
        &self,
        request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        self.current()?.retry_turn(request).await
    }

    async fn request_cancel(
        &self,
        request: ironclaw_turns::CancelRunRequest,
    ) -> Result<ironclaw_turns::CancelRunResponse, TurnError> {
        self.current()?.request_cancel(request).await
    }

    async fn get_run_state(
        &self,
        request: ironclaw_turns::GetRunStateRequest,
    ) -> Result<ironclaw_turns::TurnRunState, TurnError> {
        self.current()?.get_run_state(request).await
    }
}

pub(super) struct TriggerCreatorPairingHook {
    pub(super) scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(super) conversations: tokio::sync::OnceCell<RebornFilesystemConversationServices>,
}

#[async_trait::async_trait]
impl TriggerCreateHook for TriggerCreatorPairingHook {
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
        // A trigger's creator actor has no channel display name.
        None::<String>,
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
