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
