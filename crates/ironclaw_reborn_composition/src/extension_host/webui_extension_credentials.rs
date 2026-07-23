use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthContinuationRef, AuthErrorCode, AuthProductError, CredentialAccountLabel,
    CredentialAccountSelectionRequest,
};
use ironclaw_host_api::{ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind};
use ironclaw_product::{
    ExtensionCredentialSetupService, ExtensionCredentialStatusRequest,
    ExtensionCredentialSubmitRequest, LifecycleExtensionCredentialSetup,
};

use crate::{
    RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest, RebornProductAuthServices,
    product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionRequest,
};

const EXTENSION_CREDENTIAL_SETUP_TTL_SECONDS: i64 = 300;

#[derive(Clone)]
pub(crate) struct ProductAuthExtensionCredentialSetup {
    product_auth: Arc<RebornProductAuthServices>,
}

impl ProductAuthExtensionCredentialSetup {
    pub(crate) fn new(product_auth: Arc<RebornProductAuthServices>) -> Self {
        Self { product_auth }
    }
}

#[async_trait]
impl ExtensionCredentialSetupService for ProductAuthExtensionCredentialSetup {
    async fn credential_status(
        &self,
        request: ExtensionCredentialStatusRequest,
    ) -> Result<Option<ironclaw_auth::CredentialAccountProjection>, ProductSurfaceError> {
        let selector = self
            .product_auth
            .runtime_credential_account_selection_service();
        let account = selector
            .select_unique_configured_runtime_account(
                RuntimeCredentialAccountSelectionRequest::new(
                    CredentialAccountSelectionRequest::new(request.scope.clone(), request.provider)
                        .for_extension(request.requester_extension),
                    request.scope,
                    runtime_credential_setup(request.setup),
                    request.provider_scopes,
                ),
            )
            .await
            .map_err(|error| match error {
                AuthProductError::CredentialMissing
                | AuthProductError::CrossScopeDenied
                | AuthProductError::AccountSelectionRequired => None,
                other => Some(map_auth_error(other.into())),
            });
        match account {
            Ok(account) => Ok(Some(account.projection())),
            Err(None) => Ok(None),
            Err(Some(error)) => Err(error),
        }
    }

    async fn submit_manual_token(
        &self,
        request: ExtensionCredentialSubmitRequest,
    ) -> Result<ironclaw_auth::CredentialAccountId, ProductSurfaceError> {
        let label =
            CredentialAccountLabel::new(request.label).map_err(|_| invalid_auth_setup_request())?;
        let expires_at =
            Utc::now() + ChronoDuration::seconds(EXTENSION_CREDENTIAL_SETUP_TTL_SECONDS);
        let mut setup = RebornManualTokenSetupRequest::new(
            request.scope.clone(),
            request.provider,
            label,
            AuthContinuationRef::SetupOnly,
            expires_at,
        );
        if let Some(binding) = request.existing_account {
            setup = setup.with_update_binding(binding);
        }
        let challenge = self
            .product_auth
            .request_manual_token_setup(setup)
            .await
            .map_err(map_auth_error)?;
        let submitted = self
            .product_auth
            .submit_manual_token(RebornManualTokenSubmitRequest::new(
                request.scope,
                challenge.interaction_id,
                request.secret,
            ))
            .await
            .map_err(map_auth_error)?;
        Ok(submitted.account_id)
    }
}

