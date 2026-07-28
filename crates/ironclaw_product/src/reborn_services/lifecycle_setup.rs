use std::collections::BTreeSet;

use ironclaw_auth::AuthProductScope;
use ironclaw_host_api::{
    ExtensionId, InstallationState, LifecyclePublicState, ProductSurfaceCaller,
    ProductSurfaceError, ProductSurfaceValidationCode,
};

use crate::{
    ChannelConfigProductService, LifecycleExtensionCredentialRequirement, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleProductAction, LifecycleProductContext, LifecycleProductResponse,
    LifecycleProductService, LifecycleProductSurfaceContext, ProductSetupExtensionRequest,
    ProductSurfaceFailure, RebornChannelConfigField, RebornExtensionCredentialSetup,
    RebornExtensionSetupField, RebornExtensionSetupSecret, RebornSetupExtensionResponse,
    RebornViewDescriptor, lifecycle_product_surface_error,
};

use super::{
    ExtensionCredentialSetupService,
    extension_credentials::{ExtensionCredentialReadiness, credential_scope},
    extension_onboarding, extension_setup_credentials,
    extension_setup_credentials::SetupSubmitPayload,
    views,
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
    service: &dyn LifecycleProductService,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    channel_config: Option<&dyn ChannelConfigProductService>,
    caller: ProductSurfaceCaller,
    params: serde_json::Value,
) -> Result<RebornSetupExtensionResponse, ProductSurfaceError> {
    let package_id = views::required_string_view_param(params, "package_id")?;
    let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, package_id)
        .map_err(ProductSurfaceFailure::from)
        .map_err(map_lifecycle_error)?;
    setup_extension(
        service,
        extension_credentials,
        channel_config,
        caller,
        package_ref,
        ProductSetupExtensionRequest::default(),
    )
    .await
}

