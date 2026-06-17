use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionPackage,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, HostPortId, PermissionMode,
    ResourceEstimate, ResourceProfile, ResourceUsage, RuntimeDispatchErrorKind,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use serde::Deserialize;

use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::lifecycle::RebornLocalSkillManagementPort;

use super::model::{
    IRONHUB_CAPABILITY_IDS, IRONHUB_INFO_CAPABILITY_ID, IRONHUB_INSTALL_CAPABILITY_ID,
    IRONHUB_SEARCH_CAPABILITY_ID, IronHubCommand, IronHubCommandError, IronHubEntryKind,
    IronHubInstallOptions,
};
use super::service::IronHubService;

pub(crate) fn extend_builtin_first_party_package(
    mut package: ExtensionPackage,
) -> Result<ExtensionPackage, ExtensionError> {
    package
        .manifest
        .capabilities
        .extend(capability_manifests()?);
    ExtensionPackage::from_manifest(package.manifest, package.root)
}

pub(crate) fn insert_handlers(
    registry: &mut FirstPartyCapabilityRegistry,
    skill_management: Arc<RebornLocalSkillManagementPort>,
    extension_management: Arc<RebornLocalExtensionManagementPort>,
) -> Result<(), ironclaw_host_api::HostApiError> {
    let handler = Arc::new(IronHubCapabilityHandler {
        skill_management,
        extension_management,
    });
    for capability_id in IRONHUB_CAPABILITY_IDS {
        registry.insert_handler(CapabilityId::new(capability_id)?, handler.clone());
    }
    Ok(())
}

fn capability_manifests() -> Result<Vec<CapabilityManifest>, ExtensionError> {
    Ok(vec![
        capability_manifest(
            IRONHUB_SEARCH_CAPABILITY_ID,
            "Search the signed IronHub catalog for tools and skills",
            vec![EffectKind::Network],
            PermissionMode::Allow,
        )?,
        capability_manifest(
            IRONHUB_INFO_CAPABILITY_ID,
            "Inspect one signed IronHub catalog entry",
            vec![EffectKind::Network],
            PermissionMode::Allow,
        )?,
        capability_manifest(
            IRONHUB_INSTALL_CAPABILITY_ID,
            "Install a tool or skill from the signed IronHub catalog into Reborn local-dev state",
            vec![EffectKind::Network, EffectKind::WriteFilesystem],
            PermissionMode::Ask,
        )?,
    ])
}

fn capability_manifest(
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
        required_host_ports: vec![HostPortId::new("host.runtime.http_egress")?],
        runtime_credentials: Vec::new(),
        resource_profile: Some(ResourceProfile {
            default_estimate: ResourceEstimate {
                wall_clock_ms: Some(1_000),
                output_bytes: Some(32 * 1024),
                ..ResourceEstimate::default()
            },
            hard_ceiling: None,
        }),
    })
}

struct IronHubCapabilityHandler {
    skill_management: Arc<RebornLocalSkillManagementPort>,
    extension_management: Arc<RebornLocalExtensionManagementPort>,
}

#[derive(Debug, Deserialize)]
struct SearchInput {
    #[serde(default)]
    query: String,
}

#[derive(Debug, Deserialize)]
struct InfoInput {
    name: String,
    #[serde(default)]
    kind: Option<IronHubEntryKind>,
}

#[derive(Debug, Deserialize)]
struct InstallInput {
    name: String,
    #[serde(default)]
    kind: Option<IronHubEntryKind>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    expected_version: Option<String>,
    #[serde(default)]
    expected_artifact_digest: Option<String>,
}

#[async_trait]
impl FirstPartyCapabilityHandler for IronHubCapabilityHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let Some(runtime_http_egress) = request.services.runtime_http_egress.clone() else {
            return Err(FirstPartyCapabilityError::new(
                RuntimeDispatchErrorKind::Executor,
            ));
        };
        let capability_id = request.capability_id.clone();
        let service = IronHubService::new_with_runtime_egress(
            Arc::clone(&self.skill_management),
            Arc::clone(&self.extension_management),
            runtime_http_egress,
            capability_id.clone(),
            request.scope,
        );
        let command = match capability_id.as_str() {
            IRONHUB_SEARCH_CAPABILITY_ID => {
                let input: SearchInput = parse_capability_input(request.input)?;
                IronHubCommand::Search { query: input.query }
            }
            IRONHUB_INFO_CAPABILITY_ID => {
                let input: InfoInput = parse_capability_input(request.input)?;
                IronHubCommand::Info {
                    name: input.name,
                    kind: input.kind,
                }
            }
            IRONHUB_INSTALL_CAPABILITY_ID => {
                let input: InstallInput = parse_capability_input(request.input)?;
                IronHubCommand::Install {
                    name: input.name,
                    options: IronHubInstallOptions {
                        kind: input.kind,
                        force: input.force,
                        acknowledge_unverified: false,
                        expected_version: input.expected_version,
                        expected_artifact_digest: input.expected_artifact_digest,
                    },
                }
            }
            _ => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::UndeclaredCapability,
                ));
            }
        };
        let response = service.execute(command).await.map_err(capability_error)?;
        let output = serde_json::to_value(response)
            .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputDecode))?;
        Ok(FirstPartyCapabilityResult::new(
            output,
            ResourceUsage {
                wall_clock_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                ..ResourceUsage::default()
            },
        ))
    }
}

fn parse_capability_input<T>(input: serde_json::Value) -> Result<T, FirstPartyCapabilityError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(input)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))
}

fn capability_error(error: IronHubCommandError) -> FirstPartyCapabilityError {
    let kind = match error {
        IronHubCommandError::InvalidInput { .. } => RuntimeDispatchErrorKind::InputEncode,
        IronHubCommandError::LocalRuntimeUnavailable
        | IronHubCommandError::RuntimeHttpEgressUnavailable => RuntimeDispatchErrorKind::Executor,
        IronHubCommandError::Catalog { .. }
        | IronHubCommandError::Install { .. }
        | IronHubCommandError::Product(_) => RuntimeDispatchErrorKind::OperationFailed,
    };
    FirstPartyCapabilityError::new(kind)
}
