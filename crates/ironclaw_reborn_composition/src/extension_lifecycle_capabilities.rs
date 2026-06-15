use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, CredentialStageError, EffectKind, HostApiError,
    PermissionMode, ResourceEstimate, ResourceProfile, ResourceUsage, RuntimeDispatchErrorKind,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef, ProductWorkflowError};
use serde::Deserialize;

use crate::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use crate::extension_lifecycle::{ExtensionActivationMode, RebornLocalExtensionManagementPort};
use crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionService;

pub(crate) const EXTENSION_SEARCH_CAPABILITY_ID: &str = "builtin.extension_search";
pub(crate) const EXTENSION_INSTALL_CAPABILITY_ID: &str = "builtin.extension_install";
pub(crate) const EXTENSION_ACTIVATE_CAPABILITY_ID: &str = "builtin.extension_activate";
pub(crate) const EXTENSION_REMOVE_CAPABILITY_ID: &str = "builtin.extension_remove";

pub(crate) const EXTENSION_LIFECYCLE_CAPABILITY_IDS: [&str; 4] = [
    EXTENSION_SEARCH_CAPABILITY_ID,
    EXTENSION_INSTALL_CAPABILITY_ID,
    EXTENSION_ACTIVATE_CAPABILITY_ID,
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
    extension_management: Arc<RebornLocalExtensionManagementPort>,
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
            "Search the local Reborn extension catalog by extension, product, provider, or service name. The catalog includes host-bundled extensions that are not installed yet and installed extensions that are inactive. For connect, enable, install, or integrate requests, use this for discovery only, then continue with builtin.extension_install for the matching extension instead of asking the user to configure credentials from search results.",
            vec![EffectKind::ReadFilesystem],
            PermissionMode::Allow,
        )?,
        lifecycle_manifest(
            EXTENSION_INSTALL_CAPABILITY_ID,
            "Install a searched Reborn extension into durable local-dev lifecycle state. Installation does not require credentials. After install succeeds, immediately call builtin.extension_activate for the same extension so activation can publish tools or raise the auth gate. If install fails because the extension is already installed, use builtin.extension_activate instead.",
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            PermissionMode::Ask,
        )?,
        lifecycle_manifest(
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            "Activate an installed Reborn extension for the model-visible local-dev capability surface. Use after install succeeds or when install reports the extension is already installed. This is the step that opens the credential/auth gate when required; do not ask the user for credentials before calling it.",
            vec![
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
            ],
            PermissionMode::Ask,
        )?,
        lifecycle_manifest(
            EXTENSION_REMOVE_CAPABILITY_ID,
            "Remove an installed Reborn extension from durable local-dev lifecycle state",
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
        output_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/builtin/{schema_name}.output.v1.json"
        ))?,
        prompt_doc_ref: None,
        required_host_ports: Vec::new(),
        runtime_credentials: Vec::new(),
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate {
                wall_clock_ms: Some(100),
                output_bytes: Some(16 * 1024),
                ..ResourceEstimate::default()
            },
            hard_ceiling: None,
        }),
    })
}

struct ExtensionLifecycleToolHandler {
    extension_management: Arc<RebornLocalExtensionManagementPort>,
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
                self.extension_management.search(&input.query).await
            }
            EXTENSION_INSTALL_CAPABILITY_ID => {
                let input: ExtensionIdInput = parse_input(request.input)?;
                self.extension_management
                    .install(extension_package_ref(input.extension_id)?)
                    .await
            }
            EXTENSION_ACTIVATE_CAPABILITY_ID => {
                let input: ExtensionIdInput = parse_input(request.input)?;
                let package_ref = extension_package_ref(input.extension_id)?;
                let requirements = self
                    .extension_management
                    .activation_credential_requirements(&package_ref)
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
                    .activate_with_credential_gate(package_ref, mode, credential_gate)
                    .await
            }
            EXTENSION_REMOVE_CAPABILITY_ID => {
                let input: ExtensionIdInput = parse_input(request.input)?;
                self.extension_management
                    .remove(extension_package_ref(input.extension_id)?)
                    .await
            }
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        }
        .map_err(lifecycle_error)?;

        let output = serde_json::to_value(response)
            .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputDecode))?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            resource_usage(started),
        ))
    }
}

fn resource_usage(started: Instant) -> ResourceUsage {
    ResourceUsage {
        wall_clock_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        ..ResourceUsage::default()
    }
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
    LifecyclePackageRef::new(LifecyclePackageKind::Extension, id)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))
}