fn map_auth_error(error: crate::RebornAuthProductError) -> ProductSurfaceError {
    match error.code {
        AuthErrorCode::InvalidRequest | AuthErrorCode::MalformedCallback => {
            invalid_auth_setup_request()
        }
        AuthErrorCode::CrossScopeDenied => services_error(
            ProductSurfaceErrorCode::Forbidden,
            ProductSurfaceErrorKind::ParticipantDenied,
            403,
            false,
        ),
        AuthErrorCode::BackendUnavailable | AuthErrorCode::MalformedConfig => services_error(
            ProductSurfaceErrorCode::Unavailable,
            ProductSurfaceErrorKind::ServiceUnavailable,
            503,
            error.retryable,
        ),
        AuthErrorCode::AccountSelectionRequired
        | AuthErrorCode::ProviderIdentityAlreadyConnected
        | AuthErrorCode::LifecycleActivationFailed => services_error(
            ProductSurfaceErrorCode::Conflict,
            ProductSurfaceErrorKind::BlockedAuthentication,
            409,
            false,
        ),
        AuthErrorCode::CredentialMissing
        | AuthErrorCode::UnknownOrExpiredFlow
        | AuthErrorCode::ProviderDenied
        | AuthErrorCode::TokenExchangeFailed
        | AuthErrorCode::RefreshFailed
        | AuthErrorCode::Canceled
        | AuthErrorCode::FlowAlreadyTerminal => services_error(
            ProductSurfaceErrorCode::Internal,
            ProductSurfaceErrorKind::BlockedAuthentication,
            500,
            error.retryable,
        ),
    }
}

fn runtime_credential_setup(
    setup: LifecycleExtensionCredentialSetup,
) -> ironclaw_host_api::RuntimeCredentialAccountSetup {
    match setup {
        LifecycleExtensionCredentialSetup::ManualToken => {
            ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken
        }
        LifecycleExtensionCredentialSetup::OAuth { scopes } => {
            ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth { scopes }
        }
        LifecycleExtensionCredentialSetup::Pairing => {
            ironclaw_host_api::RuntimeCredentialAccountSetup::Pairing
        }
    }
}

fn invalid_auth_setup_request() -> ProductSurfaceError {
    services_error(
        ProductSurfaceErrorCode::InvalidRequest,
        ProductSurfaceErrorKind::Validation,
        400,
        false,
    )
}

