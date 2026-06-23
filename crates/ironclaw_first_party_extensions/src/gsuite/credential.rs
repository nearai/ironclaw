use std::sync::Arc;

use ironclaw_auth::{
    AuthProductError, AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount,
    CredentialAccountId, CredentialAccountRecordSource, CredentialAccountService,
    CredentialAccountStatus, CredentialRecoveryProjection, CredentialRefreshRequest,
    GOOGLE_PROVIDER_ID, ProviderScope,
};
use ironclaw_host_api::{ExtensionId, ResourceScope, SecretHandle};
use thiserror::Error;

use super::account_policy::gsuite_google_account_visible_to_requester;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleCredential {
    pub account_id: CredentialAccountId,
    pub account_scope: AuthProductScope,
    pub access_secret_scope: ResourceScope,
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
    account_records: Arc<dyn CredentialAccountRecordSource>,
}

impl GoogleCredentialResolver {
    pub fn new(
        accounts: Arc<dyn CredentialAccountService>,
        account_records: Arc<dyn CredentialAccountRecordSource>,
    ) -> Self {
        Self {
            accounts,
            account_records,
        }
    }

    pub async fn resolve(
        &self,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        required_scopes: &[ProviderScope],
    ) -> Result<GoogleCredential, GoogleCredentialError> {
        // Look up the owner's Google account at tenant/user/agent/project
        // granularity. The runtime `scope` carries the current chat thread, but
        // credentials are owned by the user, not the thread, so the thread/
        // mission sub-scope must be stripped or the account authorized in one
        // thread is invisible in the next. Staging still uses the real runtime
        // scope via this method's `scope` parameter and
        // `credential.access_secret_scope`.
        let auth_scope = AuthProductScope::credential_owner(scope, AuthSurface::Api);
        let provider = google_provider_id()?;
        let account = match self
            .select_configured_account_for_gsuite_requester(
                &auth_scope,
                requester_extension,
                &provider,
                required_scopes,
            )
            .await
        {
            Ok(account) => account,
            Err(GoogleCredentialError::Auth(
                AuthProductError::CredentialMissing
                | AuthProductError::CrossScopeDenied
                | AuthProductError::AccountSelectionRequired,
            )) => {
                return self
                    .recovery_required(scope, requester_extension, provider)
                    .await;
            }
            Err(error) => return Err(error),
        };
        self.credential_from_account(
            scope,
            requester_extension,
            provider,
            account,
            required_scopes,
        )
        .await
    }

    pub async fn resolve_account(
        &self,
        scope: &ResourceScope,
        account_scope: &AuthProductScope,
        requester_extension: &ExtensionId,
        account_id: CredentialAccountId,
        required_scopes: &[ProviderScope],
    ) -> Result<GoogleCredential, GoogleCredentialError> {
        let provider = google_provider_id()?;
        let account = self
            .recoverable_lookup(
                self.account_by_id(
                    account_scope,
                    provider.clone(),
                    requester_extension.clone(),
                    account_id,
                )
                .await,
                scope,
                requester_extension,
                &provider,
            )
            .await?;
        self.credential_from_account(
            scope,
            requester_extension,
            provider,
            account,
            required_scopes,
        )
        .await
    }

