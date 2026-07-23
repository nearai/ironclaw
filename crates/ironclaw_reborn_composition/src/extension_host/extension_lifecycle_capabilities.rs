// arch-exempt: large_file, model-visible extension removal adapter and caller tests, plan #5905
use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityDisplayOutputPreview, CapabilityId, CapabilityProfileSchemaRef, CredentialStageError,
    DispatchInputIssue, DispatchInputIssueCode, EffectKind, HostApiError, OriginGateMatrix,
    OriginGatePolicy, PermissionMode, ResourceEstimate, ResourceProfile, ResourceUsage,
    RuntimeDispatchErrorKind,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product::{
    LifecyclePackageKind, LifecyclePackageRef, LifecycleProductPayload, LifecycleProductResponse,
    ProductWorkflowError,
};
use serde::Deserialize;

use crate::extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, ExtensionManagementPort,
};
use crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountSelectionService;

pub(crate) const EXTENSION_SEARCH_CAPABILITY_ID: &str = "builtin.extension_search";
pub(crate) const EXTENSION_INSTALL_CAPABILITY_ID: &str = "builtin.extension_install";
pub(crate) const EXTENSION_REMOVE_CAPABILITY_ID: &str = "builtin.extension_remove";

pub(crate) const EXTENSION_LIFECYCLE_CAPABILITY_IDS: [&str; 3] = [
    EXTENSION_SEARCH_CAPABILITY_ID,
    EXTENSION_INSTALL_CAPABILITY_ID,
    EXTENSION_REMOVE_CAPABILITY_ID,
];

pub(crate) fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package.manifest.capabilities.extend(manifests()?);
    ExtensionPackage::from_manifest(package.manifest, package.root)
}

pub(crate) fn insert_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    extension_management: Arc<ExtensionManagementPort>,
    credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
) -> Result<(), HostApiError> {
    let handler = Arc::new(ExtensionLifecycleToolHandler {
        extension_management,
        credential_accounts,
    });
    for capability_id in EXTENSION_LIFECYCLE_CAPABILITY_IDS {
        registry.insert_handler(CapabilityId::new(capability_id)?, handler.clone());
    }
    Ok(())
}

fn manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        lifecycle_manifest(
            EXTENSION_SEARCH_CAPABILITY_ID,
            "Search the local Reborn extension catalog by extension, product, provider, or service name, including extensions not installed yet or whose setup is incomplete. For connect, enable, install, pair, authenticate, or integrate requests, use this for discovery only, then continue with builtin.extension_install for the matching extension instead of inventing setup instructions. Installation publishes tools internally when manifest-declared personal setup is ready. For routine, trigger, or notification delivery, prefer configured outbound delivery targets before installing an external channel.",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
        )?,
        lifecycle_manifest(
            EXTENSION_INSTALL_CAPABILITY_ID,
            "Install a searched Reborn extension and complete every internal lifecycle checkpoint that is currently possible. The result is either active or blocked on the extension manifest's personal auth/pairing setup; there is no separate user activation step. If setup is required, follow the returned typed auth or connection guidance and continue the original request after setup completes.",
            vec![
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
            ],
            PermissionMode::Ask,
        )?,
        lifecycle_manifest(
            EXTENSION_REMOVE_CAPABILITY_ID,
            "Remove an installed Reborn extension from durable local-dev lifecycle state. Use this when the user asks to uninstall, remove, disable, disconnect, unpair, unlink, or revoke access for an extension, integration, app, account, external channel, or the current external chat. Pass the extension's registry id as extension_id; removal also performs extension-owned cleanup such as authentication, identity, and channel bindings when supported.",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Ask,
        )?,
    ])
}

