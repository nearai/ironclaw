use std::sync::Arc;

use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountId,
    CredentialAccountLookupRequest, CredentialAccountSelectionRequest, CredentialAccountService,
    CredentialAccountStatus, CredentialRecoveryProjection, CredentialRefreshRequest,
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
    #[error("Google credential recovery is required")]
    Recovery(CredentialRecoveryProjection),
    #[error("Google credential account is missing required scopes")]
    MissingScopes { missing_scopes: Vec<ProviderScope> },
    #[error("Google credential account has no access secret")]
    MissingAccessSecret,
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
        let selected_account = self
            .recoverable_result(
                self.accounts
                    .select_unique_configured_account(
                        CredentialAccountSelectionRequest::new(
                            auth_scope.clone(),
                            provider.clone(),
                        )
                        .for_extension(requester_extension.clone()),
                    )
                    .await,
                scope,
                requester_extension,
                &provider,
            )
            .await?;
        self.resolve_account(
            scope,
            requester_extension,
            selected_account.id,
            required_scopes,
        )
        .await
    }

    pub async fn resolve_account(
        &self,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        account_id: CredentialAccountId,
        required_scopes: &[ProviderScope],
    ) -> Result<GoogleCredential, GoogleCredentialError> {
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let provider = google_provider_id()?;
        let account = self
            .recoverable_lookup(
                self.accounts
                    .get_account(
                        CredentialAccountLookupRequest::new(auth_scope, account_id)
                            .for_extension(requester_extension.clone()),
                    )
                    .await,
                scope,
                requester_extension,
                &provider,
            )
            .await?;
        if account.status != CredentialAccountStatus::Configured {
            return self
                .recovery_required(scope, requester_extension, provider)
                .await;
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
            return Err(GoogleCredentialError::MissingScopes { missing_scopes });
        }
        Ok(GoogleCredential {
            account_id: account.id,
            access_secret,
            granted_scopes: account.scopes,
            missing_scopes,
        })
    }

    pub async fn refresh(
        &self,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        account_id: CredentialAccountId,
    ) -> Result<(), GoogleCredentialError> {
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let provider = google_provider_id()?;
        self.recoverable_result(
            self.accounts
                .refresh_account(
                    CredentialRefreshRequest::new(auth_scope, provider.clone(), account_id)
                        .for_extension(requester_extension.clone()),
                )
                .await,
            scope,
            requester_extension,
            &provider,
        )
        .await
        .map(|_| ())
    }

    async fn recovery_required(
        &self,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        provider: AuthProviderId,
    ) -> Result<GoogleCredential, GoogleCredentialError> {
        Err(self
            .recovery_error(scope, requester_extension, provider)
            .await)
    }

    async fn recovery_error(
        &self,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        provider: AuthProviderId,
    ) -> GoogleCredentialError {
        match self
            .project_recovery(scope, requester_extension, provider)
            .await
        {
            Ok(recovery) => GoogleCredentialError::Recovery(recovery),
            Err(error) => error,
        }
    }

    async fn recoverable_result<T>(
        &self,
        result: Result<T, AuthProductError>,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        provider: &AuthProviderId,
    ) -> Result<T, GoogleCredentialError> {
        match result {
            Ok(value) => Ok(value),
            Err(
                AuthProductError::CredentialMissing
                | AuthProductError::CrossScopeDenied
                | AuthProductError::AccountSelectionRequired,
            ) => Err(self
                .recovery_error(scope, requester_extension, provider.clone())
                .await),
            Err(error) => Err(GoogleCredentialError::Auth(error)),
        }
    }

    async fn recoverable_lookup<T>(
        &self,
        result: Result<Option<T>, AuthProductError>,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        provider: &AuthProviderId,
    ) -> Result<T, GoogleCredentialError> {
        match self
            .recoverable_result(result, scope, requester_extension, provider)
            .await?
        {
            Some(value) => Ok(value),
            None => Err(self
                .recovery_error(scope, requester_extension, provider.clone())
                .await),
        }
    }

    async fn project_recovery(
        &self,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        provider: AuthProviderId,
    ) -> Result<CredentialRecoveryProjection, GoogleCredentialError> {
        self.accounts
            .project_credential_recovery(
                ironclaw_auth::CredentialRecoveryRequest::new(
                    AuthProductScope::new(scope.clone(), AuthSurface::Api),
                    provider,
                )
                .for_extension(requester_extension.clone()),
            )
            .await
            .map_err(GoogleCredentialError::Auth)
    }
}

