use std::collections::BTreeSet;

use ironclaw_auth::AuthProductScope;
use ironclaw_host_api::ExtensionId;

use crate::{
    ChannelConfigFacade, LifecycleExtensionCredentialRequirement, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleProductContext, LifecycleProductFacade, LifecycleProductResponse,
    LifecycleProductSurfaceContext, ProductSurfaceError, ProductSurfaceErrorCode,
    ProductSurfaceValidationCode, ProductWorkflowError, RebornChannelConfigField,
    RebornExtensionCredentialSetup, RebornExtensionSetupField, RebornExtensionSetupSecret,
    RebornSetupExtensionResponse, RebornViewDescriptor, WebUiAuthenticatedCaller,
    WebUiSetupExtensionRequest,
};

use super::{
    ExtensionCredentialSetupService, extension_credentials::credential_scope, extension_onboarding,
    extension_setup_credentials, extension_setup_credentials::SetupSubmitPayload, views,
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
    channel_config: Option<&dyn ChannelConfigFacade>,
    caller: WebUiAuthenticatedCaller,
    params: serde_json::Value,
) -> Result<RebornSetupExtensionResponse, ProductSurfaceError> {
    let package_id = views::required_string_view_param(params, "package_id")?;
    let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
        .map_err(map_lifecycle_error)?;
    setup_extension(
        facade,
        extension_credentials,
        channel_config,
        caller,
        package_ref,
        WebUiSetupExtensionRequest::default(),
    )
    .await
}

pub(super) async fn submit_extension_setup_capability(
    facade: &dyn LifecycleProductFacade,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    channel_config: Option<&dyn ChannelConfigFacade>,
    caller: WebUiAuthenticatedCaller,
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
    setup_extension(
        facade,
        extension_credentials,
        channel_config,
        caller,
        package_ref,
        request,
    )
    .await
    .map(|_| ())
}

