use ironclaw_extensions::{
    CapabilityDeclV2, CapabilityVisibility, ExtensionAssetPath, ExtensionManifestRecord,
    ExtensionPackage, ExtensionRuntime, ManifestSource,
};
use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
use ironclaw_first_party_extensions::is_gsuite_extension_id;
use ironclaw_host_api::{
    CapabilityId, ExtensionId, RuntimeCredentialAccountProviderId, VirtualPath, sha256_digest_token,
};
use ironclaw_product_adapter_registry::product_adapter_sections;
use ironclaw_product_adapters::ProductSurfaceKind;
use ironclaw_product_workflow::{
    LifecycleExtensionCredentialRequirement, LifecycleExtensionCredentialSetup,
    LifecycleExtensionOnboarding, LifecycleExtensionRuntimeKind, LifecycleExtensionSource,
    LifecycleExtensionSummary, LifecycleExtensionSurfaceKind, LifecyclePackageKind,
    LifecyclePackageRef, ProductWorkflowError,
};
use toml::Value;

use crate::extension_credential_requirements::{
    can_merge_lifecycle_credential_setup, merge_lifecycle_credential_setup,
    product_auth_credential_source,
};
use crate::nearai_mcp::{
    NearAiMcpBootstrapConfig, NearAiMcpEndpoint, nearai_mcp_endpoint_from_env,
};

const GITHUB_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/github/manifest.toml");
const GITHUB_WASM_MODULE: &[u8] =
    include_bytes!("../../ironclaw_first_party_extensions/assets/github/wasm/github_tool.wasm");
const GOOGLE_CALENDAR_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/google-calendar/manifest.toml");
const GOOGLE_DOCS_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/google-docs/manifest.toml");
const GOOGLE_DOCS_WASM_MODULE: &[u8] = include_bytes!(
    "../../ironclaw_first_party_extensions/assets/google-docs/wasm/google_docs_tool.wasm"
);
const GOOGLE_DRIVE_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/google-drive/manifest.toml");
const GOOGLE_DRIVE_WASM_MODULE: &[u8] = include_bytes!(
    "../../ironclaw_first_party_extensions/assets/google-drive/wasm/google_drive_tool.wasm"
);
const GOOGLE_SHEETS_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/google-sheets/manifest.toml");
const GOOGLE_SHEETS_WASM_MODULE: &[u8] = include_bytes!(
    "../../ironclaw_first_party_extensions/assets/google-sheets/wasm/google_sheets_tool.wasm"
);
const GOOGLE_SLIDES_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/google-slides/manifest.toml");
const GOOGLE_SLIDES_WASM_MODULE: &[u8] = include_bytes!(
    "../../ironclaw_first_party_extensions/assets/google-slides/wasm/google_slides_tool.wasm"
);
const GMAIL_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/gmail/manifest.toml");
const NOTION_MCP_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/notion-mcp/manifest.toml");
const WEB_ACCESS_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/web-access/manifest.toml");
const NEARAI_MCP_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/nearai-mcp/manifest.toml");
#[cfg(feature = "slack-v2-host-beta")]
const SLACK_MANIFEST: &str =
    include_str!("../../ironclaw_first_party_extensions/assets/slack/manifest.toml");
const NEARAI_EXTENSION_ID: &str = HostManagedCredentialExtension::NearAi.id();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostManagedCredentialExtension {
    NearAi,
}