pub fn google_provider_id() -> Result<AuthProviderId, AuthProductError> {
    AuthProviderId::new(GOOGLE_PROVIDER_ID)
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_auth::{
        CredentialAccount, CredentialAccountChoiceRequest, CredentialAccountLabel,
        CredentialAccountListPage, CredentialAccountListRequest, CredentialAccountLookupRequest,
        CredentialAccountProjection, CredentialAccountSelectionRequest, CredentialOwnership,
        CredentialRecoveryKind, CredentialRecoveryProjection, CredentialRecoveryReason,
        CredentialRecoveryRequest, CredentialRefreshReport, CredentialRefreshRequest,
        InMemoryAuthProductServices, NewCredentialAccount,
    };
    use ironclaw_host_api::{InvocationId, UserId};

    use super::*;

    #[test]
    fn google_provider_id_returns_valid_provider() {
        assert_eq!(google_provider_id().unwrap().as_str(), GOOGLE_PROVIDER_ID);
    }

    #[test]
    fn google_credential_error_variants_are_constructible() {
        let recovery = CredentialRecoveryProjection::setup_required(
            google_provider_id().unwrap(),
            CredentialRecoveryReason::NoAccount,
            Vec::new(),
        );
        assert!(matches!(
            GoogleCredentialError::Recovery(recovery.clone()),
            GoogleCredentialError::Recovery(_)
        ));
        assert!(matches!(
            GoogleCredentialError::MissingScopes {
                missing_scopes: Vec::new()
            },
            GoogleCredentialError::MissingScopes { .. }
        ));
        assert!(matches!(
            GoogleCredentialError::Auth(AuthProductError::BackendUnavailable),
            GoogleCredentialError::Auth(AuthProductError::BackendUnavailable)
        ));
    }

    #[tokio::test]
    async fn resolve_returns_recovery_when_account_status_is_pending_setup() {
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

        let GoogleCredentialError::Recovery(recovery) = error else {
            panic!("expected recovery error");
        };
        assert_eq!(recovery.kind(), CredentialRecoveryKind::SetupRequired);
        assert_eq!(recovery.reason, CredentialRecoveryReason::PendingSetup);
    }

    #[tokio::test]
    async fn resolve_returns_recovery_when_selected_account_disappears() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let auth = InMemoryAuthProductServices::new();
        let account = auth
            .create_account(new_credential_account(
                auth_scope,
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        let resolver = GoogleCredentialResolver::new(Arc::new(MissingSelectedAccountService {
            selected: account.projection(),
        }));

        let error = resolver
            .resolve(&scope, &ExtensionId::new("gmail").unwrap(), &[])
            .await
            .unwrap_err();

        let GoogleCredentialError::Recovery(recovery) = error else {
            panic!("expected recovery error");
        };
        assert_eq!(recovery.kind(), CredentialRecoveryKind::SetupRequired);
        assert_eq!(recovery.reason, CredentialRecoveryReason::NoAccount);
    }

    #[tokio::test]
    async fn resolve_returns_missing_access_secret_when_account_has_no_access_secret() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let auth = InMemoryAuthProductServices::new();
        let mut account = auth
            .create_account(new_credential_account(
                auth_scope,
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        account.access_secret = None;
        let resolver =
            GoogleCredentialResolver::new(Arc::new(FakeCredentialAccountService { account }));

        let error = resolver
            .resolve(&scope, &ExtensionId::new("gmail").unwrap(), &[])
            .await
            .unwrap_err();

        assert!(matches!(error, GoogleCredentialError::MissingAccessSecret));
    }

    #[tokio::test]
    async fn resolve_returns_missing_scopes_when_required_scope_is_not_granted() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let auth = InMemoryAuthProductServices::new();
        let account = auth
            .create_account(new_credential_account(
                auth_scope,
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        let resolver =
            GoogleCredentialResolver::new(Arc::new(FakeCredentialAccountService { account }));

        let error = resolver
            .resolve(
                &scope,
                &ExtensionId::new("gmail").unwrap(),
                &[ProviderScope::new("https://www.googleapis.com/auth/calendar.events").unwrap()],
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            GoogleCredentialError::MissingScopes { missing_scopes }
            if missing_scopes == vec![ProviderScope::new("https://www.googleapis.com/auth/calendar.events").unwrap()]
        ));
    }

    #[tokio::test]
    async fn resolve_returns_configured_credential_when_account_has_secret_and_scopes() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let auth = InMemoryAuthProductServices::new();
        let account = auth
            .create_account(new_credential_account(
                auth_scope,
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        let resolver = GoogleCredentialResolver::new(Arc::new(FakeCredentialAccountService {
            account: account.clone(),
        }));

        let credential = resolver
            .resolve(
                &scope,
                &ExtensionId::new("gmail").unwrap(),
                &[ProviderScope::new("https://www.googleapis.com/auth/gmail.send").unwrap()],
            )
            .await
            .unwrap();

        assert_eq!(credential.account_id, account.id);
        assert_eq!(
            credential.access_secret,
            SecretHandle::new("google-access-token").unwrap()
        );
        assert!(credential.missing_scopes.is_empty());
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

    struct MissingSelectedAccountService {
        selected: CredentialAccountProjection,
    }

    fn recovery_projection_for_account(
        account: &CredentialAccount,
    ) -> CredentialRecoveryProjection {
        let provider = google_provider_id().unwrap();
        match account.status {
            CredentialAccountStatus::Configured => {
                CredentialRecoveryProjection::configured(provider, account.projection())
            }
            CredentialAccountStatus::PendingSetup => CredentialRecoveryProjection::setup_required(
                provider,
                CredentialRecoveryReason::PendingSetup,
                vec![account.projection()],
            ),
            CredentialAccountStatus::Missing => CredentialRecoveryProjection::setup_required(
                provider,
                CredentialRecoveryReason::AccountMissing,
                vec![account.projection()],
            ),
            CredentialAccountStatus::Inactive => CredentialRecoveryProjection::setup_required(
                provider,
                CredentialRecoveryReason::AccountInactive,
                vec![account.projection()],
            ),
            CredentialAccountStatus::Expired => CredentialRecoveryProjection::reauthorize_required(
                provider,
                CredentialRecoveryReason::AccountExpired,
                vec![account.projection()],
            ),
            CredentialAccountStatus::RefreshFailed => {
                CredentialRecoveryProjection::reauthorize_required(
                    provider,
                    CredentialRecoveryReason::RefreshFailed,
                    vec![account.projection()],
                )
            }
            CredentialAccountStatus::Revoked => CredentialRecoveryProjection::reauthorize_required(
                provider,
                CredentialRecoveryReason::AccountRevoked,
                vec![account.projection()],
            ),
        }
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
            request: CredentialAccountLookupRequest,
        ) -> Result<Option<CredentialAccount>, AuthProductError> {
            Ok((request.account_id == self.account.id).then(|| self.account.clone()))
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

        async fn project_credential_recovery(
            &self,
            _request: CredentialRecoveryRequest,
        ) -> Result<CredentialRecoveryProjection, AuthProductError> {
            Ok(recovery_projection_for_account(&self.account))
        }

        async fn select_configured_account(
            &self,
            _request: CredentialAccountChoiceRequest,
        ) -> Result<CredentialAccountProjection, AuthProductError> {
            unreachable!("Google credential resolver tests use unique selection")
        }

        async fn refresh_account(
            &self,
            _request: CredentialRefreshRequest,
        ) -> Result<CredentialRefreshReport, AuthProductError> {
            unreachable!("Google credential resolver tests do not refresh accounts")
        }
    }

    #[async_trait]
    impl CredentialAccountService for MissingSelectedAccountService {
        async fn create_account(
            &self,
            _request: NewCredentialAccount,
        ) -> Result<CredentialAccount, AuthProductError> {
            Err(AuthProductError::BackendUnavailable)
        }

        async fn get_account(
            &self,
            _request: CredentialAccountLookupRequest,
        ) -> Result<Option<CredentialAccount>, AuthProductError> {
            Ok(None)
        }

        async fn list_accounts(
            &self,
            _request: CredentialAccountListRequest,
        ) -> Result<CredentialAccountListPage, AuthProductError> {
            Ok(CredentialAccountListPage {
                accounts: vec![self.selected.clone()],
                next_cursor: None,
            })
        }

        async fn update_status(
            &self,
            _scope: &AuthProductScope,
            _account_id: CredentialAccountId,
            _status: CredentialAccountStatus,
        ) -> Result<CredentialAccount, AuthProductError> {
            Err(AuthProductError::BackendUnavailable)
        }

        async fn select_unique_configured_account(
            &self,
            _request: CredentialAccountSelectionRequest,
        ) -> Result<CredentialAccountProjection, AuthProductError> {
            Ok(self.selected.clone())
        }

        async fn project_credential_recovery(
            &self,
            _request: CredentialRecoveryRequest,
        ) -> Result<CredentialRecoveryProjection, AuthProductError> {
            Ok(CredentialRecoveryProjection::setup_required(
                google_provider_id().unwrap(),
                CredentialRecoveryReason::NoAccount,
                Vec::new(),
            ))
        }

        async fn select_configured_account(
            &self,
            _request: CredentialAccountChoiceRequest,
        ) -> Result<CredentialAccountProjection, AuthProductError> {
            unreachable!("Google credential resolver tests use unique selection")
        }

        async fn refresh_account(
            &self,
            _request: CredentialRefreshRequest,
        ) -> Result<CredentialRefreshReport, AuthProductError> {
            unreachable!("Google credential resolver tests do not refresh accounts")
        }
    }
}
