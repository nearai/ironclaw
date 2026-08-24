use super::*;

use ironclaw_host_api::{execution_policy::TurnExecutionPolicy, resource::ResourceScope};

#[async_trait::async_trait]
pub(crate) trait TriggerExecutionPolicyPreflight: Send + Sync {
    async fn validate(
        &self,
        scope: &ResourceScope,
        policy: &TurnExecutionPolicy,
    ) -> Result<(), TriggerError>;
}

pub(crate) struct TriggerCreatorPairingHook {
    pub(super) scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(super) conversations: tokio::sync::OnceCell<RebornFilesystemConversationServices>,
    pub(super) execution_preflight: tokio::sync::OnceCell<Arc<dyn TriggerExecutionPolicyPreflight>>,
}

impl TriggerCreatorPairingHook {
    pub(crate) fn bind_execution_preflight(
        &self,
        preflight: Arc<dyn TriggerExecutionPolicyPreflight>,
    ) -> Result<(), TriggerError> {
        self.execution_preflight
            .set(preflight)
            .map_err(|_| TriggerError::Backend {
                reason: "trigger execution preflight was already bound".to_string(),
            })
    }
}

#[async_trait::async_trait]
impl TriggerCreateHook for TriggerCreatorPairingHook {
    async fn validate_execution_policy(
        &self,
        scope: &ResourceScope,
        policy: &TurnExecutionPolicy,
    ) -> Result<(), TriggerError> {
        // An unbound preflight is a composition wiring fault, not a defect in
        // the caller's contract — `Backend` (like double binding above), never
        // `InvalidRecord`, which would tell the caller to fix a valid contract.
        let preflight = self
            .execution_preflight
            .get()
            .ok_or_else(|| TriggerError::Backend {
                reason: "trigger execution preflight is not bound".to_string(),
            })?;
        preflight.validate(scope, policy).await
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