fn services_error(
    code: ProductSurfaceErrorCode,
    kind: ProductSurfaceErrorKind,
    status_code: u16,
    retryable: bool,
) -> ProductSurfaceError {
    ProductSurfaceError {
        code,
        kind,
        status_code,
        retryable,
        field: None,
        validation_code: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::ProductAuthExtensionCredentialSetup;
    use async_trait::async_trait;
    use ironclaw_auth::{
        AuthContinuationEvent, AuthProductError, AuthProductScope, AuthProviderId, AuthSurface,
        CredentialAccountLabel, CredentialAccountService, CredentialAccountStatus,
        CredentialOwnership, InMemoryAuthProductServices, NewCredentialAccount, ProviderScope,
    };
    use ironclaw_host_api::{
        ExtensionId, InvocationId, ResourceScope, SecretHandle, TenantId, UserId,
    };
    use ironclaw_product::{
        ExtensionCredentialSetupService, ExtensionCredentialStatusRequest,
        LifecycleExtensionCredentialSetup,
    };

    use crate::{RebornAuthContinuationDispatcher, RebornProductAuthServices};

    struct NoopDispatcher;

    #[async_trait]
    impl RebornAuthContinuationDispatcher for NoopDispatcher {
        async fn dispatch_auth_continuation(
            &self,
            _event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
        async fn dispatch_canceled_auth_continuation(
            &self,
            _event: AuthContinuationEvent,
        ) -> Result<(), AuthProductError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn credential_status_reports_most_recent_of_multiple_reusable_accounts() {
        // Runtime default rule (#auth-gate-reuse): multiple reusable, unbound
        // accounts for one provider no longer surface as ambiguous. The runtime
        // resolver — which `credential_status` shares — deterministically selects
        // the most-recently-created account, so the extension is reported as
        // connected against that account rather than as needing reconnect. This
        // keeps the status surface consistent with the credential the runtime
        // gate will actually use. (Per-account selection is a setup-time picker
        // concern, tracked separately.)
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let service = ProductAuthExtensionCredentialSetup::new(Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopDispatcher)),
        ));
        let scope = test_scope();
        seed_account(&shared, scope.clone(), "notion primary").await;
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        seed_account(&shared, scope.clone(), "notion secondary").await;

        let status = service
            .credential_status(ExtensionCredentialStatusRequest {
                scope,
                provider: AuthProviderId::new("notion").expect("provider"),
                setup: LifecycleExtensionCredentialSetup::ManualToken,
                provider_scopes: Vec::new(),
                requester_extension: ExtensionId::new("notion").expect("extension"),
            })
            .await
            .expect("status lookup should not block setup");

        let account =
            status.expect("most-recent reusable account must resolve, not stay ambiguous");
        assert_eq!(account.label.as_str(), "notion secondary");
    }

    #[tokio::test]
    async fn credential_status_treats_unauthorized_accounts_as_reconnectable() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let service = ProductAuthExtensionCredentialSetup::new(Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopDispatcher)),
        ));
        let scope = test_scope();
        shared
            .create_account(NewCredentialAccount {
                scope: scope.clone(),
                provider: AuthProviderId::new("notion").expect("provider"),
                label: CredentialAccountLabel::new("admin notion").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::SharedAdminManaged,
                owner_extension: None,
                granted_extensions: vec![ExtensionId::new("other-extension").expect("extension")],
                access_secret: Some(SecretHandle::new("admin-notion-access").expect("secret")),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .expect("seed admin account");

        let status = service
            .credential_status(ExtensionCredentialStatusRequest {
                scope,
                provider: AuthProviderId::new("notion").expect("provider"),
                setup: LifecycleExtensionCredentialSetup::ManualToken,
                provider_scopes: Vec::new(),
                requester_extension: ExtensionId::new("notion").expect("extension"),
            })
            .await
            .expect("status lookup should not block setup");

        assert!(status.is_none());
    }

    #[tokio::test]
    async fn credential_status_finds_callback_surface_google_oauth_account_for_gsuite_extensions() {
        let shared = Arc::new(InMemoryAuthProductServices::new());
        let service = ProductAuthExtensionCredentialSetup::new(Arc::new(
            RebornProductAuthServices::from_shared(shared.clone(), Arc::new(NoopDispatcher)),
        ));
        let ui_scope = test_scope();
        let callback_scope =
            AuthProductScope::new(ui_scope.resource.clone(), AuthSurface::Callback);
        shared
            .create_account(NewCredentialAccount {
                scope: callback_scope,
                provider: AuthProviderId::new("google").expect("provider"),
                label: CredentialAccountLabel::new("work google").expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("google-access").expect("secret")),
                refresh_secret: None,
                scopes: vec![
                    ProviderScope::new("https://www.googleapis.com/auth/gmail.modify")
                        .expect("gmail scope"),
                    ProviderScope::new("https://www.googleapis.com/auth/calendar.events")
                        .expect("calendar scope"),
                ],
            })
            .await
            .expect("seed google account");

        for (extension, scope) in [
            ("gmail", "https://www.googleapis.com/auth/gmail.modify"),
            (
                "google-calendar",
                "https://www.googleapis.com/auth/calendar.events",
            ),
        ] {
            let status = service
                .credential_status(ExtensionCredentialStatusRequest {
                    scope: ui_scope.clone(),
                    provider: AuthProviderId::new("google").expect("provider"),
                    setup: LifecycleExtensionCredentialSetup::OAuth {
                        scopes: vec![scope.to_string()],
                    },
                    provider_scopes: vec![ProviderScope::new(scope).expect("scope")],
                    requester_extension: ExtensionId::new(extension).expect("extension"),
                })
                .await
                .expect("status lookup should succeed");

            assert!(
                status.is_some(),
                "{extension} should see callback-surface Google OAuth account as configured"
            );
        }
    }

    async fn seed_account(
        shared: &InMemoryAuthProductServices,
        scope: AuthProductScope,
        label: &str,
    ) {
        let handle_label = label.replace(' ', "-");
        shared
            .create_account(NewCredentialAccount {
                scope,
                provider: AuthProviderId::new("notion").expect("provider"),
                label: CredentialAccountLabel::new(label.to_string()).expect("label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    SecretHandle::new(format!("{handle_label}-access")).expect("secret"),
                ),
                refresh_secret: None,
                scopes: Vec::new(),
            })
            .await
            .expect("seed account");
    }

    fn test_scope() -> AuthProductScope {
        AuthProductScope::new(
            ResourceScope {
                tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
                user_id: UserId::new("user-alpha").expect("user"),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            AuthSurface::Callback,
        )
    }
}