fn lifecycle_manifest(
    id: &str,
    description: &str,
    effects: Vec<EffectKind>,
    default_permission: PermissionMode,
) -> Result<CapabilityManifest, ExtensionError> {
    let schema_name = id.strip_prefix("builtin.").unwrap_or(id).replace('.', "-");
    Ok(CapabilityManifest {
        id: CapabilityId::new(id)?,
        implements: Vec::new(),
        description: description.to_string(),
        effects,
        default_permission,
        visibility: CapabilityVisibility::Model,
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
        EXTENSION_INSTALL_CAPABILITY_ID | EXTENSION_REMOVE_CAPABILITY_ID
    ) {
        matrix.product = OriginGatePolicy::ConsentSufficient;
    }
    matrix
}

struct ExtensionLifecycleToolHandler {
    extension_management: Arc<ExtensionManagementPort>,
    credential_accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
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
        let response = match request.capability_id.as_str() {
            EXTENSION_SEARCH_CAPABILITY_ID => {
                let input: SearchInput = parse_input(request.input)?;
                let credential_gate = RuntimeExtensionActivationCredentialGate::new(
                    request.scope.clone(),
                    Arc::clone(&self.credential_accounts),
                );
                self.extension_management
                    .search(&input.query, Some(&credential_gate), &request.scope.user_id)
                    .await
                    .map_err(lifecycle_error)
            }
            EXTENSION_INSTALL_CAPABILITY_ID => {
                let input: ExtensionIdInput = parse_input(request.input)?;
                // The dispatch scope carries the acting user; admin deployment
                // configuration is a separate manifest-declared state machine.
                let package_ref = extension_package_ref(input.extension_id)?;
                let install = self
                    .extension_management
                    .install(package_ref.clone(), &request.scope.user_id)
                    .await
                    .map_err(lifecycle_error)?;
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
                let activation = self
                    .extension_management
                    .activate_with_credential_gate(
                        package_ref,
                        mode,
                        credential_gate,
                        &request.scope.user_id,
                    )
                    .await
                    .map_err(lifecycle_error)?;
                Ok(
                    crate::extension_host::extension_lifecycle::complete_install_response(
                        install, activation,
                    ),
                )
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
        let connection_preview = channel_connection_display_preview(&response);
        let output = serde_json::to_value(without_model_visible_connection_chrome(response))
            .map_err(|error| {
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
    let requirement = match response.payload.as_ref() {
        Some(LifecycleProductPayload::ExtensionInstall {
            connection_required: Some(requirement),
            ..
        }) => requirement,
        _ => return None,
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

/// The structured connect requirement carries render chrome for the in-chat
/// connection panel and rides the display-preview side channel only. Strip it
/// from the model-visible tool output so the model sees just activation prose.
fn without_model_visible_connection_chrome(
    mut response: LifecycleProductResponse,
) -> LifecycleProductResponse {
    if let Some(LifecycleProductPayload::ExtensionInstall {
        connection_required,
        ..
    }) = response.payload.as_mut()
    {
        *connection_required = None;
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
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, id).map_err(|_| {
        FirstPartyCapabilityError::invalid_input_issues(
            "extension id is invalid",
            vec![DispatchInputIssue::new(
                "extension_id",
                DispatchInputIssueCode::InvalidValue,
            )],
        )
    })
}

/// Fixed, host-authored, validator-safe headline for the
/// `dispatch_with_host_remediation` call below — the strict `safe_summary`
/// validator rejects `{}[]<>/` and secret-like vocabulary
/// (`agent-loop-capabilities` invariant 2), so the full `config set`
/// remediation text rides the trusted host-remediation channel instead;
/// `safe_summary` stays this short fixed literal.
const PROVIDER_INSTANCE_UNAVAILABLE_SAFE_SUMMARY: &str =
    "extension is unavailable on this instance";

fn lifecycle_error(error: ProductWorkflowError) -> FirstPartyCapabilityError {
    match error {
        // UNTRUSTED on purpose. `InvalidBindingRequest` has ~40 construction
        // sites and several interpolate externally-influenced text: a hosted
        // MCP server's live `tools/list` tool names
        // (`hosted_mcp_discovery.rs` -> `hosted_mcp_discovery_error`), the
        // MODEL-chosen `extension_id` (charset-validated only, e.g. "extension
        // {} is not installed"), and uploaded-zip entry names
        // (`extension_bundle.rs`). `HostRemediation::new` is a VALUE guard, not
        // a provenance guard — it rejects credential-SHAPED tokens but allows
        // adversarial prose — so routing this whole class onto the trusted
        // channel would stamp `ObservationTrust::HostAuthored` on attacker
        // -influenced text and skip the credential-vocabulary scan
        // `ironclaw_threads` applies to untrusted output. The trusted channel
        // is reserved for reasons built entirely from host-authored constants
        // (the `ProviderInstanceNotConfigured` arm below).
        ProductWorkflowError::InvalidBindingRequest { reason } => {
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
        ProductWorkflowError::ProviderInstanceNotConfigured => {
            FirstPartyCapabilityError::dispatch_with_diagnostic(
                RuntimeDispatchErrorKind::OperationFailed,
                Some(PROVIDER_INSTANCE_UNAVAILABLE_SAFE_SUMMARY.to_string()),
                PROVIDER_INSTANCE_UNAVAILABLE_SAFE_SUMMARY,
            )
        }
        ProductWorkflowError::UnsupportedActionKind { .. } => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode)
        }
        ProductWorkflowError::Transient { .. } => {
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
        ExtensionId, GrantConstraints, MountView, NetworkPolicy, NetworkTargetPattern,
        OriginGatePolicy, PermissionMode, Principal, ResourceScope, RuntimeKind, SecretHandle,
        TrustClass, UNGATED_LOOP_RUN_CAPABILITIES, UserId,
    };
    use ironclaw_host_runtime::{
        CapabilitySurfacePolicy, RuntimeCapabilityOutcome, RuntimeFailureKind, SurfaceKind,
        VisibleCapabilityRequest, VisibleCapabilitySurface,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::OAuthClientConfig;
    use crate::factory::{RebornRuntimeStores, build_runtime_substrate};
    use ironclaw_product::{
        ChannelConnectionRequirement, LifecyclePublicState, RebornChannelConnectStrategy,
    };

    /// Dummy but well-formed Google OAuth backend config for tests below that
    /// exercise PER-ACCOUNT credential gating (scope coalescing, shared
    /// credential reuse) rather than the provider-instance readiness map —
    /// without this, every google-family activation on a plain
    /// `crate::deployment::local_dev_build_input(..)` fixture now fails closed
    /// with `ProviderInstanceNotConfigured` before it ever reaches the
    /// per-account gate these tests target. Mirrors
    /// `factory/auth_tests.rs::local_dev_google_oauth_backend_builds_with_host_provider_config`.
    fn test_google_oauth_client_config() -> OAuthClientConfig {
        OAuthClientConfig::new(
            "itest-google-client-id.apps.googleusercontent.com",
            "http://127.0.0.1/oauth/callback/google",
            None,
        )
        .expect("valid test google oauth client config")
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
            phase: LifecyclePublicState::Active,
            blockers: Vec::new(),
            message: Some("activation guidance".to_string()),
            payload: Some(LifecycleProductPayload::ExtensionInstall {
                installed: true,
                visible_capability_ids: Vec::new(),
                next_step: "Extension is active.".to_string(),
                connection_required: Some(requirement),
            }),
        }
    }

    /// §5.3 S3 (behavior-neutral): the three extension-lifecycle capabilities
    /// declare an `origin_gate_matrix`. `extension_search` is read-only and thus
    /// Ungated for LoopRun (it is in the reviewed allowlist); install/remove
    /// carry write/network effects and gate for LoopRun. The direct WebUI
    /// ProductSurface path is consent-sufficient for install/remove;
    /// automation remains deny-by-default.
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
                EXTENSION_INSTALL_CAPABILITY_ID | EXTENSION_REMOVE_CAPABILITY_ID
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
            Some(LifecycleProductPayload::ExtensionInstall {
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
            phase: LifecyclePublicState::Active,
            blockers: Vec::new(),
            message: Some("activation guidance".to_string()),
            payload: Some(LifecycleProductPayload::ExtensionInstall {
                installed: true,
                visible_capability_ids: Vec::new(),
                next_step: "Extension is active.".to_string(),
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
            phase: LifecyclePublicState::Active,
            blockers: Vec::new(),
            message: None,
            payload: Some(LifecycleProductPayload::ExtensionInstall {
                installed: true,
                visible_capability_ids: vec!["github.search_issues".to_string()],
                next_step: "Extension is active.".to_string(),
                connection_required: None,
            }),
        };
        assert!(channel_connection_display_preview(&tool_activation).is_none());
    }

    #[tokio::test]
    async fn local_dev_agent_surface_exposes_extension_lifecycle_tools() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
            "extension-tools-surface-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let runtime = services.host_runtime.as_ref();

        let surface = runtime
            .visible_capabilities(visible_request(EXTENSION_LIFECYCLE_CAPABILITY_IDS))
            .await
            .expect("visible capabilities");
        let ids = surface_capability_ids(&surface);

        assert!(ids.contains(&EXTENSION_SEARCH_CAPABILITY_ID));
        assert!(ids.contains(&EXTENSION_INSTALL_CAPABILITY_ID));
        assert!(ids.contains(&EXTENSION_REMOVE_CAPABILITY_ID));
        assert!(!ids.contains(&"builtin.extension_activate"));

        let search = descriptor_for(&surface, EXTENSION_SEARCH_CAPABILITY_ID);
        assert_eq!(search.default_permission, PermissionMode::Allow);
        assert!(
            search.description.contains("not installed")
                && search.description.contains("setup is incomplete")
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
            install
                .description
                .contains("complete every internal lifecycle checkpoint")
                && install
                    .description
                    .contains("no separate user activation step")
                && install.description.contains("auth/pairing setup"),
            "extension_install description should explain automatic readiness progression: {}",
            install.description
        );
        assert_eq!(
            install.parameters_schema["required"],
            serde_json::json!(["extension_id"])
        );

        assert!(
            install.effects.contains(&EffectKind::Network),
            "install readiness reconciliation needs runtime HTTP egress for hosted MCP discovery"
        );
    }

    #[tokio::test]
    async fn local_dev_extension_lifecycle_tools_manage_visible_extension_surface() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
            "extension-tools-owner",
            storage_root.clone(),
        ))
        .await
        .expect("local-dev services build");
        let runtime = services.host_runtime.as_ref();
        let extension_management = services
            .local_runtime_for_test()
            .expect("local runtime substrate")
            .extension_management
            .clone();
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
        assert_eq!(install["phase"], "active");
        assert!(
            storage_root
                .join("system/extensions/web-access/manifest.toml")
                .exists()
        );

        assert!(
            install["message"].as_str().is_some_and(|message| message
                .contains("No additional authorization or configuration is needed")),
            "install success should override stale same-turn search onboarding, got {install}"
        );

        let after_install = active_extension_capability_ids(&extension_management).await;
        assert!(after_install.iter().any(|id| id == "web-access.search"));
        assert!(
            after_install
                .iter()
                .any(|id| id == "web-access.get_content")
        );
        let health = runtime.health().await.expect("runtime health");
        assert!(
            !health
                .missing_runtime_backends
                .contains(&RuntimeKind::FirstParty),
            "active Web Access capabilities require a registered first-party runtime"
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
        assert!(!storage_root.join("system/extensions/web-access").exists());
    }

    #[tokio::test]
    async fn local_dev_extension_remove_revokes_exclusive_credential_so_reinstall_requires_auth() {
        // Regression (#slack model-B): before the pairing->OAuth swap, removing an
        // extension cleared its credentials, so the agent could not silently
        // re-add it. OAuth personal credentials are stored `UserReusable` and are
        // preserved across extension removal by default, so without an explicit
        // provider-scoped cleanup on remove the agent re-installs the bundled
        // extension and re-activates on the surviving token — no OAuth re-consent.
        // Removing an extension whose credential provider it exclusively owns must
        // revoke that credential so re-activation raises the auth gate again.
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
            "extension-tools-remove-revoke-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");

        let context = execution_context([EXTENSION_INSTALL_CAPABILITY_ID]);
        seed_configured_account(&services, &context.resource_scope, "github").await;
        let install = invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await
        .expect("install auto-advances with a configured credential");
        assert_eq!(install["phase"], "active");

        let remove = invoke_json(
            &services,
            EXTENSION_REMOVE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await
        .expect("remove succeeds");
        assert_eq!(remove["payload"]["removed"], true);

        // Reinstall is the only public transition. The revoked credential must
        // force setup_required rather than silently re-adding the extension.
        let outcome = invoke_outcome(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
            panic!("expected reinstall after remove to require auth, got {outcome:?}");
        };
        assert_eq!(gate.credential_requirements.len(), 1);
        assert_eq!(gate.credential_requirements[0].provider.as_str(), "github");
    }

    #[tokio::test]
    async fn local_dev_extension_remove_preserves_shared_credential_used_by_another_extension() {
        // Exclusivity guard: removing one extension must NOT revoke a credential
        // still used by another installed extension. Gmail and Google Calendar
        // share the `google` provider; removing Gmail must leave the Google
        // credential intact so Calendar keeps working.
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_runtime_substrate(
            crate::deployment::local_dev_build_input(
                "extension-tools-remove-shared-owner",
                dir.path().join("local-dev"),
            )
            .with_vendor_oauth_client(
                ironclaw_auth::GOOGLE_PROVIDER_ID,
                test_google_oauth_client_config(),
            ),
        )
        .await
        .expect("local-dev services build");

        let context = execution_context([EXTENSION_INSTALL_CAPABILITY_ID]);
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
            let install = invoke_json(
                &services,
                EXTENSION_INSTALL_CAPABILITY_ID,
                serde_json::json!({ "extension_id": extension_id }),
            )
            .await
            .expect("install auto-advances with the shared google credential");
            assert_eq!(install["phase"], "active");
        }

        let remove = invoke_json(
            &services,
            EXTENSION_REMOVE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "gmail"}),
        )
        .await
        .expect("remove succeeds");
        assert_eq!(remove["payload"]["removed"], true);

        // Calendar still uses `google`, so the shared credential and its
        // published capabilities must survive Gmail removal.
        let extension_management = services
            .local_runtime_for_test()
            .expect("local runtime substrate")
            .extension_management
            .clone();
        let active = active_extension_capability_ids(&extension_management).await;
        assert!(
            active.iter().any(|id| id == "google-calendar.list_events"),
            "removing Gmail must not unpublish Calendar capabilities: {active:?}"
        );
    }

    #[tokio::test]
    async fn local_dev_extension_install_returns_auth_gate_for_missing_extension_credentials() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
            "extension-tools-auth-gate-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime_for_test()
            .expect("local runtime substrate")
            .extension_management
            .clone();

        let outcome = invoke_outcome(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
            panic!("expected extension install to request auth, got {outcome:?}");
        };
        assert_eq!(gate.capability_id.as_str(), EXTENSION_INSTALL_CAPABILITY_ID);
        assert_eq!(gate.credential_requirements.len(), 1);
        let requirement = &gate.credential_requirements[0];
        assert_eq!(requirement.provider.as_str(), "github");
        assert_eq!(requirement.requester_extension.as_str(), "github");

        let active = active_extension_capability_ids(&extension_management).await;
        assert!(!active.iter().any(|id| id == "github.search_issues"));

        // A second caller independently joins the package aggregate but gets
        // their own setup gate; Alice's credential/readiness is never reused.
        let outcome = crate::approval_test_support::invoke_with_local_dev_approval(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            execution_context_for_user(
                "extension-tool-foreign-user",
                [EXTENSION_INSTALL_CAPABILITY_ID],
            ),
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
            panic!("second caller must get an independent auth gate: {outcome:?}");
        };
        assert_eq!(gate.credential_requirements[0].provider.as_str(), "github");
    }

    #[tokio::test]
    async fn local_dev_extension_search_projects_setup_needed_then_active_after_internal_reconcile()
    {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
            "extension-tools-active-search-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime_for_test()
            .expect("local runtime substrate")
            .extension_management
            .clone();

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
            "available GitHub model-visible search results must not expose raw PAT requirements"
        );
        assert!(
            available_github.get("onboarding").is_none(),
            "available GitHub model-visible search results must not expose PAT setup onboarding"
        );

        let outcome = invoke_outcome(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::AuthRequired(_) = outcome else {
            panic!("credential-backed install must stop at setup_needed: {outcome:?}");
        };

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
            .expect("setup-needed search should carry setup guidance");
        assert!(
            installed_message.contains("setup is incomplete")
                && installed_message.contains("not currently callable tools")
                && installed_message.contains(EXTENSION_INSTALL_CAPABILITY_ID),
            "setup-needed GitHub search must not imply tools are active, got {installed_search}"
        );
        assert!(
            installed_github.get("credential_requirements").is_none(),
            "setup-needed GitHub search must not expose raw PAT requirements"
        );
        assert!(
            installed_github.get("onboarding").is_none(),
            "setup-needed GitHub search must not expose stale PAT onboarding"
        );

        let install_context = execution_context([EXTENSION_INSTALL_CAPABILITY_ID]);
        seed_configured_account(&services, &install_context.resource_scope, "github").await;

        let caller = install_context.resource_scope.user_id.clone();
        let credential_gate = crate::extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate::new(
            install_context.resource_scope.clone(),
            services
                .product_auth
                .runtime_credential_account_selection_service(),
        );
        let reconciled = extension_management
            .activate_with_credential_gate(
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
                    .expect("package ref"),
                crate::extension_host::extension_lifecycle::ExtensionActivationMode::Static,
                credential_gate,
                &caller,
            )
            .await
            .expect("internal readiness reconciliation succeeds");
        assert_eq!(reconciled.phase, LifecyclePublicState::Active);

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
    async fn local_dev_extension_install_returns_auth_gate_when_account_lacks_required_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_runtime_substrate(
            crate::deployment::local_dev_build_input(
                "extension-tools-scope-gate-owner",
                dir.path().join("local-dev"),
            )
            .with_vendor_oauth_client(
                ironclaw_auth::GOOGLE_PROVIDER_ID,
                test_google_oauth_client_config(),
            ),
        )
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime_for_test()
            .expect("local runtime substrate")
            .extension_management
            .clone();

        let install_context = execution_context([EXTENSION_INSTALL_CAPABILITY_ID]);
        seed_configured_account_with_scopes(
            &services,
            &install_context.resource_scope,
            "google",
            &["https://www.googleapis.com/auth/calendar.readonly"],
            true,
        )
        .await;

        let outcome = invoke_outcome(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
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
    async fn local_dev_extension_install_coalesces_gmail_oauth_scopes_into_one_auth_gate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_runtime_substrate(
            crate::deployment::local_dev_build_input(
                "extension-tools-gmail-scope-union-owner",
                dir.path().join("local-dev"),
            )
            .with_vendor_oauth_client(
                ironclaw_auth::GOOGLE_PROVIDER_ID,
                test_google_oauth_client_config(),
            ),
        )
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime_for_test()
            .expect("local runtime substrate")
            .extension_management
            .clone();

        let outcome = invoke_outcome(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "gmail"}),
        )
        .await;
        let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
            panic!("expected Gmail install to request auth, got {outcome:?}");
        };
        assert_eq!(gate.capability_id.as_str(), EXTENSION_INSTALL_CAPABILITY_ID);
        assert_eq!(
            gate.credential_requirements.len(),
            1,
            "Gmail install should ask for one Google OAuth gate"
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
    async fn local_dev_extension_install_maps_corrupt_configured_account_to_backend() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
            "extension-tools-corrupt-auth-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime_for_test()
            .expect("local runtime substrate")
            .extension_management
            .clone();

        let install_context = execution_context([EXTENSION_INSTALL_CAPABILITY_ID]);
        seed_configured_account_with_scopes(
            &services,
            &install_context.resource_scope,
            "github",
            &[],
            false,
        )
        .await;

        let outcome = invoke_outcome(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("expected corrupt configured account to fail, got {outcome:?}");
        };
        assert_eq!(failure.kind, RuntimeFailureKind::Backend);

        let active = active_extension_capability_ids(&extension_management).await;
        assert!(!active.iter().any(|id| id == "github.search_issues"));
    }

    /// Runtime-dispatched hosted-MCP readiness with the P2 staging fix:
    /// reconciliation stages the connection-template capability's network policy
    /// and product-auth credential under the discovery scope, so live
    /// `tools/list` runs through the REAL host egress pipeline (the scripted
    /// double sits at the network transport, under staged-policy checks and
    /// staged-credential injection) and the ceiling-validated discovered
    /// tools publish as model-visible capabilities. Before this fix nothing
    /// staged the discovery plan — the request keyed on the dispatch-minted
    /// invocation scope found no policy/credential, failed transient, and
    /// fell back to the bundled manifest with zero model-visible tools.
    #[tokio::test]
    async fn local_dev_extension_install_hosted_mcp_stages_discovery_and_publishes_tools() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let discovery_script = std::sync::Arc::new(
            crate::extension_host::extension_lifecycle::hosted_mcp_test_support::HostedMcpDiscoveryNetworkScript::with_tool_name("notion-search")
                // Real hosted MCP providers may return verbose prose. The
                // generic MCP boundary must bound it without dropping the
                // entire catalog or preventing activation.
                .with_tool_description("provider documentation ".repeat(320))
                // The manifest owns the provider-specific ceiling (Notion's
                // is 256). Discovery must not silently replace it with a
                // lower MCP-client constant.
                .with_tool_count(129),
        );
        let services = build_runtime_substrate(
            crate::deployment::local_dev_build_input(
                "extension-tools-hosted-mcp-owner",
                storage_root.clone(),
            )
            .with_network_http_egress_for_test(discovery_script.clone()),
        )
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime_for_test()
            .expect("local runtime substrate")
            .extension_management
            .clone();

        let install_context = execution_context([EXTENSION_INSTALL_CAPABILITY_ID]);
        seed_configured_account(&services, &install_context.resource_scope, "notion").await;
        // The account's access token must exist as real material: discovery
        // staging leases it from the secret store into the one-shot
        // injection store.
        let owner_scope = ironclaw_auth::AuthProductScope::credential_owner(
            &install_context.resource_scope,
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

        let install = invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "notion"}),
        )
        .await
        .expect("hosted MCP install and readiness reconciliation succeeds");
        assert_eq!(install["phase"], "active");

        // Live discovery ran through the staged pipeline: the discovered
        // tool is model-visible.
        let active = active_extension_capability_ids(&extension_management).await;
        assert!(
            active.iter().any(|id| id == "notion.notion-search-0"),
            "discovered hosted-MCP tool must be model-visible after staged discovery; got {active:?}"
        );
        assert!(
            active.iter().any(|id| id == "notion.notion-search-128"),
            "every tool within the manifest ceiling must publish; got {} tools",
            active.len()
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
        assert!(
            storage_root
                .join("system/extensions/notion/manifest.toml")
                .exists()
        );
    }

    #[tokio::test]
    async fn hosted_mcp_restart_reconciles_ready_tools_and_isolates_missing_credentials() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let owner = "extension-tool-test-user";
        let initial_script = Arc::new(
            crate::extension_host::extension_lifecycle::hosted_mcp_test_support::HostedMcpDiscoveryNetworkScript::with_tool_name(
                "notion-before-restart",
            ),
        );
        let initial = build_runtime_substrate(
            crate::deployment::local_dev_build_input(owner, storage_root.clone())
                .with_network_http_egress_for_test(initial_script),
        )
        .await
        .expect("initial services build");
        let install_context = production_local_context([EXTENSION_INSTALL_CAPABILITY_ID]);
        let install_scope = install_context.resource_scope.clone();
        seed_configured_account(&initial, &install_scope, "notion").await;
        let owner_scope = AuthProductScope::credential_owner(&install_scope, AuthSurface::Api);
        initial
            .secret_store()
            .put(
                owner_scope.resource,
                SecretHandle::new("notion-test-token").expect("secret handle"),
                ironclaw_secrets::SecretMaterial::from("notion-access-token"),
                None,
            )
            .await
            .expect("seed durable Notion token");

        let installed = crate::approval_test_support::invoke_json_with_local_dev_approval(
            &initial,
            EXTENSION_INSTALL_CAPABILITY_ID,
            install_context,
            serde_json::json!({"extension_id": "notion"}),
        )
        .await
        .expect("install credential-ready Notion");
        assert_eq!(installed["phase"], "active");
        let missing = crate::approval_test_support::invoke_with_local_dev_approval(
            &initial,
            EXTENSION_INSTALL_CAPABILITY_ID,
            production_local_context([EXTENSION_INSTALL_CAPABILITY_ID]),
            serde_json::json!({"extension_id": "nearai"}),
        )
        .await;
        assert!(
            matches!(missing, RuntimeCapabilityOutcome::AuthRequired(_)),
            "credential-missing hosted MCP install should remain setup-needed: {missing:?}"
        );
        drop(initial);

        let restart_script = Arc::new(
            crate::extension_host::extension_lifecycle::hosted_mcp_test_support::HostedMcpDiscoveryNetworkScript::with_tool_name(
                "notion-after-restart",
            ),
        );
        let restarted = build_runtime_substrate(
            crate::deployment::local_dev_build_input(owner, storage_root)
                .with_network_http_egress_for_test(restart_script.clone()),
        )
        .await
        .expect("services restart over the same durable root");
        let extension_management = restarted
            .local_runtime_for_test()
            .expect("local runtime substrate")
            .extension_management
            .clone();
        let active = active_extension_capability_ids(&extension_management).await;
        assert!(
            active.iter().any(|id| id == "notion.notion-after-restart"),
            "startup must republish the newly discovered Notion contract: {active:?}"
        );
        assert!(
            !active.iter().any(|id| id == "nearai.web_search"),
            "missing NEAR AI credentials must not publish a static hosted-MCP fallback: {active:?}"
        );

        let mut dispatch_context = production_local_context(["notion.notion-after-restart"]);
        let dispatch_grant = dispatch_context
            .grants
            .grants
            .first_mut()
            .expect("Notion capability grant");
        dispatch_grant
            .constraints
            .allowed_effects
            .push(EffectKind::UseSecret);
        dispatch_grant
            .constraints
            .secrets
            .push(SecretHandle::new("notion_access_token").expect("credential handle"));
        let outcome = crate::approval_test_support::invoke_with_local_dev_approval(
            &restarted,
            "notion.notion-after-restart",
            dispatch_context,
            serde_json::json!({"query": "restart proof"}),
        )
        .await;
        assert!(
            matches!(outcome, RuntimeCapabilityOutcome::Completed(_)),
            "the republished capability must resolve and dispatch through the generic host: {outcome:?}"
        );
        let calls = restart_script.authorized_methods();
        assert!(
            calls
                .iter()
                .any(|(method, authorized)| method == "tools/call" && *authorized),
            "generic runtime dispatch must reach the hosted MCP provider with credentials: {calls:?}"
        );
    }

    #[tokio::test]
    async fn local_dev_extension_lifecycle_tool_lists_all_and_rejects_malformed_inputs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_runtime_substrate(crate::deployment::local_dev_build_input(
            "extension-tools-invalid-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
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
            "list-all extension search should return the bundled local-dev packages"
        );
        assert_eq!(
            invoke_json(
                &services,
                EXTENSION_INSTALL_CAPABILITY_ID,
                serde_json::json!({})
            )
            .await,
            Err(RuntimeFailureKind::InvalidInput)
        );
        assert_eq!(
            invoke_json(
                &services,
                EXTENSION_INSTALL_CAPABILITY_ID,
                serde_json::json!({"extension_id": "unknown-extension"})
            )
            .await,
            Err(RuntimeFailureKind::InvalidInput)
        );
    }

    async fn invoke_json(
        services: &RebornRuntimeStores,
        capability_id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeFailureKind> {
        crate::approval_test_support::invoke_json_with_local_dev_approval(
            services,
            capability_id,
            execution_context([capability_id]),
            input,
        )
        .await
    }

    async fn invoke_outcome(
        services: &RebornRuntimeStores,
        capability_id: &str,
        input: serde_json::Value,
    ) -> RuntimeCapabilityOutcome {
        crate::approval_test_support::invoke_with_local_dev_approval(
            services,
            capability_id,
            execution_context([capability_id]),
            input,
        )
        .await
    }

    async fn seed_configured_account(
        services: &RebornRuntimeStores,
        scope: &ResourceScope,
        provider: &str,
    ) {
        seed_configured_account_with_scopes(services, scope, provider, &[], true).await;
    }

    async fn seed_configured_account_with_scopes(
        services: &RebornRuntimeStores,
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

    async fn active_extension_capability_ids(
        extension_management: &ExtensionManagementPort,
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

    fn production_local_context<'a>(
        capability_ids: impl IntoIterator<Item = &'a str>,
    ) -> ExecutionContext {
        let mut context = execution_context_for_user("extension-tool-test-user", capability_ids);
        let scope = production_local_scope(context.user_id.clone());
        context.invocation_id = scope.invocation_id;
        context.tenant_id = scope.tenant_id.clone();
        context.agent_id = scope.agent_id.clone();
        context.project_id = scope.project_id.clone();
        context.resource_scope = scope;
        context
    }

    fn production_local_scope(user_id: UserId) -> ResourceScope {
        ResourceScope {
            tenant_id: ironclaw_host_api::TenantId::new("reborn-cli")
                .expect("production local tenant"),
            user_id,
            agent_id: Some(
                ironclaw_host_api::AgentId::new("reborn-cli-agent")
                    .expect("production local agent"),
            ),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        }
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

    /// Provider-instance failures expose only a generic caller-safe message.
    /// Administrator handles and remediation stay exclusively on the
    /// authorized Admin Configuration surface.
    #[test]
    fn provider_instance_not_configured_carries_no_admin_configuration_metadata() {
        ironclaw_turns::run_profile::LoopSafeSummary::new(
            PROVIDER_INSTANCE_UNAVAILABLE_SAFE_SUMMARY,
        )
        .expect("fixed safe_summary must pass the strict LoopSafeSummary validator");

        let mapped = lifecycle_error(ProductWorkflowError::ProviderInstanceNotConfigured);

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
            Some(PROVIDER_INSTANCE_UNAVAILABLE_SAFE_SUMMARY.to_string())
        );
        let detail = format!("{:?}", detail.expect("generic detail must be present"));
        for forbidden in ["client_id", "client_secret", "config set", "restart"] {
            assert!(
                !detail.contains(forbidden),
                "leaked `{forbidden}`: {detail}"
            );
        }
    }

    /// `InvalidBindingRequest` keeps its existing `InputEncode` kind and
    /// carries its reason on the UNTRUSTED diagnostic channel. Several of its
    /// ~40 construction sites interpolate externally-influenced text (hosted
    /// MCP tool names, the model-chosen `extension_id`, uploaded-zip entry
    /// names), so the whole class must stay scanned; only reasons built
    /// entirely from host-authored constants may ride the trusted channel.
    #[test]
    fn invalid_binding_request_carries_reason_on_the_untrusted_diagnostic_channel() {
        let mapped = lifecycle_error(ProductWorkflowError::InvalidBindingRequest {
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
        let mapped = lifecycle_error(ProductWorkflowError::Transient {
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
        let mapped = lifecycle_error(ProductWorkflowError::InvalidBindingRequest {
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