fn lifecycle_error(error: ProductWorkflowError) -> FirstPartyCapabilityError {
    let kind = match error {
        ProductWorkflowError::InvalidBindingRequest { .. }
        | ProductWorkflowError::UnsupportedActionKind { .. } => {
            RuntimeDispatchErrorKind::InputEncode
        }
        ProductWorkflowError::Transient { .. } => RuntimeDispatchErrorKind::OperationFailed,
        _ => RuntimeDispatchErrorKind::OperationFailed,
    };
    FirstPartyCapabilityError::new(kind)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use ironclaw_auth::{
        AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel,
        CredentialAccountStatus, CredentialOwnership, NewCredentialAccount, ProviderScope,
    };
    use ironclaw_host_api::{
        CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, CapabilitySet, ExecutionContext,
        ExtensionId, GrantConstraints, MountView, NetworkPolicy, NetworkTargetPattern,
        PermissionMode, Principal, ResourceScope, RuntimeKind, SecretHandle, TrustClass, UserId,
    };
    use ironclaw_host_runtime::{
        CapabilitySurfacePolicy, RuntimeCapabilityOutcome, RuntimeFailureKind, SurfaceKind,
        VisibleCapabilityRequest, VisibleCapabilitySurface,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

    use crate::product_auth_runtime_credentials::runtime_account_owner_scope;

    use super::*;
    use crate::{RebornBuildInput, RebornServices, build_reborn_services};

    #[tokio::test]
    async fn local_dev_agent_surface_exposes_extension_lifecycle_tools() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "extension-tools-surface-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let runtime = services
            .host_runtime
            .as_ref()
            .expect("host runtime composed");

        let surface = runtime
            .visible_capabilities(visible_request(EXTENSION_LIFECYCLE_CAPABILITY_IDS))
            .await
            .expect("visible capabilities");
        let ids = surface_capability_ids(&surface);

        assert!(ids.contains(&EXTENSION_SEARCH_CAPABILITY_ID));
        assert!(ids.contains(&EXTENSION_INSTALL_CAPABILITY_ID));
        assert!(ids.contains(&EXTENSION_ACTIVATE_CAPABILITY_ID));
        assert!(ids.contains(&EXTENSION_REMOVE_CAPABILITY_ID));

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
                && install
                    .description
                    .contains(EXTENSION_ACTIVATE_CAPABILITY_ID)
                && install.description.contains("does not require credentials")
                && install.description.contains("immediately call"),
            "extension_install description should route successful installs and already-installed failures to activation: {}",
            install.description
        );
        assert_eq!(
            install.parameters_schema["required"],
            serde_json::json!(["extension_id"])
        );

        let activate = descriptor_for(&surface, EXTENSION_ACTIVATE_CAPABILITY_ID);
        assert!(
            activate.description.contains("credential/auth gate")
                && activate.description.contains("do not ask the user"),
            "extension_activate description should teach the model to raise auth through activation: {}",
            activate.description
        );

        assert!(
            activate.effects.contains(&EffectKind::Network),
            "hosted MCP activation needs runtime HTTP egress for discovery"
        );
    }

    #[tokio::test]
    async fn local_dev_extension_lifecycle_tools_manage_visible_extension_surface() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "extension-tools-owner",
            storage_root.clone(),
        ))
        .await
        .expect("local-dev services build");
        let runtime = services
            .host_runtime
            .as_ref()
            .expect("host runtime composed");
        let extension_management = services
            .local_runtime
            .as_ref()
            .expect("local runtime substrate")
            .extension_management
            .as_ref()
            .expect("extension management")
            .clone();
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
        assert!(
            storage_root
                .join("system/extensions/web-access/manifest.toml")
                .exists()
        );

        let before_activate = active_extension_capability_ids(&extension_management).await;
        assert!(!before_activate.iter().any(|id| id == "web-access.search"));

        let activate = invoke_json(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "web-access"}),
        )
        .await
        .expect("activate succeeds");
        assert_eq!(activate["payload"]["activated"], true);

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
        assert!(!storage_root.join("system/extensions/web-access").exists());
    }

    #[tokio::test]
    async fn local_dev_extension_activate_returns_auth_gate_for_missing_extension_credentials() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "extension-tools-auth-gate-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime
            .as_ref()
            .expect("local runtime substrate")
            .extension_management
            .as_ref()
            .expect("extension management")
            .clone();

        invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await
        .expect("install succeeds");

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
    }

    #[tokio::test]
    async fn local_dev_extension_activate_returns_auth_gate_when_account_lacks_required_scope() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "extension-tools-scope-gate-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime
            .as_ref()
            .expect("local runtime substrate")
            .extension_management
            .as_ref()
            .expect("extension management")
            .clone();

        invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "google-calendar"}),
        )
        .await
        .expect("install succeeds");
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
    async fn local_dev_extension_activate_coalesces_gmail_oauth_scopes_into_one_auth_gate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "extension-tools-gmail-scope-union-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime
            .as_ref()
            .expect("local runtime substrate")
            .extension_management
            .as_ref()
            .expect("extension management")
            .clone();

        invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "gmail"}),
        )
        .await
        .expect("install succeeds");

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
    async fn local_dev_extension_activate_maps_corrupt_configured_account_to_backend() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "extension-tools-corrupt-auth-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime
            .as_ref()
            .expect("local runtime substrate")
            .extension_management
            .as_ref()
            .expect("extension management")
            .clone();

        invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await
        .expect("install succeeds");
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
        assert_eq!(failure.kind, RuntimeFailureKind::Backend);

        let active = active_extension_capability_ids(&extension_management).await;
        assert!(!active.iter().any(|id| id == "github.search_issues"));
    }

    #[tokio::test]
    async fn local_dev_extension_activate_routes_hosted_mcp_discovery_through_runtime_egress() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "extension-tools-hosted-mcp-owner",
            storage_root.clone(),
        ))
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime
            .as_ref()
            .expect("local runtime substrate")
            .extension_management
            .as_ref()
            .expect("extension management")
            .clone();

        invoke_json(
            &services,
            EXTENSION_INSTALL_CAPABILITY_ID,
            serde_json::json!({"extension_id": "notion"}),
        )
        .await
        .expect("install succeeds");
        let activate_context = execution_context([EXTENSION_ACTIVATE_CAPABILITY_ID]);
        seed_configured_account(&services, &activate_context.resource_scope, "notion").await;

        let activate = invoke_json(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "notion"}),
        )
        .await
        .expect("hosted MCP activation succeeds");
        assert_eq!(activate["payload"]["activated"], true);

        let active = active_extension_capability_ids(&extension_management).await;
        assert!(active.iter().any(|id| id == "notion.notion-get-self"));
        assert!(
            storage_root
                .join("system/extensions/notion/manifest.toml")
                .exists()
        );
    }

    #[tokio::test]
    async fn local_dev_extension_lifecycle_tool_lists_all_and_rejects_malformed_inputs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
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
        let outcome = invoke_outcome(
            &services,
            EXTENSION_ACTIVATE_CAPABILITY_ID,
            serde_json::json!({"extension_id": "github"}),
        )
        .await;
        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("expected uninstalled extension activation to fail, got {outcome:?}");
        };
        assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
    }

    async fn invoke_json(
        services: &RebornServices,
        capability_id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeFailureKind> {
        crate::approval_test_support::invoke_json_with_local_dev_approval(
            services,
            capability_id,
            execution_context([capability_id]),
            input,
            trust_decision(),
        )
        .await
    }

    async fn invoke_outcome(
        services: &RebornServices,
        capability_id: &str,
        input: serde_json::Value,
    ) -> RuntimeCapabilityOutcome {
        crate::approval_test_support::invoke_with_local_dev_approval(
            services,
            capability_id,
            execution_context([capability_id]),
            input,
            trust_decision(),
        )
        .await
    }

    async fn seed_configured_account(
        services: &RebornServices,
        scope: &ResourceScope,
        provider: &str,
    ) {
        seed_configured_account_with_scopes(services, scope, provider, &[], true).await;
    }

    async fn seed_configured_account_with_scopes(
        services: &RebornServices,
        scope: &ResourceScope,
        provider: &str,
        scopes: &[&str],
        include_access_secret: bool,
    ) {
        services
            .product_auth
            .as_ref()
            .expect("product auth")
            .credential_account_service()
            .create_account(NewCredentialAccount {
                scope: AuthProductScope::new(runtime_account_owner_scope(scope), AuthSurface::Api),
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
        let caller = ExtensionId::new("extension-tool-test-caller").expect("valid extension id");
        ExecutionContext::local_default(
            UserId::new("extension-tool-test-user").expect("valid user id"),
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
        .expect("valid execution context")
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
}