impl HostManagedCredentialExtension {
    const fn id(self) -> &'static str {
        match self {
            Self::NearAi => "nearai",
        }
    }

    fn from_package_ref(package_ref: &LifecyclePackageRef) -> Option<Self> {
        #[cfg(not(feature = "root-llm-provider"))]
        {
            let _ = package_ref;
            None
        }
        #[cfg(feature = "root-llm-provider")]
        {
            if package_ref.kind != LifecyclePackageKind::Extension {
                return None;
            }
            match package_ref.id.as_str() {
                id if id == Self::NearAi.id() => Some(Self::NearAi),
                _ => None,
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AvailableExtensionAsset {
    pub(crate) path: String,
    pub(crate) content: AvailableExtensionAssetContent,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AvailableExtensionAssetContent {
    Bytes(Vec<u8>),
    Filesystem(VirtualPath),
}

#[derive(Debug)]
pub(crate) struct AvailableExtensionPackage {
    pub(crate) package_ref: LifecyclePackageRef,
    pub(crate) manifest_toml: String,
    pub(crate) package: ExtensionPackage,
    /// Surface kinds projected once from the manifest record at construction and
    /// cached here. Deliberately not re-derived in `summary()`: the projection
    /// (`product_adapter_sections`) needs the full `ExtensionManifestRecord`, and
    /// each loader parses the manifest exactly once (see
    /// `surface_kinds_from_manifest_record`). Keep in sync at construction.
    pub(crate) surface_kinds: Vec<LifecycleExtensionSurfaceKind>,
    pub(crate) assets: Vec<AvailableExtensionAsset>,
}

impl AvailableExtensionPackage {
    pub(crate) fn summary(&self) -> LifecycleExtensionSummary {
        let visible_capability_ids = visible_capability_ids(self)
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>();
        let visible_read_only_capability_ids = visible_read_only_capability_ids(self)
            .map(|id| id.as_str().to_string())
            .collect::<Vec<_>>();
        LifecycleExtensionSummary {
            package_ref: self.package_ref.clone(),
            name: self.package.manifest.name.clone(),
            version: self.package.manifest.version.clone(),
            description: self.package.manifest.description.clone(),
            source: LifecycleExtensionSource::HostBundled,
            runtime_kind: runtime_kind(&self.package.manifest.runtime),
            surface_kinds: self.surface_kinds.clone(),
            visible_capability_ids,
            visible_read_only_capability_ids,
            credential_requirements: credential_requirements(self),
            onboarding: onboarding(&self.package_ref),
        }
    }
}

fn onboarding(package_ref: &LifecyclePackageRef) -> Option<LifecycleExtensionOnboarding> {
    if is_host_managed_credential_extension(package_ref) {
        return Some(onboarding_message(
            "NEAR AI MCP uses the NEAR AI credentials configured for the assistant. If NEAR AI is not configured yet, add a NEAR AI API key in assistant inference settings before activating this extension.",
            Some(
                "Configure NEAR AI for the assistant with an API key; MCP reuses that credential.",
            ),
            None,
            "After NEAR AI is configured for the assistant, activate NEAR AI MCP to publish its tools.",
        ));
    }

    match package_ref.id.as_str() {
        "github" => Some(onboarding_message(
            "GitHub needs a personal access token before its repository and pull request tools can run.",
            Some(
                "Create a GitHub personal access token with the repository permissions you want IronClaw to use, then paste it here.",
            ),
            Some("https://github.com/settings/personal-access-tokens/new"),
            "After saving the token, activate GitHub to publish its tools.",
        )),
        "gmail" => Some(onboarding_message(
            "Gmail needs Google OAuth authorization before mail tools can run.",
            Some("Authorize the Google account that IronClaw should use for Gmail."),
            None,
            "After authorization completes, activate Gmail to publish its tools.",
        )),
        "google-calendar" => Some(onboarding_message(
            "Google Calendar needs Google OAuth authorization before calendar tools can run.",
            Some("Authorize the Google account that IronClaw should use for Google Calendar."),
            None,
            "After authorization completes, activate Google Calendar to publish its tools.",
        )),
        "notion" => Some(onboarding_message(
            "Notion needs OAuth authorization before MCP tools can run.",
            Some("Authorize the Notion workspace that IronClaw should access."),
            None,
            "After authorization completes, activate Notion to publish its MCP tools.",
        )),
        "web-access" => Some(onboarding_message(
            "Web Access does not need credentials. Activate it to make web search and saved-result retrieval tools available.",
            Some("No credentials are required for Web Access."),
            None,
            "Activate Web Access to publish its tools.",
        )),
        _ => None,
    }
}

fn onboarding_message(
    instructions: &str,
    credential_instructions: Option<&str>,
    setup_url: Option<&str>,
    credential_next_step: &str,
) -> LifecycleExtensionOnboarding {
    LifecycleExtensionOnboarding {
        instructions: instructions.to_string(),
        credential_instructions: credential_instructions.map(str::to_string),
        setup_url: setup_url.map(str::to_string),
        credential_next_step: Some(credential_next_step.to_string()),
    }
}

fn runtime_kind(runtime: &ExtensionRuntime) -> LifecycleExtensionRuntimeKind {
    match runtime {
        ExtensionRuntime::Mcp { .. } => LifecycleExtensionRuntimeKind::McpServer,
        ExtensionRuntime::Wasm { .. } => LifecycleExtensionRuntimeKind::WasmTool,
        ExtensionRuntime::FirstParty { .. } => LifecycleExtensionRuntimeKind::FirstParty,
        ExtensionRuntime::System { .. } => LifecycleExtensionRuntimeKind::System,
        ExtensionRuntime::Script { .. } => LifecycleExtensionRuntimeKind::Script,
    }
}

fn is_host_managed_credential_extension(package_ref: &LifecyclePackageRef) -> bool {
    HostManagedCredentialExtension::from_package_ref(package_ref).is_some()
}

fn credential_requirements(
    package: &AvailableExtensionPackage,
) -> Vec<LifecycleExtensionCredentialRequirement> {
    if is_host_managed_credential_extension(&package.package_ref) {
        return Vec::new();
    }

    let mut groups: Vec<CredentialRequirementGroup> = Vec::new();
    for capability in &package.package.manifest.capabilities {
        for requirement in &capability.runtime_credentials {
            let Some((provider, setup)) = product_auth_credential_source(requirement) else {
                continue;
            };
            let handle = requirement.handle.as_str().to_string();
            if let Some(seen) = groups.iter_mut().find(|seen| {
                seen.handle == handle
                    && seen.provider == provider
                    && can_merge_lifecycle_credential_setup(&seen.setup, &setup)
            }) {
                seen.required |= requirement.required;
                merge_lifecycle_credential_setup(&mut seen.setup, setup);
                continue;
            }
            groups.push(CredentialRequirementGroup {
                handle,
                provider,
                required: requirement.required,
                setup,
            });
        }
    }
    groups
        .iter()
        .enumerate()
        .map(|(index, group)| {
            let has_distinct_source = groups.iter().any(|other| {
                other.handle == group.handle
                    && (other.provider != group.provider || other.setup != group.setup)
            });
            LifecycleExtensionCredentialRequirement {
                name: if has_distinct_source {
                    credential_requirement_name(&groups[..=index], group)
                } else {
                    group.handle.clone()
                },
                provider: group.provider.as_str().to_string(),
                required: group.required,
                setup: group.setup.clone(),
            }
        })
        .collect()
}

struct CredentialRequirementGroup {
    handle: String,
    provider: RuntimeCredentialAccountProviderId,
    required: bool,
    setup: LifecycleExtensionCredentialSetup,
}

fn credential_requirement_name(
    seen_groups: &[CredentialRequirementGroup],
    group: &CredentialRequirementGroup,
) -> String {
    let ordinal = seen_groups
        .iter()
        .filter(|seen| seen.handle == group.handle)
        .count();
    format!("{}__{}", group.handle, ordinal)
}

#[derive(Debug, Default)]
pub(crate) struct AvailableExtensionCatalog {
    packages: Vec<AvailableExtensionPackage>,
}

impl AvailableExtensionCatalog {
    pub(crate) fn from_packages(packages: Vec<AvailableExtensionPackage>) -> Self {
        Self { packages }
    }

    #[cfg(test)]
    pub(crate) fn from_first_party_assets() -> Result<Self, ProductWorkflowError> {
        Self::from_first_party_assets_with_nearai_mcp_config(None)
    }

    pub(crate) fn from_first_party_assets_with_nearai_mcp_config(
        nearai_mcp_config: Option<&NearAiMcpBootstrapConfig>,
    ) -> Result<Self, ProductWorkflowError> {
        #[cfg_attr(not(feature = "slack-v2-host-beta"), allow(unused_mut))]
        let mut packages = vec![
            github_package()?,
            notion_mcp_package()?,
            web_access_package()?,
            nearai_mcp_package(nearai_mcp_config)?,
            google_calendar_package()?,
            google_docs_package()?,
            google_drive_package()?,
            google_sheets_package()?,
            google_slides_package()?,
            gmail_package()?,
        ];
        #[cfg(feature = "slack-v2-host-beta")]
        packages.push(slack_package()?);
        Ok(Self::from_packages(packages))
    }

    pub(crate) fn extend(&mut self, other: Self) {
        for package in other.packages {
            if let Some(existing) = self
                .packages
                .iter_mut()
                .find(|existing| existing.package_ref == package.package_ref)
            {
                *existing = package;
            } else {
                self.packages.push(package);
            }
        }
    }

    pub(crate) async fn from_filesystem_root<F>(
        fs: &F,
        root: &VirtualPath,
    ) -> Result<Self, ProductWorkflowError>
    where
        F: RootFilesystem + ?Sized,
    {
        Ok(Self::from_packages(
            load_filesystem_packages(fs, root).await?,
        ))
    }

    pub(crate) fn search<'a>(
        &'a self,
        query: &str,
    ) -> impl Iterator<Item = &'a AvailableExtensionPackage> + 'a {
        let normalized_query = query.trim().to_ascii_lowercase();
        self.packages
            .iter()
            .filter(move |package| package_matches_search(package, &normalized_query))
    }

    pub(crate) fn resolve(
        &self,
        package_ref: &LifecyclePackageRef,
    ) -> Result<&AvailableExtensionPackage, ProductWorkflowError> {
        package_ref.require_kind(LifecyclePackageKind::Extension)?;
        self.packages
            .iter()
            .find(|package| &package.package_ref == package_ref)
            .ok_or_else(|| ProductWorkflowError::InvalidBindingRequest {
                reason: "available extension was not found".to_string(),
            })
    }
}

fn package_matches_search(package: &AvailableExtensionPackage, normalized_query: &str) -> bool {
    normalized_query.is_empty()
        || package_search_terms(package)
            .iter()
            .any(|term| term.contains(normalized_query))
}

fn package_search_terms(package: &AvailableExtensionPackage) -> Vec<String> {
    let mut terms = Vec::new();
    push_search_term(&mut terms, package.package_ref.id.as_str());
    push_search_term(&mut terms, &package.package.manifest.name);
    push_search_term(&mut terms, &package.package.manifest.description);
    if let ExtensionRuntime::FirstParty { service } = &package.package.manifest.runtime {
        push_search_term(&mut terms, service);
    }
    for capability in &package.package.manifest.capabilities {
        for credential in &capability.runtime_credentials {
            if let Some((provider, _setup)) = product_auth_credential_source(credential) {
                push_search_term(&mut terms, provider.as_str());
            }
        }
    }
    if is_gsuite_extension_id(&package.package.manifest.id) {
        for alias in [
            "google",
            "gsuite",
            "g suite",
            "workspace",
            "google workspace",
        ] {
            push_search_term(&mut terms, alias);
        }
    }
    terms
}

fn push_search_term(terms: &mut Vec<String>, term: impl AsRef<str>) {
    let term = term.as_ref().trim().to_ascii_lowercase();
    if !term.is_empty() {
        terms.push(term);
    }
}

fn github_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package("github", "GitHub", GITHUB_MANIFEST, github_assets())
}

fn notion_mcp_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package(
        "notion",
        "Notion MCP",
        NOTION_MCP_MANIFEST,
        notion_mcp_assets(),
    )
}

fn web_access_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package(
        "web-access",
        "Web Access",
        WEB_ACCESS_MANIFEST,
        web_access_assets(),
    )
}

fn nearai_mcp_package(
    config: Option<&NearAiMcpBootstrapConfig>,
) -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    let manifest = nearai_mcp_manifest_toml_for_config(config)?;
    bundled_extension_package(
        NEARAI_EXTENSION_ID,
        "NEAR AI",
        &manifest,
        nearai_mcp_assets(&manifest),
    )
}

fn google_calendar_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package(
        "google-calendar",
        "Google Calendar",
        GOOGLE_CALENDAR_MANIFEST,
        google_calendar_assets(),
    )
}

fn google_docs_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package(
        "google-docs",
        "Google Docs",
        GOOGLE_DOCS_MANIFEST,
        google_docs_assets(),
    )
}

fn google_drive_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package(
        "google-drive",
        "Google Drive",
        GOOGLE_DRIVE_MANIFEST,
        google_drive_assets(),
    )
}

fn google_sheets_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package(
        "google-sheets",
        "Google Sheets",
        GOOGLE_SHEETS_MANIFEST,
        google_sheets_assets(),
    )
}

fn google_slides_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package(
        "google-slides",
        "Google Slides",
        GOOGLE_SLIDES_MANIFEST,
        google_slides_assets(),
    )
}

fn gmail_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package("gmail", "Gmail", GMAIL_MANIFEST, gmail_assets())
}

#[cfg(feature = "slack-v2-host-beta")]
fn slack_package() -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    bundled_extension_package("slack", "Slack", SLACK_MANIFEST, slack_assets())
}

pub(crate) fn google_calendar_manifest_digest() -> String {
    sha256_digest_token(GOOGLE_CALENDAR_MANIFEST.as_bytes())
}

pub(crate) fn google_docs_manifest_digest() -> String {
    sha256_digest_token(GOOGLE_DOCS_MANIFEST.as_bytes())
}

pub(crate) fn google_drive_manifest_digest() -> String {
    sha256_digest_token(GOOGLE_DRIVE_MANIFEST.as_bytes())
}

pub(crate) fn google_sheets_manifest_digest() -> String {
    sha256_digest_token(GOOGLE_SHEETS_MANIFEST.as_bytes())
}

pub(crate) fn google_slides_manifest_digest() -> String {
    sha256_digest_token(GOOGLE_SLIDES_MANIFEST.as_bytes())
}

pub(crate) fn gmail_manifest_digest() -> String {
    sha256_digest_token(GMAIL_MANIFEST.as_bytes())
}

pub(crate) fn notion_mcp_manifest_digest() -> String {
    sha256_digest_token(NOTION_MCP_MANIFEST.as_bytes())
}

pub(crate) fn web_access_manifest_digest() -> String {
    sha256_digest_token(WEB_ACCESS_MANIFEST.as_bytes())
}

#[cfg(feature = "slack-v2-host-beta")]
pub(crate) fn slack_manifest_digest() -> String {
    sha256_digest_token(SLACK_MANIFEST.as_bytes())
}

pub(crate) fn nearai_mcp_manifest_toml_for_config(
    config: Option<&NearAiMcpBootstrapConfig>,
) -> Result<String, ProductWorkflowError> {
    let endpoint = match config {
        Some(config) => config.endpoint().map_err(map_binding_error)?,
        None => nearai_mcp_endpoint_from_env().map_err(map_binding_error)?,
    };
    nearai_mcp_manifest_toml_for_endpoint(&endpoint)
}

