//! Channel-agnostic external-identity lookup port.
//!
//! Maps a channel provider identity (`slack`, `telegram`, …) plus an
//! installation-scoped provider user id to a Reborn [`UserId`]. Originally
//! Slack-owned inside composition; moved here unchanged so every channel
//! host — composition's Slack module and standalone channel host crates —
//! consumes one definition. Composition's Slack module re-exports these
//! names for its existing consumers.

use ironclaw_conversations::ExternalActorBindingEpoch;
use ironclaw_host_api::UserId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RebornUserIdentityLookupError {
    #[error("reborn user identity backend unavailable: {0}")]
    Backend(String),
    #[error("stored user identity is invalid: {0}")]
    InvalidUserId(String),
}

#[async_trait::async_trait]
pub trait RebornUserIdentityLookup: Send + Sync {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError>;

    async fn resolve_user_identity_with_binding_epoch(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<(UserId, Option<ExternalActorBindingEpoch>)>, RebornUserIdentityLookupError>
    {
        self.resolve_user_identity(provider, provider_user_id)
            .await
            .map(|resolved| resolved.map(|user_id| (user_id, None)))
    }

    async fn user_identity_binding_epoch_is_current(
        &self,
        provider: &str,
        provider_user_id: &str,
        expected_user_id: &UserId,
        expected_epoch: &ExternalActorBindingEpoch,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        Ok(self
            .resolve_user_identity_with_binding_epoch(provider, provider_user_id)
            .await?
            .is_some_and(|(user_id, epoch)| {
                user_id == *expected_user_id && epoch.as_ref() == Some(expected_epoch)
            }))
    }

    /// Whether the given IronClaw user has any binding for `provider` — the
    /// reverse of [`RebornUserIdentityLookup::resolve_user_identity`]. Used to
    /// tell whether the calling user has personally connected a channel.
    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError>;

    /// Whether the given IronClaw user has a provider binding whose provider
    /// user id starts with `provider_user_id_prefix`. Channel connection state
    /// uses this for installation-scoped providers, where a user bound in one
    /// installation must not satisfy setup in another.
    async fn user_has_provider_binding_with_provider_user_id_prefix(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        if provider_user_id_prefix.is_none() {
            return self.user_has_provider_binding(provider, user_id).await;
        }
        Err(RebornUserIdentityLookupError::Backend(
            "scoped provider binding lookup is unavailable".to_string(),
        ))
    }
}
