use ironclaw_auth::AuthProductScope;
use ironclaw_host_api::ExtensionId;

use crate::{
    LifecycleExtensionCredentialRequirement, LifecyclePackageRef, LifecycleProductContext,
    LifecycleProductFacade, LifecycleProductResponse, LifecycleProductSurfaceContext,
    ProductWorkflowError, RebornServicesError, RebornServicesErrorCode,
    RebornSetupExtensionResponse, WebUiAuthenticatedCaller, WebUiInboundValidationCode,
    WebUiInboundValidationError, WebUiSetupExtensionRequest,
};

use super::{
    ExtensionCredentialSetupService, extension_credentials::credential_scope, extension_onboarding,
    extension_setup_credentials,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupAction {
    View,
    Submit,
}

pub(super) async fn setup_extension(
    facade: &dyn LifecycleProductFacade,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
    request: WebUiSetupExtensionRequest,
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let action = setup_action(&request)?;
    let scope = credential_scope(&caller, &package_ref);
    let extension_id = ExtensionId::new(package_ref.id.as_str())
        .map_err(|_| RebornServicesError::internal_invariant())?;
    let context = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
        tenant_id: caller.tenant_id,
        user_id: caller.user_id,
        agent_id: caller.agent_id,
        project_id: caller.project_id,
    });
    let lifecycle = project_package(facade, context.clone(), package_ref.clone()).await?;
    let requirements = extension_setup_credentials::requirements(&lifecycle);
    if action == SetupAction::Submit {
        extension_setup_credentials::submit_manual_tokens(
            extension_credentials,
            scope.clone(),
            &extension_id,
            &requirements,
            request,
        )
        .await?;
        let refreshed = project_package(facade, context, package_ref).await?;
        let refreshed_requirements = extension_setup_credentials::requirements(&refreshed);
        return setup_extension_response(
            extension_credentials,
            scope,
            &extension_id,
            refreshed,
            &refreshed_requirements,
        )
        .await;
    }
    setup_extension_response(
        extension_credentials,
        scope,
        &extension_id,
        lifecycle,
        &requirements,
    )
    .await
}

async fn project_package(
    facade: &dyn LifecycleProductFacade,
    context: LifecycleProductContext,
    package_ref: LifecyclePackageRef,
) -> Result<LifecycleProductResponse, RebornServicesError> {
    facade
        .project_package(context, package_ref)
        .await
        .map_err(map_lifecycle_error)
}

async fn setup_extension_response(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    lifecycle: LifecycleProductResponse,
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let package_ref = lifecycle
        .package_ref
        .clone()
        .ok_or_else(RebornServicesError::internal_invariant)?;
    let secrets = extension_setup_credentials::project(
        extension_credentials,
        scope,
        extension_id,
        requirements,
    )
    .await?;
    let onboarding = extension_onboarding::from_lifecycle(&lifecycle).onboarding;
    Ok(RebornSetupExtensionResponse {
        package_ref,
        phase: lifecycle.phase,
        blockers: lifecycle.blockers,
        onboarding,
        payload: lifecycle.payload,
        secrets,
        fields: Vec::new(),
    })
}

fn setup_action(request: &WebUiSetupExtensionRequest) -> Result<SetupAction, RebornServicesError> {
    match request.action.as_deref() {
        None => Ok(SetupAction::View),
        Some("submit") => Ok(SetupAction::Submit),
        Some(_) => Err(validation_error(
            "action",
            WebUiInboundValidationCode::InvalidValue,
        )),
    }
}

pub(super) fn validation_error(
    field: &'static str,
    code: WebUiInboundValidationCode,
) -> RebornServicesError {
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
