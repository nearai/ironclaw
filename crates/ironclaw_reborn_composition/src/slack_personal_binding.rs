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
use crate::slack_serve::SlackInstallationSelector;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornUserIdentityBinding {
    pub provider: RebornIdentityProvider,
    pub provider_user_id: RebornIdentityProviderUserId,
    pub user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RebornIdentityProvider(String);

impl RebornIdentityProvider {
    pub fn new(value: impl Into<String>) -> Result<Self, RebornUserIdentityBindingError> {
        let value = value.into();
        validate_binding_id("identity provider", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RebornIdentityProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RebornIdentityProviderUserId(String);

impl RebornIdentityProviderUserId {
    pub fn new(value: impl Into<String>) -> Result<Self, RebornUserIdentityBindingError> {
        let value = value.into();
        validate_binding_id("identity provider user", &value)?;
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

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RebornUserIdentityBindingError {
    #[error("reborn user identity binding backend unavailable: {0}")]
    Backend(String),
    #[error("invalid {field} id: {reason}")]
    InvalidIdentityId {
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
pub struct SlackPersonalUserBindingRequest {
    pub tenant_id: TenantId,
    pub installation_id: AdapterInstallationId,
    pub slack_user_id: SlackBindingUserId,
    pub team_id: SlackBindingTeamId,
    pub enterprise_id: Option<SlackBindingEnterpriseId>,
    pub api_app_id: Option<SlackBindingApiAppId>,
}

macro_rules! slack_binding_id_type {
    ($name:ident, $field:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, SlackPersonalUserBindingError> {
                let value = value.into();
                validate_slack_id($field, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

slack_binding_id_type!(SlackBindingUserId, "slack user");
slack_binding_id_type!(SlackBindingTeamId, "slack team");
slack_binding_id_type!(SlackBindingEnterpriseId, "slack enterprise");
slack_binding_id_type!(SlackBindingApiAppId, "slack app");

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
        authenticated_user_id: UserId,
        request: SlackPersonalUserBindingRequest,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        let installation = self
            .installations
            .iter()
            .find(|installation| {
                installation.tenant_id == request.tenant_id
                    && installation.installation_id == request.installation_id
            })
            .ok_or_else(|| SlackPersonalUserBindingError::UnknownInstallation {
                tenant_id: request.tenant_id.clone(),
                installation_id: request.installation_id.clone(),
            })?;

        if !tenant_app_selector_matches_request(&installation.selector, &request)? {
            return Err(
                SlackPersonalUserBindingError::SlackInstallationContextMismatch {
                    tenant_id: request.tenant_id.clone(),
                    installation_id: request.installation_id.clone(),
                },
            );
        }

        let binding = RebornUserIdentityBinding {
            provider: RebornIdentityProvider::new(SLACK_IDENTITY_PROVIDER)?,
            provider_user_id: RebornIdentityProviderUserId::new(
                slack_user_identity_provider_user_id(
                    &request.installation_id,
                    request.slack_user_id.as_str(),
                ),
            )?,
            user_id: authenticated_user_id,
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
    #[error(
        "slack installation {installation_id} for tenant {tenant_id} has no app-scoped Slack context"
    )]
    InstallationNotAppScoped {
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

fn validate_binding_id(
    field: &'static str,
    value: &str,
) -> Result<(), RebornUserIdentityBindingError> {
    if value.is_empty() {
        return Err(RebornUserIdentityBindingError::InvalidIdentityId {
            field,
            reason: "must not be empty",
        });
    }
    if value.chars().any(|character| character.is_control()) {
        return Err(RebornUserIdentityBindingError::InvalidIdentityId {
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

fn tenant_app_selector_matches_request(
    selector: &SlackInstallationSelector,
    request: &SlackPersonalUserBindingRequest,
) -> Result<bool, SlackPersonalUserBindingError> {
    match selector {
        SlackInstallationSelector::Team { .. }
        | SlackInstallationSelector::EnterpriseTeam { .. } => {
            Err(SlackPersonalUserBindingError::InstallationNotAppScoped {
                tenant_id: request.tenant_id.clone(),
                installation_id: request.installation_id.clone(),
            })
        }
        SlackInstallationSelector::AppTeam {
            api_app_id,
            team_id,
        } => Ok(team_id.as_str() == request.team_id.as_str()
            && request
                .api_app_id
                .as_ref()
                .map(SlackBindingApiAppId::as_str)
                == Some(api_app_id.as_str())),
        SlackInstallationSelector::InstallUser { .. }
        | SlackInstallationSelector::EnterpriseInstallUser { .. }
        | SlackInstallationSelector::AppInstallUser { .. } => {
            Err(SlackPersonalUserBindingError::InstallationNotTenantScoped {
                tenant_id: request.tenant_id.clone(),
                installation_id: request.installation_id.clone(),
            })
        }
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
                user("user:alice"),
                request("tenant-alpha", "install-alpha").with_app(),
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
                user("user:alice"),
                request("tenant-beta", "install-alpha").with_app(),
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
                user("user:alice"),
                request("tenant-alpha", "install-beta").with_app(),
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

        let error = service
            .bind_personal_user(user("user:alice"), request("tenant-alpha", "install-alpha"))
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
        let mut request = request("tenant-alpha", "install-alpha").with_app();
        request.team_id = slack_team_id("T-other");

        let error = service
            .bind_personal_user(user("user:alice"), request)
            .await
            .expect_err("wrong team is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::SlackInstallationContextMismatch { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_unscoped_team_selector_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(SlackInstallationSelector::team("T-team"), store.clone());

        let error = service
            .bind_personal_user(
                user("user:alice"),
                request("tenant-alpha", "install-alpha").with_app(),
            )
            .await
            .expect_err("team-only selector is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::InstallationNotAppScoped { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_enterprise_team_selector_without_write() {
        let store = Arc::new(RecordingBindingStore::default());
        let service = service(
            SlackInstallationSelector::enterprise_team("E-enterprise", "T-team"),
            store.clone(),
        );
        let request = request("tenant-alpha", "install-alpha")
            .with_app()
            .with_enterprise();

        let error = service
            .bind_personal_user(user("user:alice"), request)
            .await
            .expect_err("enterprise/team selector without app context is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::InstallationNotAppScoped { .. }
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
            .bind_personal_user(user("user:alice"), request("tenant-alpha", "install-alpha"))
            .await
            .expect_err("install-user scoped app is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::InstallationNotTenantScoped { .. }
        ));
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_invalid_slack_user_without_write() {
        let store = Arc::new(RecordingBindingStore::default());

        let error = SlackBindingUserId::new("").expect_err("invalid slack user is rejected");

        assert_eq!(
            error,
            SlackPersonalUserBindingError::InvalidSlackId {
                field: "slack user",
                reason: "must not be empty",
            }
        );
        assert_eq!(store.bindings(), Vec::<RebornUserIdentityBinding>::new());
    }

    #[tokio::test]
    async fn bind_personal_user_rejects_invalid_context_ids_without_write() {
        let store = Arc::new(RecordingBindingStore::default());

        assert_eq!(
            SlackBindingTeamId::new("").expect_err("empty team id is rejected"),
            SlackPersonalUserBindingError::InvalidSlackId {
                field: "slack team",
                reason: "must not be empty",
            }
        );
        assert_eq!(
            SlackBindingEnterpriseId::new("E-enterprise\n")
                .expect_err("control character enterprise id is rejected"),
            SlackPersonalUserBindingError::InvalidSlackId {
                field: "slack enterprise",
                reason: "must not contain control characters",
            }
        );
        assert_eq!(
            SlackBindingApiAppId::new("A-app\t").expect_err("control character app id is rejected"),
            SlackPersonalUserBindingError::InvalidSlackId {
                field: "slack app",
                reason: "must not contain control characters",
            }
        );
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
                user("user:alice"),
                request("tenant-alpha", "install-alpha").with_app(),
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

    fn request(tenant_id: &str, installation_id: &str) -> SlackPersonalUserBindingRequest {
        SlackPersonalUserBindingRequest {
            tenant_id: tenant(tenant_id),
            installation_id: installation(installation_id),
            slack_user_id: slack_user_id("U123"),
            team_id: slack_team_id("T-team"),
            enterprise_id: None,
            api_app_id: None,
        }
    }

    trait RequestExt {
        fn with_app(self) -> Self;
        fn with_enterprise(self) -> Self;
    }

    impl RequestExt for SlackPersonalUserBindingRequest {
        fn with_app(mut self) -> Self {
            self.api_app_id = Some(slack_api_app_id("A-app"));
            self
        }

        fn with_enterprise(mut self) -> Self {
            self.enterprise_id = Some(slack_enterprise_id("E-enterprise"));
            self
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

    fn provider(value: &str) -> RebornIdentityProvider {
        RebornIdentityProvider::new(value).expect("valid identity provider")
    }

    fn provider_user_id(value: &str) -> RebornIdentityProviderUserId {
        RebornIdentityProviderUserId::new(value).expect("valid provider user id")
    }

    fn slack_user_id(value: &str) -> SlackBindingUserId {
        SlackBindingUserId::new(value).expect("valid slack user id")
    }

    fn slack_team_id(value: &str) -> SlackBindingTeamId {
        SlackBindingTeamId::new(value).expect("valid slack team id")
    }

    fn slack_enterprise_id(value: &str) -> SlackBindingEnterpriseId {
        SlackBindingEnterpriseId::new(value).expect("valid slack enterprise id")
    }

    fn slack_api_app_id(value: &str) -> SlackBindingApiAppId {
        SlackBindingApiAppId::new(value).expect("valid slack app id")
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
