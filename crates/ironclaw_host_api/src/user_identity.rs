//! Host-owned provider-identity binding vocabulary for channel surfaces.
//!
//! Product adapters expose external actor ids; host-owned binding stores map
//! those provider identities to Reborn [`UserId`] values. This module contains
//! only neutral IDs, records, and store/lookup traits. Product routing and
//! concrete persistence live in owning service crates.

use crate::{AdapterInstallationId, UserId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornIdentityProviderId(String);

impl RebornIdentityProviderId {
    pub fn new(value: impl Into<String>) -> Result<Self, RebornUserIdentityBindingError> {
        let value = value.into();
        validate_identity_value("provider", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RebornIdentityProviderId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornIdentityProviderUserId(String);

impl RebornIdentityProviderUserId {
    pub fn new(value: impl Into<String>) -> Result<Self, RebornUserIdentityBindingError> {
        let value = value.into();
        validate_identity_value("provider_user_id", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RebornIdentityProviderUserId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornUserIdentityBinding {
    pub provider: RebornIdentityProviderId,
    pub provider_user_id: RebornIdentityProviderUserId,
    pub user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RebornUserIdentityBindingError {
    #[error("reborn user identity binding backend unavailable: {0}")]
    Backend(String),
    #[error("provider identity is already bound to a different reborn user")]
    ProviderIdentityAlreadyBound,
    #[error("invalid reborn user identity {field}: {reason}")]
    InvalidIdentityField {
        field: &'static str,
        reason: &'static str,
    },
}

#[async_trait::async_trait]
pub trait RebornUserIdentityBindingStore: Send + Sync {
    async fn bind_user_identity(
        &self,
        binding: RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError>;
}

#[async_trait::async_trait]
pub trait RebornUserIdentityBindingDeleteStore: Send + Sync {
    async fn delete_user_identity_bindings_for_user(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<usize, RebornUserIdentityBindingError>;
}

fn validate_identity_value(
    field: &'static str,
    value: &str,
) -> Result<(), RebornUserIdentityBindingError> {
    if value.is_empty() {
        return Err(RebornUserIdentityBindingError::InvalidIdentityField {
            field,
            reason: "must not be empty",
        });
    }
    if value.chars().any(|character| character.is_control()) {
        return Err(RebornUserIdentityBindingError::InvalidIdentityField {
            field,
            reason: "must not contain control characters",
        });
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
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

    /// Whether the given IronClaw user has any binding for `provider` — the
    /// reverse of [`Self::resolve_user_identity`]. Used to tell whether the
    /// calling user has personally connected a channel.
    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError>;

    /// Like [`Self::user_has_provider_binding`], but only counts bindings
    /// whose provider user id starts with `provider_user_id_prefix` (the
    /// installation-scoped composite key prefix). Backends that cannot
    /// enumerate bindings report unavailability instead of guessing.
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

/// Installation-scoped composite key for a provider identity binding: the
/// same external user id under two adapter installations is two bindings.
pub fn installation_scoped_provider_user_id(
    installation_id: &AdapterInstallationId,
    external_actor_id: &str,
) -> String {
    format!("{}:{external_actor_id}", installation_id.as_str())
}