    async fn credential_from_account(
        &self,
        scope: &ResourceScope,
        requester_extension: &ExtensionId,
        provider: AuthProviderId,
        account: CredentialAccount,
        required_scopes: &[ProviderScope],
    ) -> Result<GoogleCredential, GoogleCredentialError> {
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
            account_scope: account.scope.clone(),
            access_secret_scope: account.scope.resource.clone(),
            access_secret,
            granted_scopes: account.scopes,
            missing_scopes,
        })
    }

    pub async fn refresh(
        &self,
        scope: &ResourceScope,
        account_scope: &AuthProductScope,
        requester_extension: &ExtensionId,
        account_id: CredentialAccountId,
    ) -> Result<(), GoogleCredentialError> {
        let provider = google_provider_id()?;
        let account = self
            .recoverable_lookup(
                self.account_by_id(
                    account_scope,
                    provider.clone(),
                    requester_extension.clone(),
                    account_id,
                )
                .await,
                scope,
                requester_extension,
                &provider,
            )
            .await?;
        self.recoverable_result(
            self.accounts
                .refresh_account(refresh_request_for_account(
                    &account,
                    provider.clone(),
                    account_id,
                    requester_extension,
                ))
                .await,
            scope,
            requester_extension,
            &provider,
        )
        .await
        .map(|_| ())
    }

    async fn account_by_id(
        &self,
        scope: &AuthProductScope,
        provider: AuthProviderId,
        requester_extension: ExtensionId,
        account_id: CredentialAccountId,
    ) -> Result<Option<CredentialAccount>, AuthProductError> {
        // Owner-scope the read so a known account is found from any thread of
        // the same owner, not just the thread/session it was authorized in.
        // `credential_owner` is session-agnostic (it builds a fresh owner scope
        // with no `session_id`), which is what we want for a known-account-id
        // lookup across the owner.
        let owner_scope = AuthProductScope::credential_owner(&scope.resource, scope.surface);
        let account = self
            .account_records
            .accounts_for_owner(&owner_scope)
            .await?
            .into_iter()
            .find(|account| account.id == account_id);
        let Some(account) = account else {
            return Ok(None);
        };
        if account.provider != provider {
            return Ok(None);
        }
        if !gsuite_google_account_visible_to_requester(&account, &requester_extension) {
            return Err(AuthProductError::CrossScopeDenied);
        }
        Ok(Some(account))
    }

    async fn select_configured_account_for_gsuite_requester(
        &self,
        auth_scope: &AuthProductScope,
        requester_extension: &ExtensionId,
        provider: &AuthProviderId,
        required_scopes: &[ProviderScope],
    ) -> Result<CredentialAccount, GoogleCredentialError> {
        let configured = self
            .account_records
            .accounts_for_owner(auth_scope)
            .await?
            .into_iter()
            .filter(|account| {
                account.provider == *provider
                    && account.status == CredentialAccountStatus::Configured
                    && gsuite_google_account_visible_to_requester(account, requester_extension)
            })
            .collect::<Vec<_>>();
        if configured.is_empty() {
            return Err(AuthProductError::CredentialMissing.into());
        }
        let scoped = configured
            .iter()
            .filter(|account| account_has_provider_scopes(account, required_scopes))
            .cloned()
            .collect::<Vec<_>>();
        match scoped.as_slice() {
            [] => Err(GoogleCredentialError::MissingScopes {
                missing_scopes: required_scopes.to_vec(),
            }),
            [account] => Ok(account.clone()),
            _ => Err(AuthProductError::AccountSelectionRequired.into()),
        }
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
                    AuthProductScope::credential_owner(scope, AuthSurface::Api),
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

fn refresh_request_for_account(
    account: &CredentialAccount,
    provider: AuthProviderId,
    account_id: CredentialAccountId,
    requester_extension: &ExtensionId,
) -> CredentialRefreshRequest {
    let request = CredentialRefreshRequest::new(account.scope.clone(), provider, account_id);
    if account.is_authorized_for_requester(Some(requester_extension)) {
        return request.for_extension(requester_extension.clone());
    }
    if let Some(owner_extension) = account.owner_extension.clone()
        && account.is_authorized_for_requester(Some(&owner_extension))
    {
        return request.for_extension(owner_extension);
    }
    request
}

fn account_has_provider_scopes(
    account: &CredentialAccount,
    required_scopes: &[ProviderScope],
) -> bool {
    required_scopes
        .iter()
        .all(|required| account.scopes.iter().any(|scope| scope == required))
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use ironclaw_auth::{
        CredentialAccount, CredentialAccountChoiceRequest, CredentialAccountLabel,
        CredentialAccountListPage, CredentialAccountListRequest, CredentialAccountLookupRequest,
        CredentialAccountOwnerScope, CredentialAccountProjection, CredentialAccountRecordSource,
        CredentialAccountSelectionRequest, CredentialOwnership, CredentialRecoveryKind,
        CredentialRecoveryProjection, CredentialRecoveryReason, CredentialRecoveryRequest,
        CredentialRefreshReport, CredentialRefreshRequest, InMemoryAuthProductServices,
        NewCredentialAccount,
    };
    use ironclaw_host_api::{InvocationId, ThreadId, UserId};

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
        let account_service = Arc::new(FakeCredentialAccountService {
            account: account.clone(),
        });
        let resolver =
            GoogleCredentialResolver::new(account_service.clone(), account_service.clone());

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
        let account_service = Arc::new(MissingSelectedAccountService {
            selected: account.projection(),
        });
        let resolver =
            GoogleCredentialResolver::new(account_service.clone(), account_service.clone());

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
    async fn resolve_projects_recovery_from_owner_scope_across_thread() {
        let user = UserId::new("alice").unwrap();
        let mut thread_a = ResourceScope::local_default(user.clone(), InvocationId::new()).unwrap();
        thread_a.thread_id = Some(ThreadId::new("thread-a").unwrap());
        let auth_scope = AuthProductScope::new(thread_a.clone(), AuthSurface::Api);

        let auth = Arc::new(InMemoryAuthProductServices::new());
        let account = auth
            .create_account(new_credential_account(
                auth_scope.clone(),
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        let account_service = Arc::new(ThreadSensitiveRecoveryService {
            account: account.clone(),
            recovery_scope: std::sync::Mutex::new(None),
        });
        let resolver =
            GoogleCredentialResolver::new(account_service.clone(), account_service.clone());

        let mut thread_b = ResourceScope::local_default(user, InvocationId::new()).unwrap();
        thread_b.thread_id = Some(ThreadId::new("thread-b").unwrap());

        let error = resolver
            .resolve(&thread_b, &ExtensionId::new("third-party").unwrap(), &[])
            .await
            .unwrap_err();

        let GoogleCredentialError::Recovery(recovery) = error else {
            panic!("expected recovery error");
        };
        assert_eq!(recovery.kind(), CredentialRecoveryKind::Configured);
        assert_eq!(
            recovery.selected_account().map(|account| account.id),
            Some(account.id)
        );

        let recorded_scope = account_service
            .recovery_scope
            .lock()
            .unwrap()
            .clone()
            .expect("recovery scope recorded");
        assert_eq!(
            recorded_scope,
            AuthProductScope::credential_owner(&thread_b, AuthSurface::Api)
        );
        assert!(
            recorded_scope.resource.thread_id.is_none(),
            "recovery scope must be owner-scoped"
        );
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
                auth_scope.clone(),
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        account.access_secret = None;
        let account_service = Arc::new(FakeCredentialAccountService { account });
        let resolver =
            GoogleCredentialResolver::new(account_service.clone(), account_service.clone());

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
        let auth = Arc::new(InMemoryAuthProductServices::new());
        auth.create_account(new_credential_account(
            auth_scope,
            CredentialAccountStatus::Configured,
        ))
        .await
        .unwrap();
        let resolver = GoogleCredentialResolver::new(auth.clone(), auth.clone());

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
        let account_service = Arc::new(FakeCredentialAccountService {
            account: account.clone(),
        });
        let resolver =
            GoogleCredentialResolver::new(account_service.clone(), account_service.clone());

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

    #[tokio::test]
    async fn resolve_finds_owner_account_authorized_in_a_different_thread() {
        // Regression (#4920-follow-up): a Google credential a user authorizes in
        // one chat thread MUST stay resolvable from a new thread of the same
        // owner. Credentials are owned by tenant/user/agent/project, never by the
        // thread they happened to be authorized in.
        let user = UserId::new("alice").unwrap();
        let mut thread_a = ResourceScope::local_default(user.clone(), InvocationId::new()).unwrap();
        thread_a.thread_id = Some(ThreadId::new("thread-a").unwrap());
        let auth_scope = AuthProductScope::new(thread_a.clone(), AuthSurface::Api);

        let auth = Arc::new(InMemoryAuthProductServices::new());
        let account = auth
            .create_account(new_credential_account(
                auth_scope,
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        let resolver = GoogleCredentialResolver::new(auth.clone(), auth.clone());

        let mut thread_b = ResourceScope::local_default(user, InvocationId::new()).unwrap();
        thread_b.thread_id = Some(ThreadId::new("thread-b").unwrap());

        let credential = resolver
            .resolve(
                &thread_b,
                &ExtensionId::new("gmail").unwrap(),
                &[ProviderScope::new("https://www.googleapis.com/auth/gmail.send").unwrap()],
            )
            .await
            .expect("owner credential must resolve from a different thread");

        assert_eq!(credential.account_id, account.id);
        assert_eq!(
            credential.access_secret,
            SecretHandle::new("google-access-token").unwrap()
        );
    }

    #[tokio::test]
    async fn resolve_finds_extension_owned_account_authorized_in_a_different_thread() {
        // The cross-thread guarantee is not limited to UserReusable accounts:
        // an ExtensionOwned Google account stays resolvable for an authorized
        // gsuite-sibling requester from any thread of the owner.
        let user = UserId::new("alice").unwrap();
        let calendar_scope =
            ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();
        let mut thread_a = ResourceScope::local_default(user.clone(), InvocationId::new()).unwrap();
        thread_a.thread_id = Some(ThreadId::new("thread-a").unwrap());
        let auth_scope = AuthProductScope::new(thread_a.clone(), AuthSurface::Api);

        let auth = Arc::new(InMemoryAuthProductServices::new());
        let account = auth
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: google_provider_id().unwrap(),
                label: CredentialAccountLabel::new("work google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::ExtensionOwned,
                owner_extension: Some(ExtensionId::new("google-drive").unwrap()),
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("google-access-token").unwrap()),
                refresh_secret: None,
                scopes: vec![calendar_scope.clone()],
            })
            .await
            .unwrap();
        let resolver = GoogleCredentialResolver::new(auth.clone(), auth.clone());

        let mut thread_b = ResourceScope::local_default(user, InvocationId::new()).unwrap();
        thread_b.thread_id = Some(ThreadId::new("thread-b").unwrap());

        let credential = resolver
            .resolve(
                &thread_b,
                &ExtensionId::new("google-calendar").unwrap(),
                &[calendar_scope],
            )
            .await
            .expect("extension-owned credential must resolve from a different thread");

        assert_eq!(credential.account_id, account.id);
    }

    #[tokio::test]
    async fn resolve_reuses_gsuite_owned_google_account_for_gsuite_requester() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let auth = InMemoryAuthProductServices::new();
        let calendar_scope =
            ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();
        let mut account = auth
            .create_account(new_credential_account(
                auth_scope,
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        account.ownership = CredentialOwnership::ExtensionOwned;
        account.owner_extension = Some(ExtensionId::new("google-drive").unwrap());
        account.scopes = vec![calendar_scope.clone()];
        let account_service = Arc::new(FakeCredentialAccountService {
            account: account.clone(),
        });
        let resolver =
            GoogleCredentialResolver::new(account_service.clone(), account_service.clone());

        let credential = resolver
            .resolve(
                &scope,
                &ExtensionId::new("google-calendar").unwrap(),
                &[calendar_scope],
            )
            .await
            .unwrap();

        assert_eq!(credential.account_id, account.id);
        assert_eq!(
            credential.access_secret,
            SecretHandle::new("google-access-token").unwrap()
        );
    }

    #[tokio::test]
    async fn resolve_denies_unbound_google_account_to_third_party_requester() {
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
        let account_service = Arc::new(FakeCredentialAccountService { account });
        let resolver =
            GoogleCredentialResolver::new(account_service.clone(), account_service.clone());

        let error = resolver
            .resolve(
                &scope,
                &ExtensionId::new("third-party").unwrap(),
                &[ProviderScope::new("https://www.googleapis.com/auth/gmail.send").unwrap()],
            )
            .await
            .unwrap_err();

        let GoogleCredentialError::Recovery(recovery) = error else {
            panic!("expected recovery error");
        };
        assert_eq!(recovery.kind(), CredentialRecoveryKind::Configured);
    }

    #[tokio::test]
    async fn resolve_account_reuses_gsuite_owned_google_account_for_gsuite_requester() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let auth = InMemoryAuthProductServices::new();
        let calendar_scope =
            ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();
        let mut account = auth
            .create_account(new_credential_account(
                auth_scope.clone(),
                CredentialAccountStatus::Configured,
            ))
            .await
            .unwrap();
        account.ownership = CredentialOwnership::ExtensionOwned;
        account.owner_extension = Some(ExtensionId::new("google-drive").unwrap());
        account.scopes = vec![calendar_scope.clone()];
        let account_service = Arc::new(FakeCredentialAccountService {
            account: account.clone(),
        });
        let resolver =
            GoogleCredentialResolver::new(account_service.clone(), account_service.clone());

        let credential = resolver
            .resolve_account(
                &scope,
                &auth_scope,
                &ExtensionId::new("google-calendar").unwrap(),
                account.id,
                &[calendar_scope],
            )
            .await
            .unwrap();

        assert_eq!(credential.account_id, account.id);
    }

    #[tokio::test]
    async fn resolve_account_reuses_gsuite_owned_google_account_from_durable_store() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let auth = Arc::new(InMemoryAuthProductServices::new());
        let calendar_scope =
            ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();
        let account = auth
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: google_provider_id().unwrap(),
                label: CredentialAccountLabel::new("work google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::ExtensionOwned,
                owner_extension: Some(ExtensionId::new("google-drive").unwrap()),
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("google-access-token").unwrap()),
                refresh_secret: None,
                scopes: vec![calendar_scope.clone()],
            })
            .await
            .unwrap();
        let resolver = GoogleCredentialResolver::new(auth.clone(), auth.clone());

        let credential = resolver
            .resolve_account(
                &scope,
                &auth_scope,
                &ExtensionId::new("google-calendar").unwrap(),
                account.id,
                &[calendar_scope],
            )
            .await
            .unwrap();

        assert_eq!(credential.account_id, account.id);
    }

    #[tokio::test]
    async fn resolve_account_finds_owner_account_authorized_in_a_different_thread() {
        // Regression for the owner-scoped known-account lookup in
        // `account_by_id`: an account authorized in one thread must resolve by id
        // from a different thread of the same owner. The `resolve()` path already
        // has cross-thread coverage; this locks the same guarantee on the
        // `resolve_account` / `account_by_id` path so a future change that
        // reintroduces thread-bound lookup there cannot slip through.
        let user = UserId::new("alice").unwrap();
        let mut thread_a = ResourceScope::local_default(user.clone(), InvocationId::new()).unwrap();
        thread_a.thread_id = Some(ThreadId::new("thread-a").unwrap());
        let create_scope = AuthProductScope::new(thread_a, AuthSurface::Api);
        let calendar_scope =
            ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();

        let auth = Arc::new(InMemoryAuthProductServices::new());
        let account = auth
            .create_account(NewCredentialAccount {
                scope: create_scope,
                provider: google_provider_id().unwrap(),
                label: CredentialAccountLabel::new("work google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::ExtensionOwned,
                owner_extension: Some(ExtensionId::new("google-drive").unwrap()),
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("google-access-token").unwrap()),
                refresh_secret: None,
                scopes: vec![calendar_scope.clone()],
            })
            .await
            .unwrap();
        let resolver = GoogleCredentialResolver::new(auth.clone(), auth.clone());

        let mut thread_b = ResourceScope::local_default(user, InvocationId::new()).unwrap();
        thread_b.thread_id = Some(ThreadId::new("thread-b").unwrap());
        let lookup_scope = AuthProductScope::new(thread_b.clone(), AuthSurface::Api);

        let credential = resolver
            .resolve_account(
                &thread_b,
                &lookup_scope,
                &ExtensionId::new("google-calendar").unwrap(),
                account.id,
                &[calendar_scope],
            )
            .await
            .expect("known account must resolve by id from a different thread");

        assert_eq!(credential.account_id, account.id);
    }

    #[tokio::test]
    async fn refresh_reuses_gsuite_owned_google_account_from_durable_store() {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let auth_scope = AuthProductScope::new(scope.clone(), AuthSurface::Api);
        let auth = Arc::new(InMemoryAuthProductServices::new());
        let calendar_scope =
            ProviderScope::new("https://www.googleapis.com/auth/calendar.readonly").unwrap();
        let stale_access = SecretHandle::new("google-stale-access-token").unwrap();
        let account = auth
            .create_account(NewCredentialAccount {
                scope: auth_scope.clone(),
                provider: google_provider_id().unwrap(),
                label: CredentialAccountLabel::new("work google").unwrap(),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::ExtensionOwned,
                owner_extension: Some(ExtensionId::new("google-drive").unwrap()),
                granted_extensions: Vec::new(),
                access_secret: Some(stale_access.clone()),
                refresh_secret: Some(SecretHandle::new("google-refresh-token").unwrap()),
                scopes: vec![calendar_scope],
            })
            .await
            .unwrap();
        let resolver = GoogleCredentialResolver::new(auth.clone(), auth.clone());

        resolver
            .refresh(
                &scope,
                &auth_scope,
                &ExtensionId::new("google-calendar").unwrap(),
                account.id,
            )
            .await
            .unwrap();

        let updated = auth
            .accounts_for_owner(&auth_scope)
            .await
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.id == account.id)
            .expect("account remains stored");
        assert_ne!(updated.access_secret, Some(stale_access));
        assert_eq!(updated.status, CredentialAccountStatus::Configured);
    }

    #[tokio::test]
    async fn resolve_account_returns_recovery_when_account_has_wrong_provider() {
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
        account.provider = AuthProviderId::new("github").unwrap();
        let account_service = Arc::new(FakeCredentialAccountService {
            account: account.clone(),
        });
        let resolver =
            GoogleCredentialResolver::new(account_service.clone(), account_service.clone());

        let error = resolver
            .resolve_account(
                &scope,
                &auth_scope,
                &ExtensionId::new("gmail").unwrap(),
                account.id,
                &[],
            )
            .await
            .unwrap_err();

        assert!(matches!(error, GoogleCredentialError::Recovery(_)));
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

    struct ThreadSensitiveRecoveryService {
        account: CredentialAccount,
        recovery_scope: std::sync::Mutex<Option<AuthProductScope>>,
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
    impl CredentialAccountRecordSource for FakeCredentialAccountService {
        async fn accounts_for_owner(
            &self,
            scope: &AuthProductScope,
        ) -> Result<Vec<CredentialAccount>, AuthProductError> {
            let owner = CredentialAccountOwnerScope::from_scope(scope);
            Ok(owner
                .matches(&self.account)
                .then(|| self.account.clone())
                .into_iter()
                .collect())
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
    impl CredentialAccountRecordSource for ThreadSensitiveRecoveryService {
        async fn accounts_for_owner(
            &self,
            scope: &AuthProductScope,
        ) -> Result<Vec<CredentialAccount>, AuthProductError> {
            let owner = CredentialAccountOwnerScope::from_scope(scope);
            Ok(owner
                .matches(&self.account)
                .then(|| self.account.clone())
                .into_iter()
                .collect())
        }
    }

    #[async_trait]
    impl CredentialAccountService for ThreadSensitiveRecoveryService {
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
            request: CredentialRecoveryRequest,
        ) -> Result<CredentialRecoveryProjection, AuthProductError> {
            *self.recovery_scope.lock().unwrap() = Some(request.scope.clone());
            if request.scope.resource.thread_id.is_none() {
                Ok(CredentialRecoveryProjection::configured(
                    google_provider_id().unwrap(),
                    self.account.projection(),
                ))
            } else {
                Ok(CredentialRecoveryProjection::setup_required(
                    google_provider_id().unwrap(),
                    CredentialRecoveryReason::NoAccount,
                    Vec::new(),
                ))
            }
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
    impl CredentialAccountRecordSource for MissingSelectedAccountService {
        async fn accounts_for_owner(
            &self,
            _scope: &AuthProductScope,
        ) -> Result<Vec<CredentialAccount>, AuthProductError> {
            Ok(Vec::new())
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
