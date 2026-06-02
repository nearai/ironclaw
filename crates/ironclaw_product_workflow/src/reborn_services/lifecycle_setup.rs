use ironclaw_auth::{AuthProductScope, AuthSurface};
use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope};
use uuid::Uuid;

use crate::{
    LifecycleExtensionCredentialRequirement, LifecyclePackageRef, LifecycleProductContext,
    LifecycleProductFacade, LifecycleProductResponse, LifecycleProductSurfaceContext,
    ProductWorkflowError, RebornExtensionSetupSecret, RebornServicesError, RebornServicesErrorCode,
    RebornSetupExtensionResponse, WebUiAuthenticatedCaller, WebUiInboundValidationCode,
    WebUiInboundValidationError, WebUiSetupExtensionRequest,
};

use super::{ExtensionCredentialSetupService, extension_setup_credentials};

pub(super) async fn setup_extension(
    facade: &dyn LifecycleProductFacade,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
    request: WebUiSetupExtensionRequest,
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let scope = setup_scope(&caller, &package_ref);
    let extension_id = ExtensionId::new(package_ref.id.as_str())
        .map_err(|_| RebornServicesError::internal_invariant())?;
    let lifecycle = facade
        .project_package(
            LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                tenant_id: caller.tenant_id.clone(),
                user_id: caller.user_id.clone(),
                agent_id: caller.agent_id.clone(),
                project_id: caller.project_id.clone(),
            }),
            package_ref.clone(),
        )
        .await
        .map_err(map_lifecycle_error)?;
    let requirements = extension_setup_credentials::requirements(&lifecycle);
    if request.action.as_deref() == Some("submit") {
        extension_setup_credentials::submit_manual_tokens(
            extension_credentials,
            scope.clone(),
            &extension_id,
            &requirements,
            request,
        )
        .await?;
        let refreshed = facade
            .project_package(
                LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                    tenant_id: caller.tenant_id,
                    user_id: caller.user_id,
                    agent_id: caller.agent_id,
                    project_id: caller.project_id,
                }),
                package_ref,
            )
            .await
            .map_err(map_lifecycle_error)?;
        return setup_extension_response(extension_credentials, scope, &extension_id, refreshed)
            .await;
    }
    if request.action.is_some() {
        return Err(validation_error(
            "action",
            WebUiInboundValidationCode::InvalidValue,
        ));
    }
    setup_extension_response(extension_credentials, scope, &extension_id, lifecycle).await
}

async fn setup_extension_response(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    lifecycle: LifecycleProductResponse,
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let package_ref = lifecycle
        .package_ref
        .clone()
        .ok_or_else(RebornServicesError::internal_invariant)?;
    let secrets = setup_secrets(
        extension_credentials,
        scope,
        extension_id,
        &extension_setup_credentials::requirements(&lifecycle),
    )
    .await?;
    Ok(RebornSetupExtensionResponse {
        package_ref,
        phase: lifecycle.phase,
        blockers: lifecycle.blockers,
        payload: lifecycle.payload,
        secrets,
        fields: Vec::new(),
        onboarding: None,
    })
}

async fn setup_secrets(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<Vec<RebornExtensionSetupSecret>, RebornServicesError> {
    extension_setup_credentials::project(extension_credentials, scope, extension_id, requirements)
        .await
}

fn setup_scope(
    caller: &WebUiAuthenticatedCaller,
    package_ref: &LifecyclePackageRef,
) -> AuthProductScope {
    let seed = format!(
        "webui-v2-extension-setup:{}:{}:{}:{}:{}",
        caller.tenant_id.as_str(),
        caller.user_id.as_str(),
        caller.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        caller
            .project_id
            .as_ref()
            .map(|id| id.as_str())
            .unwrap_or(""),
        package_ref.id.as_str()
    );
    AuthProductScope::new(
        ResourceScope {
            tenant_id: caller.tenant_id.clone(),
            user_id: caller.user_id.clone(),
            agent_id: caller.agent_id.clone(),
            project_id: caller.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::from_uuid(Uuid::new_v5(
                &Uuid::NAMESPACE_OID,
                seed.as_bytes(),
            )),
        },
        AuthSurface::Web,
    )
}

fn validation_error(field: &'static str, code: WebUiInboundValidationCode) -> RebornServicesError {
    RebornServicesError::from(WebUiInboundValidationError::new(field, code))
}

pub(super) fn map_lifecycle_error(error: ProductWorkflowError) -> RebornServicesError {
    match error {
        ProductWorkflowError::InvalidBindingRequest { .. }
        | ProductWorkflowError::UnsupportedActionKind { .. } => {
            RebornServicesError::from_status(RebornServicesErrorCode::InvalidRequest, 400, false)
        }
        ProductWorkflowError::BindingAccessDenied => {
            RebornServicesError::from_status(RebornServicesErrorCode::Forbidden, 403, false)
        }
        ProductWorkflowError::Transient { .. } => RebornServicesError::service_unavailable(true),
        ProductWorkflowError::BindingResolutionFailed { .. }
        | ProductWorkflowError::BindingRequired { .. }
        | ProductWorkflowError::TurnSubmissionRejected { .. }
        | ProductWorkflowError::TurnSubmissionFailed { .. }
        | ProductWorkflowError::TurnResumeRejected { .. }
        | ProductWorkflowError::TurnResumeDenied { .. }
        | ProductWorkflowError::ApprovalInteractionRejected { .. }
        | ProductWorkflowError::AuthInteractionRejected { .. }
        | ProductWorkflowError::AuthContinuationRejected { .. }
        | ProductWorkflowError::BeforeInboundPolicyFailed { .. }
        | ProductWorkflowError::DuplicateAction { .. }
        | ProductWorkflowError::UnknownInstallation => RebornServicesError::internal_invariant(),
    }
}
