use std::sync::Arc;

use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountId,
    CredentialAccountSelectionRequest, CredentialAccountService, CredentialAccountStatus,
    GOOGLE_PROVIDER_ID, ProviderScope,
};
use ironclaw_host_api::{ExtensionId, ResourceScope, SecretHandle};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleCredential {
    pub account_id: CredentialAccountId,
    pub access_secret: SecretHandle,
    pub granted_scopes: Vec<ProviderScope>,
    pub missing_scopes: Vec<ProviderScope>,
}

#[derive(Debug, Error)]
pub enum GoogleCredentialError {
    #[error("Google credential account is missing")]
    Missing,
    #[error("Google credential account requires account selection")]
    AccountSelectionRequired,
    #[error("Google credential account is not configured")]
    NotConfigured,
    #[error("Google credential account has no access secret")]
    MissingAccessSecret,
    #[error("Google credential account is missing required scopes")]
    MissingScopes,
    #[error(transparent)]
    Auth(#[from] AuthProductError),
    #[error(transparent)]
    HostApi(#[from] ironclaw_host_api::HostApiError),
}

#[derive(Clone)]
pub struct GoogleCredentialResolver {
    accounts: Arc<dyn CredentialAccountService>,
}

impl GoogleCredentialResolver {
    pub fn new(accounts: Arc<dyn CredentialAccountService>) -> Self {
        Self { accounts }
    }

    pub async fn resolve(
        &self,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        required_scopes: &[ProviderScope],
    ) -> Result<GoogleCredential, GoogleCredentialError> {
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let provider = google_provider_id()?;
        let selected = self
            .accounts
            .select_unique_configured_account(
                CredentialAccountSelectionRequest::new(auth_scope.clone(), provider)
                    .for_extension(requester_extension.clone()),
            )
            .await
            .map_err(map_selection_error)?;
        let account = self
            .accounts
            .get_account(&auth_scope, selected.id)
            .await?
            .ok_or(GoogleCredentialError::Missing)?;
        if account.status != CredentialAccountStatus::Configured {
            return Err(GoogleCredentialError::NotConfigured);
        }
        let access_secret = account
            .access_secret
            .clone()
            .ok_or(GoogleCredentialError::MissingAccessSecret)?;
        let missing_scopes = required_scopes
            .iter()
            .filter(|required| !account.scopes.contains(required))
            .cloned()
            .collect::<Vec<_>>();
        if !missing_scopes.is_empty() {
            return Err(GoogleCredentialError::MissingScopes);
        }
        Ok(GoogleCredential {
            account_id: account.id,
            access_secret,
            granted_scopes: account.scopes,
            missing_scopes,
        })
    }
}

pub fn google_provider_id() -> Result<AuthProviderId, AuthProductError> {
    AuthProviderId::new(GOOGLE_PROVIDER_ID)
}

fn map_selection_error(error: AuthProductError) -> GoogleCredentialError {
    match error {
        AuthProductError::CredentialMissing => GoogleCredentialError::Missing,
        AuthProductError::AccountSelectionRequired => {
            GoogleCredentialError::AccountSelectionRequired
        }
        other => GoogleCredentialError::Auth(other),
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_auth::{
        CredentialAccount, CredentialAccountLabel, CredentialAccountListPage,
        CredentialAccountListRequest, CredentialAccountProjection, CredentialOwnership,
        InMemoryAuthProductServices, NewCredentialAccount,
    };
    use ironclaw_host_api::{InvocationId, UserId};

    use super::*;

    #[test]
    fn google_provider_id_returns_valid_provider() {
        assert_eq!(google_provider_id().unwrap().as_str(), GOOGLE_PROVIDER_ID);
    }

    #[test]
    fn map_selection_error_tests() {
        assert!(matches!(
            map_selection_error(AuthProductError::CredentialMissing),
            GoogleCredentialError::Missing
        ));
        assert!(matches!(
            map_selection_error(AuthProductError::AccountSelectionRequired),
            GoogleCredentialError::AccountSelectionRequired
        ));
        assert!(matches!(
            map_selection_error(AuthProductError::BackendUnavailable),
            GoogleCredentialError::Auth(AuthProductError::BackendUnavailable)
        ));
    }

    #[tokio::test]
    async fn resolve_returns_not_configured_when_account_status_unconfigured() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let auth = InMemoryAuthProductServices::new();
        let mut account = auth
            .create_account(new_credential_account(
                auth_scope.clone(),
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        account.status = CredentialAccountStatus::PendingSetup;
        let resolver = GoogleCredentialResolver::new(Arc::new(FakeCredentialAccountService {
            account: account.clone(),
        }));

        let error = resolver
            .resolve(
                &scope,
                &ExtensionId::new("gmail").unwrap(),
                &[ProviderScope::new("https://www.googleapis.com/auth/gmail.send").unwrap()],
            )
            .await
            .unwrap_err();

        assert!(matches!(error, GoogleCredentialError::NotConfigured));
    }

    fn new_credential_account(
        scope: AuthProductScope,
        status: CredentialAccountStatus,
    ) -> NewCredentialAccount {
        NewCredentialAccount {
            scope,
            provider: google_provider_id().unwrap(),
            label: CredentialAccountLabel::new("work google").unwrap(),
            status,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google-access-token").unwrap()),
            refresh_secret: None,
            scopes: vec![ProviderScope::new("https://www.googleapis.com/auth/gmail.send").unwrap()],
        }
    }

    struct FakeCredentialAccountService {
        account: CredentialAccount,
    }

    #[async_trait]
    impl CredentialAccountService for FakeCredentialAccountService {
        async fn create_account(
            &self,
            _request: NewCredentialAccount,
        ) -> Result<CredentialAccount, AuthProductError> {
            Ok(self.account.clone())
        }

        async fn get_account(
            &self,
            _scope: &AuthProductScope,
            account_id: CredentialAccountId,
        ) -> Result<Option<CredentialAccount>, AuthProductError> {
            Ok((account_id == self.account.id).then(|| self.account.clone()))
        }

        async fn list_accounts(
            &self,
            _request: CredentialAccountListRequest,
        ) -> Result<CredentialAccountListPage, AuthProductError> {
            Ok(CredentialAccountListPage {
                accounts: vec![self.account.projection()],
                next_cursor: None,
            })
        }

        async fn update_status(
            &self,
            _scope: &AuthProductScope,
            _account_id: CredentialAccountId,
            _status: CredentialAccountStatus,
        ) -> Result<CredentialAccount, AuthProductError> {
            Ok(self.account.clone())
        }

        async fn select_unique_configured_account(
            &self,
            _request: CredentialAccountSelectionRequest,
        ) -> Result<CredentialAccountProjection, AuthProductError> {
            Ok(self.account.projection())
        }
    }
}