fn nearai_mcp_manifest_toml_for_endpoint(
    endpoint: &NearAiMcpEndpoint,
) -> Result<String, ProductWorkflowError> {
    let mut manifest = toml::from_str::<Value>(NEARAI_MCP_MANIFEST).map_err(|error| {
        map_binding_error(format!("bundled NEAR AI manifest TOML is invalid: {error}"))
    })?;
    let runtime = manifest
        .get_mut("runtime")
        .and_then(Value::as_table_mut)
        .ok_or_else(|| map_binding_error("bundled NEAR AI manifest lacks runtime table"))?;
    runtime.insert("url".to_string(), Value::String(endpoint.url.clone()));

    let capabilities = manifest
        .get_mut("capabilities")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| map_binding_error("bundled NEAR AI manifest lacks capabilities array"))?;
    let search = capabilities
        .first_mut()
        .and_then(Value::as_table_mut)
        .ok_or_else(|| map_binding_error("bundled NEAR AI manifest lacks search capability"))?;
    let runtime_credentials = search
        .get_mut("runtime_credentials")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| map_binding_error("bundled NEAR AI manifest lacks runtime credentials"))?;
    let credential = runtime_credentials
        .first_mut()
        .and_then(Value::as_table_mut)
        .ok_or_else(|| map_binding_error("bundled NEAR AI manifest lacks runtime credential"))?;
    let audience = credential
        .get_mut("audience")
        .and_then(Value::as_table_mut)
        .ok_or_else(|| {
            map_binding_error("bundled NEAR AI manifest lacks runtime credential audience")
        })?;
    audience.insert(
        "host_pattern".to_string(),
        Value::String(endpoint.host_pattern.clone()),
    );
    if let Some(port) = endpoint.port {
        audience.insert("port".to_string(), Value::Integer(i64::from(port)));
    } else {
        audience.remove("port");
    }

    toml::to_string(&manifest).map_err(|error| {
        map_binding_error(format!(
            "bundled NEAR AI manifest TOML render failed: {error}"
        ))
    })
}

fn bundled_extension_package(
    id: &str,
    label: &str,
    manifest_toml: &str,
    assets: Vec<AvailableExtensionAsset>,
) -> Result<AvailableExtensionPackage, ProductWorkflowError> {
    let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, id)?;
    let root = VirtualPath::new(format!("/system/extensions/{id}")).map_err(map_binding_error)?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected bundled {label} extension: {error}"),
        }
    })?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contracts rejected bundled {label} extension: {error}"),
            }
        })?;
    let record = ExtensionManifestRecord::from_toml_with_contracts(
        manifest_toml,
        ManifestSource::HostBundled,
        &host_ports,
        None,
        &contracts,
    )
    .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
        reason: format!("bundled {label} extension manifest is invalid: {error}"),
    })?;
    let surface_kinds = surface_kinds_from_manifest_record(&record, label)?;
    let manifest = record.manifest().clone().try_into().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("bundled {label} extension manifest is invalid: {error}"),
        }
    })?;
    let package = ExtensionPackage::from_manifest_toml(manifest, root, record.raw_toml()).map_err(
        |error| ProductWorkflowError::InvalidBindingRequest {
            reason: format!("bundled {label} extension package is invalid: {error}"),
        },
    )?;
    Ok(AvailableExtensionPackage {
        package_ref,
        manifest_toml: record.raw_toml().to_string(),
        package,
        surface_kinds,
        assets,
    })
}

fn surface_kinds_from_manifest_record(
    record: &ExtensionManifestRecord,
    label: &str,
) -> Result<Vec<LifecycleExtensionSurfaceKind>, ProductWorkflowError> {
    let adapters = product_adapter_sections(record).map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("{label} ProductAdapter manifest projection is invalid: {error}"),
        }
    })?;
    let mut surface_kinds = Vec::new();
    if adapters
        .iter()
        .any(|adapter| adapter.surface_kind() == ProductSurfaceKind::ExternalChannel)
    {
        surface_kinds.push(LifecycleExtensionSurfaceKind::ExternalChannel);
    }
    Ok(surface_kinds)
}

fn github_assets() -> Vec<AvailableExtensionAsset> {
    macro_rules! github_schema_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("schemas/github/", $path),
                include_bytes!(concat!(
                    "../../ironclaw_first_party_extensions/assets/github/schemas/github/",
                    $path
                )),
            )
        };
    }
    macro_rules! github_prompt_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("prompts/github/", $path),
                include_bytes!(concat!(
                    "../../ironclaw_first_party_extensions/assets/github/prompts/github/",
                    $path
                )),
            )
        };
    }

    vec![
        bytes_asset("manifest.toml", GITHUB_MANIFEST.as_bytes()),
        github_schema_asset!("comment_issue.input.v1.json"),
        github_schema_asset!("comment_issue.output.v1.json"),
        github_schema_asset!("create_branch.input.v1.json"),
        github_schema_asset!("create_issue.input.v1.json"),
        github_schema_asset!("create_issue_comment.input.v1.json"),
        github_schema_asset!("create_or_update_file.input.v1.json"),
        github_schema_asset!("create_pr_review.input.v1.json"),
        github_schema_asset!("create_pull_request.input.v1.json"),
        github_schema_asset!("create_release.input.v1.json"),
        github_schema_asset!("create_repo.input.v1.json"),
        github_schema_asset!("delete_file.input.v1.json"),
        github_schema_asset!("fork_repo.input.v1.json"),
        github_schema_asset!("get_combined_status.input.v1.json"),
        github_schema_asset!("get_file_content.input.v1.json"),
        github_schema_asset!("get_issue.input.v1.json"),
        github_schema_asset!("get_issue.output.v1.json"),
        github_schema_asset!("get_pull_request.input.v1.json"),
        github_schema_asset!("get_pull_request_files.input.v1.json"),
        github_schema_asset!("get_pull_request_reviews.input.v1.json"),
        github_schema_asset!("get_repo.input.v1.json"),
        github_schema_asset!("get_authenticated_user.input.v1.json"),
        github_schema_asset!("get_workflow_runs.input.v1.json"),
        github_schema_asset!("handle_webhook.input.v1.json"),
        github_schema_asset!("list_branches.input.v1.json"),
        github_schema_asset!("list_issue_comments.input.v1.json"),
        github_schema_asset!("list_issues.input.v1.json"),
        github_schema_asset!("list_pull_request_comments.input.v1.json"),
        github_schema_asset!("list_pull_requests.input.v1.json"),
        github_schema_asset!("list_releases.input.v1.json"),
        github_schema_asset!("list_repos.input.v1.json"),
        github_schema_asset!("merge_pull_request.input.v1.json"),
        github_schema_asset!("raw_output.v1.json"),
        github_schema_asset!("reply_pull_request_comment.input.v1.json"),
        github_schema_asset!("search_code.input.v1.json"),
        github_schema_asset!("search_issues.input.v1.json"),
        github_schema_asset!("search_issues.output.v1.json"),
        github_schema_asset!("search_issues_pull_requests.input.v1.json"),
        github_schema_asset!("search_repositories.input.v1.json"),
        github_schema_asset!("trigger_workflow.input.v1.json"),
        github_prompt_asset!("comment_issue.md"),
        github_prompt_asset!("create_branch.md"),
        github_prompt_asset!("create_issue.md"),
        github_prompt_asset!("create_issue_comment.md"),
        github_prompt_asset!("create_or_update_file.md"),
        github_prompt_asset!("create_pr_review.md"),
        github_prompt_asset!("create_pull_request.md"),
        github_prompt_asset!("create_release.md"),
        github_prompt_asset!("create_repo.md"),
        github_prompt_asset!("delete_file.md"),
        github_prompt_asset!("fork_repo.md"),
        github_prompt_asset!("get_combined_status.md"),
        github_prompt_asset!("get_file_content.md"),
        github_prompt_asset!("get_issue.md"),
        github_prompt_asset!("get_pull_request.md"),
        github_prompt_asset!("get_pull_request_files.md"),
        github_prompt_asset!("get_pull_request_reviews.md"),
        github_prompt_asset!("get_repo.md"),
        github_prompt_asset!("get_authenticated_user.md"),
        github_prompt_asset!("get_workflow_runs.md"),
        github_prompt_asset!("handle_webhook.md"),
        github_prompt_asset!("list_branches.md"),
        github_prompt_asset!("list_issue_comments.md"),
        github_prompt_asset!("list_issues.md"),
        github_prompt_asset!("list_pull_request_comments.md"),
        github_prompt_asset!("list_pull_requests.md"),
        github_prompt_asset!("list_releases.md"),
        github_prompt_asset!("list_repos.md"),
        github_prompt_asset!("merge_pull_request.md"),
        github_prompt_asset!("reply_pull_request_comment.md"),
        github_prompt_asset!("search_code.md"),
        github_prompt_asset!("search_issues.md"),
        github_prompt_asset!("search_issues_pull_requests.md"),
        github_prompt_asset!("search_repositories.md"),
        github_prompt_asset!("trigger_workflow.md"),
        bytes_asset("wasm/github_tool.wasm", GITHUB_WASM_MODULE),
    ]
}