pub(super) async fn setup_extension(
    facade: &dyn LifecycleProductFacade,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    channel_config: Option<&dyn ChannelConfigFacade>,
    caller: WebUiAuthenticatedCaller,
    package_ref: LifecyclePackageRef,
    request: WebUiSetupExtensionRequest,
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
        let mut submit = extension_setup_credentials::parse_submit_payload(request)?;
        if channel_config.is_none() && !submit.fields.is_empty() {
            return Err(ProductSurfaceError::service_unavailable(true));
        }
        let channel_fields = channel_field_status(channel_config, &extension_id).await?;
        let channel_values =
            route_channel_config_values(&mut submit, &channel_fields, &requirements)?;
        if !channel_values.is_empty() {
            let port =
                channel_config.ok_or_else(|| ProductSurfaceError::service_unavailable(true))?;
            port.save_values(&extension_id, channel_values).await?;
        }
        extension_setup_credentials::submit_manual_tokens(
            extension_credentials,
            scope.clone(),
            &extension_id,
            &requirements,
            submit.secrets,
        )
        .await?;
        let refreshed = project_package(facade, context, package_ref).await?;
        let refreshed_requirements = extension_setup_credentials::requirements(&refreshed);
        return setup_extension_response(
            extension_credentials,
            channel_config,
            scope,
            &extension_id,
            refreshed,
            &refreshed_requirements,
        )
        .await;
    }
    setup_extension_response(
        extension_credentials,
        channel_config,
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

async fn channel_field_status(
    channel_config: Option<&dyn ChannelConfigFacade>,
    extension_id: &ExtensionId,
) -> Result<Vec<RebornChannelConfigField>, ProductSurfaceError> {
    match channel_config {
        Some(port) => port.field_status(extension_id).await,
        None => Ok(Vec::new()),
    }
}

/// Split the submitted payload into channel-config values (routed to the
/// configure port) and credential secrets (left for the credential path).
/// Secret channel-config fields ride the `secrets` map under their handle;
/// a name that is also a declared credential requirement keeps the existing
/// credential path. Non-secret values ride the `fields` map and must match
/// a declared non-secret field handle.
fn route_channel_config_values(
    submit: &mut SetupSubmitPayload,
    channel_fields: &[RebornChannelConfigField],
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<Vec<(String, String)>, ProductSurfaceError> {
    let requirement_names: BTreeSet<&str> = requirements
        .iter()
        .map(|requirement| requirement.name.as_str())
        .collect();
    let mut values = Vec::new();
    for field in channel_fields.iter().filter(|field| field.secret) {
        if requirement_names.contains(field.name.as_str()) {
            continue;
        }
        if let Some(value) = submit.secrets.remove(&field.name) {
            values.push((field.name.clone(), value));
        }
    }
    for (name, value) in std::mem::take(&mut submit.fields) {
        if !channel_fields
            .iter()
            .any(|field| !field.secret && field.name == name)
        {
            return Err(validation_error(
                "fields",
                ProductSurfaceValidationCode::InvalidValue,
            ));
        }
        values.push((name, value));
    }
    Ok(values)
}

async fn setup_extension_response(
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    channel_config: Option<&dyn ChannelConfigFacade>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    lifecycle: LifecycleProductResponse,
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<RebornSetupExtensionResponse, ProductSurfaceError> {
    let package_ref = lifecycle
        .package_ref
        .clone()
        .ok_or_else(ProductSurfaceError::internal_invariant)?;
    let mut secrets = extension_setup_credentials::project(
        extension_credentials,
        scope,
        extension_id,
        requirements,
    )
    .await?;
    let channel_fields = channel_field_status(channel_config, extension_id).await?;
    // Secret channel-config fields surface in the existing secrets shape
    // (presence only — stored values are never echoed); a credential
    // requirement with the same name keeps its richer projection.
    for field in channel_fields.iter().filter(|field| field.secret) {
        if secrets.iter().any(|secret| secret.name == field.name) {
            continue;
        }
        secrets.push(RebornExtensionSetupSecret {
            name: field.name.clone(),
            provider: extension_id.as_str().to_string(),
            prompt: field.label.clone(),
            optional: false,
            provided: field.provided,
            setup: RebornExtensionCredentialSetup::ManualToken,
            credential_ref: None,
        });
    }
    secrets.sort_by_key(|secret| !secret.provided);
    let fields = channel_fields
        .iter()
        .filter(|field| !field.secret)
        .map(|field| RebornExtensionSetupField {
            name: field.name.clone(),
            prompt: field.label.clone(),
            optional: false,
            placeholder: None,
        })
        .collect();
    let onboarding = extension_onboarding::from_lifecycle(&lifecycle).onboarding;
    Ok(RebornSetupExtensionResponse {
        package_ref,
        phase: lifecycle.phase,
        blockers: lifecycle.blockers,
        onboarding,
        payload: lifecycle.payload,
        secrets,
        fields,
    })
}

fn setup_action(request: &WebUiSetupExtensionRequest) -> Result<SetupAction, ProductSurfaceError> {
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
        // WebUI gets a plain 400 with no free text (the wire contract has no
        // free-text field): the exact `config set` remediation reaches users
        // via the LLM tool path and `ironclaw status`; bespoke WebUI
        // messaging is a deliberately deferred scope-cut.
        ProductWorkflowError::ProviderInstanceNotConfigured { .. } => {
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

    /// Scope-cut: the WebUI facade gets a plain sanitized 400, never the
    /// host-authored `reason` text — no free-text field exists on the wire
    /// contract (see the variant's doc comment in
    /// `ironclaw_product::error`).
    #[test]
    fn provider_instance_not_configured_maps_to_sanitized_400() {
        let error = ProductWorkflowError::ProviderInstanceNotConfigured {
            reason: "ironclaw config set google.client_id <id>.apps.googleusercontent.com"
                .to_string(),
        };

        let mapped = map_lifecycle_error(error);

        assert_eq!(mapped.code, ProductSurfaceErrorCode::InvalidRequest);
        assert_eq!(mapped.status_code, 400);
        assert!(!mapped.retryable);
    }
}
