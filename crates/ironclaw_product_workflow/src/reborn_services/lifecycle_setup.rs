use ironclaw_common::ExtensionName;

use crate::{
    LifecyclePackageKind, LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductResponse, LifecycleProductSurfaceContext, ProductWorkflowError,
    RebornServicesError, RebornServicesErrorCode, RebornSetupExtensionResponse,
    WebUiAuthenticatedCaller, WebUiSetupExtensionRequest,
};

pub(super) async fn setup_extension(
    facade: &dyn LifecycleProductFacade,
    caller: WebUiAuthenticatedCaller,
    extension_name: ExtensionName,
    request: WebUiSetupExtensionRequest,
) -> Result<RebornSetupExtensionResponse, RebornServicesError> {
    let lifecycle = facade
        .execute(
            LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                tenant_id: caller.tenant_id,
                user_id: caller.user_id,
                agent_id: caller.agent_id,
                project_id: caller.project_id,
            }),
            LifecycleProductAction::ExtensionConfigure {
                package_ref: crate::lifecycle::lifecycle_package_ref(
                    LifecyclePackageKind::Extension,
                    extension_name.as_str(),
                )
                .map_err(map_lifecycle_error)?,
                payload: setup_extension_lifecycle_payload(request),
            },
        )
        .await
        .map_err(map_lifecycle_error)?;
    Ok(setup_extension_response(extension_name, lifecycle))
}

fn setup_extension_lifecycle_payload(
    request: WebUiSetupExtensionRequest,
) -> Option<serde_json::Value> {
    match (request.action, request.payload) {
        (None, None) => None,
        (action, payload) => Some(serde_json::json!({
            "action": action,
            "payload": payload,
        })),
    }
}

fn setup_extension_response(
    extension_name: ExtensionName,
    lifecycle: LifecycleProductResponse,
) -> RebornSetupExtensionResponse {
    RebornSetupExtensionResponse {
        extension_name,
        phase: lifecycle.phase,
        blockers: lifecycle.blockers,
        package_ref: lifecycle.package_ref,
        payload: lifecycle.payload,
    }
}

fn map_lifecycle_error(error: ProductWorkflowError) -> RebornServicesError {
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
        | ProductWorkflowError::AuthContinuationRejected { .. }
        | ProductWorkflowError::BeforeInboundPolicyFailed { .. }
        | ProductWorkflowError::DuplicateAction { .. }
        | ProductWorkflowError::UnknownInstallation => {
            RebornServicesError::internal_invariant()
        }
    }
}
