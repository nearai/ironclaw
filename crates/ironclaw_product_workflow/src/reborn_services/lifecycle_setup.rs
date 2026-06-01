use crate::{
    LifecyclePackageRef, LifecycleProductContext, LifecycleProductFacade, LifecycleProductResponse,
    LifecycleProductSurfaceContext, ProductWorkflowError, RebornServicesError,
    RebornServicesErrorCode, RebornSetupExtensionResponse, WebUiAuthenticatedCaller,
    WebUiSetupExtensionRequest,
};

pub(super) async fn setup_extension(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
    _request: WebUiSetupExtensionRequest,
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let lifecycle = facade
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
    setup_extension_response(lifecycle)
}

fn setup_extension_response(
    lifecycle: LifecycleProductResponse,
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let package_ref = lifecycle
        .package_ref
        .ok_or_else(RebornServicesError::internal_invariant)?;
    Ok(RebornSetupExtensionResponse {
        package_ref,
        phase: lifecycle.phase,
        blockers: lifecycle.blockers,
        payload: lifecycle.payload,
    })
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
