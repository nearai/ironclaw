// arch-exempt: large_file, model-visible extension removal adapter and caller tests, plan #5905
use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityDisplayOutputPreview, CapabilityId, CapabilityProfileSchemaRef, CredentialStageError,
    DispatchInputIssue, DispatchInputIssueCode, EffectKind, HostApiError, InstallationState,
    OriginGateMatrix, OriginGatePolicy, PermissionMode, ResourceEstimate, ResourceProfile,
    ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgress,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductPayload, LifecycleProductResponse,
    ProductSurfaceFailure, RebornChannelConnectStrategy,
};
use serde::Deserialize;

use crate::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use ironclaw_auth::RuntimeCredentialAccountSelectionService;
use ironclaw_extension_host::ExtensionActivationMode;

pub const EXTENSION_SEARCH_CAPABILITY_ID: &str = "builtin.extension_search";
pub const EXTENSION_INSTALL_CAPABILITY_ID: &str = "builtin.extension_install";
pub const EXTENSION_ACTIVATE_CAPABILITY_ID: &str = "builtin.extension_activate";
pub const EXTENSION_REMOVE_CAPABILITY_ID: &str = "builtin.extension_remove";

pub const EXTENSION_LIFECYCLE_CAPABILITY_IDS: [&str; 3] = [
    EXTENSION_SEARCH_CAPABILITY_ID,
    EXTENSION_INSTALL_CAPABILITY_ID,
    EXTENSION_REMOVE_CAPABILITY_ID,
];

const EXTENSION_LIFECYCLE_HANDLER_IDS: [&str; 4] = [
    EXTENSION_SEARCH_CAPABILITY_ID,
    EXTENSION_INSTALL_CAPABILITY_ID,
    EXTENSION_ACTIVATE_CAPABILITY_ID,
    EXTENSION_REMOVE_CAPABILITY_ID,
];

pub fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.extend(manifests()?);
    ExtensionPackage::from_manifest(package.manifest, package.root)
}

pub fn insert_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    extension_management: Arc<RebornLocalExtensionManagementPort>,
    credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
) -> Result<(), HostApiError> {
    let handler = Arc::new(ExtensionLifecycleToolHandler {
        extension_management,
        credential_accounts,
        runtime_http_egress,
    });
    for capability_id in EXTENSION_LIFECYCLE_HANDLER_IDS {
        registry.insert_handler(CapabilityId::new(capability_id)?, handler.clone());
    }
    Ok(())
}

fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    let manifests = vec![
        lifecycle_manifest(
            EXTENSION_SEARCH_CAPABILITY_ID,
            "Search the local Reborn extension catalog by extension, product, provider, or service name. The catalog includes host-bundled extensions that are not installed yet and installed extensions that are inactive. For connect, enable, install, pair, authenticate, or integrate requests, use this for discovery only, then continue with builtin.extension_install for the matching extension instead of asking the user to configure credentials from search results. For routine, trigger, or notification delivery, prefer configured outbound delivery targets before installing an external channel.",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
        )?,
        lifecycle_manifest(
            EXTENSION_INSTALL_CAPABILITY_ID,
            "Install a searched Reborn extension into durable standalone lifecycle state. Installation also attempts activation: when an extension does not require credentials or credentials are already available it publishes tools immediately, and when credentials are missing it raises the auth gate. If install reports the extension is already installed, report the installed state or credential gate it returns instead of calling a separate activation tool.",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Ask,
        )?,
        lifecycle_manifest_with_visibility(
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            "Internal extension activation adapter retained for product/API continuations. Model callers use builtin.extension_install, which performs install-driven activation.",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Ask,
            CapabilityVisibility::Api,
            EXTENSION_INSTALL_CAPABILITY_ID,
        )?,
        lifecycle_manifest(
            EXTENSION_REMOVE_CAPABILITY_ID,
            "Remove an installed Reborn extension from durable standalone lifecycle state. Use this when the user asks to uninstall, remove, disable, disconnect, unpair, unlink, or revoke access for an extension, integration, app, account, external channel, or the current external chat. Pass the extension's registry id as extension_id; removal also performs extension-owned cleanup such as authentication, identity, and channel bindings when supported.",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Ask,
        )?,
    ];
    debug_assert_eq!(
        manifests
            .iter()
            .filter(|manifest| manifest.visibility == CapabilityVisibility::Model)
            .map(|manifest| manifest.id.as_str())
            .collect::<Vec<_>>(),
        EXTENSION_LIFECYCLE_CAPABILITY_IDS
    );
    Ok(manifests)
}

fn lifecycle_manifest(
    id: &str,
    description: &str,
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
) -> Result<CapabilityManifest, ExtensionError> {
    lifecycle_manifest_with_visibility(
        id,
        description,
        effects,
        default_permission,
        CapabilityVisibility::Model,
        id,
    )
}

fn lifecycle_manifest_with_visibility(
    id: &str,
    description: &str,
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
    visibility: CapabilityVisibility,
    schema_id: &str,
) -> Result<CapabilityManifest, ExtensionError> {
    let schema_name = schema_id
        .strip_prefix("builtin.")
        .unwrap_or(schema_id)
        .replace('.', "-");
    Ok(CapabilityManifest {
        id: CapabilityId::new(id)?,
        description: description.to_string(),
        effects,
        default_permission,
        visibility,
        input_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/builtin/{schema_name}.input.v1.json"
        ))?,
        output_schema_ref: Some(CapabilityProfileSchemaRef::new(format!(
            "schemas/builtin/{schema_name}.output.v1.json"
        ))?),
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(100)
                .set_output_bytes(16 * 1024),
            hard_ceiling: None,
        }),
        origin_gate_matrix: Some(lifecycle_origin_gate_matrix(id)),
    })
}

fn lifecycle_origin_gate_matrix(id: &str) -> OriginGateMatrix {
    let mut matrix = OriginGateMatrix::builtin_loop_run_seed(id);
    if matches!(
        id,
        EXTENSION_INSTALL_CAPABILITY_ID
            | EXTENSION_ACTIVATE_CAPABILITY_ID
            | EXTENSION_REMOVE_CAPABILITY_ID
    ) {
        matrix.product = OriginGatePolicy::ConsentSufficient;
    }
    matrix
}

struct ExtensionLifecycleToolHandler {
    extension_management: Arc<RebornLocalExtensionManagementPort>,
    credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
}

#[derive(Debug, Deserialize)]
struct SearchInput {
    #[serde(default)]
    query: String,
}

#[derive(Debug, Deserialize)]
struct ExtensionIdInput {
    extension_id: String,
}