fn notion_mcp_assets() -> Vec<AvailableExtensionAsset> {
    macro_rules! notion_schema_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("schemas/notion/", $path),
                include_bytes!(concat!(
                    "../../ironclaw_first_party_extensions/assets/notion-mcp/schemas/notion/",
                    $path
                )),
            )
        };
    }
    macro_rules! notion_prompt_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("prompts/notion/", $path),
                include_bytes!(concat!(
                    "../../ironclaw_first_party_extensions/assets/notion-mcp/prompts/notion/",
                    $path
                )),
            )
        };
    }

    vec![
        bytes_asset("manifest.toml", NOTION_MCP_MANIFEST.as_bytes()),
        notion_schema_asset!("notion-search.input.v1.json"),
        notion_schema_asset!("notion-search.output.v1.json"),
        notion_schema_asset!("notion-fetch.input.v1.json"),
        notion_schema_asset!("notion-fetch.output.v1.json"),
        notion_schema_asset!("notion-create-pages.input.v1.json"),
        notion_schema_asset!("notion-create-pages.output.v1.json"),
        notion_schema_asset!("notion-update-page.input.v1.json"),
        notion_schema_asset!("notion-update-page.output.v1.json"),
        notion_schema_asset!("notion-move-pages.input.v1.json"),
        notion_schema_asset!("notion-move-pages.output.v1.json"),
        notion_schema_asset!("notion-duplicate-page.input.v1.json"),
        notion_schema_asset!("notion-duplicate-page.output.v1.json"),
        notion_schema_asset!("notion-create-database.input.v1.json"),
        notion_schema_asset!("notion-create-database.output.v1.json"),
        notion_schema_asset!("notion-update-data-source.input.v1.json"),
        notion_schema_asset!("notion-update-data-source.output.v1.json"),
        notion_schema_asset!("notion-create-view.input.v1.json"),
        notion_schema_asset!("notion-create-view.output.v1.json"),
        notion_schema_asset!("notion-update-view.input.v1.json"),
        notion_schema_asset!("notion-update-view.output.v1.json"),
        notion_schema_asset!("notion-query-data-sources.input.v1.json"),
        notion_schema_asset!("notion-query-data-sources.output.v1.json"),
        notion_schema_asset!("notion-query-database-view.input.v1.json"),
        notion_schema_asset!("notion-query-database-view.output.v1.json"),
        notion_schema_asset!("notion-create-comment.input.v1.json"),
        notion_schema_asset!("notion-create-comment.output.v1.json"),
        notion_schema_asset!("notion-get-comments.input.v1.json"),
        notion_schema_asset!("notion-get-comments.output.v1.json"),
        notion_schema_asset!("notion-get-teams.input.v1.json"),
        notion_schema_asset!("notion-get-teams.output.v1.json"),
        notion_schema_asset!("notion-get-users.input.v1.json"),
        notion_schema_asset!("notion-get-users.output.v1.json"),
        notion_schema_asset!("notion-get-user.input.v1.json"),
        notion_schema_asset!("notion-get-user.output.v1.json"),
        notion_schema_asset!("notion-get-self.input.v1.json"),
        notion_schema_asset!("notion-get-self.output.v1.json"),
        notion_prompt_asset!("notion-search.md"),
        notion_prompt_asset!("notion-fetch.md"),
        notion_prompt_asset!("notion-create-pages.md"),
        notion_prompt_asset!("notion-update-page.md"),
        notion_prompt_asset!("notion-move-pages.md"),
        notion_prompt_asset!("notion-duplicate-page.md"),
        notion_prompt_asset!("notion-create-database.md"),
        notion_prompt_asset!("notion-update-data-source.md"),
        notion_prompt_asset!("notion-create-view.md"),
        notion_prompt_asset!("notion-update-view.md"),
        notion_prompt_asset!("notion-query-data-sources.md"),
        notion_prompt_asset!("notion-query-database-view.md"),
        notion_prompt_asset!("notion-create-comment.md"),
        notion_prompt_asset!("notion-get-comments.md"),
        notion_prompt_asset!("notion-get-teams.md"),
        notion_prompt_asset!("notion-get-users.md"),
        notion_prompt_asset!("notion-get-user.md"),
        notion_prompt_asset!("notion-get-self.md"),
    ]
}

fn web_access_assets() -> Vec<AvailableExtensionAsset> {
    vec![
        bytes_asset("manifest.toml", WEB_ACCESS_MANIFEST.as_bytes()),
        bytes_asset(
            "schemas/web-access/search.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/web-access/schemas/web-access/search.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/web-access/search.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/web-access/schemas/web-access/search.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/web-access/get_content.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/web-access/schemas/web-access/get_content.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/web-access/get_content.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/web-access/schemas/web-access/get_content.output.v1.json"
            ),
        ),
        bytes_asset(
            "prompts/web-access/search.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/web-access/prompts/web-access/search.md"
            ),
        ),
        bytes_asset(
            "prompts/web-access/get_content.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/web-access/prompts/web-access/get_content.md"
            ),
        ),
    ]
}

fn nearai_mcp_assets(manifest: &str) -> Vec<AvailableExtensionAsset> {
    vec![
        bytes_asset("manifest.toml", manifest.as_bytes()),
        bytes_asset(
            "schemas/nearai/web_search.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/nearai-mcp/schemas/nearai/web_search.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/nearai/web_search.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/nearai-mcp/schemas/nearai/web_search.output.v1.json"
            ),
        ),
        bytes_asset(
            "prompts/nearai/web_search.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/nearai-mcp/prompts/nearai/web_search.md"
            ),
        ),
    ]
}

fn google_calendar_assets() -> Vec<AvailableExtensionAsset> {
    vec![
        bytes_asset("manifest.toml", GOOGLE_CALENDAR_MANIFEST.as_bytes()),
        bytes_asset(
            "schemas/google-calendar/list_calendars.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/list_calendars.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/list_calendars.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/list_calendars.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/list_events.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/list_events.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/list_events.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/list_events.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/get_event.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/get_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/get_event.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/get_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/find_free_slots.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/find_free_slots.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/find_free_slots.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/find_free_slots.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/create_event.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/create_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/create_event.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/create_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/update_event.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/update_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/update_event.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/update_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/delete_event.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/delete_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/delete_event.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/delete_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/add_attendees.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/add_attendees.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/add_attendees.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/add_attendees.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/set_reminder.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/set_reminder.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/set_reminder.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/schemas/google-calendar/set_reminder.output.v1.json"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/list_calendars.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/list_calendars.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/list_events.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/list_events.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/get_event.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/get_event.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/find_free_slots.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/find_free_slots.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/create_event.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/create_event.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/update_event.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/update_event.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/delete_event.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/delete_event.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/add_attendees.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/add_attendees.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/set_reminder.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/google-calendar/prompts/google-calendar/set_reminder.md"
            ),
        ),
    ]
}

macro_rules! google_wasm_assets {
    ($id:literal, $manifest:expr, $wasm_file:literal, $wasm_module:expr, [$($operation:literal),+ $(,)?]) => {{
        vec![
            bytes_asset("manifest.toml", $manifest.as_bytes()),
            bytes_asset(
                concat!("schemas/", $id, "/raw_output.v1.json"),
                include_bytes!(concat!(
                    "../../ironclaw_first_party_extensions/assets/",
                    $id,
                    "/schemas/",
                    $id,
                    "/raw_output.v1.json"
                )),
            ),
            $(
                bytes_asset(
                    concat!("schemas/", $id, "/", $operation, ".input.v1.json"),
                    include_bytes!(concat!(
                        "../../ironclaw_first_party_extensions/assets/",
                        $id,
                        "/schemas/",
                        $id,
                        "/",
                        $operation,
                        ".input.v1.json"
                    )),
                ),
                bytes_asset(
                    concat!("prompts/", $id, "/", $operation, ".md"),
                    include_bytes!(concat!(
                        "../../ironclaw_first_party_extensions/assets/",
                        $id,
                        "/prompts/",
                        $id,
                        "/",
                        $operation,
                        ".md"
                    )),
                ),
            )+
            bytes_asset(concat!("wasm/", $wasm_file), $wasm_module),
        ]
    }};
}

fn google_docs_assets() -> Vec<AvailableExtensionAsset> {
    google_wasm_assets!(
        "google-docs",
        GOOGLE_DOCS_MANIFEST,
        "google_docs_tool.wasm",
        GOOGLE_DOCS_WASM_MODULE,
        [
            "create_document",
            "get_document",
            "read_content",
            "insert_text",
            "delete_content",
            "replace_text",
            "format_text",
            "format_paragraph",
            "insert_table",
            "create_list",
            "batch_update"
        ]
    )
}

fn google_drive_assets() -> Vec<AvailableExtensionAsset> {
    google_wasm_assets!(
        "google-drive",
        GOOGLE_DRIVE_MANIFEST,
        "google_drive_tool.wasm",
        GOOGLE_DRIVE_WASM_MODULE,
        [
            "list_files",
            "get_file",
            "download_file",
            "upload_file",
            "update_file",
            "create_folder",
            "delete_file",
            "trash_file",
            "share_file",
            "list_permissions",
            "remove_permission",
            "list_shared_drives"
        ]
    )
}

fn google_sheets_assets() -> Vec<AvailableExtensionAsset> {
    google_wasm_assets!(
        "google-sheets",
        GOOGLE_SHEETS_MANIFEST,
        "google_sheets_tool.wasm",
        GOOGLE_SHEETS_WASM_MODULE,
        [
            "create_spreadsheet",
            "get_spreadsheet",
            "read_values",
            "batch_read_values",
            "write_values",
            "append_values",
            "clear_values",
            "add_sheet",
            "delete_sheet",
            "rename_sheet",
            "format_cells"
        ]
    )
}

