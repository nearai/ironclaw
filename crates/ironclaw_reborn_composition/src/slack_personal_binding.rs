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
    pub provider: String,
    pub provider_user_id: String,
    pub user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RebornUserIdentityBindingError {
    #[error("reborn user identity binding backend unavailable: {0}")]
    Backend(String),
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
    pub user_id: UserId,
    pub slack_user_id: String,
    pub team_id: String,
    pub enterprise_id: Option<String>,
    pub api_app_id: Option<String>,
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
        request: SlackPersonalUserBindingRequest,
    ) -> Result<RebornUserIdentityBinding, SlackPersonalUserBindingError> {
        validate_slack_id("slack user", &request.slack_user_id)?;
        validate_slack_id("slack team", &request.team_id)?;
        validate_optional_slack_id("slack enterprise", request.enterprise_id.as_deref())?;
        validate_optional_slack_id("slack app", request.api_app_id.as_deref())?;

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
            provider: SLACK_IDENTITY_PROVIDER.to_string(),
            provider_user_id: slack_user_identity_provider_user_id(
                &request.installation_id,
                &request.slack_user_id,
            ),
            user_id: request.user_id,
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
    request: &SlackPersonalUserBindingRequest,
) -> Result<bool, SlackPersonalUserBindingError> {
    match selector {
        SlackInstallationSelector::Team { team_id } => Ok(team_id.as_str() == request.team_id),
        SlackInstallationSelector::AppTeam {
            api_app_id,
            team_id,
        } => Ok(team_id.as_str() == request.team_id
            && request.api_app_id.as_deref() == Some(api_app_id.as_str())),
        SlackInstallationSelector::EnterpriseTeam {
            enterprise_id,
            team_id,
        } => Ok(team_id.as_str() == request.team_id
            && request.enterprise_id.as_deref() == Some(enterprise_id.as_str())),
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
            .bind_personal_user(request("tenant-alpha", "install-alpha", "user:alice").with_app())
            .await
            .expect("binding succeeds");

        assert_eq!(
            binding,
            RebornUserIdentityBinding {
                provider: "slack".to_string(),
                provider_user_id: "install-alpha:U123".to_string(),
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
            .bind_personal_user(request("tenant-beta", "install-alpha", "user:alice").with_app())
            .await
            .expect_err("wrong tenant is rejected");

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
            .bind_personal_user(request("tenant-alpha", "install-alpha", "user:alice"))
            .await
            .expect_err("wrong app is rejected");

        assert!(matches!(
            error,
            SlackPersonalUserBindingError::SlackInstallationContextMismatch { .. }
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
            .bind_personal_user(request("tenant-alpha", "install-alpha", "user:alice"))
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
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );
        let mut request = request("tenant-alpha", "install-alpha", "user:alice").with_app();
        request.slack_user_id = String::new();

        let error = service
            .bind_personal_user(request)
            .await
            .expect_err("invalid slack user is rejected");

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
    async fn bind_personal_user_propagates_store_error() {
        let store = Arc::new(RecordingBindingStore::with_error(
            RebornUserIdentityBindingError::Backend("store down".into()),
        ));
        let service = service(
            SlackInstallationSelector::app_team("A-app", "T-team"),
            store.clone(),
        );

        let error = service
            .bind_personal_user(request("tenant-alpha", "install-alpha", "user:alice").with_app())
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
                provider: "slack".to_string(),
                provider_user_id: "install-alpha:U123".to_string(),
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

    fn request(
        tenant_id: &str,
        installation_id: &str,
        user_id: &str,
    ) -> SlackPersonalUserBindingRequest {
        SlackPersonalUserBindingRequest {
            tenant_id: tenant(tenant_id),
            installation_id: installation(installation_id),
            user_id: user(user_id),
            slack_user_id: "U123".to_string(),
            team_id: "T-team".to_string(),
            enterprise_id: None,
            api_app_id: None,
        }
    }

    trait RequestExt {
        fn with_app(self) -> Self;
    }

    impl RequestExt for SlackPersonalUserBindingRequest {
        fn with_app(mut self) -> Self {
            self.api_app_id = Some("A-app".to_string());
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
