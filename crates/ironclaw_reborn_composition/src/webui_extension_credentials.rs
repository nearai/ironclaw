use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthContinuationRef, AuthErrorCode, AuthProductError, CredentialAccountLabel,
    CredentialAccountSelectionRequest,
};
use ironclaw_product_workflow::{
    ExtensionCredentialSetupService, ExtensionCredentialStatusRequest,
    ExtensionCredentialSubmitRequest, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind,
};

use crate::{
    RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest, RebornProductAuthServices,
    product_auth_runtime_credentials::RuntimeCredentialAccountSelectionRequest,
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
    ) -> Result<Option<ironclaw_auth::CredentialAccountProjection>, RebornServicesError> {
        let selector = self
            .product_auth
            .runtime_credential_account_selection_service();
        let account = selector
            .select_unique_configured_runtime_account(
                RuntimeCredentialAccountSelectionRequest::new(
                    CredentialAccountSelectionRequest::new(request.scope.clone(), request.provider)
                        .for_extension(request.requester_extension),
                    request.scope,
                ),
            )
            .await
            .map_err(|error| match error {
                AuthProductError::CredentialMissing => None,
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
    ) -> Result<ironclaw_auth::CredentialAccountId, RebornServicesError> {
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

fn map_auth_error(error: crate::RebornAuthProductError) -> RebornServicesError {
    match error.code {
        AuthErrorCode::InvalidRequest | AuthErrorCode::MalformedCallback => {
            invalid_auth_setup_request()
        }
        AuthErrorCode::CrossScopeDenied => services_error(
            RebornServicesErrorCode::Forbidden,
            RebornServicesErrorKind::ParticipantDenied,
            403,
            false,
        ),
        AuthErrorCode::BackendUnavailable => services_error(
            RebornServicesErrorCode::Unavailable,
            RebornServicesErrorKind::ServiceUnavailable,
            503,
            error.retryable,
        ),
        AuthErrorCode::AccountSelectionRequired => services_error(
            RebornServicesErrorCode::Conflict,
            RebornServicesErrorKind::BlockedAuthentication,
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
            RebornServicesErrorCode::Internal,
            RebornServicesErrorKind::BlockedAuthentication,
            500,
            error.retryable,
        ),
    }
}

fn invalid_auth_setup_request() -> RebornServicesError {
    services_error(
        RebornServicesErrorCode::InvalidRequest,
        RebornServicesErrorKind::Validation,
        400,
        false,
    )
}

fn services_error(
    code: RebornServicesErrorCode,
    kind: RebornServicesErrorKind,
    status_code: u16,
    retryable: bool,
) -> RebornServicesError {
    RebornServicesError {
        code,
        kind,
        status_code,
        retryable,
        field: None,
        validation_code: None,
    }
}