fn google_slides_assets() -> Vec<AvailableExtensionAsset> {
    google_wasm_assets!(
        "google-slides",
        GOOGLE_SLIDES_MANIFEST,
        "google_slides_tool.wasm",
        GOOGLE_SLIDES_WASM_MODULE,
        [
            "create_presentation",
            "get_presentation",
            "get_thumbnail",
            "create_slide",
            "delete_object",
            "insert_text",
            "delete_text",
            "replace_all_text",
            "create_shape",
            "insert_image",
            "format_text",
            "format_paragraph",
            "replace_shapes_with_image",
            "batch_update"
        ]
    )
}

fn gmail_assets() -> Vec<AvailableExtensionAsset> {
    vec![
        bytes_asset("manifest.toml", GMAIL_MANIFEST.as_bytes()),
        bytes_asset(
            "schemas/gmail/list_messages.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/list_messages.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/list_messages.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/list_messages.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/get_message.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/get_message.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/get_message.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/get_message.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/send_message.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/send_message.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/send_message.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/send_message.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/create_draft.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/create_draft.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/create_draft.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/create_draft.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/reply_to_message.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/reply_to_message.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/reply_to_message.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/reply_to_message.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/trash_message.input.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/trash_message.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/gmail/trash_message.output.v1.json",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/schemas/gmail/trash_message.output.v1.json"
            ),
        ),
        bytes_asset(
            "prompts/gmail/list_messages.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/list_messages.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/get_message.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/get_message.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/send_message.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/send_message.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/create_draft.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/create_draft.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/reply_to_message.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/reply_to_message.md"
            ),
        ),
        bytes_asset(
            "prompts/gmail/trash_message.md",
            include_bytes!(
                "../../ironclaw_first_party_extensions/assets/gmail/prompts/gmail/trash_message.md"
            ),
        ),
    ]
}

#[cfg(feature = "slack-v2-host-beta")]
fn slack_assets() -> Vec<AvailableExtensionAsset> {
    vec![bytes_asset("manifest.toml", SLACK_MANIFEST.as_bytes())]
}

fn bytes_asset(path: &str, bytes: &[u8]) -> AvailableExtensionAsset {
    AvailableExtensionAsset {
        path: path.to_string(),
        content: AvailableExtensionAssetContent::Bytes(bytes.to_vec()),
    }
}

pub(crate) async fn materialize_available_extension<F>(
    fs: &F,
    extension: &AvailableExtensionPackage,
) -> Result<(), ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let mut written_paths = Vec::new();
    for asset in &extension.assets {
        let path = extension_asset_path(&extension.package.id, &asset.path)?;
        let bytes = match &asset.content {
            AvailableExtensionAssetContent::Bytes(bytes) => bytes.clone(),
            AvailableExtensionAssetContent::Filesystem(source_path) => {
                match fs.read_file(source_path).await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        for written_path in written_paths.iter().rev() {
                            let _ = fs.delete(written_path).await;
                        }
                        return Err(ProductWorkflowError::Transient {
                            reason: format!(
                                "failed to read available extension asset {}: {error}",
                                asset.path
                            ),
                        });
                    }
                }
            }
        };
        if existing_asset_matches(fs, &path, &bytes).await {
            continue;
        }
        if let Err(error) = fs.write_file(&path, &bytes).await {
            for written_path in written_paths.iter().rev() {
                let _ = fs.delete(written_path).await;
            }
            return Err(ProductWorkflowError::Transient {
                reason: format!(
                    "failed to materialize extension asset {}: {error}",
                    asset.path
                ),
            });
        }
        written_paths.push(path);
    }
    Ok(())
}

async fn existing_asset_matches<F>(fs: &F, path: &VirtualPath, bytes: &[u8]) -> bool
where
    F: RootFilesystem + ?Sized,
{
    match fs.read_file(path).await {
        Ok(existing) => existing == bytes,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => false,
        Err(_) => false,
    }
}

async fn load_filesystem_packages<F>(
    fs: &F,
    root: &VirtualPath,
) -> Result<Vec<AvailableExtensionPackage>, ProductWorkflowError>
where
    F: RootFilesystem + ?Sized,
{
    let mut entries = match fs.list_dir(root).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(ProductWorkflowError::Transient {
                reason: format!("failed to list available extensions: {error}"),
            });
        }
    };
    entries.sort_by(|left, right| left.name.cmp(&right.name));

    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        ProductWorkflowError::InvalidBindingRequest {
            reason: format!("host port catalog rejected available extension: {error}"),
        }
    })?;
    let contracts =
        ironclaw_host_runtime::default_host_api_contract_registry().map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("host API contract registry rejected available extension: {error}"),
            }
        })?;

    let mut packages = Vec::new();
    for entry in entries {
        if entry.file_type != FileType::Directory {
            continue;
        }
        let Ok(extension_id) = ExtensionId::new(entry.name.clone()) else {
            continue;
        };
        if reserved_host_bundled_extension_id(&extension_id) {
            continue;
        }
        let manifest_path = VirtualPath::new(format!(
            "{}/manifest.toml",
            entry.path.as_str().trim_end_matches('/')
        ))
        .map_err(map_binding_error)?;
        let manifest_bytes = match fs.read_file(&manifest_path).await {
            Ok(bytes) => bytes,
            Err(FilesystemError::NotFound { .. }) | Err(FilesystemError::MountNotFound { .. }) => {
                continue;
            }
            Err(error) => {
                return Err(ProductWorkflowError::Transient {
                    reason: format!("failed to read available extension manifest: {error}"),
                });
            }
        };
        let manifest_toml = String::from_utf8(manifest_bytes).map_err(|error| {
            ProductWorkflowError::InvalidBindingRequest {
                reason: format!("available extension manifest is not UTF-8: {error}"),
            }
        })?;
        let record = ExtensionManifestRecord::from_toml_with_contracts(
            manifest_toml,
            ManifestSource::HostBundled,
            &host_ports,
            None,
            &contracts,
        )
        .map_err(map_binding_error)?;
        let surface_kinds = surface_kinds_from_manifest_record(&record, entry.name.as_str())?;
        let manifest = record
            .manifest()
            .clone()
            .try_into()
            .map_err(map_binding_error)?;
        let package = ExtensionPackage::from_manifest_toml(manifest, entry.path, record.raw_toml())
            .map_err(map_binding_error)?;
        let mut assets = vec![AvailableExtensionAsset {
            path: "manifest.toml".to_string(),
            content: AvailableExtensionAssetContent::Bytes(record.raw_toml().as_bytes().to_vec()),
        }];
        if let ExtensionRuntime::Wasm { module } = &package.manifest.runtime {
            let module_path = module
                .resolve_under(&package.root)
                .map_err(map_binding_error)?;
            assets.push(AvailableExtensionAsset {
                path: module.as_str().to_string(),
                content: AvailableExtensionAssetContent::Filesystem(module_path),
            });
        }
        packages.push(AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(
                LifecyclePackageKind::Extension,
                package.id.as_str(),
            )?,
            manifest_toml: record.raw_toml().to_string(),
            package,
            surface_kinds,
            assets,
        });
    }
    Ok(packages)
}

fn reserved_host_bundled_extension_id(extension_id: &ExtensionId) -> bool {
    matches!(
        extension_id.as_str(),
        "github" | "notion" | "web-access" | "slack" | NEARAI_EXTENSION_ID
    ) || is_gsuite_extension_id(extension_id)
}

fn extension_asset_path(
    extension_id: &ExtensionId,
    asset_path: &str,
) -> Result<VirtualPath, ProductWorkflowError> {
    let root = VirtualPath::new(format!("/system/extensions/{}", extension_id.as_str()))
        .map_err(map_binding_error)?;
    ExtensionAssetPath::new(asset_path.to_string())
        .map_err(map_binding_error)?
        .resolve_under(&root)
        .map_err(map_binding_error)
}

fn map_binding_error(error: impl std::fmt::Display) -> ProductWorkflowError {
    ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    }
}

pub(crate) fn visible_capability_ids(
    extension: &AvailableExtensionPackage,
) -> impl Iterator<Item = &CapabilityId> {
    visible_capabilities(extension).map(|capability| &capability.id)
}

pub(crate) fn visible_read_only_capability_ids(
    extension: &AvailableExtensionPackage,
) -> impl Iterator<Item = &CapabilityId> {
    visible_capabilities(extension)
        .filter(|capability| !capability.effects.iter().any(|effect| effect.is_write()))
        .map(|capability| &capability.id)
}