pub(super) async fn submit_extension_setup_capability(
    service: &dyn LifecycleProductService,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    channel_config: Option<&dyn ChannelConfigProductService>,
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
        .map_err(ProductSurfaceFailure::from)
        .map_err(map_lifecycle_error)?;
    let request = serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|_| validation_error("input", ProductSurfaceValidationCode::InvalidValue))?;
    setup_extension(
        service,
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
    service: &dyn LifecycleProductService,
    extension_credentials: Option<&dyn ExtensionCredentialSetupService>,
    channel_config: Option<&dyn ChannelConfigProductService>,
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
    let lifecycle = project_package(service, context.clone(), package_ref.clone()).await?;
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
        let _activation = service
            .execute(
                context.clone(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await?;
        let refreshed = project_package(service, context, package_ref).await?;
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
    service: &dyn LifecycleProductService,
    context: LifecycleProductContext,
    package_ref: LifecyclePackageRef,
) -> Result<LifecycleProductResponse, ProductSurfaceError> {
    service.project_package(context, package_ref).await
}

async fn channel_field_status(
    channel_config: Option<&dyn ChannelConfigProductService>,
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
    channel_config: Option<&dyn ChannelConfigProductService>,
    scope: AuthProductScope,
    extension_id: &ExtensionId,
    lifecycle: LifecycleProductResponse,
    requirements: &[LifecycleExtensionCredentialRequirement],
) -> Result<RebornSetupExtensionResponse, ProductSurfaceError> {
    let package_ref = lifecycle
        .package_ref
        .clone()
        .ok_or_else(ProductSurfaceError::internal_invariant)?;
    let (mut secrets, credential_readiness) = extension_setup_credentials::project(
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
        phase: setup_public_phase(lifecycle.phase, credential_readiness),
        blockers: lifecycle.blockers,
        onboarding,
        payload: lifecycle.payload,
        secrets,
        fields,
    })
}

/// The setup route's caller-visible phase (§6.1). The host checkpoint alone
/// cannot say `active` for a caller whose required credentials are missing:
/// an extension the runtime is serving is still `setup_needed` for them.
fn setup_public_phase(
    lifecycle_phase: InstallationState,
    readiness: ExtensionCredentialReadiness,
) -> LifecyclePublicState {
    match (
        LifecyclePublicState::from_host_checkpoint(lifecycle_phase),
        readiness,
    ) {
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

pub(super) fn map_lifecycle_error(error: ProductSurfaceFailure) -> ProductSurfaceError {
    lifecycle_product_surface_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ExtensionCredentialStatusRequest, ExtensionCredentialSubmitRequest,
        LifecycleExtensionCredentialSetup, LifecycleExtensionRuntimeKind, LifecycleExtensionSource,
        LifecycleExtensionSummary, LifecycleInstalledExtensionSummary, LifecycleProductPayload,
    };
    use async_trait::async_trait;
    use ironclaw_auth::{CredentialAccountId, CredentialAccountProjection};
    use ironclaw_host_api::{
        CapabilitySurfaceKind, InstallationState, ProductSurfaceErrorCode, TenantId, UserId,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    /// Scope-cut: the WebUI service gets a plain sanitized 400, never the
    /// host-authored `reason` text — no free-text field exists on the wire
    /// contract (see the variant's doc comment in
    /// `ironclaw_product::error`).
    #[test]
    fn provider_instance_not_configured_maps_to_sanitized_400() {
        let error = ProductSurfaceFailure::ProviderInstanceNotConfigured {
            reason: "ironclaw config set google.client_id <id>.apps.googleusercontent.com"
                .to_string(),
        };

        let mapped = map_lifecycle_error(error);

        assert_eq!(mapped.code, ProductSurfaceErrorCode::InvalidRequest);
        assert_eq!(mapped.status_code, 400);
        assert!(!mapped.retryable);
    }

    #[tokio::test]
    async fn submit_extension_setup_activates_after_manual_credentials_are_stored() {
        let activated = Arc::new(AtomicBool::new(false));
        let service = RecordingLifecycleService {
            activated: Arc::clone(&activated),
        };
        let credentials = AcceptingCredentialSetupService;
        let caller = ProductSurfaceCaller::new(
            TenantId::new("setup-tenant").expect("tenant"),
            UserId::new("setup-user").expect("user"),
            None,
            None,
        );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
            .expect("package ref");

        let response = setup_extension(
            &service,
            Some(&credentials),
            None,
            caller,
            package_ref,
            ProductSetupExtensionRequest {
                client_action_id: None,
                action: Some("submit".to_string()),
                payload: Some(serde_json::json!({
                    "secrets": {
                        "github_runtime_token": "github-token"
                    }
                })),
            },
        )
        .await
        .expect("setup submit should store credentials and activate");

        assert!(activated.load(Ordering::SeqCst));
        assert_eq!(response.phase, LifecyclePublicState::Active);
        assert!(
            response
                .secrets
                .iter()
                .any(|secret| secret.name == "github_runtime_token" && secret.provided)
        );
    }

    struct RecordingLifecycleService {
        activated: Arc<AtomicBool>,
    }

    #[async_trait]
    impl LifecycleProductService for RecordingLifecycleService {
        async fn execute(
            &self,
            _context: LifecycleProductContext,
            action: LifecycleProductAction,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            match action {
                LifecycleProductAction::ExtensionActivate { package_ref } => {
                    self.activated.store(true, Ordering::SeqCst);
                    Ok(LifecycleProductResponse {
                        package_ref: Some(package_ref),
                        phase: InstallationState::Active,
                        blockers: Vec::new(),
                        message: None,
                        payload: Some(LifecycleProductPayload::ExtensionActivate {
                            activated: true,
                            visible_capability_ids: vec!["github.get_workflow_runs".to_string()],
                            connection_required: None,
                        }),
                    })
                }
                _ => Err(ProductSurfaceError::service_unavailable(false)),
            }
        }

        async fn project_package(
            &self,
            _context: LifecycleProductContext,
            package_ref: LifecyclePackageRef,
        ) -> Result<LifecycleProductResponse, ProductSurfaceError> {
            Ok(LifecycleProductResponse {
                package_ref: Some(package_ref),
                phase: if self.activated.load(Ordering::SeqCst) {
                    InstallationState::Active
                } else {
                    InstallationState::Installed
                },
                blockers: Vec::new(),
                message: None,
                payload: Some(LifecycleProductPayload::ExtensionList {
                    extensions: vec![LifecycleInstalledExtensionSummary {
                        summary: LifecycleExtensionSummary {
                            package_ref: LifecyclePackageRef::new(
                                LifecyclePackageKind::Extension,
                                "github",
                            )
                            .expect("package ref"),
                            name: "github".to_string(),
                            version: "1.0.0".to_string(),
                            description: "GitHub".to_string(),
                            source: LifecycleExtensionSource::HostBundled,
                            runtime_kind: LifecycleExtensionRuntimeKind::WasmTool,
                            surface_kinds: vec![CapabilitySurfaceKind::Tool],
                            channel_directions: None,
                            channel_connection: None,
                            channel_presentation: None,
                            visible_capability_ids: vec!["github.get_workflow_runs".to_string()],
                            visible_read_only_capability_ids: Vec::new(),
                            credential_requirements: vec![
                                LifecycleExtensionCredentialRequirement {
                                    name: "github_runtime_token".to_string(),
                                    provider: "github".to_string(),
                                    required: true,
                                    setup: LifecycleExtensionCredentialSetup::ManualToken,
                                },
                            ],
                            onboarding: None,
                        },
                        phase: if self.activated.load(Ordering::SeqCst) {
                            InstallationState::Active
                        } else {
                            InstallationState::Installed
                        },
                        install_scope: None,
                    }],
                    count: 1,
                }),
            })
        }
    }

    struct AcceptingCredentialSetupService;

    #[async_trait]
    impl ExtensionCredentialSetupService for AcceptingCredentialSetupService {
        async fn credential_status(
            &self,
            request: ExtensionCredentialStatusRequest,
        ) -> Result<Option<CredentialAccountProjection>, ProductSurfaceError> {
            if request.provider.as_str() == "github" {
                Ok(Some(CredentialAccountProjection {
                    id: CredentialAccountId::new(),
                    provider: request.provider,
                    label: ironclaw_auth::CredentialAccountLabel::new("github")
                        .expect("credential label"),
                    status: ironclaw_auth::CredentialAccountStatus::Configured,
                    ownership: ironclaw_auth::CredentialOwnership::UserReusable,
                    owner_extension: None,
                    granted_extensions: Vec::new(),
                    secret_handle_count: 1,
                }))
            } else {
                Ok(None)
            }
        }

        async fn submit_manual_token(
            &self,
            _request: ExtensionCredentialSubmitRequest,
        ) -> Result<CredentialAccountId, ProductSurfaceError> {
            Ok(CredentialAccountId::new())
        }
    }
}
