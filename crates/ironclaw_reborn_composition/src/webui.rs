use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_auth::{
    AuthContinuationRef, AuthErrorCode, AuthProductError, CredentialAccountLabel,
    CredentialAccountSelectionRequest,
};
use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ExtensionCredentialSetupService, ExtensionCredentialStatusRequest,
    ExtensionCredentialSubmitRequest, RebornServices as ProductRebornServices, RebornServicesApi,
    RebornServicesError, RebornServicesErrorCode, RebornServicesErrorKind,
};

use crate::{
    RebornBuildError, RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest,
    RebornProductAuthServices, RebornReadiness, RebornRuntime,
    lifecycle::RebornLocalLifecycleFacade,
};

const EXTENSION_CREDENTIAL_SETUP_TTL_SECONDS: i64 = 300;

/// WebUI-facing Reborn service bundle for host composition.
///
/// This bundle deliberately exposes facade-shaped product handles consumed
/// by WebChat v2 and the optional product-auth OAuth routes. HTTP
/// routing, auth middleware, static assets, and SSE transport stay in the
/// WebUI crate (or, when the `webui-v2-beta` feature is on, the
/// [`crate::webui_serve`] module in this crate); lower runtime handles stay
/// behind the existing Reborn runtime / composition services.
#[derive(Clone)]
pub struct RebornWebuiBundle {
    pub api: Arc<dyn RebornServicesApi>,
    pub product_auth: Option<Arc<RebornProductAuthServices>>,
    pub readiness: RebornReadiness,
}

impl std::fmt::Debug for RebornWebuiBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornWebuiBundle")
            .field("api", &"Arc<dyn RebornServicesApi>")
            .field("product_auth", &self.product_auth.is_some())
            .field("readiness", &self.readiness)
            .finish()
    }
}

/// Compose the WebUI-facing product facade from an already-built Reborn runtime.
///
/// This function does not create a second turn coordinator, thread service,
/// host runtime or route server. It reuses the runtime's existing task-level
/// composition and attaches the runtime-owned projection stream unless the
/// caller supplies a custom stream.
pub fn build_webui_services(
    runtime: &RebornRuntime,
    event_stream: Option<Arc<dyn ProjectionStream>>,
) -> Result<RebornWebuiBundle, RebornBuildError> {
    let services = runtime.services();

    let mut api = ProductRebornServices::new(
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    )
    .with_approval_interactions(runtime.webui_approval_interaction_service())
    .with_auth_interactions(runtime.webui_auth_interaction_service());
    if let Some(skill_activation_source) = runtime.webui_skill_activation_source() {
        let activation_recorder = Arc::clone(&skill_activation_source);
        let activation_clearer = skill_activation_source;
        api = api.with_skill_activation_hooks(
            move |scope, accepted_message_ref, message| {
                activation_recorder
                    .record_user_message(scope.clone(), accepted_message_ref.clone(), message)
                    .map_err(|_| RebornServicesError {
                        code: RebornServicesErrorCode::Internal,
                        kind: RebornServicesErrorKind::Internal,
                        status_code: 500,
                        retryable: false,
                        field: None,
                        validation_code: None,
                    })
            },
            move |scope, accepted_message_ref| {
                activation_clearer
                    .clear_accepted_message(scope, accepted_message_ref)
                    .map_err(|_| RebornServicesError {
                        code: RebornServicesErrorCode::Internal,
                        kind: RebornServicesErrorKind::Internal,
                        status_code: 500,
                        retryable: false,
                        field: None,
                        validation_code: None,
                    })
            },
        );
    }
    if let Some(local_runtime) = &services.local_runtime {
        let mut lifecycle_facade =
            RebornLocalLifecycleFacade::new(local_runtime.skill_management.clone());
        if let Some(extension_management) = &local_runtime.extension_management {
            lifecycle_facade =
                lifecycle_facade.with_extension_management(extension_management.clone());
        }
        api = api.with_lifecycle_product_facade(Arc::new(lifecycle_facade));
    }
    if let Some(product_auth) = &services.product_auth {
        api = api.with_extension_credentials(Arc::new(ProductAuthExtensionCredentialSetup::new(
            Arc::clone(product_auth),
        )));
    }
    api = api.with_event_stream(event_stream.unwrap_or_else(|| runtime.webui_event_stream()));

    Ok(RebornWebuiBundle {
        api: Arc::new(api),
        product_auth: services.product_auth.clone(),
        readiness: services.readiness,
    })
}

#[derive(Clone)]
struct ProductAuthExtensionCredentialSetup {
    product_auth: Arc<RebornProductAuthServices>,
}

impl ProductAuthExtensionCredentialSetup {
    fn new(product_auth: Arc<RebornProductAuthServices>) -> Self {
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
                CredentialAccountSelectionRequest::new(request.scope, request.provider)
                    .for_extension(request.requester_extension),
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