#[async_trait]
impl FirstPartyCapabilityHandler for ExtensionLifecycleToolHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let mut connection_preview_source = None;
        let response = match request.capability_id.as_str() {
            EXTENSION_SEARCH_CAPABILITY_ID => {
                let input: SearchInput = parse_input(request.input)?;
                let credential_gate = RuntimeExtensionActivationCredentialGate::new(
                    request.scope.clone(),
                    Arc::clone(&self.credential_accounts),
                );
                self.extension_management
                    .search(
                        &input.query,
                        Some(
                            &credential_gate
                                as &dyn ironclaw_extension_host::ExtensionActivationCredentialGate,
                        ),
                        &request.scope.user_id,
                    )
                    .await
                    .map_err(lifecycle_error)
            }
            EXTENSION_INSTALL_CAPABILITY_ID => {
                let input: ExtensionIdInput = parse_input(request.input)?;
                let package_ref = extension_package_ref(input.extension_id)?;
                // The dispatch scope carries the ACTING user, so a chat-driven
                // install derives the same owner the WebUI path would (#5459
                // P1): operator → tenant-shared, member → private.
                let install_response = self
                    .extension_management
                    .install(package_ref.clone(), &request.scope.user_id)
                    .await
                    .map_err(lifecycle_error)?;
                let requirements = self
                    .extension_management
                    .activation_credential_requirements(&package_ref, &request.scope.user_id)
                    .await
                    .map_err(install_activation_readiness_error)?;
                let credential_gate = RuntimeExtensionActivationCredentialGate::new(
                    request.scope.clone(),
                    Arc::clone(&self.credential_accounts),
                );
                let missing_requirements = credential_gate
                    .missing_requirements(requirements)
                    .await
                    .map_err(credential_stage_error)?;
                if !missing_requirements.is_empty() {
                    return Err(FirstPartyCapabilityError::auth_required_for_credentials(
                        missing_requirements,
                    )
                    .with_usage(resource_usage(started)));
                }
                let mode = ExtensionActivationMode::from_dispatch_context(
                    request.scope.clone(),
                    request
                        .services
                        .runtime_http_egress
                        .clone()
                        .or_else(|| self.runtime_http_egress.clone()),
                );
                match self
                    .extension_management
                    .activate_with_credential_gate(
                        package_ref.clone(),
                        mode,
                        credential_gate,
                        &request.scope.user_id,
                    )
                    .await
                {
                    Ok(activation_response)
                        if activation_response.phase == InstallationState::Active =>
                    {
                        connection_preview_source = Some(activation_response.clone());
                        Ok(install_response_with_activation(
                            install_response,
                            &activation_response,
                        ))
                    }
                    Ok(activation_response)
                        if activation_response_has_credential_blocker(&activation_response) =>
                    {
                        let requirements = self
                            .extension_management
                            .activation_credential_requirements(
                                &package_ref,
                                &request.scope.user_id,
                            )
                            .await
                            .map_err(install_activation_readiness_error)?;
                        if requirements.is_empty() {
                            Ok(install_response)
                        } else {
                            Err(FirstPartyCapabilityError::auth_required_for_credentials(
                                requirements,
                            )
                            .with_usage(resource_usage(started)))
                        }
                    }
                    Ok(_) => Ok(install_response),
                    Err(error) => install_activation_error(error, install_response),
                }
            }
            EXTENSION_ACTIVATE_CAPABILITY_ID => {
                let input: ExtensionIdInput = parse_input(request.input)?;
                let package_ref = extension_package_ref(input.extension_id)?;
                let requirements = self
                    .extension_management
                    .activation_credential_requirements(&package_ref, &request.scope.user_id)
                    .await
                    .map_err(lifecycle_error)?;
                let credential_gate = RuntimeExtensionActivationCredentialGate::new(
                    request.scope.clone(),
                    Arc::clone(&self.credential_accounts),
                );
                let missing_requirements = credential_gate
                    .missing_requirements(requirements)
                    .await
                    .map_err(credential_stage_error)?;
                if !missing_requirements.is_empty() {
                    return Err(FirstPartyCapabilityError::auth_required_for_credentials(
                        missing_requirements,
                    )
                    .with_usage(resource_usage(started)));
                }
                let mode = ExtensionActivationMode::from_dispatch_context(
                    request.scope.clone(),
                    request.services.runtime_http_egress.clone(),
                );
                self.extension_management
                    .activate_with_credential_gate(
                        package_ref,
                        mode,
                        credential_gate,
                        &request.scope.user_id,
                    )
                    .await
                    .map_err(lifecycle_error)
            }
            EXTENSION_REMOVE_CAPABILITY_ID => {
                let input: ExtensionIdInput = parse_input(request.input)?;
                self.extension_management
                    .remove(
                        extension_package_ref(input.extension_id)?,
                        &request.scope,
                        request.authenticated_actor_user_id.as_ref(),
                    )
                    .await
                    .map_err(lifecycle_error)
            }
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        }?;

        // An inbound-channel activation carries a structured connection
        // requirement; surface it as a display preview so WebChat opens the
        // in-chat OAuth connection panel from structured state.
        let connection_preview = channel_connection_display_preview(
            connection_preview_source.as_ref().unwrap_or(&response),
        );
        let response = without_model_visible_connection_chrome(response);
        let output =
            ironclaw_product::public_lifecycle_response_json(&response).map_err(|error| {
                tracing::debug!(
                    target: "ironclaw::reborn::extension_lifecycle",
                    ?error,
                    "extension lifecycle output serialization failed"
                );
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputDecode)
            })?;
        Ok(
            FirstPartyCapabilityResult::new(output, resource_usage(started))
                .with_display_preview(connection_preview),
        )
    }
}

/// Output-kind discriminator the WebChat frontend matches to open the in-chat
/// channel connection panel. Must stay in sync with
/// `CHANNEL_CONNECTION_REQUIRED_OUTPUT_KIND` in `static/js/pages/chat/hooks/useChat.js`.
const CHANNEL_CONNECTION_REQUIRED_OUTPUT_KIND: &str = "channel_connection_required";

fn channel_connection_display_preview(
    response: &LifecycleProductResponse,
) -> Option<CapabilityDisplayOutputPreview> {
    let Some(LifecycleProductPayload::ExtensionActivate {
        connection_required: Some(requirement),
        ..
    }) = response.payload.as_ref()
    else {
        return None;
    };
    let output_preview = match serde_json::to_string(requirement) {
        Ok(preview) => preview,
        Err(error) => {
            tracing::debug!(
                target: "ironclaw::reborn::extension_lifecycle",
                ?error,
                "failed to serialize channel-connection requirement; skipping in-chat connection preview"
            );
            return None;
        }
    };
    Some(CapabilityDisplayOutputPreview {
        output_summary: Some(format!(
            "Connect {} to continue.",
            display_channel_name(&requirement.channel)
        )),
        output_preview,
        output_kind: CHANNEL_CONNECTION_REQUIRED_OUTPUT_KIND.to_string(),
        subtitle: None,
        truncated: false,
    })
}

/// Structured channel connection requirements carry render chrome for WebUI.
/// Keep model-useful connection guidance in search output, but strip static
/// failure copy so it is not presented as live state.
fn without_model_visible_connection_chrome(
    mut response: LifecycleProductResponse,
) -> LifecycleProductResponse {
    match response.payload.as_mut() {
        Some(LifecycleProductPayload::ExtensionActivate {
            connection_required,
            ..
        }) => *connection_required = None,
        Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) => {
            for extension in extensions {
                match extension.summary.channel_connection.as_mut() {
                    Some(connection)
                        if connection.strategy
                            == RebornChannelConnectStrategy::WebGeneratedCode =>
                    {
                        connection.error_message.clear();
                    }
                    Some(_) => {
                        extension.summary.channel_connection = None;
                    }
                    None => {}
                }
            }
        }
        _ => {}
    }
    response
}

fn display_channel_name(channel: &str) -> String {
    let mut chars = channel.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => channel.to_string(),
    }
}

fn install_response_with_activation(
    mut install_response: LifecycleProductResponse,
    activation_response: &LifecycleProductResponse,
) -> LifecycleProductResponse {
    install_response.phase = activation_response.phase;
    install_response.blockers = activation_response.blockers.clone();
    install_response.message = activation_response.message.clone();

    let activation_visible_capability_ids = match activation_response.payload.as_ref() {
        Some(LifecycleProductPayload::ExtensionActivate {
            visible_capability_ids,
            ..
        }) => Some(visible_capability_ids.clone()),
        _ => None,
    };
    if let Some(LifecycleProductPayload::ExtensionInstall {
        visible_capability_ids,
        next_step,
        ..
    }) = install_response.payload.as_mut()
    {
        if let Some(activation_visible_capability_ids) = activation_visible_capability_ids {
            *visible_capability_ids = activation_visible_capability_ids;
        }
        *next_step = if activation_response.phase == InstallationState::Active {
            "Activation completed; model-visible extension tools are ready.".to_string()
        } else {
            "Activation did not complete; inspect the lifecycle phase and blockers.".to_string()
        };
    }
    install_response
}

fn activation_response_has_credential_blocker(response: &LifecycleProductResponse) -> bool {
    matches!(
        response.payload.as_ref(),
        Some(LifecycleProductPayload::ExtensionActivate {
            activated: false,
            ..
        })
    )
}

fn install_activation_readiness_error(error: ProductSurfaceFailure) -> FirstPartyCapabilityError {
    match error {
        ProductSurfaceFailure::ProviderInstanceNotConfigured { .. } => {
            provider_instance_unavailable_error()
        }
        error => lifecycle_error(error),
    }
}

fn install_activation_error(
    error: ProductSurfaceFailure,
    install_response: LifecycleProductResponse,
) -> Result<LifecycleProductResponse, FirstPartyCapabilityError> {
    match error {
        ProductSurfaceFailure::ProviderInstanceNotConfigured { .. } => {
            Err(provider_instance_unavailable_error())
        }
        ProductSurfaceFailure::Transient { reason } => {
            tracing::debug!(
                target: "ironclaw::reborn::extension_lifecycle",
                %reason,
                "post-install activation reconciliation failed; returning installed lifecycle state"
            );
            Ok(install_response)
        }
        ProductSurfaceFailure::InvalidBindingRequest { reason }
            if reason.starts_with("hosted MCP discovery failed:")
                || reason
                    == "generic extension host rejected the activation: hosted MCP discovery published no callable tools" =>
        {
            tracing::debug!(
                target: "ironclaw::reborn::extension_lifecycle",
                %reason,
                "post-install hosted MCP discovery failed; returning installed lifecycle state"
            );
            Ok(install_response)
        }
        error => Err(lifecycle_error(error)),
    }
}

fn resource_usage(started: Instant) -> ResourceUsage {
    ResourceUsage::default()
        .set_wall_clock_ms(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
}

fn credential_stage_error(error: CredentialStageError) -> FirstPartyCapabilityError {
    match error {
        CredentialStageError::AuthRequired => FirstPartyCapabilityError::auth_required(),
        CredentialStageError::Backend => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        }
    }
}

