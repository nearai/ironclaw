use ironclaw_product_contracts::channel_config::ChannelConfigProductService;
use ironclaw_product_contracts::lifecycle_service::{
    LifecycleProductContext, LifecycleProductService, LifecycleProductSurfaceContext,
};
use ironclaw_product_contracts::package_lifecycle::ChannelConfigField as RebornChannelConfigField;
use ironclaw_product_contracts::views::RebornViewDescriptor;
use std::collections::BTreeSet;

use ironclaw_auth::AuthProductScope;
use ironclaw_extension_contracts::hosted_mcp::HostedMcpAuthSelection;
use ironclaw_extension_contracts::state::{InstallationState, LifecyclePublicState};
use ironclaw_host_api::ids::ExtensionId;
use ironclaw_product_contracts::surface::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceValidationCode,
};

use crate::{
    LifecycleExtensionCredentialRequirement, LifecyclePackageKind, LifecyclePackageRef,
    LifecycleProductAction, LifecycleProductResponse, ProductSurfaceFailure,
    RebornExtensionCredentialSetup, RebornExtensionSetupField, RebornExtensionSetupSecret,
    RebornSetupExtensionResponse, lifecycle_product_surface_error,
};
use ironclaw_product_contracts::inbound_requests::ProductSetupExtensionRequest;

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
    SelectAuth,
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
    if action == SetupAction::SelectAuth {
        let auth_selection = parse_hosted_mcp_auth_selection(&request)?;
        let selection = service
            .execute(
                context.clone(),
                LifecycleProductAction::ExtensionSelectHostedMcpAuth {
                    package_ref: package_ref.clone(),
                    auth_selection,
                },
            )
            .await?;
        let refreshed =
            project_package_after_mutation(service, context, package_ref, selection.message)
                .await?;
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
        let activation = service
            .execute(
                context.clone(),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: package_ref.clone(),
                },
            )
            .await?;
        let refreshed =
            project_package_after_mutation(service, context, package_ref, activation.message)
                .await?;
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

async fn project_package_after_mutation(
    service: &dyn LifecycleProductService,
    context: LifecycleProductContext,
    package_ref: LifecyclePackageRef,
    mutation_message: Option<String>,
) -> Result<LifecycleProductResponse, ProductSurfaceError> {
    let mut projected = project_package(service, context, package_ref).await?;
    if projected.message.is_none() {
        projected.message = mutation_message;
    }
    Ok(projected)
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
        message: lifecycle.message,
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
        Some("select_auth") => Ok(SetupAction::SelectAuth),
        Some(_) => Err(validation_error(
            "action",
            ProductSurfaceValidationCode::InvalidValue,
        )),
    }
}

