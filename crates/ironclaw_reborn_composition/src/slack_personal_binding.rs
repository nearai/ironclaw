//! Slack personal user binding service.
//!
//! The service owns the write-side boundary for "one tenant Slack app, many
//! personal Reborn user bindings". It validates that a proven Slack user came
//! from the configured tenant app installation, then delegates persistence to a
//! host-owned store port.

use std::sync::Arc;

use ironclaw_host_api::{TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use thiserror::Error;

use crate::slack_actor_identity::{SLACK_IDENTITY_PROVIDER, slack_user_identity_provider_user_id};
use crate::slack_serve::{
    SlackApiAppId, SlackEnterpriseId, SlackInstallationSelector, SlackTeamId, SlackUserId,
};

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

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RebornUserIdentityBindingError {
    #[error("reborn user identity binding backend unavailable: {0}")]
    Backend(String),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackPersonalBindingInstallation {
    pub tenant_id: TenantId,
    pub installation_id: AdapterInstallationId,
    pub selector: SlackInstallationSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackPersonalBindingPrincipal {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackPersonalUserBindingRequest {
    pub installation_id: AdapterInstallationId,
    pub slack_user_id: SlackUserId,
    pub team_id: SlackTeamId,
    pub enterprise_id: Option<SlackEnterpriseId>,
    pub api_app_id: SlackApiAppId,
}

#[derive(Clone)]
pub struct SlackPersonalUserBindingService {
    installations: Arc<[SlackPersonalBindingInstallation]>,
    store: Arc<dyn RebornUserIdentityBindingStore>,
}

impl SlackPersonalUserBindingService {
    pub fn new(
        installations: impl IntoIterator<Item = SlackPersonalBindingInstallation>,
        store: Arc<dyn RebornUserIdentityBindingStore>,
    ) -> Self {
        Self {
            installations: installations.into_iter().collect::<Vec<_>>().into(),
            store,
        }
    }

    pub async fn bind_personal_user(
        &self,
        principal: SlackPersonalBindingPrincipal,
        request: SlackPersonalUserBindingRequest,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
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

        if !tenant_app_selector_matches_request(&installation.selector, &principal, &request)? {
            return Err(
                SlackPersonalUserBindingError::SlackInstallationContextMismatch {
                    tenant_id: principal.tenant_id.clone(),
                    installation_id: request.installation_id.clone(),
                },
            );
        }

        self.bind_validated_actor(
            request.installation_id,
            request.slack_user_id,
            principal.user_id,
        )
        .await
    }

    pub async fn bind_installation_actor(
        &self,
        principal: SlackPersonalBindingPrincipal,
        installation_id: AdapterInstallationId,
        slack_user_id: SlackUserId,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        self.validate_installation_actor(&principal, &installation_id, &slack_user_id)?;
        self.bind_validated_actor(installation_id, slack_user_id, principal.user_id)
            .await
    }

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

    async fn bind_validated_actor(
        &self,
        installation_id: AdapterInstallationId,
        slack_user_id: SlackUserId,
        user_id: UserId,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new(SLACK_IDENTITY_PROVIDER)?,
            provider_user_id: RebornIdentityProviderUserId::new(
                slack_user_identity_provider_user_id(&installation_id, slack_user_id.as_str()),
            )?,
            user_id,
        };
        self.store
            .bind_user_identity(binding.clone())
            .await
            .map_err(SlackPersonalUserBindingError::BindingStore)?;
        Ok(binding)
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
pub enum SlackPersonalUserBindingError {
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
        _ => unreachable!("ensure_tenant_app_scoped rejects non-AppTeam selectors"),
    }
}

fn ensure_tenant_app_scoped(
    selector: &SlackInstallationSelector,
    principal: &SlackPersonalBindingPrincipal,
    installation_id: &AdapterInstallationId,
) -> Result<(), SlackPersonalUserBindingError> {
    match selector {
        SlackInstallationSelector::AppTeam { .. } => Ok(()),
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
    }
}
