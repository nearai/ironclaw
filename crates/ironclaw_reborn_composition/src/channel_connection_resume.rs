//! Local-dev read model for the channel-connection gate resume.
//!
//! Enumerates the caller's runs currently `BlockedAuth` on a
//! `RuntimeCredentialAccountSetup::ChannelPairing { channel }` gate, straight
//! from the durable turn-state snapshot. Unlike the OAuth
//! `LocalDevAuthInteractionReadModel`, a channel-pairing gate has no backing
//! `AuthFlowRecord`, so this read model is driven purely by the blocked
//! turn-run records plus their persisted credential requirements.
//!
//! Scope safety: the scan is bounded to the caller's `tenant_id` + explicit
//! owner `user_id`. A run with any other owner (a different WebUI user, a shared
//! team subject) is never returned, so the resume service can never touch
//! another caller's parked run.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement};
use ironclaw_product_workflow::{
    ChannelConnectionResumeReadModel, ChannelConnectionResumeScope, ChannelConnectionResumeService,
    ChannelPairingBlockedRun, DefaultChannelConnectionResumeService, ProductWorkflowError,
};
use ironclaw_turns::{TurnCoordinator, TurnPersistenceSnapshot, TurnStatus};

use crate::factory::LocalDevTurnStateStore;

/// Compose the channel-connection gate resume service from the runtime's durable
/// turn-state snapshot source and turn coordinator. Shared by the dynamic and
/// static Slack host-beta mount builders so both wire the same resume behavior.
pub(crate) fn build_channel_connection_resume_service(
    turn_state: Arc<LocalDevTurnStateStore>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
) -> Arc<dyn ChannelConnectionResumeService> {
    Arc::new(DefaultChannelConnectionResumeService::new(
        Arc::new(LocalDevChannelConnectionResumeReadModel::new(turn_state)),
        turn_coordinator,
    ))
}

pub(crate) struct LocalDevChannelConnectionResumeReadModel {
    turn_state: Arc<LocalDevTurnStateStore>,
}

impl LocalDevChannelConnectionResumeReadModel {
    pub(crate) fn new(turn_state: Arc<LocalDevTurnStateStore>) -> Self {
        Self { turn_state }
    }

    async fn snapshot(&self) -> Result<TurnPersistenceSnapshot, ProductWorkflowError> {
        // The durable filesystem store returns an async `Result`; the in-memory
        // authority (no-DB builds, or any build with `inmemory-turn-state`)
        // returns a sync infallible snapshot. Mirrors
        // `LocalDevAuthInteractionReadModel::snapshot`.
        #[cfg(all(
            any(feature = "libsql", feature = "postgres"),
            not(feature = "inmemory-turn-state")
        ))]
        {
            self.turn_state
                .persistence_snapshot()
                .await
                .map_err(|error| {
                    tracing::debug!(
                        %error,
                        "channel-connection resume read model could not read turn persistence snapshot"
                    );
                    resume_read_model_unavailable()
                })
        }
        #[cfg(any(
            feature = "inmemory-turn-state",
            not(any(feature = "libsql", feature = "postgres"))
        ))]
        {
            Ok(self.turn_state.persistence_snapshot())
        }
    }
}

#[async_trait]
impl ChannelConnectionResumeReadModel for LocalDevChannelConnectionResumeReadModel {
    async fn channel_pairing_blocked_runs(
        &self,
        scope: &ChannelConnectionResumeScope,
        channel: &str,
    ) -> Result<Vec<ChannelPairingBlockedRun>, ProductWorkflowError> {
        let snapshot = self.snapshot().await?;
        let mut resumable = Vec::new();
        for run in &snapshot.runs {
            if run.status != TurnStatus::BlockedAuth {
                continue;
            }
            // Strict caller scoping: same tenant and same explicit owner user.
            if run.scope.tenant_id != scope.tenant_id
                || run.scope.explicit_owner_user_id() != Some(&scope.user_id)
            {
                continue;
            }
            let Some(gate_ref) = run.gate_ref.clone() else {
                continue;
            };
            if !run
                .credential_requirements
                .iter()
                .any(|requirement| matches_channel_pairing(requirement, channel))
            {
                continue;
            }
            // The run record does not carry the actor; join it from the parent
            // turn record. A blocked run without its turn is a persistence
            // integrity fault, not an empty result — fail loud.
            let Some(actor) = snapshot
                .turns
                .iter()
                .find(|turn| turn.turn_id == run.turn_id)
                .map(|turn| turn.actor.clone())
            else {
                tracing::warn!(
                    run_id = %run.run_id,
                    "channel-connection resume read model found a blocked run with no parent turn record"
                );
                return Err(ProductWorkflowError::Transient {
                    reason: "blocked channel-pairing run is missing its turn actor".to_string(),
                });
            };
            resumable.push(ChannelPairingBlockedRun {
                scope: run.scope.clone(),
                actor,
                run_id: run.run_id,
                gate_ref,
                source_binding_ref: run.source_binding_ref.clone(),
                reply_target_binding_ref: run.reply_target_binding_ref.clone(),
            });
        }
        resumable.sort_by_key(|run| run.run_id.as_uuid());
        Ok(resumable)
    }
}

fn matches_channel_pairing(requirement: &RuntimeCredentialAuthRequirement, channel: &str) -> bool {
    match &requirement.setup {
        RuntimeCredentialAccountSetup::ChannelPairing {
            channel: requirement_channel,
        } => requirement_channel.trim().eq_ignore_ascii_case(channel),
        RuntimeCredentialAccountSetup::ManualToken
        | RuntimeCredentialAccountSetup::OAuth { .. } => false,
    }
}

// Only the durable snapshot branch above can fail; the in-memory authority
// returns an infallible snapshot. Gate this helper with the same cfg as that
// call site so it isn't compiled (and flagged dead) under `inmemory-turn-state`
// or no-DB builds.
#[cfg(all(
    any(feature = "libsql", feature = "postgres"),
    not(feature = "inmemory-turn-state")
))]
fn resume_read_model_unavailable() -> ProductWorkflowError {
    ProductWorkflowError::Transient {
        reason: "channel-connection resume read model is unavailable".to_string(),
    }
}
