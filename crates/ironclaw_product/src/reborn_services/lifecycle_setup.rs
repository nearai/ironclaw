use ironclaw_auth::AuthProductScope;
use ironclaw_host_api::{
    ExtensionId, ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode,
    ProductSurfaceValidationCode,
};

use crate::{
    LifecycleExtensionCredentialRequirement, LifecyclePackageKind, LifecyclePackageRef,
    LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
    LifecycleProductResponse, LifecycleProductSurfaceContext, LifecyclePublicState,
    ProductSetupExtensionRequest, ProductWorkflowError, RebornSetupExtensionResponse,
    RebornViewDescriptor,
};

use super::{
    ExtensionCredentialSetupService,
    extension_credentials::{ExtensionCredentialReadiness, credential_scope},
    extension_onboarding, extension_setup_credentials, views,
};

pub const EXTENSION_SETUP_VIEW: RebornViewDescriptor = RebornViewDescriptor {
    id: "extension_setup",
    paginated: false,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetupAction {
    View,
    Submit,
}

pub(super) async fn setup_extension_view(
    facade: &dyn LifecycleProductFacade,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: ProductSurfaceCaller,
    params: serde_json::Value,
) -> Result<RebornSetupExtensionResponse, ProductSurfaceError> {
    let package_id = views::required_string_view_param(params, "package_id")?;
    let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
        .map_err(map_lifecycle_error)?;
    setup_extension(
        facade,
        extension_credentials,
        caller,
        package_ref,
        ProductSetupExtensionRequest::default(),
    )
    .await
}

pub(super) async fn submit_extension_setup_capability(
    facade: &dyn LifecycleProductFacade,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: ProductSurfaceCaller,
    input: serde_json::Value,
) -> Result<(), ProductSurfaceError> {
    let mut object = match input {
        serde_json::Value::Object(object) => object,
        _ => {
            return Err(validation_error(
                "input",
                ProductSurfaceValidationCode::InvalidValue,
            ));
        }
    };
    let package_id = object
        .remove("extension_id")
        .or_else(|| object.remove("package_id"))
        .and_then(|value| value.as_str().map(ToString::to_string))
        .ok_or_else(|| {
            validation_error("extension_id", ProductSurfaceValidationCode::MissingField)
        })?;
    let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
        .map_err(map_lifecycle_error)?;
    let request = serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|_| validation_error("input", ProductSurfaceValidationCode::InvalidValue))?;
    setup_extension(facade, extension_credentials, caller, package_ref, request)
        .await
        .map(|_| ())
}

pub(super) async fn setup_extension(
    facade: &dyn LifecycleProductFacade,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    caller: ProductSurfaceCaller,
    package_ref: LifecyclePackageRef,
    request: ProductSetupExtensionRequest,
) -> Result<RebornSetupExtensionResponse, ProductSurfaceError> {
    let action = setup_action(&request)?;
    let scope = credential_scope(&caller, &package_ref);
    let extension_id = ExtensionId::new(package_ref.id.as_str())
        .map_err(|_| ProductSurfaceError::internal_invariant())?;
    let context = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
        tenant_id: caller.tenant_id,
        user_id: caller.user_id,
        agent_id: caller.agent_id,
        project_id: caller.project_id,
    });
    let lifecycle = project_package(facade, context.clone(), package_ref.clone()).await?;
    let requirements = extension_setup_credentials::requirements(&lifecycle);
    if action == SetupAction::Submit {
        let submit = extension_setup_credentials::parse_submit_payload(request)?;
        if !submit.fields.is_empty() {
            return Err(validation_error(
                "fields",
                ProductSurfaceValidationCode::InvalidValue,
            ));
        }
        extension_setup_credentials::submit_manual_tokens(
            extension_credentials,
            scope.clone(),
            &extension_id,
            &requirements,
            submit.secrets,
        )
        .await?;
        // Saving the caller's final setup input completes the single install
        // transition. Activation/publication is an internal checkpoint, not a
        // second user action: attempt it immediately, then project the
        // authoritative caller-scoped state. If another requirement is still
        // missing, the projection remains `setup_needed`.
        facade
            .execute(
                context.clone(),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: package_ref.clone(),
                },
            )
            .await
            .map_err(map_lifecycle_error)?;
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
) -> Result<LifecycleProductResponse, ProductSurfaceError> {
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
) -> Result<RebornSetupExtensionResponse, ProductSurfaceError> {
    let package_ref = lifecycle
        .package_ref
        .clone()
        .ok_or_else(ProductSurfaceError::internal_invariant)?;
    let (secrets, readiness) = extension_setup_credentials::project(
        extension_credentials,
        scope,
        extension_id,
        requirements,
    )
    .await?;
    let phase = setup_public_phase(lifecycle.phase, readiness);
    let onboarding =
        extension_onboarding::from_lifecycle_with_credential_status(&lifecycle, readiness, false)
            .onboarding;
    Ok(RebornSetupExtensionResponse {
        package_ref,
        phase,
        blockers: lifecycle.blockers,
        onboarding,
        payload: lifecycle.payload,
        secrets,
    })
}