fn parse_hosted_mcp_auth_selection(
    request: &ProductSetupExtensionRequest,
) -> Result<HostedMcpAuthSelection, ProductSurfaceError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SelectionPayload {
        auth_selection: HostedMcpAuthSelection,
    }
    let payload = request
        .payload
        .clone()
        .ok_or_else(|| validation_error("payload", ProductSurfaceValidationCode::MissingField))?;
    let selection = serde_json::from_value::<SelectionPayload>(payload)
        .map_err(|_| validation_error("payload", ProductSurfaceValidationCode::InvalidValue))?
        .auth_selection;
    if matches!(selection, HostedMcpAuthSelection::Auto) {
        return Err(validation_error(
            "payload",
            ProductSurfaceValidationCode::InvalidValue,
        ));
    }
    Ok(selection)
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
        LifecycleExtensionCredentialSetup, LifecycleExtensionOnboarding,
        LifecycleExtensionRuntimeKind, LifecycleExtensionSource, LifecycleExtensionSummary,
        LifecycleInstalledExtensionSummary, LifecycleProductPayload, LifecycleReadinessBlocker,
    };
    use async_trait::async_trait;
    use ironclaw_auth::{CredentialAccountId, CredentialAccountProjection};
    use ironclaw_extension_contracts::{state::InstallationState, surface::CapabilitySurfaceKind};
    use ironclaw_host_api::ids::{TenantId, UserId};
    use ironclaw_product_contracts::surface::ProductSurfaceErrorCode;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    /// Scope-cut: the WebUI service gets a plain sanitized 400, never the
    /// host-authored `reason` text — no free-text field exists on the wire
    /// contract (see the variant's doc comment in
    /// `ironclaw_assistant::error`).
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
            auth_selected: Arc::new(AtomicBool::new(false)),
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
        assert_eq!(response.message.as_deref(), Some("activation complete"));
        assert!(
            response
                .secrets
                .iter()
                .any(|secret| secret.name == "github_runtime_token" && secret.provided)
        );
    }

    #[tokio::test]
    async fn select_auth_returns_the_reprojected_setup_contract() {
        let auth_selected = Arc::new(AtomicBool::new(false));
        let service = RecordingLifecycleService {
            activated: Arc::new(AtomicBool::new(false)),
            auth_selected: Arc::clone(&auth_selected),
        };
        let credentials = AcceptingCredentialSetupService;
        let caller = ProductSurfaceCaller::new(
            TenantId::new("setup-tenant").expect("tenant"),
            UserId::new("setup-user").expect("user"),
            None,
            None,
        );
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "mcp-custom")
            .expect("package ref");

        let response = setup_extension(
            &service,
            Some(&credentials),
            None,
            caller,
            package_ref,
            ProductSetupExtensionRequest {
                client_action_id: None,
                action: Some("select_auth".to_string()),
                payload: Some(serde_json::json!({
                    "auth_selection": { "kind": "bearer" }
                })),
            },
        )
        .await
        .expect("auth selection should return its post-selection setup state");

        assert!(auth_selected.load(Ordering::SeqCst));
        assert_eq!(response.phase, LifecyclePublicState::SetupNeeded);
        assert!(
            response
                .blockers
                .iter()
                .any(|blocker| matches!(blocker, LifecycleReadinessBlocker::Credential { .. }))
        );
        assert!(response.secrets.iter().any(|secret| {
            secret.name == "hosted_mcp_bearer_token" && !secret.provided && !secret.optional
        }));
        assert_eq!(
            response
                .onboarding
                .as_ref()
                .and_then(|onboarding| onboarding.credential_instructions.as_deref()),
            Some("Paste the bearer token for this MCP server.")
        );
        assert_eq!(response.message.as_deref(), Some("auth selection complete"));
    }

    struct RecordingLifecycleService {
        activated: Arc<AtomicBool>,
        auth_selected: Arc<AtomicBool>,
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
                        message: Some("activation complete".to_string()),
                        payload: Some(LifecycleProductPayload::ExtensionActivate {
                            activated: true,
                            visible_capability_ids: vec!["github.get_workflow_runs".to_string()],
                            connection_required: None,
                        }),
                    })
                }
                LifecycleProductAction::ExtensionSelectHostedMcpAuth { package_ref, .. } => {
                    self.auth_selected.store(true, Ordering::SeqCst);
                    Ok(LifecycleProductResponse {
                        package_ref: Some(package_ref),
                        phase: InstallationState::Installed,
                        blockers: Vec::new(),
                        message: Some("auth selection complete".to_string()),
                        payload: None,
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
            let auth_selected = self.auth_selected.load(Ordering::SeqCst);
            Ok(LifecycleProductResponse {
                package_ref: Some(package_ref.clone()),
                phase: if self.activated.load(Ordering::SeqCst) {
                    InstallationState::Active
                } else {
                    InstallationState::Installed
                },
                blockers: auth_selected
                    .then_some(LifecycleReadinessBlocker::Credential { ref_id: None })
                    .into_iter()
                    .collect(),
                message: None,
                payload: Some(LifecycleProductPayload::ExtensionList {
                    extensions: vec![LifecycleInstalledExtensionSummary {
                        summary: LifecycleExtensionSummary {
                            package_ref: package_ref.clone(),
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
                                    name: if auth_selected {
                                        "hosted_mcp_bearer_token".to_string()
                                    } else {
                                        "github_runtime_token".to_string()
                                    },
                                    provider: if auth_selected {
                                        "mcp-custom".to_string()
                                    } else {
                                        "github".to_string()
                                    },
                                    required: true,
                                    setup: LifecycleExtensionCredentialSetup::ManualToken,
                                },
                            ],
                            onboarding: auth_selected.then_some(LifecycleExtensionOnboarding {
                                instructions: "Configure MCP authentication.".to_string(),
                                credential_instructions: Some(
                                    "Paste the bearer token for this MCP server.".to_string(),
                                ),
                                setup_url: None,
                                credential_next_step: Some(
                                    "Save the token to activate this MCP server.".to_string(),
                                ),
                            }),
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
                    scopes: Vec::new(),
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