fn visible_capabilities(
    extension: &AvailableExtensionPackage,
) -> impl Iterator<Item = &CapabilityDeclV2> {
    extension
        .package
        .manifest
        .capabilities
        .iter()
        .filter(|capability| capability.visibility == CapabilityVisibility::Model)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap, HashSet},
        sync::{Arc, Mutex},
        time::SystemTime,
    };

    use async_trait::async_trait;
    use ironclaw_extensions::{ExtensionManifest, ManifestSource};
    use ironclaw_filesystem::{
        BackendCapabilities, DirEntry, FileStat, FilesystemError, FilesystemOperation,
        InMemoryBackend,
    };
    use ironclaw_host_api::{
        EffectKind, HostPortCatalog, PermissionMode, RuntimeCredentialAccountSetup,
        RuntimeCredentialRequirementSource,
    };

    use super::*;
    use crate::nearai_mcp::nearai_mcp_endpoint_from_base;

    #[test]
    fn visible_capability_ids_include_write_effects() {
        let extension = test_extension_package();

        let visible = visible_capability_ids(&extension)
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(
            visible,
            vec![
                CapabilityId::new("fixture.search").unwrap(),
                CapabilityId::new("fixture.write").unwrap()
            ]
        );
    }

    #[test]
    fn visible_read_only_capability_ids_excludes_write_effects() {
        let extension = test_extension_package();

        let visible = visible_read_only_capability_ids(&extension)
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(visible, vec![CapabilityId::new("fixture.search").unwrap()]);
        assert!(EffectKind::ExternalWrite.is_write());
        assert!(!EffectKind::Network.is_write());
    }

    #[test]
    fn bundled_first_party_manifest_asset_refs_are_packaged() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();

        for extension_id in [
            "github",
            "notion",
            "web-access",
            NEARAI_EXTENSION_ID,
            "google-calendar",
            "google-docs",
            "google-drive",
            "google-sheets",
            "google-slides",
            "gmail",
        ] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id).unwrap();
            let package = catalog.resolve(&package_ref).unwrap();
            let assets = package
                .assets
                .iter()
                .map(|asset| asset.path.as_str())
                .collect::<HashSet<_>>();

            for capability in &package.package.manifest.capabilities {
                assert!(
                    assets.contains(capability.input_schema_ref.as_str()),
                    "{extension_id} capability {} missing input schema asset {}",
                    capability.id,
                    capability.input_schema_ref.as_str()
                );
                assert!(
                    assets.contains(capability.output_schema_ref.as_str()),
                    "{extension_id} capability {} missing output schema asset {}",
                    capability.id,
                    capability.output_schema_ref.as_str()
                );
                if let Some(prompt_doc_ref) = &capability.prompt_doc_ref {
                    assert!(
                        assets.contains(prompt_doc_ref.as_str()),
                        "{extension_id} capability {} missing prompt doc asset {}",
                        capability.id,
                        prompt_doc_ref.as_str()
                    );
                }
            }
        }
    }

    #[test]
    fn bundled_gsuite_extensions_match_google_workspace_aliases() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let expected = BTreeSet::from([
            "gmail",
            "google-calendar",
            "google-docs",
            "google-drive",
            "google-sheets",
            "google-slides",
        ]);

        for query in ["google", "gsuite", "workspace"] {
            let ids = catalog
                .search(query)
                .map(|package| package.package_ref.id.as_str())
                .collect::<BTreeSet<_>>();

            assert!(
                expected.is_subset(&ids),
                "{query} should discover every GSuite package; got {ids:?}"
            );
        }
    }

    #[test]
    fn bundled_github_read_only_capabilities_default_allow_without_relaxing_writes() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").unwrap();
        let github = catalog.resolve(&package_ref).unwrap();
        let mut allowed_read_only = BTreeSet::new();
        let mut ask_required = BTreeSet::new();
        let sensitive_token_backed_reads = BTreeSet::from(["github.search_code"]);

        for capability in &github.package.manifest.capabilities {
            let requires_explicit_approval = capability.effects.iter().any(|effect| {
                effect.is_write() || matches!(effect, EffectKind::DispatchCapability)
            }) || sensitive_token_backed_reads
                .contains(capability.id.as_str());
            if requires_explicit_approval {
                assert_eq!(
                    capability.default_permission,
                    PermissionMode::Ask,
                    "{} should still ask before effectful or broad token-backed GitHub actions",
                    capability.id
                );
                ask_required.insert(capability.id.as_str());
            } else {
                assert_eq!(
                    capability.default_permission,
                    PermissionMode::Allow,
                    "{} should not require an extra approval prompt for GitHub reads",
                    capability.id
                );
                allowed_read_only.insert(capability.id.as_str());
            }
        }

        assert!(allowed_read_only.contains("github.get_repo"));
        assert!(allowed_read_only.contains("github.get_authenticated_user"));
        assert!(allowed_read_only.contains("github.list_branches"));
        assert!(ask_required.contains("github.search_code"));
        assert!(ask_required.contains("github.create_issue"));
        assert!(ask_required.contains("github.handle_webhook"));
    }

    #[test]
    fn bundled_web_access_defers_github_repository_tasks_to_github_extension() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "web-access").unwrap();
        let package = catalog.resolve(&package_ref).unwrap();
        let search = package
            .package
            .manifest
            .capabilities
            .iter()
            .find(|capability| capability.id.as_str() == "web-access.search")
            .expect("web access search capability");
        assert!(
            search
                .description
                .contains("Prefer GitHub extension capabilities"),
            "web-access.search description should route GitHub repository data to GitHub tools"
        );

        let prompt_asset = package
            .assets
            .iter()
            .find(|asset| asset.path == "prompts/web-access/search.md")
            .expect("web access search prompt");
        let AvailableExtensionAssetContent::Bytes(bytes) = &prompt_asset.content else {
            panic!("web access prompt should be bundled bytes");
        };
        let prompt = std::str::from_utf8(bytes).expect("prompt should be UTF-8");
        assert!(
            prompt.contains("prefer the GitHub extension capabilities"),
            "web-access.search prompt should route GitHub repository data to GitHub tools"
        );
    }

    #[test]
    fn bundled_extension_summaries_include_onboarding_messages() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();

        for (extension_id, expected_instructions) in [
            ("github", "GitHub needs a personal access token"),
            ("gmail", "Gmail needs Google OAuth authorization"),
            (
                "google-calendar",
                "Google Calendar needs Google OAuth authorization",
            ),
            ("notion", "Notion needs OAuth authorization"),
            #[cfg(feature = "root-llm-provider")]
            (
                NEARAI_EXTENSION_ID,
                "NEAR AI MCP uses the NEAR AI credentials",
            ),
            ("web-access", "Web Access does not need credentials"),
        ] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id).unwrap();
            let summary = catalog.resolve(&package_ref).unwrap().summary();
            let onboarding = summary
                .onboarding
                .as_ref()
                .expect("bundled extension onboarding");

            assert!(
                onboarding.instructions.contains(expected_instructions),
                "{extension_id} onboarding instructions should include `{expected_instructions}`; got `{}`",
                onboarding.instructions,
            );
            assert!(
                onboarding.credential_next_step.is_some(),
                "{extension_id} must include the next user step"
            );
            if matches!(extension_id, "gmail" | "google-calendar" | "notion") {
                assert!(
                    onboarding
                        .credential_instructions
                        .as_deref()
                        .is_some_and(|instructions| {
                            instructions.starts_with("Authorize ")
                                && !instructions.contains("Install")
                        }),
                    "{extension_id} configure onboarding should not repeat install-first copy"
                );
                assert!(
                    onboarding
                        .credential_next_step
                        .as_deref()
                        .is_some_and(|step| {
                            step.starts_with("After authorization completes")
                                && step.contains("activate")
                                && !step.contains("Install")
                        }),
                    "{extension_id} configure next step should describe post-authorization activation"
                );
            } else if extension_id == "github" {
                assert!(
                    onboarding
                        .credential_instructions
                        .as_deref()
                        .is_some_and(|instructions| {
                            (instructions.contains("Paste") || instructions.contains("paste"))
                                && !instructions.contains("Install")
                        }),
                    "{extension_id} configure onboarding should describe token entry without install-first copy"
                );
                assert!(
                    onboarding
                        .credential_next_step
                        .as_deref()
                        .is_some_and(|step| {
                            step.starts_with("After saving")
                                && step.contains("activate")
                                && !step.contains("Install")
                        }),
                    "{extension_id} configure next step should describe activation after saving credentials"
                );
            } else if extension_id == NEARAI_EXTENSION_ID {
                assert_eq!(
                    onboarding.credential_instructions.as_deref(),
                    Some(
                        "Configure NEAR AI for the assistant with an API key; MCP reuses that credential."
                    )
                );
                assert_eq!(
                    onboarding.credential_next_step.as_deref(),
                    Some(
                        "After NEAR AI is configured for the assistant, activate NEAR AI MCP to publish its tools."
                    )
                );
            } else if extension_id == "web-access" {
                assert_eq!(
                    onboarding.credential_next_step.as_deref(),
                    Some("Activate Web Access to publish its tools."),
                    "web-access configure next step should not repeat install-first copy"
                );
            } else {
                assert!(
                    onboarding
                        .credential_next_step
                        .as_deref()
                        .is_some_and(|step| step.contains("Install") && step.contains("activate")),
                    "{extension_id} onboarding should preserve install-then-activate ordering"
                );
            }
        }
    }

    #[test]
    fn host_managed_credential_extension_detection_is_centralized() {
        let nearai_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, NEARAI_EXTENSION_ID)
                .expect("valid NEAR AI extension ref");
        let notion_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion")
            .expect("valid Notion extension ref");
        let mcp_ref = LifecyclePackageRef::new(LifecyclePackageKind::Mcp, NEARAI_EXTENSION_ID)
            .expect("valid MCP ref");

        #[cfg(feature = "root-llm-provider")]
        assert!(is_host_managed_credential_extension(&nearai_ref));
        #[cfg(not(feature = "root-llm-provider"))]
        assert!(!is_host_managed_credential_extension(&nearai_ref));
        assert!(!is_host_managed_credential_extension(&notion_ref));
        assert!(!is_host_managed_credential_extension(&mcp_ref));
    }

    #[test]
    fn bundled_nearai_keeps_runtime_credentials_out_of_browser_setup_summary() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, NEARAI_EXTENSION_ID).unwrap();
        let package = catalog.resolve(&package_ref).unwrap();
        let summary = package.summary();

        #[cfg(feature = "root-llm-provider")]
        assert!(
            summary.credential_requirements.is_empty(),
            "NEAR AI MCP uses assistant-level NEAR AI credentials and must not \
             project an extension credential setup prompt"
        );
        #[cfg(not(feature = "root-llm-provider"))]
        assert!(
            !summary.credential_requirements.is_empty(),
            "NEAR AI MCP should only suppress extension credential setup prompts \
             when the root NEAR AI provider owns the credential"
        );

        let search = package
            .package
            .manifest
            .capabilities
            .iter()
            .find(|capability| capability.id.as_str() == "nearai.web_search")
            .expect("nearai web search capability");
        assert_eq!(search.runtime_credentials.len(), 1);
        assert_eq!(
            search.runtime_credentials[0].handle,
            ironclaw_host_api::SecretHandle::new("llm_nearai_api_key").unwrap()
        );
    }

    #[test]
    fn bundled_notion_projects_oauth_credential_setup() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").unwrap();
        let summary = catalog.resolve(&package_ref).unwrap().summary();

        assert_eq!(summary.credential_requirements.len(), 1);
        let requirement = &summary.credential_requirements[0];
        assert_eq!(requirement.provider, "notion");
        assert!(matches!(
            &requirement.setup,
            LifecycleExtensionCredentialSetup::OAuth { scopes } if scopes.is_empty()
        ));
    }

    #[test]
    fn bundled_google_credentials_project_single_oauth_setup_with_scope_union() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();

        for extension_id in [
            "google-calendar",
            "google-docs",
            "google-drive",
            "google-sheets",
            "google-slides",
            "gmail",
        ] {
            let package_ref =
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, extension_id).unwrap();
            let package = catalog.resolve(&package_ref).unwrap();
            let summary = package.summary();
            let google_requirements = summary
                .credential_requirements
                .iter()
                .filter(|requirement| requirement.provider == "google")
                .collect::<Vec<_>>();

            let mut credential_count = 0;
            let mut expected_setup_scopes = BTreeSet::new();
            for capability in &package.package.manifest.capabilities {
                for credential in &capability.runtime_credentials {
                    let RuntimeCredentialRequirementSource::ProductAuthAccount { provider, setup } =
                        &credential.source
                    else {
                        panic!(
                            "{extension_id} capability {} should use a Google product auth account",
                            capability.id
                        );
                    };
                    assert_eq!(provider.as_str(), "google");

                    let RuntimeCredentialAccountSetup::OAuth { scopes } = setup else {
                        panic!(
                            "{extension_id} capability {} should declare OAuth setup",
                            capability.id
                        );
                    };
                    assert_eq!(
                        scopes, &credential.provider_scopes,
                        "{extension_id} capability {} OAuth setup scopes should match requested provider scopes",
                        capability.id
                    );
                    expected_setup_scopes.extend(scopes.iter().cloned());
                    credential_count += 1;
                }
            }

            assert_eq!(
                google_requirements.len(),
                1,
                "{extension_id} lifecycle setup should show one Google OAuth request"
            );
            let LifecycleExtensionCredentialSetup::OAuth { scopes } = &google_requirements[0].setup
            else {
                panic!("{extension_id} should expose Google OAuth setup");
            };
            assert_eq!(
                scopes.iter().cloned().collect::<BTreeSet<_>>(),
                expected_setup_scopes,
                "{extension_id} lifecycle setup should include every capability OAuth scope"
            );
            assert!(
                credential_count > 0,
                "{extension_id} should declare runtime credentials"
            );
        }
    }

    #[test]
    fn bundled_gsuite_wasm_capabilities_are_operation_scoped() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-drive").unwrap();
        let package = catalog.resolve(&package_ref).unwrap();
        let capabilities = package
            .package
            .manifest
            .capabilities
            .iter()
            .map(|capability| (capability.id.as_str(), capability))
            .collect::<HashMap<_, _>>();

        assert!(!capabilities.contains_key("google-drive.execute"));
        assert!(capabilities.contains_key("google-drive.list_files"));
        assert!(capabilities.contains_key("google-drive.upload_file"));

        let summary = package.summary();
        assert!(
            summary
                .visible_capability_ids
                .contains(&"google-drive.upload_file".to_string())
        );
        assert!(
            summary
                .visible_read_only_capability_ids
                .contains(&"google-drive.list_files".to_string())
        );
        assert!(
            !summary
                .visible_read_only_capability_ids
                .contains(&"google-drive.upload_file".to_string())
        );

        let list_files = capabilities["google-drive.list_files"];
        assert_eq!(
            list_files.runtime_credentials[0].provider_scopes,
            vec!["https://www.googleapis.com/auth/drive.readonly".to_string()]
        );
        assert!(!list_files.effects.contains(&EffectKind::ExternalWrite));

        let upload_file = capabilities["google-drive.upload_file"];
        assert!(upload_file.effects.contains(&EffectKind::ExternalWrite));
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn bundled_slack_package_declares_product_adapter_channel_surface() {
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack").unwrap();
        let package = catalog.resolve(&package_ref).unwrap();

        assert_eq!(package.package.manifest.id.as_str(), "slack");
        assert!(matches!(
            package.package.manifest.runtime,
            ExtensionRuntime::FirstParty { ref service } if service == "slack_v2_host_beta"
        ));
        assert_eq!(package.package.manifest.capabilities.len(), 0);
        assert!(package.package.manifest.host_apis.iter().any(|host_api| {
            host_api.id.as_str() == "ironclaw.product_adapter/v1"
                && host_api.section.as_str() == "product_adapter.inbound"
        }));

        let summary = package.summary();
        assert_eq!(
            summary.surface_kinds,
            vec![LifecycleExtensionSurfaceKind::ExternalChannel]
        );
        assert_eq!(summary.visible_capability_ids, Vec::<String>::new());
    }

    #[test]
    fn non_channel_product_adapter_surface_does_not_project_channel_surface() {
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "web-product"
name = "Web Product"
version = "0.1.0"
description = "A web product adapter."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "web_product"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.web"

[product_adapter.web]
surface_kind = "web"

[product_adapter.web.auth]
kind = "bearer_token"

[product_adapter.web.capabilities]
flags = ["inbound_messages"]

[[product_adapter.web.required_credentials]]
handle = "web_token"
"#;

        let package = bundled_extension_package("web-product", "Web Product", MANIFEST, Vec::new())
            .expect("valid package");

        assert_eq!(package.summary().surface_kinds, Vec::new());
    }

    #[tokio::test]
    async fn materialize_bundled_github_writes_manifest_schema_refs() {
        let fs = InMemoryBackend::default();
        let catalog = AvailableExtensionCatalog::from_first_party_assets().unwrap();
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").unwrap();
        let github = catalog.resolve(&package_ref).unwrap();

        materialize_available_extension(&fs, github).await.unwrap();

        let get_repo_schema = fs
            .read_file(
                &VirtualPath::new(
                    "/system/extensions/github/schemas/github/get_repo.input.v1.json",
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            std::str::from_utf8(&get_repo_schema)
                .unwrap()
                .contains("GitHub get_repo input")
        );
        fs.read_file(
            &VirtualPath::new("/system/extensions/github/prompts/github/get_repo.md").unwrap(),
        )
        .await
        .unwrap();
    }

    #[test]
    fn bundled_manifest_digests_are_sha256_tokens() {
        assert!(notion_mcp_manifest_digest().starts_with("sha256:"));
        assert!(google_calendar_manifest_digest().starts_with("sha256:"));
        assert!(google_docs_manifest_digest().starts_with("sha256:"));
        assert!(google_drive_manifest_digest().starts_with("sha256:"));
        assert!(google_sheets_manifest_digest().starts_with("sha256:"));
        assert!(google_slides_manifest_digest().starts_with("sha256:"));
        assert!(gmail_manifest_digest().starts_with("sha256:"));
        #[cfg(feature = "slack-v2-host-beta")]
        assert!(slack_manifest_digest().starts_with("sha256:"));
    }

    #[test]
    fn nearai_manifest_renderer_uses_validated_endpoint_fields() {
        let endpoint =
            nearai_mcp_endpoint_from_base(Some("https://10.0.0.12:8443/%22%0Atrust=%22system"))
                .unwrap();

        let manifest_toml = nearai_mcp_manifest_toml_for_endpoint(&endpoint).unwrap();
        let manifest: Value = toml::from_str(&manifest_toml).unwrap();

        assert_eq!(manifest["trust"].as_str(), Some("first_party_requested"));
        assert_eq!(
            manifest["runtime"]["url"].as_str(),
            Some("https://10.0.0.12:8443/%22%0Atrust=%22system/mcp")
        );
        assert_eq!(
            manifest["capabilities"][0]["runtime_credentials"][0]["audience"]["host_pattern"]
                .as_str(),
            Some("10.0.0.12")
        );
        assert_eq!(
            manifest["capabilities"][0]["runtime_credentials"][0]["audience"]["port"].as_integer(),
            Some(8443)
        );
    }

    #[test]
    fn catalog_extend_replaces_duplicate_package_refs() {
        let stale = test_extension_package_with_wasm_bytes(b"stale");
        let bundled = test_extension_package_with_wasm_bytes(b"bundled");
        let mut catalog = AvailableExtensionCatalog::from_packages(vec![stale]);
        catalog.extend(AvailableExtensionCatalog::from_packages(vec![bundled]));

        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").unwrap();
        let package = catalog.resolve(&package_ref).unwrap();
        let wasm = package
            .assets
            .iter()
            .find(|asset| asset.path == "wasm/fixture.wasm")
            .expect("wasm asset");

        assert_eq!(
            wasm.content,
            AvailableExtensionAssetContent::Bytes(b"bundled".to_vec())
        );
        assert_eq!(catalog.search("fixture").count(), 1);
    }

    #[tokio::test]
    async fn materialize_fails_on_filesystem_error_and_rolls_back_written_assets() {
        let fs = FailingWriteFilesystem::default();
        let extension = test_extension_package();

        let error = materialize_available_extension(&fs, &extension)
            .await
            .expect_err("second write fails");

        assert!(matches!(error, ProductWorkflowError::Transient { .. }));
        let state = fs.state.lock().unwrap();
        assert_eq!(
            state.writes,
            vec![
                "/system/extensions/fixture/manifest.toml".to_string(),
                "/system/extensions/fixture/wasm/fixture.wasm".to_string()
            ]
        );
        assert_eq!(
            state.deletes,
            vec!["/system/extensions/fixture/manifest.toml".to_string()]
        );
    }

    #[tokio::test]
    async fn materialize_skips_matching_existing_assets() {
        let fs = RecordingMaterializeFilesystem::default();
        let extension = test_extension_package();
        for asset in &extension.assets {
            let path = extension_asset_path(&extension.package.id, &asset.path).unwrap();
            let AvailableExtensionAssetContent::Bytes(bytes) = &asset.content else {
                panic!("test fixture assets are byte-backed");
            };
            fs.files
                .lock()
                .unwrap()
                .insert(path.as_str().to_string(), bytes.clone());
        }

        materialize_available_extension(&fs, &extension)
            .await
            .expect("matching assets already materialized");

        assert!(
            fs.writes.lock().unwrap().is_empty(),
            "restore should not rewrite already materialized matching assets"
        );
    }

    #[tokio::test]
    async fn filesystem_catalog_loads_manifest_and_runtime_assets() {
        let fs = InMemoryBackend::default();
        let extension = test_extension_package();
        for asset in &extension.assets {
            let path = extension_asset_path(&extension.package.id, &asset.path).unwrap();
            let AvailableExtensionAssetContent::Bytes(bytes) = &asset.content else {
                panic!("test fixture assets are byte-backed");
            };
            fs.write_file(&path, bytes).await.unwrap();
        }

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
        )
        .await
        .unwrap();
        let results = catalog.search("fixture").collect::<Vec<_>>();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].package_ref, extension.package_ref);
        assert_eq!(
            results[0]
                .assets
                .iter()
                .map(|asset| asset.path.as_str())
                .collect::<Vec<_>>(),
            vec!["manifest.toml", "wasm/fixture.wasm"]
        );
    }

    #[tokio::test]
    async fn filesystem_catalog_skips_extension_dirs_without_manifest() {
        let fs = InMemoryBackend::default();
        fs.write_file(
            &VirtualPath::new("/system/extensions/incomplete/cache/leftover").unwrap(),
            b"stale",
        )
        .await
        .unwrap();

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(catalog.search("").count(), 0);
    }

    #[tokio::test]
    async fn filesystem_catalog_skips_reserved_host_bundled_extension_ids() {
        let fs = InMemoryBackend::default();
        fs.write_file(
            &VirtualPath::new("/system/extensions/gmail/manifest.toml").unwrap(),
            b"not parsed because gmail is host-bundled",
        )
        .await
        .unwrap();
        fs.write_file(
            &VirtualPath::new("/system/extensions/slack/manifest.toml").unwrap(),
            b"not parsed because slack is host-bundled",
        )
        .await
        .unwrap();

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(catalog.search("").count(), 0);
        assert_eq!(catalog.search("slack").count(), 0);
    }

    #[tokio::test]
    async fn filesystem_manifest_external_channel_surface_kind_projects_to_lifecycle_surface() {
        const MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "channel-ext"
name = "Channel Ext"
version = "0.1.0"
description = "A filesystem-discovered external channel extension."
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "channel_ext_host"

[[host_api]]
id = "ironclaw.product_adapter/v1"
section = "product_adapter.inbound"

[product_adapter.inbound]
surface_kind = "external_channel"

[product_adapter.inbound.auth]
kind = "request_signature"
header_name = "X-Channel-Signature"
timestamp_header_name = "X-Channel-Timestamp"

[product_adapter.inbound.capabilities]
flags = ["inbound_messages"]

[[product_adapter.inbound.required_credentials]]
handle = "channel_ext_token"

[[product_adapter.inbound.egress]]
host = "example.com"
credential_handle = "channel_ext_token"
"#;

        let fs = InMemoryBackend::default();
        fs.write_file(
            &VirtualPath::new("/system/extensions/channel-ext/manifest.toml").unwrap(),
            MANIFEST.as_bytes(),
        )
        .await
        .unwrap();

        let catalog = AvailableExtensionCatalog::from_filesystem_root(
            &fs,
            &VirtualPath::new("/system/extensions").unwrap(),
        )
        .await
        .unwrap();

        let results = catalog.search("channel-ext").collect::<Vec<_>>();
        assert_eq!(results.len(), 1, "filesystem manifest should be loaded");

        let package = results.into_iter().next().unwrap();
        assert_eq!(
            package.summary().surface_kinds,
            vec![LifecycleExtensionSurfaceKind::ExternalChannel],
            "filesystem-loaded external_channel manifest must project ExternalChannel surface kind"
        );
    }

    #[derive(Default)]
    struct FailingWriteFilesystem {
        state: Arc<Mutex<FailingWriteState>>,
    }

    #[derive(Default)]
    struct FailingWriteState {
        writes: Vec<String>,
        deletes: Vec<String>,
    }

    #[derive(Default)]
    struct RecordingMaterializeFilesystem {
        files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        writes: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl RootFilesystem for RecordingMaterializeFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::default()
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
            })
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            let files = self.files.lock().unwrap();
            let Some(bytes) = files.get(path.as_str()) else {
                return Err(FilesystemError::NotFound {
                    path: path.clone(),
                    operation: FilesystemOperation::Stat,
                });
            };
            Ok(FileStat {
                path: path.clone(),
                file_type: FileType::File,
                len: bytes.len() as u64,
                modified: Some(SystemTime::UNIX_EPOCH),
                sensitive: false,
            })
        }

        async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            self.files
                .lock()
                .unwrap()
                .get(path.as_str())
                .cloned()
                .ok_or_else(|| FilesystemError::NotFound {
                    path: path.clone(),
                    operation: FilesystemOperation::ReadFile,
                })
        }

        async fn write_file(
            &self,
            path: &VirtualPath,
            bytes: &[u8],
        ) -> Result<(), FilesystemError> {
            self.writes.lock().unwrap().push(path.as_str().to_string());
            self.files
                .lock()
                .unwrap()
                .insert(path.as_str().to_string(), bytes.to_vec());
            Ok(())
        }
    }

    #[async_trait]
    impl RootFilesystem for FailingWriteFilesystem {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::default()
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
            })
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
            })
        }

        async fn write_file(
            &self,
            path: &VirtualPath,
            _bytes: &[u8],
        ) -> Result<(), FilesystemError> {
            self.state
                .lock()
                .unwrap()
                .writes
                .push(path.as_str().to_string());
            if path.as_str().ends_with("fixture.wasm") {
                return Err(FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::WriteFile,
                    reason: "write rejected".to_string(),
                });
            }
            Ok(())
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.state
                .lock()
                .unwrap()
                .deletes
                .push(path.as_str().to_string());
            Ok(())
        }
    }

    fn test_extension_package() -> AvailableExtensionPackage {
        test_extension_package_with_wasm_bytes(b"wasm")
    }

    fn test_extension_package_with_wasm_bytes(wasm_bytes: &[u8]) -> AvailableExtensionPackage {
        static MANIFEST: &str = r#"
schema_version = "reborn.extension_manifest.v2"
id = "fixture"
name = "Fixture"
version = "0.1.0"
description = "fixture extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/fixture.wasm"

[[capabilities]]
id = "fixture.search"
description = "Search"
effects = ["network"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/search.input.json"
output_schema_ref = "schemas/search.output.json"

[[capabilities]]
id = "fixture.write"
description = "Write"
effects = ["external_write"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/write.input.json"
output_schema_ref = "schemas/write.output.json"
"#;
        let manifest = ExtensionManifest::parse(
            MANIFEST,
            ManifestSource::HostBundled,
            &HostPortCatalog::empty(),
        )
        .expect("manifest");
        let package = ExtensionPackage::from_manifest(
            manifest,
            VirtualPath::new("/system/extensions/fixture").unwrap(),
        )
        .expect("package");
        AvailableExtensionPackage {
            package_ref: LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture")
                .unwrap(),
            manifest_toml: MANIFEST.to_string(),
            package,
            surface_kinds: Vec::new(),
            assets: vec![
                AvailableExtensionAsset {
                    path: "manifest.toml".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(MANIFEST.as_bytes().to_vec()),
                },
                AvailableExtensionAsset {
                    path: "wasm/fixture.wasm".to_string(),
                    content: AvailableExtensionAssetContent::Bytes(wasm_bytes.to_vec()),
                },
            ],
        }
    }
}
