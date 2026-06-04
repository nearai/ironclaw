use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, VerifyingKey};
use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionManifest, ExtensionPackage,
    ManifestSource,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, HostPortId, InvocationId, NetworkMethod,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, PermissionMode, ResourceEstimate,
    ResourceProfile, ResourceScope, ResourceUsage, RuntimeDispatchErrorKind, RuntimeHttpEgress,
    RuntimeHttpEgressRequest, RuntimeKind, UserId, VirtualPath, sha256_digest_token,
};
use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult,
};
use ironclaw_product_workflow::{
    LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    LifecycleProductPayload, LifecycleProductResponse, ProductWorkflowError,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;

use crate::available_extensions::{
    AvailableExtensionAsset, AvailableExtensionAssetContent, AvailableExtensionPackage,
};
use crate::extension_lifecycle::RebornLocalExtensionManagementPort;
use crate::factory::RebornServices;
use crate::lifecycle::{RebornLocalSkillManagementPort, response_with_payload};

pub(crate) const DEFAULT_IRONHUB_MANIFEST_URL: &str =
    "https://hub.ironclaw.com/api/catalog/manifest.json";

const MANIFEST_VERIFY_KEYS: &[(&str, &str)] = &[(
    "5895a21abea89672",
    "f64d2d3a3228b16ca59450364d26b278071a1a425544f242504033341d8459bd",
)];
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_SIGNED_MANIFEST_BYTES: u64 = MAX_MANIFEST_BYTES * 2;
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_WASM_BYTES: u64 = 16 * 1024 * 1024;
const MANIFEST_CACHE_TTL: Duration = Duration::from_secs(60);
const MANIFEST_CACHE_MAX_ENTRIES: usize = 64;
const GENERIC_TOOL_INPUT_SCHEMA: &[u8] = br#"{"type":"object","additionalProperties":true}"#;
const GENERIC_TOOL_OUTPUT_SCHEMA: &[u8] =
    br#"{"description":"Raw JSON output from the installed IronHub tool"}"#;
pub(crate) const IRONHUB_SEARCH_CAPABILITY_ID: &str = "builtin.ironhub_search";
pub(crate) const IRONHUB_INFO_CAPABILITY_ID: &str = "builtin.ironhub_info";
pub(crate) const IRONHUB_INSTALL_CAPABILITY_ID: &str = "builtin.ironhub_install";
const IRONHUB_CAPABILITY_IDS: [&str; 3] = [
    IRONHUB_SEARCH_CAPABILITY_ID,
    IRONHUB_INFO_CAPABILITY_ID,
    IRONHUB_INSTALL_CAPABILITY_ID,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronHubEntryKind {
    Tool,
    Skill,
}

impl IronHubEntryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Skill => "skill",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IronHubProvenance {
    #[serde(alias = "repo")]
    Official,
    Trusted,
    Verified,
    #[default]
    #[serde(alias = "community")]
    New,
}

impl IronHubProvenance {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Trusted => "trusted",
            Self::Verified => "verified",
            Self::New => "new",
        }
    }

    pub fn is_community_unverified(self) -> bool {
        matches!(self, Self::New)
    }

    pub fn trust_label(self) -> &'static str {
        match self {
            Self::Official => "NEAR-vetted (official)",
            Self::Trusted => "community, trusted publisher",
            Self::Verified => "community, verified publisher",
            Self::New => "UNVERIFIED community (new author)",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IronHubManifest {
    pub version: String,
    pub generated_at: String,
    pub release_tag: String,
    pub repo: String,
    #[serde(default)]
    pub tools: Vec<IronHubToolEntry>,
    #[serde(default)]
    pub skills: Vec<IronHubSkillEntry>,
}

impl IronHubManifest {
    fn find_tool(&self, name: &str) -> Option<&IronHubToolEntry> {
        self.tools.iter().find(|entry| entry.name == name)
    }

    fn find_skill(&self, name: &str) -> Option<&IronHubSkillEntry> {
        self.skills.iter().find(|entry| entry.name == name)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IronHubToolEntry {
    pub name: String,
    pub crate_name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub provenance: IronHubProvenance,
    pub wasm: IronHubArtifact,
    pub capabilities: IronHubArtifact,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IronHubSkillEntry {
    pub name: String,
    #[serde(default)]
    pub trunk: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub provenance: IronHubProvenance,
    pub skill_md: IronHubArtifact,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct IronHubArtifact {
    pub url: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IronHubInstallOptions {
    pub kind: Option<IronHubEntryKind>,
    pub force: bool,
    pub acknowledge_unverified: bool,
    pub expected_version: Option<String>,
    pub expected_artifact_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IronHubInstallOutcome {
    pub kind: IronHubEntryKind,
    pub name: String,
    pub version: String,
    pub release_tag: String,
    pub provenance: IronHubProvenance,
    pub artifact_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IronHubCommand {
    Search {
        query: String,
    },
    List {
        kind: Option<IronHubEntryKind>,
    },
    Info {
        name: String,
    },
    Install {
        name: String,
        options: IronHubInstallOptions,
    },
}

#[derive(Debug, Error)]
pub enum IronHubCommandError {
    #[error("IronHub is available only for local-dev Reborn services")]
    LocalRuntimeUnavailable,
    #[error("IronHub runtime HTTP egress is unavailable")]
    RuntimeHttpEgressUnavailable,
    #[error("invalid IronHub input: {reason}")]
    InvalidInput { reason: String },
    #[error("IronHub catalog failed: {reason}")]
    Catalog { reason: String },
    #[error("IronHub install failed: {reason}")]
    Install { reason: String },
    #[error("IronHub lifecycle failed: {0}")]
    Product(#[from] ProductWorkflowError),
}

#[derive(Debug, Deserialize)]
struct SignedManifestEnvelope {
    v: u8,
    key_id: String,
    manifest_b64: String,
    sig: String,
}

struct CachedManifest {
    manifest: Arc<IronHubManifest>,
    fetched_at: Instant,
}

static MANIFEST_CACHE: LazyLock<std::sync::Mutex<HashMap<String, CachedManifest>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
static INSTALL_LOCKS: LazyLock<std::sync::Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> =
    LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

pub async fn execute_reborn_ironhub_command(
    services: &RebornServices,
    command: IronHubCommand,
) -> Result<LifecycleProductResponse, IronHubCommandError> {
    let local_runtime = services
        .local_runtime
        .as_ref()
        .ok_or(IronHubCommandError::LocalRuntimeUnavailable)?;
    let extension_management = local_runtime
        .extension_management
        .as_ref()
        .ok_or(IronHubCommandError::LocalRuntimeUnavailable)?;
    let runtime_http_egress = local_runtime
        .runtime_http_egress
        .as_ref()
        .ok_or(IronHubCommandError::RuntimeHttpEgressUnavailable)?;
    let scope = ResourceScope::local_default(
        UserId::new("reborn-cli").map_err(invalid_input)?,
        InvocationId::new(),
    )
    .map_err(invalid_input)?;
    let service = IronHubService::new(
        Arc::clone(&local_runtime.skill_management),
        Arc::clone(extension_management),
        Arc::clone(runtime_http_egress),
        scope,
    );
    service.execute(command).await
}

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
}

#[derive(Debug, Deserialize)]
struct InstallInput {
    name: String,
    #[serde(default)]
    kind: Option<IronHubEntryKind>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    acknowledge_unverified: bool,
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
        let service = IronHubService::new(
            Arc::clone(&self.skill_management),
            Arc::clone(&self.extension_management),
            runtime_http_egress,
            request.scope,
        );
        let command = match request.capability_id.as_str() {
            IRONHUB_SEARCH_CAPABILITY_ID => {
                let input: SearchInput = parse_capability_input(request.input)?;
                IronHubCommand::Search { query: input.query }
            }
            IRONHUB_INFO_CAPABILITY_ID => {
                let input: InfoInput = parse_capability_input(request.input)?;
                IronHubCommand::Info { name: input.name }
            }
            IRONHUB_INSTALL_CAPABILITY_ID => {
                let input: InstallInput = parse_capability_input(request.input)?;
                IronHubCommand::Install {
                    name: input.name,
                    options: IronHubInstallOptions {
                        kind: input.kind,
                        force: input.force,
                        acknowledge_unverified: input.acknowledge_unverified,
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

pub fn render_reborn_ironhub_response(label: &str, response: &LifecycleProductResponse) -> String {
    let mut output = String::new();
    push_line(&mut output, format_args!("IronHub {label}"));
    push_line(
        &mut output,
        format_args!("phase: {}", phase_label(response.phase)),
    );
    if let Some(package_ref) = &response.package_ref {
        push_line(
            &mut output,
            format_args!(
                "package: {}/{}",
                package_kind_label(package_ref.kind),
                package_ref.id.as_str()
            ),
        );
    }
    if let Some(message) = &response.message {
        push_line(
            &mut output,
            format_args!("message: {}", terminal_safe(message)),
        );
    }
    match response.payload.as_ref() {
        Some(LifecycleProductPayload::ExtensionSearch { extensions, count }) => {
            push_line(&mut output, format_args!("count: {count}"));
            for extension in extensions {
                push_line(
                    &mut output,
                    format_args!(
                        "- tool {} {} ({})",
                        extension.package_ref.id.as_str(),
                        terminal_safe(&extension.version),
                        terminal_safe(&extension.description)
                    ),
                );
            }
        }
        Some(LifecycleProductPayload::CatalogSearch {
            tools,
            skills,
            count,
        }) => {
            push_line(&mut output, format_args!("count: {count}"));
            for tool in tools {
                push_line(
                    &mut output,
                    format_args!(
                        "- tool {} {} ({})",
                        tool.package_ref.id.as_str(),
                        terminal_safe(&tool.version),
                        terminal_safe(&tool.description)
                    ),
                );
            }
            for skill in skills {
                push_line(
                    &mut output,
                    format_args!(
                        "- skill {} {} ({})",
                        skill.name.as_str(),
                        terminal_safe(&skill.version),
                        terminal_safe(&skill.description)
                    ),
                );
            }
        }
        Some(LifecycleProductPayload::SkillSearch {
            skills,
            count,
            truncated,
            ..
        }) => {
            push_line(&mut output, format_args!("count: {count}"));
            push_line(&mut output, format_args!("truncated: {truncated}"));
            for skill in skills {
                push_line(
                    &mut output,
                    format_args!(
                        "- skill {} {} ({})",
                        skill.name.as_str(),
                        terminal_safe(&skill.version),
                        terminal_safe(&skill.description)
                    ),
                );
            }
        }
        Some(LifecycleProductPayload::ExtensionInstall {
            installed,
            visible_capability_ids,
        }) => {
            push_line(&mut output, format_args!("installed: {installed}"));
            for id in visible_capability_ids {
                push_line(
                    &mut output,
                    format_args!("visible_capability: {}", terminal_safe(id)),
                );
            }
        }
        Some(LifecycleProductPayload::SkillInstall { installed, name }) => {
            push_line(&mut output, format_args!("installed: {installed}"));
            push_line(&mut output, format_args!("skill: {}", name.as_str()));
        }
        _ => {}
    }
    output
}

pub(crate) struct IronHubService {
    skill_management: Arc<RebornLocalSkillManagementPort>,
    extension_management: Arc<RebornLocalExtensionManagementPort>,
    runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    scope: ResourceScope,
    manifest_url: String,
}

impl IronHubService {
    pub(crate) fn new(
        skill_management: Arc<RebornLocalSkillManagementPort>,
        extension_management: Arc<RebornLocalExtensionManagementPort>,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
        scope: ResourceScope,
    ) -> Self {
        Self {
            skill_management,
            extension_management,
            runtime_http_egress,
            scope,
            manifest_url: resolve_manifest_url(),
        }
    }

    pub(crate) async fn execute(
        &self,
        command: IronHubCommand,
    ) -> Result<LifecycleProductResponse, IronHubCommandError> {
        match command {
            IronHubCommand::Search { query } => self.search(&query).await,
            IronHubCommand::List { kind } => self.list(kind).await,
            IronHubCommand::Info { name } => self.info(&name).await,
            IronHubCommand::Install { name, options } => self.install(&name, options).await,
        }
    }

    async fn search(&self, query: &str) -> Result<LifecycleProductResponse, IronHubCommandError> {
        let manifest = self.fetch_manifest_cached().await?;
        let query = query.trim().to_ascii_lowercase();
        let tools = manifest
            .tools
            .iter()
            .filter(|entry| entry_matches(&entry.name, &entry.description, &query))
            .map(tool_summary)
            .collect::<Result<Vec<_>, _>>()?;
        let skills = manifest
            .skills
            .iter()
            .filter(|entry| entry_matches(&entry.name, &entry.description, &query))
            .map(skill_summary)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(response_with_payload(
            None,
            LifecyclePhase::Discovered,
            LifecycleProductPayload::CatalogSearch {
                count: tools.len() + skills.len(),
                tools,
                skills,
            },
        ))
    }

    async fn list(
        &self,
        kind: Option<IronHubEntryKind>,
    ) -> Result<LifecycleProductResponse, IronHubCommandError> {
        let manifest = self.fetch_manifest_cached().await?;
        match kind {
            Some(IronHubEntryKind::Skill) => {
                let skills = manifest
                    .skills
                    .iter()
                    .map(skill_summary)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(response_with_payload(
                    None,
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::SkillSearch {
                        count: skills.len(),
                        limit: skills.len(),
                        truncated: false,
                        skills,
                    },
                ))
            }
            None => {
                let tools = manifest
                    .tools
                    .iter()
                    .map(tool_summary)
                    .collect::<Result<Vec<_>, _>>()?;
                let skills = manifest
                    .skills
                    .iter()
                    .map(skill_summary)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(response_with_payload(
                    None,
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::CatalogSearch {
                        count: tools.len() + skills.len(),
                        tools,
                        skills,
                    },
                ))
            }
            Some(IronHubEntryKind::Tool) => {
                let tools = manifest
                    .tools
                    .iter()
                    .map(tool_summary)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(response_with_payload(
                    None,
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::ExtensionSearch {
                        count: tools.len(),
                        extensions: tools,
                    },
                ))
            }
        }
    }

    async fn info(&self, name: &str) -> Result<LifecycleProductResponse, IronHubCommandError> {
        validate_hub_name(name)?;
        let manifest = self.fetch_manifest_cached().await?;
        let kind = classify(&manifest, name, None)?;
        let response = match kind {
            IronHubEntryKind::Tool => {
                let tool = manifest
                    .find_tool(name)
                    .ok_or_else(|| catalog_error("tool not found"))?;
                response_with_payload(
                    Some(package_ref(LifecyclePackageKind::Extension, &tool.name)?),
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::ExtensionSearch {
                        extensions: vec![tool_summary(tool)?],
                        count: 1,
                    },
                )
            }
            IronHubEntryKind::Skill => {
                let skill = manifest
                    .find_skill(name)
                    .ok_or_else(|| catalog_error("skill not found"))?;
                response_with_payload(
                    Some(package_ref(LifecyclePackageKind::Skill, &skill.name)?),
                    LifecyclePhase::Discovered,
                    LifecycleProductPayload::SkillSearch {
                        skills: vec![skill_summary(skill)?],
                        count: 1,
                        limit: 1,
                        truncated: false,
                    },
                )
            }
        };
        Ok(response)
    }

    async fn install(
        &self,
        name: &str,
        options: IronHubInstallOptions,
    ) -> Result<LifecycleProductResponse, IronHubCommandError> {
        validate_hub_name(name)?;
        let manifest = self.fetch_manifest_cached().await?;
        let (kind, provenance, artifact_digest) =
            classify_gate_and_digest(&manifest, name, options.kind, &options)?;
        let lock_key = format!("{}:{name}", kind.as_str());
        let lock = install_lock(&lock_key);
        let _guard = lock.lock().await;
        match kind {
            IronHubEntryKind::Skill => {
                let entry = manifest
                    .find_skill(name)
                    .ok_or_else(|| catalog_error("skill not found"))?;
                let content = self
                    .download_verified(&entry.skill_md, MAX_METADATA_BYTES)
                    .await?;
                let content =
                    String::from_utf8(content).map_err(|error| IronHubCommandError::Install {
                        reason: format!("skill markdown is not UTF-8: {error}"),
                    })?;
                let result = self
                    .skill_management
                    .install_from_ironhub(Some(&entry.name), &content, &entry.skill_md.url)
                    .await
                    .map_err(|error| IronHubCommandError::Install {
                        reason: error.to_string(),
                    })?;
                let mut response = response_with_payload(
                    Some(package_ref(LifecyclePackageKind::Skill, &result.name)?),
                    LifecyclePhase::Installed,
                    LifecycleProductPayload::SkillInstall {
                        installed: true,
                        name: LifecyclePackageId::new(result.name).map_err(product_error)?,
                    },
                );
                response.message = Some(install_message(
                    IronHubEntryKind::Skill,
                    name,
                    entry.version.as_str(),
                    provenance,
                    &artifact_digest,
                ));
                Ok(response)
            }
            IronHubEntryKind::Tool => {
                let entry = manifest
                    .find_tool(name)
                    .ok_or_else(|| catalog_error("tool not found"))?;
                let wasm = self.download_verified(&entry.wasm, MAX_WASM_BYTES).await?;
                let capabilities = self
                    .download_verified(&entry.capabilities, MAX_METADATA_BYTES)
                    .await?;
                let package = ironhub_tool_package(entry, &wasm, &capabilities)?;
                let mut response = self
                    .extension_management
                    .install_available_package(package, options.force)
                    .await
                    .map_err(IronHubCommandError::Product)?;
                response.message = Some(install_message(
                    IronHubEntryKind::Tool,
                    name,
                    entry.version.as_str(),
                    provenance,
                    &artifact_digest,
                ));
                Ok(response)
            }
        }
    }

    async fn fetch_manifest_cached(&self) -> Result<IronHubManifest, IronHubCommandError> {
        let now = Instant::now();
        if let Some(hit) = manifest_cache_get(&self.manifest_url, now) {
            return Ok((*hit).clone());
        }
        let manifest = Arc::new(self.fetch_manifest().await?);
        manifest_cache_put(&self.manifest_url, Arc::clone(&manifest), now);
        Ok((*manifest).clone())
    }

    async fn fetch_manifest(&self) -> Result<IronHubManifest, IronHubCommandError> {
        validate_artifact_url("hub-manifest", "manifest_url", &self.manifest_url)?;
        let envelope = self
            .download_url(&self.manifest_url, MAX_SIGNED_MANIFEST_BYTES)
            .await?;
        let bytes =
            verify_signed_manifest(&envelope).map_err(|reason| IronHubCommandError::Catalog {
                reason: format!("signed manifest verification failed: {reason}"),
            })?;
        if bytes.len() > usize::try_from(MAX_MANIFEST_BYTES).unwrap_or(usize::MAX) {
            return Err(IronHubCommandError::Catalog {
                reason: "manifest exceeds size cap".to_string(),
            });
        }
        serde_json::from_slice(&bytes).map_err(|error| IronHubCommandError::Catalog {
            reason: format!("manifest parse failed: {error}"),
        })
    }

    async fn download_verified(
        &self,
        artifact: &IronHubArtifact,
        max_bytes: u64,
    ) -> Result<Vec<u8>, IronHubCommandError> {
        validate_artifact(artifact, max_bytes)?;
        let bytes = self.download_url(&artifact.url, max_bytes).await?;
        let actual = sha256_digest_token(&bytes);
        if !actual.eq_ignore_ascii_case(&artifact.sha256) {
            return Err(IronHubCommandError::Install {
                reason: format!(
                    "checksum mismatch for {}: expected {}, got {}",
                    artifact.url, artifact.sha256, actual
                ),
            });
        }
        Ok(bytes)
    }

    async fn download_url(
        &self,
        url: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, IronHubCommandError> {
        let request = RuntimeHttpEgressRequest {
            runtime: RuntimeKind::FirstParty,
            scope: self.scope.clone(),
            capability_id: CapabilityId::new("builtin.ironhub_fetch").map_err(invalid_input)?,
            method: NetworkMethod::Get,
            url: url.to_string(),
            headers: Vec::new(),
            body: Vec::new(),
            network_policy: network_policy_for_url(url, max_bytes)?,
            credential_injections: Vec::new(),
            response_body_limit: Some(max_bytes),
            save_body_to: None,
            timeout_ms: Some(30_000),
        };
        let response = self
            .runtime_http_egress
            .execute(request)
            .await
            .map_err(|error| IronHubCommandError::Catalog {
                reason: error.stable_runtime_reason().to_string(),
            })?;
        if !(200..300).contains(&response.status) {
            return Err(IronHubCommandError::Catalog {
                reason: format!("download returned HTTP {}", response.status),
            });
        }
        if response.body.len() > usize::try_from(max_bytes).unwrap_or(usize::MAX) {
            return Err(IronHubCommandError::Catalog {
                reason: "download exceeds size cap".to_string(),
            });
        }
        Ok(response.body)
    }
}

fn ironhub_tool_package(
    entry: &IronHubToolEntry,
    wasm: &[u8],
    capabilities: &[u8],
) -> Result<AvailableExtensionPackage, IronHubCommandError> {
    validate_hub_name(&entry.name)?;
    let manifest_toml = generic_tool_manifest(entry);
    let root = VirtualPath::new(format!("/system/extensions/{}", entry.name))
        .map_err(|error| install_error(error.to_string()))?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog()
        .map_err(|error| install_error(error.to_string()))?;
    let contracts = ironclaw_host_runtime::default_host_api_contract_registry()
        .map_err(|error| install_error(error.to_string()))?;
    let manifest = ExtensionManifest::parse_with_optional_host_api_contracts(
        &manifest_toml,
        ManifestSource::RegistryInstalled,
        &host_ports,
        &contracts,
    )
    .map_err(|error| install_error(error.to_string()))?;
    let package = ExtensionPackage::from_manifest_toml(manifest, root, &manifest_toml)
        .map_err(|error| install_error(error.to_string()))?;
    let package_ref = package_ref(LifecyclePackageKind::Extension, &entry.name)?;
    Ok(AvailableExtensionPackage {
        package_ref,
        manifest_toml,
        package,
        assets: vec![
            bytes_asset("manifest.toml", manifest_toml_bytes(entry).as_slice()),
            bytes_asset(&format!("wasm/{}_tool.wasm", entry.name), wasm),
            bytes_asset("legacy/capabilities.json", capabilities),
            bytes_asset(
                &format!("schemas/{}/invoke.input.v1.json", entry.name),
                GENERIC_TOOL_INPUT_SCHEMA,
            ),
            bytes_asset(
                &format!("schemas/{}/raw_output.v1.json", entry.name),
                GENERIC_TOOL_OUTPUT_SCHEMA,
            ),
        ],
    })
}

fn manifest_toml_bytes(entry: &IronHubToolEntry) -> Vec<u8> {
    generic_tool_manifest(entry).into_bytes()
}

fn generic_tool_manifest(entry: &IronHubToolEntry) -> String {
    format!(
        r#"schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{name}"
version = "{version}"
description = "{description}"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}_tool.wasm"

[[capabilities]]
id = "{id}.invoke"
description = "{description}"
effects = ["dispatch_capability", "network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/{id}/invoke.input.v1.json"
output_schema_ref = "schemas/{id}/raw_output.v1.json"
required_host_ports = ["host.runtime.http_egress"]
"#,
        id = toml_escape(&entry.name),
        name = toml_escape(&entry.name),
        version = toml_escape(&entry.version),
        description = toml_escape(&entry.description),
    )
}

fn bytes_asset(path: &str, bytes: &[u8]) -> AvailableExtensionAsset {
    AvailableExtensionAsset {
        path: path.to_string(),
        content: AvailableExtensionAssetContent::Bytes(bytes.to_vec()),
    }
}

fn verify_signed_manifest(envelope_bytes: &[u8]) -> Result<Vec<u8>, String> {
    verify_signed_manifest_with_keys(envelope_bytes, MANIFEST_VERIFY_KEYS)
}

fn verify_signed_manifest_with_keys(
    envelope_bytes: &[u8],
    verify_keys: &[(&str, &str)],
) -> Result<Vec<u8>, String> {
    let env: SignedManifestEnvelope = serde_json::from_slice(envelope_bytes)
        .map_err(|error| format!("envelope parse failed: {error}"))?;
    if env.v != 1 {
        return Err(format!("unsupported signed-manifest version {}", env.v));
    }
    let key_hex = verify_keys
        .iter()
        .find(|(id, _)| *id == env.key_id)
        .map(|(_, key)| *key)
        .ok_or_else(|| format!("unknown manifest signing key_id '{}'", env.key_id))?;
    let verifying_key = verifying_key_from_hex(key_hex)?;
    let manifest_bytes = URL_SAFE_NO_PAD
        .decode(env.manifest_b64.as_bytes())
        .map_err(|error| format!("manifest_b64 decode failed: {error}"))?;
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(env.sig.as_bytes())
        .map_err(|error| format!("signature decode failed: {error}"))?;
    let signature = Signature::from_slice(&sig_bytes)
        .map_err(|error| format!("signature malformed: {error}"))?;
    verifying_key
        .verify_strict(&manifest_bytes, &signature)
        .map_err(|_| "manifest signature verification failed".to_string())?;
    Ok(manifest_bytes)
}

fn verifying_key_from_hex(hex: &str) -> Result<VerifyingKey, String> {
    let raw = hex::decode(hex).map_err(|error| format!("verify key is not valid hex: {error}"))?;
    let raw: [u8; 32] = raw
        .try_into()
        .map_err(|_| "verify key must be 32 bytes".to_string())?;
    VerifyingKey::from_bytes(&raw).map_err(|error| format!("invalid verify key: {error}"))
}

fn classify_gate_and_digest(
    manifest: &IronHubManifest,
    name: &str,
    hint: Option<IronHubEntryKind>,
    options: &IronHubInstallOptions,
) -> Result<(IronHubEntryKind, IronHubProvenance, String), IronHubCommandError> {
    let kind = classify(manifest, name, hint)?;
    let (version, provenance, artifact_digest) = match kind {
        IronHubEntryKind::Tool => {
            let entry = manifest
                .find_tool(name)
                .ok_or_else(|| catalog_error("tool not found"))?;
            (
                entry.version.as_str(),
                entry.provenance,
                tool_artifact_digest(entry),
            )
        }
        IronHubEntryKind::Skill => {
            let entry = manifest
                .find_skill(name)
                .ok_or_else(|| catalog_error("skill not found"))?;
            (
                entry.version.as_str(),
                entry.provenance,
                skill_artifact_digest(entry),
            )
        }
    };
    if let Some(expected) = &options.expected_version
        && expected != version
    {
        return Err(IronHubCommandError::InvalidInput {
            reason: format!(
                "catalog version for '{name}' changed: expected {expected}, current {version}"
            ),
        });
    }
    if let Some(expected) = &options.expected_artifact_digest
        && !expected.eq_ignore_ascii_case(&artifact_digest)
    {
        return Err(IronHubCommandError::InvalidInput {
            reason: format!(
                "artifact digest for '{name}' changed: expected {expected}, current {artifact_digest}"
            ),
        });
    }
    if provenance.is_community_unverified() && !options.acknowledge_unverified {
        return Err(IronHubCommandError::InvalidInput {
            reason: format!(
                "'{name}' is UNVERIFIED community content (trust tier: {}). Re-run with acknowledgement to install at your own risk.",
                provenance.as_wire()
            ),
        });
    }
    Ok((kind, provenance, artifact_digest))
}

fn classify(
    manifest: &IronHubManifest,
    name: &str,
    hint: Option<IronHubEntryKind>,
) -> Result<IronHubEntryKind, IronHubCommandError> {
    let in_tools = manifest.find_tool(name).is_some();
    let in_skills = manifest.find_skill(name).is_some();
    match (hint, in_tools, in_skills) {
        (Some(IronHubEntryKind::Tool), true, _) => Ok(IronHubEntryKind::Tool),
        (Some(IronHubEntryKind::Tool), false, _) => Err(invalid_input(format!(
            "'{name}' is not a tool in this IronHub catalog"
        ))),
        (Some(IronHubEntryKind::Skill), _, true) => Ok(IronHubEntryKind::Skill),
        (Some(IronHubEntryKind::Skill), _, false) => Err(invalid_input(format!(
            "'{name}' is not a skill in this IronHub catalog"
        ))),
        (None, true, false) => Ok(IronHubEntryKind::Tool),
        (None, false, true) => Ok(IronHubEntryKind::Skill),
        (None, true, true) => Err(invalid_input(format!(
            "'{name}' exists as both a tool and a skill; specify a kind"
        ))),
        (None, false, false) => Err(invalid_input(format!(
            "'{name}' is not in this IronHub catalog"
        ))),
    }
}

fn tool_artifact_digest(entry: &IronHubToolEntry) -> String {
    sha256_digest_token(format!("{}:{}", entry.wasm.sha256, entry.capabilities.sha256).as_bytes())
}

fn skill_artifact_digest(entry: &IronHubSkillEntry) -> String {
    sha256_digest_token(entry.skill_md.sha256.as_bytes())
}

fn validate_artifact(
    artifact: &IronHubArtifact,
    max_bytes: u64,
) -> Result<(), IronHubCommandError> {
    validate_artifact_url("artifact", "url", &artifact.url)?;
    if artifact.size_bytes > max_bytes {
        return Err(IronHubCommandError::Catalog {
            reason: format!("artifact exceeds {} byte cap", max_bytes),
        });
    }
    if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(IronHubCommandError::Catalog {
            reason: "artifact sha256 must be 64 hex characters".to_string(),
        });
    }
    Ok(())
}

fn validate_artifact_url(
    manifest_name: &str,
    field: &'static str,
    url: &str,
) -> Result<(), IronHubCommandError> {
    let parsed = url::Url::parse(url).map_err(|error| IronHubCommandError::Catalog {
        reason: format!("{manifest_name}.{field} invalid URL: {error}"),
    })?;
    if parsed.scheme() != "https" {
        return Err(IronHubCommandError::Catalog {
            reason: format!("{manifest_name}.{field} must use https"),
        });
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| IronHubCommandError::Catalog {
            reason: format!("{manifest_name}.{field} host is missing"),
        })?;
    if host_is_disallowed_target(host) || !is_allowed_artifact_host(host) {
        return Err(IronHubCommandError::Catalog {
            reason: format!("{manifest_name}.{field} host '{host}' is not allowed"),
        });
    }
    Ok(())
}

fn network_policy_for_url(url: &str, max_bytes: u64) -> Result<NetworkPolicy, IronHubCommandError> {
    validate_artifact_url("download", "url", url)?;
    let parsed = url::Url::parse(url).map_err(|error| IronHubCommandError::Catalog {
        reason: format!("invalid URL: {error}"),
    })?;
    let host = parsed
        .host_str()
        .ok_or_else(|| catalog_error("URL host is missing"))?;
    Ok(NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: host.to_ascii_lowercase(),
            port: parsed.port(),
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(max_bytes),
    })
}

fn is_allowed_artifact_host(host: &str) -> bool {
    const ALLOWED: &[&str] = &[
        "hub.ironclaw.com",
        "github.com",
        "objects.githubusercontent.com",
        "github-releases.githubusercontent.com",
        "raw.githubusercontent.com",
    ];
    ALLOWED
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
        || host.ends_with(".githubusercontent.com")
        || extra_artifact_hosts()
            .iter()
            .any(|allowed| host.eq_ignore_ascii_case(allowed))
}

fn extra_artifact_hosts() -> Vec<String> {
    std::env::var("IRONHUB_EXTRA_ARTIFACT_HOSTS")
        .ok()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|host| !host.is_empty() && !host_is_disallowed_target(host))
        .collect()
}

fn host_is_disallowed_target(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    let ip_form = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if ip_form.parse::<std::net::IpAddr>().is_ok() || host == "localhost" {
        return true;
    }
    const INTERNAL_SUFFIXES: &[&str] = &[
        ".localhost",
        ".local",
        ".internal",
        ".intranet",
        ".lan",
        ".home",
        ".corp",
        ".private",
    ];
    INTERNAL_SUFFIXES
        .iter()
        .any(|suffix| host.ends_with(suffix))
        || !host.contains('.')
}

fn validate_hub_name(name: &str) -> Result<(), IronHubCommandError> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_');
    if valid {
        Ok(())
    } else {
        Err(invalid_input(
            "name must be non-empty and contain only lowercase letters, digits, '-', '_'",
        ))
    }
}

fn tool_summary(
    entry: &IronHubToolEntry,
) -> Result<ironclaw_product_workflow::LifecycleExtensionSummary, IronHubCommandError> {
    Ok(ironclaw_product_workflow::LifecycleExtensionSummary {
        package_ref: package_ref(LifecyclePackageKind::Extension, &entry.name)?,
        name: entry.name.clone(),
        version: entry.version.clone(),
        description: format!("{} [{}]", entry.description, entry.provenance.trust_label()),
        source: ironclaw_product_workflow::LifecycleExtensionSource::Registry,
        runtime_kind: ironclaw_product_workflow::LifecycleExtensionRuntimeKind::WasmTool,
        visible_capability_ids: vec![format!("{}.invoke", entry.name)],
        visible_read_only_capability_ids: Vec::new(),
        credential_requirements: Vec::new(),
        onboarding: None,
    })
}

fn skill_summary(
    entry: &IronHubSkillEntry,
) -> Result<ironclaw_product_workflow::LifecycleSkillSummary, IronHubCommandError> {
    Ok(ironclaw_product_workflow::LifecycleSkillSummary {
        name: LifecyclePackageId::new(entry.name.clone()).map_err(product_error)?,
        version: entry.version.clone(),
        description: format!("{} [{}]", entry.description, entry.provenance.trust_label()),
        source: ironclaw_product_workflow::LifecycleSkillSource::Installed,
        keywords: Vec::new(),
        tags: Vec::new(),
        requires_skills: Vec::new(),
    })
}

fn entry_matches(name: &str, description: &str, query: &str) -> bool {
    query.is_empty()
        || name.to_ascii_lowercase().contains(query)
        || description.to_ascii_lowercase().contains(query)
}

fn package_ref(
    kind: LifecyclePackageKind,
    id: &str,
) -> Result<LifecyclePackageRef, IronHubCommandError> {
    LifecyclePackageRef::new(kind, id).map_err(product_error)
}

fn resolve_manifest_url() -> String {
    std::env::var("IRONHUB_MANIFEST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_IRONHUB_MANIFEST_URL.to_string())
}

fn manifest_cache_get(url: &str, now: Instant) -> Option<Arc<IronHubManifest>> {
    let guard = MANIFEST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.get(url)?;
    (now.duration_since(entry.fetched_at) <= MANIFEST_CACHE_TTL)
        .then(|| Arc::clone(&entry.manifest))
}

fn manifest_cache_put(url: &str, manifest: Arc<IronHubManifest>, now: Instant) {
    let mut guard = MANIFEST_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.len() >= MANIFEST_CACHE_MAX_ENTRIES && !guard.contains_key(url) {
        guard.retain(|_, entry| now.duration_since(entry.fetched_at) <= MANIFEST_CACHE_TTL);
        if guard.len() >= MANIFEST_CACHE_MAX_ENTRIES
            && let Some(victim) = guard.keys().next().cloned()
        {
            guard.remove(&victim);
        }
    }
    guard.insert(
        url.to_string(),
        CachedManifest {
            manifest,
            fetched_at: now,
        },
    );
}

fn install_lock(key: &str) -> Arc<AsyncMutex<()>> {
    let mut guard = INSTALL_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

fn install_message(
    kind: IronHubEntryKind,
    name: &str,
    version: &str,
    provenance: IronHubProvenance,
    artifact_digest: &str,
) -> String {
    format!(
        "installed {} '{}' {} from IronHub; provenance={}, artifact_digest={}",
        kind.as_str(),
        name,
        version,
        provenance.as_wire(),
        artifact_digest
    )
}

fn phase_label(phase: LifecyclePhase) -> &'static str {
    match phase {
        LifecyclePhase::Discovered => "discovered",
        LifecyclePhase::Installing => "installing",
        LifecyclePhase::Installed => "installed",
        LifecyclePhase::Configured => "configured",
        LifecyclePhase::Activating => "activating",
        LifecyclePhase::Active => "active",
        LifecyclePhase::Disabled => "disabled",
        LifecyclePhase::UpgradeRequired => "upgrade_required",
        LifecyclePhase::Failed => "failed",
        LifecyclePhase::Removing => "removing",
        LifecyclePhase::Removed => "removed",
        LifecyclePhase::UnsupportedOrLegacy => "unsupported_or_legacy",
    }
}

fn package_kind_label(kind: LifecyclePackageKind) -> &'static str {
    match kind {
        LifecyclePackageKind::Extension => "extension",
        LifecyclePackageKind::Skill => "skill",
        LifecyclePackageKind::Mcp => "mcp",
        LifecyclePackageKind::Wasm => "wasm",
    }
}

fn terminal_safe(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}

fn push_line(output: &mut String, args: std::fmt::Arguments<'_>) {
    use std::fmt::Write as _;
    let _ = output.write_fmt(args);
    output.push('\n');
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn invalid_input(error: impl std::fmt::Display) -> IronHubCommandError {
    IronHubCommandError::InvalidInput {
        reason: error.to_string(),
    }
}

fn catalog_error(reason: impl Into<String>) -> IronHubCommandError {
    IronHubCommandError::Catalog {
        reason: reason.into(),
    }
}

fn install_error(reason: impl Into<String>) -> IronHubCommandError {
    IronHubCommandError::Install {
        reason: reason.into(),
    }
}

fn product_error(error: impl std::fmt::Display) -> IronHubCommandError {
    IronHubCommandError::Product(ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_manifest_verifies_known_test_vector() {
        let envelope = br#"{"v":1,"key_id":"test-vector","manifest_b64":"eyJ2ZXJzaW9uIjoiMSIsImdlbmVyYXRlZF9hdCI6IjIwMjYtMDEtMDFUMDA6MDA6MDBaIiwicmVsZWFzZV90YWciOiJ0ZXN0IiwicmVwbyI6Im5lYXJhaS9pcm9uaHViIiwidG9vbHMiOltdLCJza2lsbHMiOltdfQ","sig":"KjsUDgi1enj3iTPNQI6gU1Bwxf01hIUItlFvX9PxgWNybPPrJNIV7vFG-G8hJOalFMwFs5zQHrxbtFDZAlgtBg"}"#;
        let manifest = verify_signed_manifest_with_keys(
            envelope,
            &[(
                "test-vector",
                "ca46572f4dcd485599cdf95442934a3e3c86e2cae766a85fbffc8d6540959928",
            )],
        )
        .expect("signed manifest verifies");

        assert_eq!(
            manifest,
            br#"{"version":"1","generated_at":"2026-01-01T00:00:00Z","release_tag":"test","repo":"nearai/ironhub","tools":[],"skills":[]}"#
        );
    }

    #[test]
    fn missing_provenance_defaults_to_unverified() {
        let manifest: IronHubManifest = serde_json::from_str(
            r#"{
                "version": "1",
                "generated_at": "2026-01-01T00:00:00Z",
                "release_tag": "test",
                "repo": "nearai/ironhub",
                "tools": [{
                    "name": "community-tool",
                    "crate_name": "community-tool",
                    "version": "0.1.0",
                    "description": "community",
                    "wasm": {
                        "url": "https://hub.ironclaw.com/community-tool.wasm",
                        "size_bytes": 1,
                        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    },
                    "capabilities": {
                        "url": "https://hub.ironclaw.com/community-tool.capabilities.json",
                        "size_bytes": 1,
                        "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    }
                }],
                "skills": [{
                    "name": "community-skill",
                    "version": "0.1.0",
                    "description": "community",
                    "skill_md": {
                        "url": "https://hub.ironclaw.com/community-skill/SKILL.md",
                        "size_bytes": 1,
                        "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    }
                }]
            }"#,
        )
        .expect("manifest parses");

        assert_eq!(manifest.tools[0].provenance, IronHubProvenance::New);
        assert_eq!(manifest.skills[0].provenance, IronHubProvenance::New);
    }

    #[test]
    fn unverified_install_requires_acknowledgement() {
        let manifest = IronHubManifest {
            version: "1".to_string(),
            generated_at: "2026-01-01T00:00:00Z".to_string(),
            release_tag: "test".to_string(),
            repo: "nearai/ironhub".to_string(),
            tools: Vec::new(),
            skills: vec![IronHubSkillEntry {
                name: "community-skill".to_string(),
                trunk: String::new(),
                version: "0.1.0".to_string(),
                description: String::new(),
                provenance: IronHubProvenance::New,
                skill_md: IronHubArtifact {
                    url: "https://hub.ironclaw.com/community-skill/SKILL.md".to_string(),
                    size_bytes: 1,
                    sha256: "c".repeat(64),
                },
            }],
        };

        let blocked = classify_gate_and_digest(
            &manifest,
            "community-skill",
            Some(IronHubEntryKind::Skill),
            &IronHubInstallOptions::default(),
        )
        .expect_err("unverified content requires acknowledgement");
        assert!(blocked.to_string().contains("UNVERIFIED community content"));

        let allowed = classify_gate_and_digest(
            &manifest,
            "community-skill",
            Some(IronHubEntryKind::Skill),
            &IronHubInstallOptions {
                acknowledge_unverified: true,
                ..IronHubInstallOptions::default()
            },
        )
        .expect("acknowledged unverified content can proceed");
        assert_eq!(allowed.0, IronHubEntryKind::Skill);
        assert_eq!(allowed.1, IronHubProvenance::New);
    }

    #[test]
    fn renderer_includes_tools_and_skills_in_mixed_search() {
        let response = response_with_payload(
            None,
            LifecyclePhase::Discovered,
            LifecycleProductPayload::CatalogSearch {
                count: 2,
                tools: vec![
                    tool_summary(&IronHubToolEntry {
                        name: "web".to_string(),
                        crate_name: "web-tool".to_string(),
                        version: "0.1.0".to_string(),
                        description: "web tool".to_string(),
                        provenance: IronHubProvenance::Official,
                        wasm: IronHubArtifact {
                            url: "https://hub.ironclaw.com/web.wasm".to_string(),
                            size_bytes: 1,
                            sha256: "a".repeat(64),
                        },
                        capabilities: IronHubArtifact {
                            url: "https://hub.ironclaw.com/web.capabilities.json".to_string(),
                            size_bytes: 1,
                            sha256: "b".repeat(64),
                        },
                    })
                    .expect("tool summary"),
                ],
                skills: vec![
                    skill_summary(&IronHubSkillEntry {
                        name: "reviewer".to_string(),
                        trunk: String::new(),
                        version: "0.2.0".to_string(),
                        description: "review skill".to_string(),
                        provenance: IronHubProvenance::Verified,
                        skill_md: IronHubArtifact {
                            url: "https://hub.ironclaw.com/reviewer/SKILL.md".to_string(),
                            size_bytes: 1,
                            sha256: "c".repeat(64),
                        },
                    })
                    .expect("skill summary"),
                ],
            },
        );

        let rendered = render_reborn_ironhub_response("search", &response);
        assert!(rendered.contains("- tool web 0.1.0"));
        assert!(rendered.contains("- skill reviewer 0.2.0"));
    }

    #[test]
    fn artifact_digest_binds_both_tool_artifacts() {
        let tool = IronHubToolEntry {
            name: "web".to_string(),
            crate_name: "web-tool".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            provenance: IronHubProvenance::Official,
            wasm: IronHubArtifact {
                url: "https://hub.ironclaw.com/web.wasm".to_string(),
                size_bytes: 1,
                sha256: "a".repeat(64),
            },
            capabilities: IronHubArtifact {
                url: "https://hub.ironclaw.com/web.capabilities.json".to_string(),
                size_bytes: 1,
                sha256: "b".repeat(64),
            },
        };
        assert_eq!(
            tool_artifact_digest(&tool),
            sha256_digest_token(format!("{}:{}", "a".repeat(64), "b".repeat(64)).as_bytes())
        );
    }

    #[test]
    fn artifact_url_rejects_internal_hosts_even_when_extra() {
        assert!(host_is_disallowed_target("localhost"));
        assert!(host_is_disallowed_target("10.0.0.1"));
        assert!(host_is_disallowed_target("service.internal"));
    }
}