fn parse_input<T>(input: serde_json::Value) -> Result<T, FirstPartyCapabilityError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(input)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))
}

fn extension_package_ref(
    id: impl Into<String>,
) -> Result<LifecyclePackageRef, FirstPartyCapabilityError> {
    let id = id.into();
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, id.clone()).map_err(|_| {
        FirstPartyCapabilityError::invalid_input_issues(
            "extension id is invalid",
            vec![
                DispatchInputIssue::new("extension_id", DispatchInputIssueCode::InvalidValue)
                    .expected("a non-empty extension id")
                    .received(if id.is_empty() {
                        "empty"
                    } else {
                        "invalid extension id"
                    }),
            ],
        )
    })
}

/// Fixed, host-authored, validator-safe headline for the
/// `dispatch_with_host_remediation` call below — the strict `safe_summary`
/// validator rejects `{}[]<>/` and secret-like vocabulary
/// (capability-access redaction invariant), so the full `config set`
/// remediation text rides the trusted host-remediation channel instead;
/// `safe_summary` stays this short fixed literal.
const PROVIDER_INSTANCE_NOT_CONFIGURED_SAFE_SUMMARY: &str =
    "extension activation requires host instance configuration";
const PROVIDER_INSTANCE_UNAVAILABLE_SAFE_SUMMARY: &str =
    "extension is unavailable on this instance";

fn provider_instance_unavailable_error() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::dispatch_with_diagnostic(
        RuntimeDispatchErrorKind::OperationFailed,
        Some(PROVIDER_INSTANCE_UNAVAILABLE_SAFE_SUMMARY.to_string()),
        PROVIDER_INSTANCE_UNAVAILABLE_SAFE_SUMMARY,
    )
}