fn setup_public_phase(
    lifecycle_phase: LifecyclePublicState,
    readiness: ExtensionCredentialReadiness,
) -> LifecyclePublicState {
    match (lifecycle_phase, readiness) {
        (LifecyclePublicState::Uninstalled, _) => LifecyclePublicState::Uninstalled,
        (_, ExtensionCredentialReadiness::MissingRequired) => LifecyclePublicState::SetupNeeded,
        (phase, _) => phase,
    }
}

fn setup_action(
    request: &ProductSetupExtensionRequest,
) -> Result<SetupAction, ProductSurfaceError> {
    match request.action.as_deref() {
        None => Ok(SetupAction::View),
        Some("submit") => Ok(SetupAction::Submit),
        Some(_) => Err(validation_error(
            "action",
            ProductSurfaceValidationCode::InvalidValue,
        )),
    }
}

pub(super) fn validation_error(
    field: &'static str,
    code: ProductSurfaceValidationCode,
) -> ProductSurfaceError {
    ProductSurfaceError::validation(field, code)
}

pub(super) fn map_lifecycle_error(error: ProductWorkflowError) -> ProductSurfaceError {
    match error {
        ProductWorkflowError::InvalidBindingRequest { .. }
        | ProductWorkflowError::UnsupportedActionKind { .. } => {
            ProductSurfaceError::from_status(ProductSurfaceErrorCode::InvalidRequest, 400, false)
        }
        // Deployment configuration metadata is operator-only; ordinary caller
        // setup receives only the typed, free-text-free unavailable result.
        ProductWorkflowError::ProviderInstanceNotConfigured => {
            ProductSurfaceError::from_status(ProductSurfaceErrorCode::InvalidRequest, 400, false)
        }
        ProductWorkflowError::BindingAccessDenied => {
            ProductSurfaceError::from_status(ProductSurfaceErrorCode::Forbidden, 403, false)
        }
        ProductWorkflowError::Transient { ref reason } => {
            // The 503 body is sanitized; without this line the cause is
            // dropped entirely and the failure is undiagnosable from logs.
            tracing::warn!(reason = %reason, "lifecycle action failed with a transient error");
            ProductSurfaceError::service_unavailable(true)
        }
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
        | ProductWorkflowError::OutboundTargetNotDirectMessage
        | ProductWorkflowError::UnknownInstallation => ProductSurfaceError::internal_invariant(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The WebUI facade gets a plain sanitized 400 with no administrator
    /// metadata because the domain error itself carries none.
    #[test]
    fn provider_instance_not_configured_maps_to_sanitized_400() {
        let error = ProductWorkflowError::ProviderInstanceNotConfigured;

        let mapped = map_lifecycle_error(error);

        assert_eq!(mapped.code, ProductSurfaceErrorCode::InvalidRequest);
        assert_eq!(mapped.status_code, 400);
        assert!(!mapped.retryable);
    }
}
