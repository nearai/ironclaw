//! Slack personal user binding service.
//!
//! The service owns the write-side boundary for "one tenant Slack app, many
//! personal Reborn user bindings". It validates that a proven Slack user came
//! from the configured tenant app installation, then delegates persistence to a
//! host-owned store port.

use std::{future::Future, pin::Pin, sync::Arc};

use ironclaw_auth::AuthFlowId;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::slack::slack_actor_identity::{
    SLACK_IDENTITY_PROVIDER, slack_user_identity_provider_user_id,
};
use crate::slack::slack_serve::{
    SlackApiAppId, SlackEnterpriseId, SlackInstallationSelector, SlackTeamId, SlackUserId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RebornIdentityProviderId(String);

impl RebornIdentityProviderId {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, RebornUserIdentityBindingError> {
        let value = value.into();
        validate_identity_value("provider", &value)?;
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RebornIdentityProviderId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RebornIdentityProviderUserId(String);

impl RebornIdentityProviderUserId {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, RebornUserIdentityBindingError> {
        let value = value.into();
        validate_identity_value("provider_user_id", &value)?;
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RebornIdentityProviderUserId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RebornUserIdentityBinding {
    pub(crate) provider: RebornIdentityProviderId,
    pub(crate) provider_user_id: RebornIdentityProviderUserId,
    pub(crate) user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackUserIdentityCleanupBinding {
    binding: RebornUserIdentityBinding,
    epoch: Option<SlackConnectionEpoch>,
}

pub(crate) struct SlackUserIdentityBindingRollback(Pin<Box<dyn Future<Output = ()> + Send>>);

impl SlackUserIdentityBindingRollback {
    pub(crate) fn new(future: impl Future<Output = ()> + Send + 'static) -> Self {
        Self(Box::pin(future))
    }

    pub(crate) fn into_future(self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        self.0
    }
}

pub(crate) struct SlackPersonalUserBindingOutcome {
    pub(crate) binding: RebornUserIdentityBinding,
    pub(crate) rollback: SlackUserIdentityBindingRollback,
}

impl SlackUserIdentityCleanupBinding {
    pub(crate) fn new(
        binding: RebornUserIdentityBinding,
        epoch: Option<SlackConnectionEpoch>,
    ) -> Self {
        Self { binding, epoch }
    }

    pub(crate) fn binding(&self) -> &RebornUserIdentityBinding {
        &self.binding
    }

    pub(crate) fn epoch(&self) -> Option<SlackConnectionEpoch> {
        self.epoch
    }
}

/// Generation stamp for one Slack connection: the OAuth flow id of the
/// callback that bound it, stamped onto the identity-binding, DM-target, and
/// conversation-pairing rows that connection produced.
///
/// This is a FENCE, not a liveness record. Attempt liveness belongs to the
/// auth-flow record; the stamp exists so the disconnect and
/// failed-connection sweeps can scope their multi-row deletions to one
/// generation (a reconnect landing mid-sweep gets a fresh stamp the sweep
/// will not touch) and so ingress can check a row against the owner's
/// currently active generation. Reusing the flow id keeps the stamp tied to
/// the already-durable authorization attempt without a second token
/// namespace. ("Epoch" survives in the name for on-disk serde compatibility
/// with rows written before the connect-attempt slot was deleted.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct SlackConnectionEpoch(AuthFlowId);

impl SlackConnectionEpoch {
    pub(crate) fn new(flow_id: AuthFlowId) -> Self {
        Self(flow_id)
    }
}

impl std::fmt::Display for SlackConnectionEpoch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Lifecycle state of an owner's connection-generation record.
///
/// `Active` is written by the callback's identity bind and is what ingress
/// authorization checks; `Disconnecting`/`Disconnected` journal the fenced
/// cleanup sweeps. `Connecting` is legacy read-compat only: it was the
/// pre-claim connect-attempt slot written at OAuth start before attempt
/// liveness moved to the auth-flow record, and is now treated everywhere as
/// a stale, never-activated generation (bindable, sweepable, never
/// authorized).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SlackConnectionState {
    Connecting,
    Active,
    Disconnecting,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackConnectionOwner {
    tenant_id: TenantId,
    user_id: UserId,
    installation_id: AdapterInstallationId,
}

/// Durable disconnect fence plus the connection generation whose derived
/// state should be cleaned. A legacy owner can need a fresh fence while its
/// identity and DM records carry no generation stamp, so these values are
/// deliberately separate. Together with the `begin_disconnect` /
/// `complete_disconnect` journal it makes a crashed sweep resumable without
/// ever deleting a fresh reconnect's differently-stamped rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SlackDisconnectFence {
    fence_epoch: SlackConnectionEpoch,
    cleanup_selector: SlackConnectionCleanupSelector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlackConnectionCleanupSelector {
    AllOwned,
    Epoch(SlackConnectionEpoch),
}

impl SlackConnectionCleanupSelector {
    pub(crate) fn epoch(self) -> Option<SlackConnectionEpoch> {
        match self {
            Self::AllOwned => None,
            Self::Epoch(epoch) => Some(epoch),
        }
    }
}

impl SlackDisconnectFence {
    pub(crate) fn new(
        fence_epoch: SlackConnectionEpoch,
        cleanup_selector: SlackConnectionCleanupSelector,
    ) -> Self {
        Self {
            fence_epoch,
            cleanup_selector,
        }
    }

    pub(crate) fn fence_epoch(self) -> SlackConnectionEpoch {
        self.fence_epoch
    }

    pub(crate) fn cleanup_selector(self) -> SlackConnectionCleanupSelector {
        self.cleanup_selector
    }
}

impl SlackConnectionOwner {
    pub(crate) fn new(
        tenant_id: TenantId,
        user_id: UserId,
        installation_id: AdapterInstallationId,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            installation_id,
        }
    }

    pub(crate) fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub(crate) fn user_id(&self) -> &UserId {
        &self.user_id
    }

    pub(crate) fn installation_id(&self) -> &AdapterInstallationId {
        &self.installation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum SlackUserBindingLifecycleError {
    #[error("slack user binding lifecycle backend unavailable: {0}")]
    Backend(String),
    #[error("slack disconnect cleanup is still in progress")]
    DisconnectInProgress,
    #[error("slack connection attempt is no longer current")]
    StaleEpoch,
}

/// Slack-specific generation-fence authority for the disconnect and
/// failed-connection sweeps. Connect-attempt liveness is NOT tracked here —
/// that is the auth-flow record's job; this store only remembers which
/// generation is active for ingress and journals multi-row cleanups so a
/// crashed sweep converges without deleting a fresh reconnect's rows.
#[async_trait::async_trait]
pub(crate) trait SlackUserBindingLifecycleStore: Send + Sync {
    async fn connection_state(
        &self,
        owner: &SlackConnectionOwner,
    ) -> Result<Option<(SlackConnectionEpoch, SlackConnectionState)>, SlackUserBindingLifecycleError>;

    async fn connection_owners_for_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<Vec<SlackConnectionOwner>, SlackUserBindingLifecycleError>;

    async fn begin_disconnect(
        &self,
        owner: &SlackConnectionOwner,
    ) -> Result<SlackDisconnectFence, SlackUserBindingLifecycleError>;

    /// Fence a failed OAuth generation from ingress while retaining enough
    /// owner/epoch state for retryable identity cleanup.
    async fn begin_failed_connection_cleanup(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
    ) -> Result<(), SlackUserBindingLifecycleError>;

    /// Settle a previously fenced failed OAuth generation after its derived
    /// identity state has been removed and verified.
    async fn complete_failed_connection_cleanup(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
    ) -> Result<(), SlackUserBindingLifecycleError>;

    async fn complete_disconnect(
        &self,
        owner: &SlackConnectionOwner,
        epoch: SlackConnectionEpoch,
    ) -> Result<(), SlackUserBindingLifecycleError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum RebornUserIdentityBindingError {
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
pub(crate) trait RebornUserIdentityBindingStore: Send + Sync {
    #[cfg(test)]
    async fn bind_user_identity(
        &self,
        binding: RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError>;

    async fn bind_user_identity_for_epoch(
        &self,
        binding: RebornUserIdentityBinding,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackUserIdentityBindingRollback, RebornUserIdentityBindingError>;
}

#[async_trait::async_trait]
pub(crate) trait RebornUserIdentityBindingDeleteStore: Send + Sync {
    async fn user_identity_bindings_for_user(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError>;

    async fn user_identity_bindings_for_user_at_epoch(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
        expected_epoch: Option<SlackConnectionEpoch>,
    ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError>;

    async fn delete_user_identity_bindings_for_user_at_epoch(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
        expected_epoch: Option<SlackConnectionEpoch>,
    ) -> Result<Vec<SlackUserIdentityCleanupBinding>, RebornUserIdentityBindingError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackPersonalBindingInstallation {
    pub(crate) tenant_id: TenantId,
    pub(crate) installation_id: AdapterInstallationId,
    pub(crate) selector: SlackInstallationSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackPersonalBindingPrincipal {
    pub(crate) tenant_id: TenantId,
    pub(crate) user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlackPersonalUserBindingRequest {
    pub(crate) installation_id: AdapterInstallationId,
    pub(crate) slack_user_id: SlackUserId,
    pub(crate) team_id: SlackTeamId,
    pub(crate) enterprise_id: Option<SlackEnterpriseId>,
    pub(crate) api_app_id: SlackApiAppId,
}

#[derive(Clone)]
pub(crate) struct SlackPersonalUserBindingService {
    installations: Arc<[SlackPersonalBindingInstallation]>,
    store: Arc<dyn RebornUserIdentityBindingStore>,
}

impl SlackPersonalUserBindingService {
    pub(crate) fn new(
        installations: impl IntoIterator<Item = SlackPersonalBindingInstallation>,
        store: Arc<dyn RebornUserIdentityBindingStore>,
    ) -> Self {
        Self {
            installations: installations.into_iter().collect::<Vec<_>>().into(),
            store,
        }
    }

    #[cfg(test)]
    pub(crate) async fn bind_personal_user(
        &self,
        principal: SlackPersonalBindingPrincipal,
        request: SlackPersonalUserBindingRequest,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        self.bind_personal_user_inner(principal, request, None)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn bind_personal_user_for_epoch(
        &self,
        principal: SlackPersonalBindingPrincipal,
        request: SlackPersonalUserBindingRequest,
        epoch: SlackConnectionEpoch,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        self.bind_personal_user_for_epoch_with_rollback(principal, request, epoch)
            .await
            .map(|outcome| outcome.binding)
    }

    pub(crate) async fn bind_personal_user_for_epoch_with_rollback(
        &self,
        principal: SlackPersonalBindingPrincipal,
        request: SlackPersonalUserBindingRequest,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackPersonalUserBindingOutcome, SlackPersonalUserBindingError> {
        self.validate_binding_request(&principal, &request)?;
        let binding = self.validated_actor_binding(
            request.installation_id,
            request.slack_user_id,
            principal.user_id,
        )?;
        let rollback = self
            .store
            .bind_user_identity_for_epoch(binding.clone(), epoch)
            .await
            .map_err(SlackPersonalUserBindingError::BindingStore)?;
        Ok(SlackPersonalUserBindingOutcome { binding, rollback })
    }

    #[cfg(test)]
    async fn bind_personal_user_inner(
        &self,
        principal: SlackPersonalBindingPrincipal,
        request: SlackPersonalUserBindingRequest,
        epoch: Option<SlackConnectionEpoch>,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        self.validate_binding_request(&principal, &request)?;
        self.bind_validated_actor(
            request.installation_id,
            request.slack_user_id,
            principal.user_id,
            epoch,
        )
        .await
    }

    fn validate_binding_request(
        &self,
        principal: &SlackPersonalBindingPrincipal,
        request: &SlackPersonalUserBindingRequest,
    ) -> Result<(), SlackPersonalUserBindingError> {
        validate_slack_id("slack user", request.slack_user_id.as_str())?;
        validate_slack_id("slack team", request.team_id.as_str())?;
        validate_optional_slack_id(
            "slack enterprise",
            request
                .enterprise_id
                .as_ref()
                .map(SlackEnterpriseId::as_str),
        )?;
        validate_slack_id("slack app", request.api_app_id.as_str())?;

        let installation = self
            .installations
            .iter()
            .find(|installation| {
                installation.tenant_id == principal.tenant_id
                    && installation.installation_id == request.installation_id
            })
            .ok_or_else(|| SlackPersonalUserBindingError::UnknownInstallation {
                tenant_id: principal.tenant_id.clone(),
                installation_id: request.installation_id.clone(),
            })?;

        if !tenant_app_selector_matches_request(&installation.selector, principal, request)? {
            return Err(
                SlackPersonalUserBindingError::SlackInstallationContextMismatch {
                    tenant_id: principal.tenant_id.clone(),
                    installation_id: request.installation_id.clone(),
                },
            );
        }

        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn bind_installation_actor(
        &self,
        principal: SlackPersonalBindingPrincipal,
        installation_id: AdapterInstallationId,
        slack_user_id: SlackUserId,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        self.validate_installation_actor(&principal, &installation_id, &slack_user_id)?;
        self.bind_validated_actor(installation_id, slack_user_id, principal.user_id, None)
            .await
    }

    #[cfg(test)]
    pub(crate) fn validate_installation_actor(
        &self,
        principal: &SlackPersonalBindingPrincipal,
        installation_id: &AdapterInstallationId,
        slack_user_id: &SlackUserId,
    ) -> Result<(), SlackPersonalUserBindingError> {
        validate_slack_id("slack user", slack_user_id.as_str())?;
        let installation = self.installation_for_principal(principal, installation_id)?;
        ensure_tenant_app_scoped(&installation.selector, principal, installation_id)
    }

    #[cfg(test)]
    fn installation_for_principal(
        &self,
        principal: &SlackPersonalBindingPrincipal,
        installation_id: &AdapterInstallationId,
    ) -> Result<&SlackPersonalBindingInstallation, SlackPersonalUserBindingError> {
        self.installations
            .iter()
            .find(|installation| {
                installation.tenant_id == principal.tenant_id
                    && installation.installation_id == *installation_id
            })
            .ok_or_else(|| SlackPersonalUserBindingError::UnknownInstallation {
                tenant_id: principal.tenant_id.clone(),
                installation_id: installation_id.clone(),
            })
    }

    #[cfg(test)]
    async fn bind_validated_actor(
        &self,
        installation_id: AdapterInstallationId,
        slack_user_id: SlackUserId,
        user_id: UserId,
        epoch: Option<SlackConnectionEpoch>,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new(SLACK_IDENTITY_PROVIDER)?,
            provider_user_id: RebornIdentityProviderUserId::new(
                slack_user_identity_provider_user_id(&installation_id, slack_user_id.as_str()),
            )?,
            user_id,
        };
        match epoch {
            Some(epoch) => self
                .store
                .bind_user_identity_for_epoch(binding.clone(), epoch)
                .await
                .map(|_| ()),
            None => self.store.bind_user_identity(binding.clone()).await,
        }
        .map_err(SlackPersonalUserBindingError::BindingStore)?;
        Ok(binding)
    }

    fn validated_actor_binding(
        &self,
        installation_id: AdapterInstallationId,
        slack_user_id: SlackUserId,
        user_id: UserId,
    ) -> Result<RebornUserIdentityBinding, RebornUserIdentityBindingError> {
        Ok(RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new(SLACK_IDENTITY_PROVIDER)?,
            provider_user_id: RebornIdentityProviderUserId::new(
                slack_user_identity_provider_user_id(&installation_id, slack_user_id.as_str()),
            )?,
            user_id,
        })
    }
}

#[async_trait::async_trait]
pub(crate) trait SlackPersonalUserBinder: Send + Sync + std::fmt::Debug {
    async fn bind_personal_user_for_epoch(
        &self,
        principal: SlackPersonalBindingPrincipal,
        request: SlackPersonalUserBindingRequest,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackPersonalUserBindingOutcome, SlackPersonalUserBindingError>;
}

#[async_trait::async_trait]
impl SlackPersonalUserBinder for SlackPersonalUserBindingService {
    async fn bind_personal_user_for_epoch(
        &self,
        principal: SlackPersonalBindingPrincipal,
        request: SlackPersonalUserBindingRequest,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackPersonalUserBindingOutcome, SlackPersonalUserBindingError> {
        SlackPersonalUserBindingService::bind_personal_user_for_epoch_with_rollback(
            self, principal, request, epoch,
        )
        .await
    }
}

impl std::fmt::Debug for SlackPersonalUserBindingService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackPersonalUserBindingService")
            .field("installations", &self.installations)
            .field("store", &"Arc<dyn RebornUserIdentityBindingStore>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum SlackPersonalUserBindingError {
    #[error(
        "slack installation is not configured for tenant {tenant_id} and installation {installation_id}"
    )]
    UnknownInstallation {
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
    },
    #[error(
        "slack installation {installation_id} for tenant {tenant_id} is install-user scoped; personal binding requires a tenant app installation"
    )]
    InstallationNotTenantScoped {
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
    },
    #[error("slack proof does not match installation {installation_id} for tenant {tenant_id}")]
    SlackInstallationContextMismatch {
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
    },
    #[error("invalid {field} id: {reason}")]
    InvalidSlackId {
        field: &'static str,
        reason: &'static str,
    },
    #[error(transparent)]
    BindingStore(#[from] RebornUserIdentityBindingError),
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

fn validate_slack_id(
    field: &'static str,
    value: &str,
) -> Result<(), SlackPersonalUserBindingError> {
    if value.is_empty() {
        return Err(SlackPersonalUserBindingError::InvalidSlackId {
            field,
            reason: "must not be empty",
        });
    }
    if value.chars().any(|character| character.is_control()) {
        return Err(SlackPersonalUserBindingError::InvalidSlackId {
            field,
            reason: "must not contain control characters",
        });
    }
    Ok(())
}

fn validate_optional_slack_id(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), SlackPersonalUserBindingError> {
    match value {
        Some(value) => validate_slack_id(field, value),
        None => Ok(()),
    }
}

fn tenant_app_selector_matches_request(
    selector: &SlackInstallationSelector,
    principal: &SlackPersonalBindingPrincipal,
    request: &SlackPersonalUserBindingRequest,
) -> Result<bool, SlackPersonalUserBindingError> {
    ensure_tenant_app_scoped(selector, principal, &request.installation_id)?;
    match selector {
        SlackInstallationSelector::AppTeam {
            api_app_id,
            team_id,
        } => Ok(team_id == &request.team_id && api_app_id == &request.api_app_id),
        SlackInstallationSelector::AppEnterpriseTeam {
            api_app_id,
            enterprise_id,
            team_id,
        } => Ok(team_id == &request.team_id
            && api_app_id == &request.api_app_id
            && request.enterprise_id.as_ref() == Some(enterprise_id)),
        _ => unreachable!("ensure_tenant_app_scoped rejects non-AppTeam selectors"),
    }
}

fn ensure_tenant_app_scoped(
    selector: &SlackInstallationSelector,
    principal: &SlackPersonalBindingPrincipal,
    installation_id: &AdapterInstallationId,
) -> Result<(), SlackPersonalUserBindingError> {
    match selector {
        SlackInstallationSelector::AppTeam { .. }
        | SlackInstallationSelector::AppEnterpriseTeam { .. } => Ok(()),
        _ => Err(SlackPersonalUserBindingError::InstallationNotTenantScoped {
            tenant_id: principal.tenant_id.clone(),
            installation_id: installation_id.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[tokio::test]
    async fn bind_personal_user_writes_installation_scoped_slack_identity() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );

        let binding = service
            .bind_personal_user(
                principal("tenant-alpha", "user:alice"),
                request("install-alpha"),
            )
            .await
            .expect("binding succeeds");

        assert_eq!(
            binding,
            RebornUserIdentityBinding {
                provider: provider("slack"),
                provider_user_id: provider_user_id("install-alpha:U123"),
                user_id: user("user:alice"),
            }
        );
        assert_eq!(store.bindings(), vec![binding]);
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_wrong_tenant_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );

        let error = service
            .bind_personal_user(
                principal("tenant-beta", "user:alice"),
                request("install-alpha"),
            )
            .await
            .expect_err("wrong tenant is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::UnknownInstallation { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_wrong_installation_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );

        let error = service
            .bind_personal_user(
                principal("tenant-alpha", "user:alice"),
                request("install-beta"),
            )
            .await
            .expect_err("wrong installation is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::UnknownInstallation { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_wrong_app_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );
        let mut request = request("install-alpha");
        request.api_app_id = SlackApiAppId::new("A-other");

        let error = service
            .bind_personal_user(principal("tenant-alpha", "user:alice"), request)
            .await
            .expect_err("wrong app is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::SlackInstallationContextMismatch { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_wrong_team_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );
        let mut request = request("install-alpha");
        request.team_id = SlackTeamId::new("T-other");

        let error = service
            .bind_personal_user(principal("tenant-alpha", "user:alice"), request)
            .await
            .expect_err("wrong team is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::SlackInstallationContextMismatch { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_wrong_enterprise_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::app_enterprise_team("A-app", "E-good", "T-team"),
            store.clone(),
        );
        let mut request = request("install-alpha");
        request.enterprise_id = Some(SlackEnterpriseId::new("E-other"));

        let error = service
            .bind_personal_user(principal("tenant-alpha", "user:alice"), request)
            .await
            .expect_err("wrong enterprise is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::SlackInstallationContextMismatch { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_non_app_scoped_installation_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(SlackInstallationSelector::team("T-team"), store.clone());

        let error = service
            .bind_personal_user(
                principal("tenant-alpha", "user:alice"),
                request("install-alpha"),
            )
            .await
            .expect_err("team-scoped app is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::InstallationNotTenantScoped { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_install_user_scoped_installation_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::team("T-team").with_install_user_id("U-install"),
            store.clone(),
        );

        let error = service
            .bind_personal_user(
                principal("tenant-alpha", "user:alice"),
                request("install-alpha"),
            )
            .await
            .expect_err("install-user scoped app is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::InstallationNotTenantScoped { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_propagates_store_error() {
        let store = Arc::new(RecordingBindingStore::with_error(
            RebornUserIdentityBindingError::Backend("store down".into()),
        ));
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );

        let error = service
            .bind_personal_user(
                principal("tenant-alpha", "user:alice"),
                request("install-alpha"),
            )
            .await
            .expect_err("store error is propagated");

        assert_eq!(
            error,
            SlackPersonalUserBindingError::BindingStore(RebornUserIdentityBindingError::Backend(
                "store down".into()
            ))
        );
        assert_eq!(
            store.bindings(),
            vec![RebornUserIdentityBinding {
                provider: provider("slack"),
                provider_user_id: provider_user_id("install-alpha:U123"),
                user_id: user("user:alice"),
            }]
        );
    }

    #[tokio::test]
    async fn bind_installation_actor_rejects_wrong_tenant_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );

        let error = service
            .bind_installation_actor(
                principal("tenant-beta", "user:alice"),
                installation("install-alpha"),
                SlackUserId::new("U123"),
            )
            .await
            .expect_err("wrong tenant is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::UnknownInstallation { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_installation_actor_rejects_unknown_installation_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );

        let error = service
            .bind_installation_actor(
                principal("tenant-alpha", "user:alice"),
                installation("install-beta"),
                SlackUserId::new("U123"),
            )
            .await
            .expect_err("unknown installation is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::UnknownInstallation { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_installation_actor_rejects_non_app_scoped_installation_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(SlackInstallationSelector::team("T-team"), store.clone());

        let error = service
            .bind_installation_actor(
                principal("tenant-alpha", "user:alice"),
                installation("install-alpha"),
                SlackUserId::new("U123"),
            )
            .await
            .expect_err("team-scoped app is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::InstallationNotTenantScoped { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_installation_actor_rejects_invalid_slack_user_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );

        let error = service
            .bind_installation_actor(
                principal("tenant-alpha", "user:alice"),
                installation("install-alpha"),
                SlackUserId::new("bad\nuser"),
            )
            .await
            .expect_err("invalid slack user is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::InvalidSlackId { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_installation_actor_propagates_store_error() {
        let store = Arc::new(RecordingBindingStore::with_error(
            RebornUserIdentityBindingError::Backend("store down".into()),
        ));
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );

        let error = service
            .bind_installation_actor(
                principal("tenant-alpha", "user:alice"),
                installation("install-alpha"),
                SlackUserId::new("U123"),
            )
            .await
            .expect_err("store error is propagated");

        assert_eq!(
            error,
            SlackPersonalUserBindingError::BindingStore(RebornUserIdentityBindingError::Backend(
                "store down".into()
            ))
        );
        assert_eq!(
            store.bindings(),
            vec![RebornUserIdentityBinding {
                provider: provider("slack"),
                provider_user_id: provider_user_id("install-alpha:U123"),
                user_id: user("user:alice"),
            }]
        );
    }

    fn service(
        selector: SlackInstallationSelector,
        store: Arc<dyn RebornUserIdentityBindingStore>,
    ) -> SlackPersonalUserBindingService {
        SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: tenant("tenant-alpha"),
                installation_id: installation("install-alpha"),
                selector,
            }],
            store,
        )
    }

    fn principal(tenant_id: &str, user_id: &str) -> SlackPersonalBindingPrincipal {
        SlackPersonalBindingPrincipal {
            tenant_id: tenant(tenant_id),
            user_id: user(user_id),
        }
    }

    fn request(installation_id: &str) -> SlackPersonalUserBindingRequest {
        SlackPersonalUserBindingRequest {
            installation_id: installation(installation_id),
            slack_user_id: SlackUserId::new("U123"),
            team_id: SlackTeamId::new("T-team"),
            enterprise_id: None,
            api_app_id: SlackApiAppId::new("A-app"),
        }
    }

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).expect("valid tenant id")
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("valid user id")
    }

    fn installation(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).expect("valid installation id")
    }

    fn provider(value: &str) -> RebornIdentityProviderId {
        RebornIdentityProviderId::new(value).expect("valid provider")
    }

    fn provider_user_id(value: &str) -> RebornIdentityProviderUserId {
        RebornIdentityProviderUserId::new(value).expect("valid provider user id")
    }

    #[derive(Default)]
    struct RecordingBindingStore {
        bindings: Mutex<Vec<RebornUserIdentityBinding>>,
        error: Option<RebornUserIdentityBindingError>,
    }

    impl RecordingBindingStore {
        fn with_error(error: RebornUserIdentityBindingError) -> Self {
            Self {
                bindings: Mutex::new(Vec::new()),
                error: Some(error),
            }
        }

        fn bindings(&self) -> Vec<RebornUserIdentityBinding> {
            self.bindings
                .lock()
                .expect("bindings lock should not be poisoned")
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityBindingStore for RecordingBindingStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            self.bindings
                .lock()
                .expect("bindings lock should not be poisoned")
                .push(binding);
            match &self.error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        async fn bind_user_identity_for_epoch(
            &self,
            binding: RebornUserIdentityBinding,
            _epoch: SlackConnectionEpoch,
        ) -> Result<SlackUserIdentityBindingRollback, RebornUserIdentityBindingError> {
            self.bind_user_identity(binding).await?;
            Ok(SlackUserIdentityBindingRollback::new(async {}))
        }
    }
}