fn lifecycle_error(error: ProductSurfaceFailure) -> FirstPartyCapabilityError {
    match error {
        // UNTRUSTED on purpose. `InvalidBindingRequest` has ~40 construction
        // sites and several interpolate externally-influenced text: a hosted
        // MCP server's live `tools/list` tool names
        // (`hosted_mcp_discovery.rs` -> `hosted_mcp_discovery_error`), the
        // MODEL-chosen `extension_id` (charset-validated only, e.g. "extension
        // {} is not installed"), and uploaded-zip entry names
        // (`ironclaw_extension_host::extension_bundle`). `HostRemediation::new` is a VALUE guard, not
        // a provenance guard — it rejects credential-SHAPED tokens but allows
        // adversarial prose — so routing this whole class onto the trusted
        // channel would stamp `ObservationTrust::HostAuthored` on attacker
        // -influenced text and skip the credential-vocabulary scan
        // `ironclaw_threads` applies to untrusted output. The trusted channel
        // is reserved for reasons built entirely from host-authored constants
        // (the `ProviderInstanceNotConfigured` arm below).
        ProductSurfaceFailure::InvalidBindingRequest { reason } => {
            FirstPartyCapabilityError::dispatch_with_diagnostic(
                RuntimeDispatchErrorKind::InputEncode,
                None,
                reason,
            )
        }
        // The third readiness axis: a provider-instance readiness failure is
        // a build-time configuration fault, not a malformed-input fault, so it
        // maps to `OperationFailed` rather than `InvalidBindingRequest`'s
        // `InputEncode` (PR #6095 misclassification precedent). Both arms are
        // non-terminal, but they deliberately ride DIFFERENT trust channels:
        // `InvalidBindingRequest` above stays UNTRUSTED
        // (`dispatch_with_diagnostic`) because its ~40 construction sites
        // interpolate externally-influenced text — MCP tool names off the
        // wire, model-supplied `extension_id`, uploaded-zip entry names. This
        // arm is the one exception routed onto the TRUSTED channel
        // (`dispatch_with_host_remediation`), because its `reason` is built
        // entirely from host-authored constants.
        ProductSurfaceFailure::ProviderInstanceNotConfigured { reason } => {
            FirstPartyCapabilityError::dispatch_with_host_remediation(
                RuntimeDispatchErrorKind::OperationFailed,
                Some(PROVIDER_INSTANCE_NOT_CONFIGURED_SAFE_SUMMARY.to_string()),
                reason,
            )
        }
        ProductSurfaceFailure::UnsupportedActionKind { .. } => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
        }
        ProductSurfaceFailure::Transient { .. } => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        }
        _ => FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_auth::{
        AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
        CredentialAccountStatus, CredentialOwnership, NewCredentialAccount, ProviderScope,
    };
    use ironclaw_host_api::{
        CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, CapabilitySet, ExecutionContext,
        ExtensionId, FailureKind, GrantConstraints, MountView, NetworkPolicy, NetworkTargetPattern,
        OriginGatePolicy, PermissionMode, Principal, ResourceScope, RuntimeKind, SecretHandle,
        TrustClass, UNGATED_LOOP_RUN_CAPABILITIES, UserId,
    };
    use ironclaw_host_runtime::{
        CapabilitySurfacePolicy, RuntimeCapabilityOutcome, SurfaceKind, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    };
    use ironclaw_trust::{
        AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustPolicy, TrustProvenance,
    };
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };

    use super::*;
    use crate::lifecycle_test_support::{
        ExtensionLifecycleTestServices, build_lifecycle_test_services,
        invoke_json_with_standalone_approval, invoke_with_standalone_approval,
    };
    use ironclaw_host_api::InstallationState;
    use ironclaw_product::{
        ChannelConnectionRequirement, LifecycleExtensionRuntimeKind, LifecycleExtensionSource,
        LifecycleExtensionSummary, LifecyclePackageKind, LifecyclePackageRef,
        LifecycleSearchExtensionSummary, RebornChannelConnectStrategy,
    };

    const TEST_OWNER_ID: &str = "extension-tool-test-user";

    async fn test_services(
        _owner_id: &str,
        network_http_egress: Option<Arc<dyn ironclaw_network::NetworkHttpEgress>>,
        google_oauth_configured: bool,
    ) -> ExtensionLifecycleTestServices {
        build_lifecycle_test_services(TEST_OWNER_ID, network_http_egress, google_oauth_configured)
            .await
    }

    fn slack_activation_response() -> LifecycleProductResponse {
        let requirement = ChannelConnectionRequirement {
            channel: "slack".to_string(),
            display_name: "Slack".to_string(),
            strategy: RebornChannelConnectStrategy::OAuth,
            instructions: "Connect Slack with OAuth from the extension configuration.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Connect Slack".to_string(),
            error_message: "Slack OAuth connection failed.".to_string(),
        };
        LifecycleProductResponse {
            package_ref: None,
            phase: InstallationState::Active,
            blockers: Vec::new(),
            message: Some("activation guidance".to_string()),
            payload: Some(LifecycleProductPayload::ExtensionActivate {
                activated: true,
                visible_capability_ids: Vec::new(),
                connection_required: Some(requirement),
            }),
        }
    }

    /// §5.3 S3 (behavior-neutral): the public extension-lifecycle capabilities
    /// declare an `origin_gate_matrix`. `extension_search` is read-only and thus
    /// Ungated for LoopRun (it is in the reviewed allowlist); install/remove and
    /// the API-only activation continuation carry write effects and gate for
    /// LoopRun. The direct WebUI ProductSurface path is consent-sufficient for
    /// install/activate/remove; automation remains deny-by-default.
    #[test]
    fn extension_lifecycle_capabilities_declare_behavior_neutral_origin_gate_matrix() {
        let manifests = manifests().expect("lifecycle manifests build");
        for manifest in &manifests {
            let matrix = manifest
                .origin_gate_matrix
                .as_ref()
                .unwrap_or_else(|| panic!("{} must declare an origin_gate_matrix", manifest.id));
            let expected_product = if matches!(
                manifest.id.as_str(),
                EXTENSION_INSTALL_CAPABILITY_ID
                    | EXTENSION_ACTIVATE_CAPABILITY_ID
                    | EXTENSION_REMOVE_CAPABILITY_ID
            ) {
                OriginGatePolicy::ConsentSufficient
            } else {
                OriginGatePolicy::Forbidden
            };
            assert_eq!(matrix.product, expected_product, "{}", manifest.id);
            assert_eq!(
                matrix.automation,
                OriginGatePolicy::Forbidden,
                "{}",
                manifest.id
            );
            let expected = if manifest.id.as_str() == EXTENSION_SEARCH_CAPABILITY_ID {
                OriginGatePolicy::Ungated
            } else {
                OriginGatePolicy::GatedUnlessGranted
            };
            assert_eq!(matrix.loop_run, expected, "{}", manifest.id);
        }
        assert!(
            UNGATED_LOOP_RUN_CAPABILITIES.contains(&EXTENSION_SEARCH_CAPABILITY_ID),
            "extension_search must be in the Ungated allowlist"
        );
        for gated in [
            EXTENSION_INSTALL_CAPABILITY_ID,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            EXTENSION_REMOVE_CAPABILITY_ID,
        ] {
            assert!(
                !UNGATED_LOOP_RUN_CAPABILITIES.contains(&gated),
                "{gated} must not be in the Ungated allowlist"
            );
        }
    }

    #[test]
    fn model_visible_output_omits_connect_chrome_on_completed_path() {
        // On the connected (completed) path the render chrome is stripped from
        // the model-visible tool output so the model sees just the activation
        // prose, never the UI strings.
        let activation = slack_activation_response();
        // The display preview keeps the full requirement...
        let preview = channel_connection_display_preview(&activation)
            .expect("inbound-channel activation carries the preview");
        assert!(preview.output_preview.contains("Connect Slack with OAuth"));

        // ...but the model-visible output must not carry the render chrome.
        let model = without_model_visible_connection_chrome(activation);
        match &model.payload {
            Some(LifecycleProductPayload::ExtensionActivate {
                connection_required,
                ..
            }) => assert!(
                connection_required.is_none(),
                "connect chrome leaked into model-visible output",
            ),
            other => panic!("unexpected payload: {other:?}"),
        }
        let serialized = serde_json::to_string(&model).unwrap();
        assert!(!serialized.contains("Connect Slack with OAuth"));
        assert!(!serialized.contains("submit_label"));
    }

    #[test]
    fn channel_connection_display_preview_marks_inbound_channel_activations() {
        // The in-chat connection panel is opened from this structured display preview,
        // never from the activation prose. Guard the exact seam: the output_kind
        // const the frontend matches, and the JSON body it parses. A renamed const
        // or a broken match arm would otherwise be invisible to Rust tests.
        let requirement = ChannelConnectionRequirement {
            channel: "slack".to_string(),
            display_name: "Slack".to_string(),
            strategy: RebornChannelConnectStrategy::OAuth,
            instructions: "Connect Slack with OAuth from the extension configuration.".to_string(),
            input_placeholder: String::new(),
            submit_label: "Connect Slack".to_string(),
            error_message: "Slack OAuth connection failed.".to_string(),
        };
        let channel_activation = LifecycleProductResponse {
            package_ref: None,
            phase: InstallationState::Active,
            blockers: Vec::new(),
            message: Some("activation guidance".to_string()),
            payload: Some(LifecycleProductPayload::ExtensionActivate {
                activated: true,
                visible_capability_ids: Vec::new(),
                connection_required: Some(requirement.clone()),
            }),
        };

        let preview = channel_connection_display_preview(&channel_activation)
            .expect("an inbound-channel activation must carry the connect display preview");
        assert_eq!(preview.output_kind, "channel_connection_required");
        let parsed: ChannelConnectionRequirement =
            serde_json::from_str(&preview.output_preview).expect("preview body is the requirement");
        assert_eq!(parsed, requirement);

        let tool_activation = LifecycleProductResponse {
            package_ref: None,
            phase: InstallationState::Active,
            blockers: Vec::new(),
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionActivate {
                activated: true,
                visible_capability_ids: vec!["github.search_issues".to_string()],
                connection_required: None,
            }),
        };
        assert!(channel_connection_display_preview(&tool_activation).is_none());
    }

    #[tokio::test]
    async fn standalone_agent_surface_exposes_extension_lifecycle_tools() {
        let services = test_services("extension-tools-surface-owner", None, false).await;
        let runtime = services.host_runtime.as_ref();

        let surface = runtime
            .visible_capabilities(visible_request(EXTENSION_LIFECYCLE_CAPABILITY_IDS))
            .await
            .expect("visible capabilities");
        let ids = surface_capability_ids(&surface);

        assert!(ids.contains(&EXTENSION_SEARCH_CAPABILITY_ID));
        assert!(ids.contains(&EXTENSION_INSTALL_CAPABILITY_ID));
        assert!(ids.contains(&EXTENSION_REMOVE_CAPABILITY_ID));
        assert!(!ids.contains(&EXTENSION_ACTIVATE_CAPABILITY_ID));

        let search = descriptor_for(&surface, EXTENSION_SEARCH_CAPABILITY_ID);
        assert_eq!(search.default_permission, PermissionMode::Allow);
        assert!(
            search.description.contains("host-bundled")
                && search.description.contains("not installed")
                && search
                    .description
                    .contains("installed extensions that are inactive")
                && search.description.contains("connect")
                && search.description.contains("service name")
                && search.description.contains("discovery only")
                && search.description.contains("external channel")
                && search.description.contains("outbound delivery targets")
                && search.description.contains(EXTENSION_INSTALL_CAPABILITY_ID),
            "extension_search description should teach the model to discover bundled or inactive integrations from generic service names: {}",
            search.description
        );
        assert_eq!(
            search.parameters_schema.get("required"),
            None,
            "extension_search query should be optional so models can list all extensions"
        );

        let install = descriptor_for(&surface, EXTENSION_INSTALL_CAPABILITY_ID);
        assert_eq!(install.default_permission, PermissionMode::Ask);
        assert!(
            install.description.contains("already installed")
                && install.description.contains("does not require credentials")
                && install.description.contains("attempts activation")
                && install.description.contains("auth gate"),
            "extension_install description should route installs through install-driven activation: {}",
            install.description
        );
        assert_eq!(
            install.parameters_schema["required"],
            serde_json::json!(["extension_id"])
        );
    }

    #[tokio::test]
    async fn model_visible_extension_search_omits_channel_connection_chrome() {
        let services = test_services("extension-search-model-output-owner", None, false).await;

        let search = invoke_json(
            &services,
            EXTENSION_SEARCH_CAPABILITY_ID,
            serde_json::json!({"query": "slack"}),
        )
        .await
        .expect("Slack search succeeds");
        let slack = search["payload"]["extensions"]
            .as_array()
            .expect("extensions array")
            .iter()
            .find(|extension| extension["package_ref"]["id"] == "slack")
            .expect("Slack search result");

        assert!(
            slack["surface_kinds"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "channel")),
            "model-visible search must still identify Slack as a channel: {slack}"
        );
        assert!(
            slack.get("channel_connection").is_none(),
            "model-visible search must strip OAuth connection chrome: {slack}"
        );
        assert!(
            !serde_json::to_string(&search)
                .expect("search response serializes")
                .contains("Slack OAuth connection failed"),
            "static OAuth failure copy must not be presented as live model-visible state"
        );

        let search = invoke_json(
            &services,
            EXTENSION_SEARCH_CAPABILITY_ID,
            serde_json::json!({"query": "telegram"}),
        )
        .await
        .expect("Telegram search succeeds");
        let telegram = search["payload"]["extensions"]
            .as_array()
            .expect("extensions array")
            .iter()
            .find(|extension| extension["package_ref"]["id"] == "telegram")
            .expect("Telegram search result");
        assert!(
            telegram["surface_kinds"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "channel")),
            "model-visible search must still identify Telegram as a channel: {telegram}"
        );
        assert!(
            telegram["channel_connection"]["instructions"]
                .as_str()
                .is_some_and(
                    |instructions| instructions.contains("IronClaw pairing panel")
                        && instructions.contains("/start")
                        && instructions.contains("displayed code")
                ),
            "generated-code connection guidance must remain model-visible: {telegram}"
        );
        assert!(
            telegram["channel_connection"]["error_message"] == "",
            "generated-code connection failure copy must stay out of model-visible lifecycle output: {telegram}"
        );
    }

    #[test]
    fn model_visible_extension_search_preserves_only_generated_code_connection_contract() {
        let response = LifecycleProductResponse {
            package_ref: None,
            phase: InstallationState::Active,
            blockers: Vec::new(),
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionSearch {
                extensions: vec![
                    search_summary("generated", RebornChannelConnectStrategy::WebGeneratedCode),
                    search_summary("oauth", RebornChannelConnectStrategy::OAuth),
                ],
                count: 2,
            }),
        };

        let filtered = without_model_visible_connection_chrome(response);
        let Some(LifecycleProductPayload::ExtensionSearch { extensions, .. }) = filtered.payload
        else {
            panic!("expected extension_search payload");
        };
        let generated = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "generated")
            .expect("generated-code summary");
        assert!(
            generated.summary.channel_connection.is_some(),
            "model-visible search must preserve WebGeneratedCode connection contract"
        );
        let oauth = extensions
            .iter()
            .find(|extension| extension.summary.package_ref.id.as_str() == "oauth")
            .expect("OAuth summary");
        assert!(
            oauth.summary.channel_connection.is_none(),
            "model-visible search must strip non-generated-code connection chrome"
        );
    }

    fn search_summary(
        id: &str,
        strategy: RebornChannelConnectStrategy,
    ) -> LifecycleSearchExtensionSummary {
        search_summary_with_phase(id, strategy, None)
    }

    fn search_summary_with_phase(
        id: &str,
        strategy: RebornChannelConnectStrategy,
        installation_phase: Option<InstallationState>,
    ) -> LifecycleSearchExtensionSummary {
        LifecycleSearchExtensionSummary {
            summary: LifecycleExtensionSummary {
                package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, id)
                    .expect("package ref"),
                name: id.to_string(),
                version: "1.0.0".to_string(),
                description: format!("{id} channel"),
                source: LifecycleExtensionSource::HostBundled,
                runtime_kind: LifecycleExtensionRuntimeKind::FirstParty,
                surface_kinds: Vec::new(),
                channel_directions: None,
                channel_connection: Some(ChannelConnectionRequirement {
                    channel: id.to_string(),
                    display_name: id.to_string(),
                    strategy,
                    instructions: "Connect this channel.".to_string(),
                    input_placeholder: String::new(),
                    submit_label: "Connect".to_string(),
                    error_message: "Connection failed.".to_string(),
                }),
                channel_presentation: None,
                visible_capability_ids: Vec::new(),
                visible_read_only_capability_ids: Vec::new(),
                credential_requirements: Vec::new(),
                onboarding: None,
            },
            installation_phase,
        }
    }

    #[tokio::test]
    async fn standalone_extension_lifecycle_tools_manage_visible_extension_surface() {
        let services = test_services("extension-tools-owner", None, false).await;
        let runtime = services.host_runtime.as_ref();
        let extension_management = services.extension_management.clone();
        let absent_remove = invoke_json(
            &services,
            EXTENSION_REMOVE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "web-access"}),
        )
        .await
        .expect("already-absent remove succeeds");
        assert_eq!(absent_remove["payload"]["removed"], false);

        let search = invoke_json(
            &services,
            EXTENSION_SEARCH_CAPABILITY_ID,
            serde_json::json!({"query": "web-access"}),
        )
        .await
        .expect("search succeeds");
        assert_eq!(search["payload"]["kind"], "extension_search");
        assert_eq!(search["payload"]["count"], 1);

        let install = invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "web-access"}),
        )
        .await
        .expect("install succeeds");
        assert_eq!(install["payload"]["installed"], true);

        let after_install = active_extension_capability_ids(&extension_management).await;
        assert!(after_install.iter().any(|id| id == "web-access.search"));
        assert!(
            after_install
                .iter()
                .any(|id| id == "web-access.get_content")
        );

        let activate = invoke_json(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "web-access"}),
        )
        .await
        .expect("activate succeeds");
        assert_eq!(activate["payload"]["activated"], true);
        assert!(
            activate["message"].as_str().is_some_and(|message| message
                .contains("No additional authorization or configuration is needed")),
            "activation success should override stale same-turn search onboarding, got {activate}"
        );

        let after_activate = active_extension_capability_ids(&extension_management).await;
        assert!(after_activate.iter().any(|id| id == "web-access.search"));
        assert!(
            after_activate
                .iter()
                .any(|id| id == "web-access.get_content")
        );
        let health = runtime.health().await.expect("runtime health");
        assert!(
            !health
                .missing_runtime_backends
                .contains(&RuntimeKind::FirstParty),
            "activated Web Access capabilities require a registered first-party runtime"
        );

        let remove = invoke_json(
            &services,
            EXTENSION_REMOVE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "web-access"}),
        )
        .await
        .expect("remove succeeds");
        assert_eq!(remove["payload"]["removed"], true);

        let after_remove = active_extension_capability_ids(&extension_management).await;
        assert!(!after_remove.iter().any(|id| id == "web-access.search"));
    }

    #[tokio::test]
    async fn standalone_extension_remove_revokes_exclusive_credential_so_reactivation_requires_auth()
     {
        // Regression (#slack model-B): before the pairing->OAuth swap, removing an
        // extension cleared its credentials, so the agent could not silently
        // re-add it. OAuth personal credentials are stored `UserReusable` and are
        // preserved across extension removal by default, so without an explicit
        // provider-scoped cleanup on remove the agent re-installs the bundled
        // extension and re-activates on the surviving token — no OAuth re-consent.
        // Removing an extension whose credential provider it exclusively owns must
        // revoke that credential so re-activation raises the auth gate again.
        let services = test_services("extension-tools-remove-revoke-owner", None, false).await;

        install_inactive(&services, "github").await;
        let context = execution_context([EXTENSION_ACTIVATE_CAPABILITY_ID]);
        seed_configured_account(&services, &context.resource_scope, "github").await;
        let activate = invoke_json(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await
        .expect("activate succeeds with a configured credential");
        assert_eq!(activate["payload"]["activated"], true);

        let remove = invoke_json(
            &services,
            EXTENSION_REMOVE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await
        .expect("remove succeeds");
        assert_eq!(remove["payload"]["removed"], true);

        // Re-install (bundled, free) then attempt to re-activate: the revoked
        // credential must force a fresh auth gate rather than silently re-adding.
        install_inactive(&services, "github").await;
        let outcome = invoke_outcome(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
            panic!("expected re-activation after remove to require auth, got {outcome:?}");
        };
        assert_eq!(gate.credential_requirements.len(), 1);
        assert_eq!(gate.credential_requirements[0].provider.as_str(), "github");
    }

    #[tokio::test]
    async fn standalone_extension_remove_preserves_shared_credential_used_by_another_extension() {
        // Exclusivity guard: removing one extension must NOT revoke a credential
        // still used by another installed extension. Gmail and Google Calendar
        // share the `google` provider; removing Gmail must leave the Google
        // credential intact so Calendar keeps working.
        let services = test_services("extension-tools-remove-shared-owner", None, true).await;

        for extension_id in ["gmail", "google-calendar"] {
            install_inactive(&services, extension_id).await;
        }
        let context = execution_context([EXTENSION_ACTIVATE_CAPABILITY_ID]);
        // One reusable Google credential covering both extensions' scopes.
        seed_configured_account_with_scopes(
            &services,
            &context.resource_scope,
            "google",
            &[
                "https://www.googleapis.com/auth/gmail.modify",
                "https://www.googleapis.com/auth/gmail.readonly",
                "https://www.googleapis.com/auth/gmail.send",
                "https://www.googleapis.com/auth/calendar.events",
                "https://www.googleapis.com/auth/calendar.readonly",
            ],
            true,
        )
        .await;
        for extension_id in ["gmail", "google-calendar"] {
            let activate = invoke_json(
                &services,
                EXTENSION_ACTIVATE_CAPABILITY_ID,
                serde_json::json!({ "extension_id": extension_id }),
            )
            .await
            .expect("activate succeeds with the shared google credential");
            assert_eq!(activate["payload"]["activated"], true);
        }

        let remove = invoke_json(
            &services,
            EXTENSION_REMOVE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "gmail"}),
        )
        .await
        .expect("remove succeeds");
        assert_eq!(remove["payload"]["removed"], true);

        // Calendar still uses `google`, so the shared credential must survive:
        // re-activation succeeds without an auth gate.
        let outcome = invoke_outcome(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "google-calendar"}),
        )
        .await;
        assert!(
            matches!(outcome, RuntimeCapabilityOutcome::Completed(_)),
            "removing gmail must not revoke the shared google credential calendar still uses, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn standalone_extension_activate_returns_auth_gate_for_missing_extension_credentials() {
        let services = test_services("extension-tools-auth-gate-owner", None, false).await;
        let extension_management = services.extension_management.clone();

        install_inactive(&services, "github").await;

        let outcome = invoke_outcome(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
            panic!("expected extension activation to request auth, got {outcome:?}");
        };
        assert_eq!(
            gate.capability_id.as_str(),
            EXTENSION_ACTIVATE_CAPABILITY_ID
        );
        assert_eq!(gate.credential_requirements.len(), 1);
        let requirement = &gate.credential_requirements[0];
        assert_eq!(requirement.provider.as_str(), "github");
        assert_eq!(requirement.requester_extension.as_str(), "github");

        let active = active_extension_capability_ids(&extension_management).await;
        assert!(!active.iter().any(|id| id == "github.search_issues"));

        // #5525 review: a foreign caller probing the same private credentialed
        // install must NOT receive the auth gate — that response confirms the
        // install exists and leaks its credential requirement shape. Ownership
        // masks before the credential preflight, so the non-owner sees the
        // same failure a missing installation would produce.
        let outcome = invoke_with_standalone_approval(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            execution_context_for_user(
                "extension-tool-foreign-user",
                [EXTENSION_ACTIVATE_CAPABILITY_ID],
            ),
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("foreign caller must get the masked failure, not an auth gate: {outcome:?}");
        };
        assert_eq!(failure.kind, FailureKind::InputEncode);
    }

    #[tokio::test]
    async fn standalone_extension_search_distinguishes_configured_from_active() {
        let services = test_services("extension-tools-active-search-owner", None, false).await;

        let available_search = invoke_json(
            &services,
            EXTENSION_SEARCH_CAPABILITY_ID,
            serde_json::json!({"query": "github"}),
        )
        .await
        .expect("available search succeeds");
        let available_extensions = available_search["payload"]["extensions"]
            .as_array()
            .expect("extensions array");
        let available_github = available_extensions
            .iter()
            .find(|extension| extension["package_ref"]["id"] == "github")
            .expect("github search result");
        assert_eq!(available_github.get("installation_phase"), None);
        assert!(
            available_github.get("credential_requirements").is_none(),
            "available GitHub model-visible search results must not expose PAT requirements before activation"
        );
        assert!(
            available_github.get("onboarding").is_none(),
            "available GitHub model-visible search results must not expose PAT setup onboarding before activation"
        );

        install_inactive(&services, "github").await;

        let installed_search = invoke_json(
            &services,
            EXTENSION_SEARCH_CAPABILITY_ID,
            serde_json::json!({"query": "github"}),
        )
        .await
        .expect("installed search succeeds");
        let installed_extensions = installed_search["payload"]["extensions"]
            .as_array()
            .expect("extensions array");
        let installed_github = installed_extensions
            .iter()
            .find(|extension| extension["package_ref"]["id"] == "github")
            .expect("github search result");
        assert_eq!(installed_github["installation_phase"], "setup_needed");
        let installed_message = installed_search["message"]
            .as_str()
            .expect("installed inactive search should carry install guidance");
        assert!(
            installed_message.contains("installed but not activated")
                && installed_message.contains("not currently callable tools")
                && installed_message.contains(EXTENSION_INSTALL_CAPABILITY_ID),
            "installed inactive GitHub search must guide install-driven activation, got {installed_search}"
        );
        assert!(
            installed_github.get("credential_requirements").is_none(),
            "installed inactive GitHub model-visible search results must not expose stale PAT requirements before activation"
        );
        assert!(
            installed_github.get("onboarding").is_none(),
            "installed inactive GitHub model-visible search results must not expose stale PAT setup onboarding before activation"
        );

        let activate_context = execution_context([EXTENSION_ACTIVATE_CAPABILITY_ID]);
        seed_configured_account(&services, &activate_context.resource_scope, "github").await;

        let configured_search = invoke_json(
            &services,
            EXTENSION_SEARCH_CAPABILITY_ID,
            serde_json::json!({"query": "github"}),
        )
        .await
        .expect("configured search succeeds");
        let configured_message = configured_search["message"]
            .as_str()
            .expect("configured ready search should carry readiness guidance");
        assert!(
            configured_message.contains("active installed extension results")
                && configured_message.contains("ready for this connection request"),
            "configured GitHub search must report ready active tools, got {configured_search}"
        );
        let extensions = configured_search["payload"]["extensions"]
            .as_array()
            .expect("extensions array");
        let github = extensions
            .iter()
            .find(|extension| extension["package_ref"]["id"] == "github")
            .expect("github search result");
        assert_eq!(github["installation_phase"], "active");
        assert!(
            github.get("credential_requirements").is_none(),
            "ready GitHub model-visible search results must not expose satisfied PAT requirements"
        );
        assert!(
            github.get("onboarding").is_none(),
            "ready GitHub model-visible search results must not expose stale PAT setup onboarding"
        );

        let activate = invoke_json(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await
        .expect("activate succeeds");
        assert_eq!(activate["payload"]["activated"], true);

        let active_search = invoke_json(
            &services,
            EXTENSION_SEARCH_CAPABILITY_ID,
            serde_json::json!({"query": "github"}),
        )
        .await
        .expect("active search succeeds");
        assert!(
            active_search["message"]
                .as_str()
                .is_some_and(|message| message.contains("active installed extension results")),
            "active GitHub search should override stale PAT onboarding, got {active_search}"
        );
        let extensions = active_search["payload"]["extensions"]
            .as_array()
            .expect("extensions array");
        let github = extensions
            .iter()
            .find(|extension| extension["package_ref"]["id"] == "github")
            .expect("github search result");
        assert_eq!(github["installation_phase"], "active");
        assert!(
            github.get("credential_requirements").is_none(),
            "active GitHub model-visible search results must not expose satisfied PAT requirements"
        );
        assert!(
            github.get("onboarding").is_none(),
            "active GitHub model-visible search results must not expose stale PAT setup onboarding"
        );
    }

    #[tokio::test]
    async fn standalone_extension_activate_returns_auth_gate_when_account_lacks_required_scope() {
        let services = test_services("extension-tools-scope-gate-owner", None, true).await;
        let extension_management = services.extension_management.clone();

        install_inactive(&services, "google-calendar").await;
        let activate_context = execution_context([EXTENSION_ACTIVATE_CAPABILITY_ID]);
        seed_configured_account_with_scopes(
            &services,
            &activate_context.resource_scope,
            "google",
            &["https://www.googleapis.com/auth/calendar.readonly"],
            true,
        )
        .await;

        let outcome = invoke_outcome(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "google-calendar"}),
        )
        .await;
        let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
            panic!("expected missing calendar.events scope to request auth, got {outcome:?}");
        };
        assert_eq!(gate.credential_requirements.len(), 1);
        let requirement = &gate.credential_requirements[0];
        assert_eq!(requirement.provider.as_str(), "google");
        assert_eq!(requirement.requester_extension.as_str(), "google-calendar");
        assert_eq!(
            requirement
                .provider_scopes
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "https://www.googleapis.com/auth/calendar.events".to_string(),
                "https://www.googleapis.com/auth/calendar.readonly".to_string(),
            ])
        );

        let active = active_extension_capability_ids(&extension_management).await;
        assert!(!active.iter().any(|id| id == "google-calendar.create_event"));
    }

    #[tokio::test]
    async fn standalone_extension_activate_coalesces_gmail_oauth_scopes_into_one_auth_gate() {
        let services = test_services("extension-tools-gmail-scope-union-owner", None, true).await;
        let extension_management = services.extension_management.clone();

        install_inactive(&services, "gmail").await;

        let outcome = invoke_outcome(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "gmail"}),
        )
        .await;
        let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
            panic!("expected Gmail activation to request auth, got {outcome:?}");
        };
        assert_eq!(
            gate.capability_id.as_str(),
            EXTENSION_ACTIVATE_CAPABILITY_ID
        );
        assert_eq!(
            gate.credential_requirements.len(),
            1,
            "Gmail activation should ask for one Google OAuth gate"
        );
        let requirement = &gate.credential_requirements[0];
        assert_eq!(requirement.provider.as_str(), "google");
        assert_eq!(requirement.requester_extension.as_str(), "gmail");
        assert_eq!(
            requirement
                .provider_scopes
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "https://www.googleapis.com/auth/gmail.modify".to_string(),
                "https://www.googleapis.com/auth/gmail.readonly".to_string(),
                "https://www.googleapis.com/auth/gmail.send".to_string(),
            ])
        );

        let active = active_extension_capability_ids(&extension_management).await;
        assert!(!active.iter().any(|id| id == "gmail.list_messages"));
    }

    #[tokio::test]
    async fn standalone_extension_activate_maps_corrupt_configured_account_to_backend() {
        let services = test_services("extension-tools-corrupt-auth-owner", None, false).await;
        let extension_management = services.extension_management.clone();

        install_inactive(&services, "github").await;
        let activate_context = execution_context([EXTENSION_ACTIVATE_CAPABILITY_ID]);
        seed_configured_account_with_scopes(
            &services,
            &activate_context.resource_scope,
            "github",
            &[],
            false,
        )
        .await;

        let outcome = invoke_outcome(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("expected corrupt configured account to fail, got {outcome:?}");
        };
        assert_eq!(failure.kind, FailureKind::Backend);

        let active = active_extension_capability_ids(&extension_management).await;
        assert!(!active.iter().any(|id| id == "github.search_issues"));
    }

    /// Runtime-dispatched hosted-MCP activation with the P2 staging fix:
    /// activation stages the connection-template capability's network policy
    /// and product-auth credential under the discovery scope, so live
    /// `tools/list` runs through the REAL host egress pipeline (the scripted
    /// double sits at the network transport, under staged-policy checks and
    /// staged-credential injection) and the ceiling-validated discovered
    /// tools publish as model-visible capabilities. Before this fix nothing
    /// staged the discovery plan — the request keyed on the dispatch-minted
    /// invocation scope found no policy/credential, failed transient, and
    /// fell back to the bundled manifest with zero model-visible tools.
    #[tokio::test]
    async fn standalone_extension_activate_hosted_mcp_stages_discovery_and_publishes_tools() {
        let discovery_script = std::sync::Arc::new(
            crate::extension_lifecycle::hosted_mcp_test_support::HostedMcpDiscoveryNetworkScript::with_tool_name("notion-search")
                // Real hosted MCP providers may return verbose prose. The
                // generic MCP boundary must bound it without dropping the
                // entire catalog or preventing activation.
                .with_tool_description("provider documentation ".repeat(320)),
        );
        let services = test_services(
            "extension-tools-hosted-mcp-owner",
            Some(discovery_script.clone()),
            false,
        )
        .await;
        let extension_management = services.extension_management.clone();

        let activate_context = execution_context([EXTENSION_ACTIVATE_CAPABILITY_ID]);
        seed_configured_account(&services, &activate_context.resource_scope, "notion").await;
        // The account's access token must exist as real material: discovery
        // staging leases it from the secret store into the one-shot
        // injection store.
        let owner_scope = ironclaw_auth::AuthProductScope::credential_owner(
            &activate_context.resource_scope,
            ironclaw_auth::AuthSurface::Api,
        );
        services
            .secret_store()
            .put(
                owner_scope.resource.clone(),
                SecretHandle::new("notion-test-token").expect("handle"),
                ironclaw_secrets::SecretMaterial::from("notion-access-token"),
                None,
            )
            .await
            .expect("seed access-token material");

        let activate = invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "notion"}),
        )
        .await;
        let activate = activate.expect("install-driven hosted MCP activation succeeds");
        assert_eq!(activate["phase"], "active");

        // Live discovery ran through the staged pipeline: the discovered
        // tool is model-visible.
        let active = active_extension_capability_ids(&extension_management).await;
        assert!(
            active.iter().any(|id| id == "notion.notion-search"),
            "discovered hosted-MCP tool must be model-visible after staged discovery; got {active:?}"
        );
        // The staged connection credential reached the vendor wire on every
        // discovery call (initialize → notifications/initialized →
        // tools/list), through the real egress pipeline's injection.
        let calls = discovery_script.authorized_methods();
        assert!(
            calls.iter().any(|(method, _)| method == "tools/list"),
            "discovery must reach tools/list; calls: {calls:?}"
        );
        assert!(
            calls.iter().all(|(_, authorized)| *authorized),
            "every discovery call must carry the staged credential; calls: {calls:?}"
        );
    }

    /// Pins the ordering the test above assumes but never checks: activation
    /// must run hosted-MCP discovery BEFORE `commit_activation` publishes,
    /// because the published `AuthorityCeiling` (`extension_allowed_effects`
    /// in `active_publication.rs`) is computed from whatever
    /// `ExtensionPackage` publish() is handed — the DISCOVERED package if
    /// discovery ran first, the pre-discovery SEED package if not. If
    /// activation is ever reordered to publish before discovery (or a new
    /// activation path publishes the seed package), an effect a live MCP
    /// server only reveals at discovery time silently drops out of the
    /// ceiling and every tool needing it gets denied at authorization with
    /// no error at the publish site.
    ///
    /// `notion`'s bundled manifest already declares every effect
    /// (`network`, `use_secret`, `external_write`) its discovered tools can
    /// ever produce, so swapping seed for discovered there changes nothing
    /// observable — it cannot discriminate this regression. `nearai` is the
    /// only other bundled hosted-MCP package, and its static manifest
    /// declares only `network` + `use_secret`: scripting a discovered tool
    /// with a `destructiveHint` annotation (see
    /// `discovered_tool_requires_external_write` in
    /// `ironclaw_extensions::hosted_mcp_discovery`) makes the DISCOVERED
    /// package carry `ExternalWrite` while the SEED package never does —
    /// exactly the shape needed to fail if publish ever ran on the wrong
    /// package.
    #[tokio::test]
    async fn local_dev_extension_activate_hosted_mcp_authority_ceiling_reflects_discovered_effects()
    {
        let discovery_script = std::sync::Arc::new(
            crate::extension_lifecycle::hosted_mcp_test_support::HostedMcpDiscoveryNetworkScript::with_tool_name("nearai-destructive-action")
                .with_destructive_hint(),
        );
        let services = test_services(
            "extension-tools-hosted-mcp-ceiling-owner",
            Some(discovery_script.clone()),
            false,
        )
        .await;

        let activate_context = execution_context([EXTENSION_ACTIVATE_CAPABILITY_ID]);
        seed_configured_account(&services, &activate_context.resource_scope, "nearai").await;
        // Real access-token material: discovery stages it from the secret
        // store into the one-shot injection store for the live egress call.
        let owner_scope = ironclaw_auth::AuthProductScope::credential_owner(
            &activate_context.resource_scope,
            ironclaw_auth::AuthSurface::Api,
        );
        services
            .secret_store()
            .put(
                owner_scope.resource.clone(),
                SecretHandle::new("nearai-test-token").expect("handle"),
                ironclaw_secrets::SecretMaterial::from("nearai-access-token"),
                None,
            )
            .await
            .expect("seed access-token material");

        let activate = invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "nearai"}),
        )
        .await;
        let activate = activate.expect("install-driven hosted MCP activation succeeds");
        assert_eq!(activate["phase"], "active");

        // Read back the published trust entry through the SAME seam
        // authorization consumes (`ironclaw_authorization::effects_are_covered`
        // reads `AuthorityCeiling::allowed_effects`), not an internal exposed
        // solely for this test: `ActiveExtensionPublisher::publish` writes the
        // ceiling through `HostTrustPolicy::mutate_with` /
        // `AdminEntry::for_local_manifest`, and `TrustPolicy::evaluate` is the
        // policy's own public read path back to that decision.
        let extension_id = ExtensionId::new("nearai").expect("valid extension id");
        let published_package = services
            .extension_management
            .active_extensions_for_test()
            .snapshot()
            .get_extension(&extension_id)
            .cloned()
            .expect("nearai package published after activation");
        let trust_input = crate::extension_trust_policy_input(&published_package)
            .expect("trust policy input derives from the published package");
        let decision = services
            .trust_policy
            .evaluate(&trust_input)
            .expect("trust policy evaluates the published package identity");

        assert!(
            decision
                .authority_ceiling
                .allowed_effects
                .contains(&EffectKind::ExternalWrite),
            "published authority ceiling must include the discovery-only ExternalWrite \
             effect (destructiveHint tool absent from nearai's static manifest); got {:?}. \
             Missing here means publish() ran on the pre-discovery seed package instead of \
             the live-discovered one.",
            decision.authority_ceiling.allowed_effects
        );
    }

    #[tokio::test]
    async fn standalone_extension_lifecycle_tool_lists_all_and_rejects_malformed_inputs() {
        let services = test_services("extension-tools-invalid-owner", None, false).await;
        let list_all = invoke_json(
            &services,
            EXTENSION_SEARCH_CAPABILITY_ID,
            serde_json::json!({}),
        )
        .await
        .expect("search without a query should list all extensions");
        assert_eq!(list_all["payload"]["kind"], "extension_search");
        assert!(
            list_all["payload"]["count"].as_u64().unwrap_or_default() > 0,
            "list-all extension search should return the bundled standalone packages"
        );
        assert_eq!(
            invoke_json(
                &services,
                EXTENSION_INSTALL_CAPABILITY_ID,
                serde_json::json!({})
            )
            .await,
            Err(FailureKind::InputEncode)
        );
        assert_eq!(
            invoke_json(
                &services,
                EXTENSION_INSTALL_CAPABILITY_ID,
                serde_json::json!({"extension_id": "unknown-extension"})
            )
            .await,
            Err(FailureKind::InputEncode)
        );
        let outcome = invoke_outcome(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("expected uninstalled extension activation to fail, got {outcome:?}");
        };
        assert_eq!(failure.kind, FailureKind::InputEncode);
    }

    async fn invoke_json(
        services: &ExtensionLifecycleTestServices,
        capability_id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, FailureKind> {
        invoke_json_with_standalone_approval(
            services,
            capability_id,
            execution_context([capability_id]),
            input,
        )
        .await
    }

    async fn invoke_outcome(
        services: &ExtensionLifecycleTestServices,
        capability_id: &str,
        input: serde_json::Value,
    ) -> RuntimeCapabilityOutcome {
        invoke_with_standalone_approval(
            services,
            capability_id,
            execution_context([capability_id]),
            input,
        )
        .await
    }

    async fn seed_configured_account(
        services: &ExtensionLifecycleTestServices,
        scope: &ResourceScope,
        provider: &str,
    ) {
        seed_configured_account_with_scopes(services, scope, provider, &[], true).await;
    }

    async fn seed_configured_account_with_scopes(
        services: &ExtensionLifecycleTestServices,
        scope: &ResourceScope,
        provider: &str,
        scopes: &[&str],
        include_access_secret: bool,
    ) {
        services
            .product_auth
            .as_ref()
            .credential_account_service()
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::credential_owner(scope, AuthSurface::Api),
                provider: AuthProviderId::new(provider).expect("valid auth provider"),
                label: CredentialAccountLabel::new(provider).expect("valid account label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: include_access_secret.then(|| {
                    SecretHandle::new(format!("{provider}-test-token"))
                        .expect("valid secret handle")
                }),
                refresh_secret: None,
                scopes: scopes
                    .iter()
                    .map(|scope| ProviderScope::new((*scope).to_string()).expect("valid scope"))
                    .collect(),
            })
            .await
            .expect("create configured account");
    }

    async fn install_inactive(services: &ExtensionLifecycleTestServices, extension_id: &str) {
        let context = execution_context([EXTENSION_ACTIVATE_CAPABILITY_ID]);
        install_inactive_for_user(services, extension_id, &context.resource_scope.user_id).await;
    }

    async fn install_inactive_for_user(
        services: &ExtensionLifecycleTestServices,
        extension_id: &str,
        caller: &UserId,
    ) {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id)
            .expect("valid extension package ref");
        services
            .extension_management
            .install(package_ref, caller)
            .await
            .expect("durable inactive install succeeds");
    }

    async fn active_extension_capability_ids(
        extension_management: &RebornLocalExtensionManagementPort,
    ) -> Vec<String> {
        extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active extension capabilities")
            .into_iter()
            .map(|capability| capability.id.as_str().to_string())
            .collect()
    }

    fn visible_request<'a>(
        capability_ids: impl IntoIterator<Item = &'a str>,
    ) -> VisibleCapabilityRequest {
        let mut provider_trust = BTreeMap::new();
        provider_trust.insert(ExtensionId::new("builtin").unwrap(), trust_decision());
        provider_trust.insert(ExtensionId::new("github").unwrap(), trust_decision());
        VisibleCapabilityRequest::new(
            execution_context(capability_ids),
            SurfaceKind::new("agent_loop").unwrap(),
        )
        .with_policy(CapabilitySurfacePolicy::allow_all())
        .with_provider_trust(provider_trust)
    }

    fn execution_context<'a>(
        capability_ids: impl IntoIterator<Item = &'a str>,
    ) -> ExecutionContext {
        execution_context_for_user("extension-tool-test-user", capability_ids)
    }

    fn execution_context_for_user<'a>(
        user: &str,
        capability_ids: impl IntoIterator<Item = &'a str>,
    ) -> ExecutionContext {
        let caller = ExtensionId::new("extension-tool-test-caller").expect("valid extension id");
        let user_id = UserId::new(user).expect("valid user id");
        let mut context = ExecutionContext::local_default(
            user_id.clone(),
            caller.clone(),
            RuntimeKind::FirstParty,
            TrustClass::FirstParty,
            CapabilitySet {
                grants: capability_ids
                    .into_iter()
                    .map(|capability_id| capability_grant(capability_id, caller.clone()))
                    .collect(),
            },
            MountView::default(),
        )
        .expect("valid execution context");
        context.authenticated_actor_user_id = Some(user_id);
        context.run_id = Some(ironclaw_host_api::RunId::new());
        context
    }

    fn capability_grant(capability_id: &str, grantee: ExtensionId) -> CapabilityGrant {
        CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: CapabilityId::new(capability_id).expect("valid capability id"),
            grantee: Principal::Extension(grantee),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: allowed_effects(),
                mounts: MountView::default(),
                network: NetworkPolicy {
                    allowed_targets: vec![NetworkTargetPattern {
                        scheme: None,
                        host_pattern: "*".to_string(),
                        port: None,
                    }],
                    deny_private_ip_ranges: true,
                    max_egress_bytes: None,
                },
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }
    }

    fn surface_capability_ids(surface: &VisibleCapabilitySurface) -> Vec<&str> {
        surface
            .capabilities
            .iter()
            .map(|capability| capability.descriptor.id.as_str())
            .collect()
    }

    fn descriptor_for<'a>(
        surface: &'a VisibleCapabilitySurface,
        capability_id: &str,
    ) -> &'a CapabilityDescriptor {
        surface
            .capabilities
            .iter()
            .find(|capability| capability.descriptor.id.as_str() == capability_id)
            .map(|capability| &capability.descriptor)
            .expect("capability descriptor")
    }

    fn allowed_effects() -> Vec<EffectKind> {
        vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
        ]
    }

    fn trust_decision() -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: allowed_effects(),
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: chrono::Utc::now(),
        }
    }

    /// The fixed `safe_summary` headline used
    /// for `ProviderInstanceNotConfigured` must itself pass the strict
    /// `LoopSafeSummary` validator (capability-access redaction invariant) —
    /// proves the summary never trips the `{}[]<>/` / secret-vocabulary
    /// rejection that would otherwise kill the whole run — and the full
    /// remediation must ride the diagnostic-detail channel, naming the exact
    /// `config set` command verbatim.
    #[test]
    fn provider_instance_not_configured_safe_summary_validates_and_diagnostic_names_config_set() {
        ironclaw_turns::run_profile::LoopSafeSummary::new(
            PROVIDER_INSTANCE_NOT_CONFIGURED_SAFE_SUMMARY,
        )
        .expect("fixed safe_summary must pass the strict LoopSafeSummary validator");

        let reason = format!(
            "{}\n\n{}",
            ironclaw_reborn_config::google_remediation_text(),
            ironclaw_reborn_config::apply_step_text()
        );
        let mapped = lifecycle_error(ProductSurfaceFailure::ProviderInstanceNotConfigured {
            reason: reason.clone(),
        });

        let FirstPartyCapabilityError::Dispatch {
            kind,
            safe_summary,
            detail,
            ..
        } = mapped
        else {
            panic!("expected a Dispatch failure, got {mapped:?}");
        };
        assert_eq!(kind, RuntimeDispatchErrorKind::OperationFailed);
        assert_eq!(
            safe_summary,
            Some(PROVIDER_INSTANCE_NOT_CONFIGURED_SAFE_SUMMARY.to_string())
        );
        let detail = detail.expect("remediation detail must be present");
        // The TRUSTED channel, not the untrusted diagnostic one: this reason is
        // host-authored, and the untrusted channel collapses it to the
        // safe-summary placeholder at the host_api boundary (#6299).
        let ironclaw_host_api::DispatchFailureDetail::HostRemediation { text } = *detail else {
            panic!("expected a HostRemediation detail, got {detail:?}");
        };
        assert!(text.as_str().contains("config set google.client_id"));
        assert_eq!(text.as_str(), reason);
    }

    /// `InvalidBindingRequest` keeps its existing `InputEncode` kind and
    /// carries its reason on the UNTRUSTED diagnostic channel. Several of its
    /// ~40 construction sites interpolate externally-influenced text (hosted
    /// MCP tool names, the model-chosen `extension_id`, uploaded-zip entry
    /// names), so the whole class must stay scanned; only reasons built
    /// entirely from host-authored constants may ride the trusted channel.
    #[test]
    fn invalid_binding_request_carries_reason_on_the_untrusted_diagnostic_channel() {
        let mapped = lifecycle_error(ProductSurfaceFailure::InvalidBindingRequest {
            reason: "telegram account setup was declared without a mounted host".to_string(),
        });

        let FirstPartyCapabilityError::Dispatch { kind, detail, .. } = mapped else {
            panic!("expected a Dispatch failure, got {mapped:?}");
        };
        assert_eq!(kind, RuntimeDispatchErrorKind::InputEncode);
        let detail = detail.expect("diagnostic detail must be present");
        let ironclaw_host_api::DispatchFailureDetail::Diagnostic { text } = *detail else {
            panic!("expected a Diagnostic detail, got {detail:?}");
        };
        assert!(text.contains("mounted host"));
    }

    #[test]
    fn transient_lifecycle_errors_map_to_retryable_backend_failure() {
        let mapped = lifecycle_error(ProductSurfaceFailure::Transient {
            reason: "temporary lifecycle store outage".to_string(),
        });

        let FirstPartyCapabilityError::Dispatch { kind, .. } = mapped else {
            panic!("expected a Dispatch failure, got {mapped:?}");
        };
        assert_eq!(kind, RuntimeDispatchErrorKind::Backend);
    }

    /// The provenance regression itself, at the unit seam: a reason carrying
    /// credential vocabulary (the shape a model-chosen `extension_id` can
    /// produce) must NOT land on the trusted host-remediation channel, where
    /// `ObservationTrust::HostAuthored` would exempt it from the downstream
    /// credential-vocabulary scan.
    #[test]
    fn model_influenced_invalid_binding_reason_never_reaches_the_trusted_channel() {
        let mapped = lifecycle_error(ProductSurfaceFailure::InvalidBindingRequest {
            reason: "extension api_key is not installed".to_string(),
        });

        let FirstPartyCapabilityError::Dispatch { detail, .. } = mapped else {
            panic!("expected a Dispatch failure, got {mapped:?}");
        };
        assert!(
            matches!(
                detail.as_deref(),
                Some(ironclaw_host_api::DispatchFailureDetail::Diagnostic { .. })
            ),
            "externally-influenced text must stay on the scanned channel, got {detail:?}"
        );
    }
}
