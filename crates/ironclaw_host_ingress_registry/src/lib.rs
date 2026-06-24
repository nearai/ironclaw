//! HostIngress host-api projection over generic extension installation state.
//!
//! ```text
//! ironclaw_extensions::ExtensionInstallationStore
//!   manifests/installations for any extension
//!
//! list_enabled_host_ingress_entries(store)
//!   -> filter enabled installations whose manifest carries ironclaw.host_ingress/v1
//!   -> project HostIngressRouteDeclaration from that section
//!   -> return Vec<HostIngressRuntimeEntry>
//! ```

#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};
use std::num::{NonZeroU32, NonZeroU64};
use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionInstallation, ExtensionInstallationError, ExtensionInstallationStore,
    ExtensionManifestRecord, ExtensionManifestV2, HostApiContractRegistry, HostApiId,
    HostApiManifestContext, HostApiManifestContract, HostApiMultiplicity, HostApiRefV2,
    ManifestSectionPath, ManifestSource, ManifestV2Error,
};
use ironclaw_host_api::{
    AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, HostApiError,
    HostIngressDeclarationError, HostIngressRouteDeclaration, HostIngressTarget, HostPortCatalog,
    IngressAckMode, IngressAuthBinding, IngressAuthPolicy, IngressAuthScheme,
    IngressAuthSchemeName, IngressCredentialHandle, IngressDrainMode, IngressPolicy,
    IngressPolicyParts, IngressRouteDescriptor, IngressRouteId, IngressRoutePattern,
    IngressScopeSource, ListenerClass, NetworkMethod, RateLimitPolicy, RateLimitScope,
    StreamingMode, WebSocketOriginPolicy,
};
use serde::Deserialize;
use thiserror::Error;

pub use ironclaw_extensions::ManifestHash;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const HOST_INGRESS_HOST_API_ID: &str = "ironclaw.host_ingress/v1";
pub const HOST_INGRESS_SECTION_PREFIX: &str = "host_ingress";

pub const SLACK_EVENTS_POLICY_PROFILE: &str = "slack_events";
pub const SLACK_EVENTS_ROUTE_ID: &str = "slack.events";
pub const SLACK_EVENTS_PATH: &str = "/webhooks/slack/events";
pub const SLACK_EVENTS_BODY_LIMIT_BYTES: u64 = 1024 * 1024;
pub const SLACK_EVENTS_MAX_REQUESTS: u32 = 12_000;
pub const SLACK_EVENTS_RATE_WINDOW_SECONDS: u32 = 60;

pub fn parse_host_ingress_manifest_record(
    raw_toml: impl Into<String>,
    source: ManifestSource,
    host_port_catalog: &HostPortCatalog,
    manifest_hash: Option<ManifestHash>,
) -> Result<ExtensionManifestRecord, Error> {
    let contract = Arc::new(HostIngressHostApiContract::new()?);
    let mut contracts = HostApiContractRegistry::new();
    contracts.register(contract)?;
    let record = ExtensionManifestRecord::from_toml_with_contracts(
        raw_toml,
        source,
        host_port_catalog,
        manifest_hash,
        &contracts,
    )
    .map_err(|error| match error {
        ExtensionInstallationError::Manifest(error) => Error::Manifest(error),
        other => Error::Installation(other),
    })?;
    host_ingress_sections(&record)?;
    Ok(record)
}

pub fn host_ingress_sections(
    record: &ExtensionManifestRecord,
) -> Result<Vec<HostIngressRouteDeclaration>, Error> {
    project_host_ingress_sections(record.raw_toml(), record.manifest())
}

/// Enabled extension installation paired with its projected host ingress declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIngressRuntimeEntry {
    installation: ExtensionInstallation,
    declaration: HostIngressRouteDeclaration,
}

impl HostIngressRuntimeEntry {
    fn new(installation: ExtensionInstallation, declaration: HostIngressRouteDeclaration) -> Self {
        Self {
            installation,
            declaration,
        }
    }

    pub fn installation(&self) -> &ExtensionInstallation {
        &self.installation
    }

    pub fn declaration(&self) -> &HostIngressRouteDeclaration {
        &self.declaration
    }
}

/// Project enabled host ingress runtime entries from any `ExtensionInstallationStore`.
///
/// Filters to enabled installations whose manifest carries an
/// `ironclaw.host_ingress/v1` host-api section, then pairs each installation
/// with its projected [`HostIngressRouteDeclaration`]. Enabled extensions
/// without host ingress sections are intentionally ignored by this projection.
pub async fn list_enabled_host_ingress_entries(
    store: &dyn ExtensionInstallationStore,
) -> Result<Vec<HostIngressRuntimeEntry>, Error> {
    let manifests = store.list_manifests().await?;
    let manifest_map: HashMap<_, _> = manifests
        .iter()
        .map(|manifest| (manifest.extension_id().clone(), manifest))
        .collect();
    let mut entries = Vec::new();
    let mut ingress_cache: HashMap<_, Vec<HostIngressRouteDeclaration>> = HashMap::new();

    for installation in store.list_enabled_installations().await? {
        let manifest = manifest_map
            .get(installation.extension_id())
            .ok_or_else(|| Error::UnknownManifest {
                extension_id: installation.extension_id().to_string(),
            })?;
        let declarations =
            if let Some(declarations) = ingress_cache.get(installation.extension_id()) {
                declarations.clone()
            } else {
                let declarations = host_ingress_sections(manifest)?;
                ingress_cache.insert(installation.extension_id().clone(), declarations.clone());
                declarations
            };
        validate_installation_against_one_manifest(manifest, &installation, &declarations)?;
        if declarations.is_empty() {
            continue;
        }
        for declaration in &declarations {
            entries.push(HostIngressRuntimeEntry::new(
                installation.clone(),
                declaration.clone(),
            ));
        }
    }

    Ok(entries)
}

// ---------------------------------------------------------------------------
// Policy profiles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IngressPolicyProfile {
    SlackEvents,
}

impl IngressPolicyProfile {
    fn from_manifest_name(name: impl Into<String>) -> Result<Self, Error> {
        let name = name.into();
        match name.as_str() {
            SLACK_EVENTS_POLICY_PROFILE => Ok(Self::SlackEvents),
            _ => Err(Error::UnknownPolicyProfile { profile: name }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SlackEvents => SLACK_EVENTS_POLICY_PROFILE,
        }
    }

    pub fn route_descriptor(self) -> Result<IngressRouteDescriptor, Error> {
        match self {
            Self::SlackEvents => slack_events_route_descriptor(),
        }
    }
}

impl std::fmt::Display for IngressPolicyProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn slack_events_route_descriptor() -> Result<IngressRouteDescriptor, Error> {
    IngressRouteDescriptor::new(
        SLACK_EVENTS_ROUTE_ID,
        NetworkMethod::Post,
        SLACK_EVENTS_PATH,
        slack_events_policy()?,
    )
    .map_err(|source| Error::ProfileBuild {
        profile: IngressPolicyProfile::SlackEvents,
        source,
    })
}

fn slack_events_policy() -> Result<IngressPolicy, Error> {
    IngressPolicy::new(IngressPolicyParts {
        listener_class: ListenerClass::PublicWebhook,
        auth: IngressAuthPolicy::Required {
            schemes: vec![IngressAuthScheme::WebhookSignature],
        },
        scope_source: IngressScopeSource::HostResolved,
        body_limit: BodyLimitPolicy::Limited {
            max_bytes: nonzero_u64(
                SLACK_EVENTS_BODY_LIMIT_BYTES,
                IngressPolicyProfile::SlackEvents,
                "body_limit.max_bytes",
            )?,
        },
        rate_limit: RateLimitPolicy::Limited {
            scope: RateLimitScope::Global,
            max_requests: nonzero_u32(
                SLACK_EVENTS_MAX_REQUESTS,
                IngressPolicyProfile::SlackEvents,
                "rate_limit.max_requests",
            )?,
            window_seconds: nonzero_u32(
                SLACK_EVENTS_RATE_WINDOW_SECONDS,
                IngressPolicyProfile::SlackEvents,
                "rate_limit.window_seconds",
            )?,
        },
        cors: CorsPolicy::NotApplicable,
        websocket_origin: WebSocketOriginPolicy::NotApplicable,
        streaming: StreamingMode::None,
        audit: AuditTraceClass::PublicCallback,
        effect_path: AllowedEffectPath::ProductWorkflow,
    })
    .map_err(|source| Error::ProfileBuild {
        profile: IngressPolicyProfile::SlackEvents,
        source,
    })
}

fn nonzero_u32(
    value: u32,
    profile: IngressPolicyProfile,
    field: &'static str,
) -> Result<NonZeroU32, Error> {
    NonZeroU32::new(value).ok_or_else(|| Error::ProfileBuild {
        profile,
        source: HostApiError::InvariantViolation {
            reason: format!("{field} must be non-zero"),
        },
    })
}

fn nonzero_u64(
    value: u64,
    profile: IngressPolicyProfile,
    field: &'static str,
) -> Result<NonZeroU64, Error> {
    NonZeroU64::new(value).ok_or_else(|| Error::ProfileBuild {
        profile,
        source: HostApiError::InvariantViolation {
            reason: format!("{field} must be non-zero"),
        },
    })
}

// ---------------------------------------------------------------------------
// HostIngress host-api contract validator
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct HostIngressHostApiContract {
    id: HostApiId,
}

impl HostIngressHostApiContract {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            id: HostApiId::new(HOST_INGRESS_HOST_API_ID)?,
        })
    }
}

impl HostApiManifestContract for HostIngressHostApiContract {
    fn id(&self) -> &HostApiId {
        &self.id
    }

    fn multiplicity(&self) -> HostApiMultiplicity {
        HostApiMultiplicity::Multiple
    }

    fn accepts_section_path(&self, section: &ManifestSectionPath) -> bool {
        is_host_ingress_section_path(section)
    }

    fn validate_section(
        &self,
        host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), String> {
        HostIngressSection::from_value(host_api, section.clone())
            .and_then(HostIngressSection::into_declaration)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn validate_section_with_context(
        &self,
        context: &HostApiManifestContext<'_>,
        host_api: &HostApiRefV2,
        section: &toml::Value,
    ) -> Result<(), String> {
        let _ = context;
        self.validate_section(host_api, section)
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error(transparent)]
    Installation(#[from] ExtensionInstallationError),
    #[error(transparent)]
    Manifest(#[from] ManifestV2Error),
    #[error("unknown host API id {id} for host ingress section")]
    UnknownHostApiId { id: String },
    #[error("unknown host ingress policy profile {profile}")]
    UnknownPolicyProfile { profile: String },
    #[error("host ingress manifest section {section} parse failed: {reason}")]
    ManifestSectionParse {
        section: ManifestSectionPath,
        reason: String,
    },
    #[error("host ingress profile {profile} failed to build: {source}")]
    ProfileBuild {
        profile: IngressPolicyProfile,
        source: HostApiError,
    },
    #[error("host ingress declaration validation failed for {section}: {source}")]
    DeclarationValidation {
        section: ManifestSectionPath,
        source: HostIngressDeclarationError,
    },
    #[error(
        "host ingress section {section} {field} does not match {profile} profile: expected {expected}, actual {actual}"
    )]
    ProfileRouteMismatch {
        section: ManifestSectionPath,
        profile: IngressPolicyProfile,
        field: &'static str,
        expected: String,
        actual: String,
    },
    #[error("installation references unknown extension manifest {extension_id}")]
    UnknownManifest { extension_id: String },
    #[error(
        "installation extension {extension_id} does not match manifest extension {manifest_extension_id}"
    )]
    ManifestExtensionMismatch {
        extension_id: String,
        manifest_extension_id: String,
    },
    #[error(
        "installation manifest hash does not match registered manifest hash for {extension_id}"
    )]
    ManifestHashMismatch { extension_id: String },
    #[error("installation is missing ingress credential handle {handle}")]
    MissingCredentialHandle { handle: IngressCredentialHandle },
}

// ---------------------------------------------------------------------------
// Internal validation helpers
// ---------------------------------------------------------------------------

fn validate_installation_against_one_manifest(
    manifest: &ExtensionManifestRecord,
    installation: &ExtensionInstallation,
    declarations: &[HostIngressRouteDeclaration],
) -> Result<(), Error> {
    if manifest.extension_id() != installation.manifest_ref().extension_id() {
        return Err(Error::ManifestExtensionMismatch {
            extension_id: installation.extension_id().to_string(),
            manifest_extension_id: installation.manifest_ref().extension_id().to_string(),
        });
    }

    match (
        manifest.manifest_hash(),
        installation.manifest_ref().manifest_hash(),
    ) {
        (Some(registered), Some(referenced)) if registered != referenced => {
            return Err(Error::ManifestHashMismatch {
                extension_id: installation.extension_id().to_string(),
            });
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(Error::ManifestHashMismatch {
                extension_id: installation.extension_id().to_string(),
            });
        }
        _ => {}
    }

    let bound: BTreeSet<_> = installation
        .credential_bindings()
        .iter()
        .map(|binding| binding.credential_handle().as_str())
        .collect();
    for handle in declarations
        .iter()
        .flat_map(|declaration| declaration.auth())
        .flat_map(IngressAuthBinding::credential_handles)
    {
        if !bound.contains(handle.as_str()) {
            return Err(Error::MissingCredentialHandle {
                handle: handle.clone(),
            });
        }
    }

    Ok(())
}

fn project_host_ingress_sections(
    raw_toml: &str,
    manifest: &ExtensionManifestV2,
) -> Result<Vec<HostIngressRouteDeclaration>, Error> {
    let root_section =
        ManifestSectionPath::new(HOST_INGRESS_SECTION_PREFIX).map_err(Error::Manifest)?;
    let value: toml::Value =
        toml::from_str(raw_toml).map_err(|error| Error::ManifestSectionParse {
            section: root_section,
            reason: error.to_string(),
        })?;

    let mut sections = Vec::new();
    for host_api in &manifest.host_apis {
        if host_api.id.as_str() != HOST_INGRESS_HOST_API_ID {
            if is_host_ingress_section_path(&host_api.section) {
                return Err(Error::UnknownHostApiId {
                    id: host_api.id.as_str().to_owned(),
                });
            }
            continue;
        }
        let section_value = section_value(&value, &host_api.section)?;
        sections.push(
            HostIngressSection::from_value(host_api, section_value.clone())?.into_declaration()?,
        );
    }
    Ok(sections)
}

fn section_value<'a>(
    root: &'a toml::Value,
    path: &ManifestSectionPath,
) -> Result<&'a toml::Value, Error> {
    let mut current = root;
    for segment in path.as_str().split('.') {
        current = current
            .as_table()
            .and_then(|table| table.get(segment))
            .ok_or_else(|| Error::ManifestSectionParse {
                section: path.clone(),
                reason: "section path does not exist".to_string(),
            })?;
    }
    Ok(current)
}

fn is_host_ingress_section_path(section: &ManifestSectionPath) -> bool {
    section.as_str() == HOST_INGRESS_SECTION_PREFIX
        || section
            .as_str()
            .strip_prefix(HOST_INGRESS_SECTION_PREFIX)
            .is_some_and(|rest| rest.starts_with('.'))
}

// ---------------------------------------------------------------------------
// Raw deserialization shapes for HostIngress sections
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostIngressSection {
    section: ManifestSectionPath,
    route_id: IngressRouteId,
    method: NetworkMethod,
    path: IngressRoutePattern,
    policy_profile: IngressPolicyProfile,
    target: HostIngressTarget,
    auth: IngressAuthBinding,
    ack: IngressAckMode,
    drain: IngressDrainMode,
}

impl HostIngressSection {
    fn from_value(host_api: &HostApiRefV2, value: toml::Value) -> Result<Self, Error> {
        if host_api.id.as_str() != HOST_INGRESS_HOST_API_ID {
            return Err(Error::UnknownHostApiId {
                id: host_api.id.as_str().to_owned(),
            });
        }
        let raw: RawHostIngressSection =
            value
                .try_into()
                .map_err(|error: toml::de::Error| Error::ManifestSectionParse {
                    section: host_api.section.clone(),
                    reason: error.to_string(),
                })?;
        Ok(Self {
            section: host_api.section.clone(),
            route_id: raw.route_id,
            method: raw.method,
            path: raw.path,
            policy_profile: IngressPolicyProfile::from_manifest_name(raw.policy_profile)?,
            target: raw.target,
            auth: raw.auth.into_binding(&host_api.section)?,
            ack: raw.ack,
            drain: raw.drain,
        })
    }

    fn into_declaration(self) -> Result<HostIngressRouteDeclaration, Error> {
        let descriptor = self.policy_profile.route_descriptor()?;
        self.validate_profile_route_identity(&descriptor)?;
        HostIngressRouteDeclaration::new(
            descriptor,
            self.target,
            vec![self.auth],
            self.ack,
            self.drain,
        )
        .map_err(|source| Error::DeclarationValidation {
            section: self.section,
            source,
        })
    }

    fn validate_profile_route_identity(
        &self,
        descriptor: &IngressRouteDescriptor,
    ) -> Result<(), Error> {
        if descriptor.route_id().as_str() != self.route_id.as_str() {
            return Err(Error::ProfileRouteMismatch {
                section: self.section.clone(),
                profile: self.policy_profile,
                field: "route_id",
                expected: descriptor.route_id().as_str().to_owned(),
                actual: self.route_id.as_str().to_owned(),
            });
        }
        if descriptor.method() != self.method {
            return Err(Error::ProfileRouteMismatch {
                section: self.section.clone(),
                profile: self.policy_profile,
                field: "method",
                expected: descriptor.method().to_string(),
                actual: self.method.to_string(),
            });
        }
        if descriptor.route_pattern().as_str() != self.path.as_str() {
            return Err(Error::ProfileRouteMismatch {
                section: self.section.clone(),
                profile: self.policy_profile,
                field: "path",
                expected: descriptor.route_pattern().as_str().to_owned(),
                actual: self.path.as_str().to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHostIngressSection {
    route_id: IngressRouteId,
    method: NetworkMethod,
    path: IngressRoutePattern,
    policy_profile: String,
    target: HostIngressTarget,
    auth: RawHostIngressAuth,
    ack: IngressAckMode,
    drain: IngressDrainMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHostIngressAuth {
    scheme: IngressAuthSchemeName,
    credential_handles: Vec<IngressCredentialHandle>,
}

impl RawHostIngressAuth {
    fn into_binding(self, section: &ManifestSectionPath) -> Result<IngressAuthBinding, Error> {
        IngressAuthBinding::new(self.scheme, self.credential_handles).map_err(|source| {
            Error::DeclarationValidation {
                section: section.clone(),
                source,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionCredentialBinding, ExtensionCredentialHandle,
        ExtensionInstallationId, ExtensionManifestRef, InMemoryExtensionInstallationStore,
        MANIFEST_SCHEMA_VERSION,
    };
    use ironclaw_host_api::{CapabilityId, SecretHandle};

    use super::*;

    fn manifest_hash(value: &str) -> ManifestHash {
        ManifestHash::new(value).expect("test manifest hash must be valid")
    }

    fn slack_manifest(extra: &str) -> String {
        format!(
            r#"
schema_version = "{schema}"
id = "slack-v2"
name = "Slack"
version = "0.1.0"
description = "Slack product adapter"
trust = "third_party"

[runtime]
kind = "wasm"
module = "adapters/slack-v2.wasm"

[[host_api]]
id = "ironclaw.host_ingress/v1"
section = "host_ingress.events"

[host_ingress.events]
route_id = "slack.events"
method = "post"
path = "/webhooks/slack/events"
policy_profile = "slack_events"
ack = "immediate"
drain = "drain_before_runtime_shutdown"

[host_ingress.events.target]
type = "product_adapter_inbound"
capability_id = "slack.events"
product_adapter_section = "product_adapter.inbound"

[host_ingress.events.auth]
scheme = "slack_v0_hmac"
credential_handles = ["slack_signing_secret"]

{extra}
"#,
            schema = MANIFEST_SCHEMA_VERSION,
        )
    }

    fn parse_slack(raw: &str) -> Result<ExtensionManifestRecord, Error> {
        parse_host_ingress_manifest_record(
            raw,
            ManifestSource::InstalledLocal,
            &HostPortCatalog::empty(),
            Some(manifest_hash("sha256:slack")),
        )
    }

    fn host_ingress_ref() -> HostApiRefV2 {
        HostApiRefV2 {
            id: HostApiId::new(HOST_INGRESS_HOST_API_ID).expect("test host API id must be valid"),
            section: ManifestSectionPath::new("host_ingress.events")
                .expect("test section path must be valid"),
        }
    }

    fn project_single_section(raw: &str) -> Result<HostIngressRouteDeclaration, Error> {
        let root: toml::Value = toml::from_str(raw).expect("test TOML must parse");
        let host_api = host_ingress_ref();
        let value = section_value(&root, &host_api.section)
            .expect("test host ingress section must exist")
            .clone();
        HostIngressSection::from_value(&host_api, value)?.into_declaration()
    }

    fn enabled_slack_installation() -> ExtensionInstallation {
        let extension_id = ironclaw_host_api::ExtensionId::new("slack-v2")
            .expect("test extension id must be valid");
        ExtensionInstallation::new(
            ExtensionInstallationId::new("acme-slack-prod")
                .expect("test installation id must be valid"),
            extension_id.clone(),
            ExtensionActivationState::Enabled,
            ExtensionManifestRef::new(extension_id, Some(manifest_hash("sha256:slack"))),
            vec![ExtensionCredentialBinding::new(
                ExtensionCredentialHandle::new("slack_signing_secret")
                    .expect("test credential handle must be valid"),
                SecretHandle::new("secret_slack_signing_secret")
                    .expect("test secret handle must be valid"),
            )],
            Utc::now(),
        )
        .expect("test installation must be valid")
    }

    #[test]
    fn slack_events_profile_projects_expected_policy_constants() {
        let descriptor = IngressPolicyProfile::SlackEvents
            .route_descriptor()
            .expect("Slack events profile must build");

        assert_eq!(descriptor.route_id().as_str(), SLACK_EVENTS_ROUTE_ID);
        assert_eq!(descriptor.method(), NetworkMethod::Post);
        assert_eq!(descriptor.route_pattern().as_str(), SLACK_EVENTS_PATH);

        let policy = descriptor.policy();
        assert_eq!(policy.listener_class(), ListenerClass::PublicWebhook);
        assert_eq!(
            policy.auth(),
            &IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::WebhookSignature],
            }
        );
        assert_eq!(policy.scope_source(), IngressScopeSource::HostResolved);
        assert_eq!(
            policy.body_limit(),
            BodyLimitPolicy::Limited {
                max_bytes: NonZeroU64::new(1_048_576).expect("test value must be non-zero"),
            }
        );
        assert_eq!(
            policy.rate_limit(),
            &RateLimitPolicy::Limited {
                scope: RateLimitScope::Global,
                max_requests: NonZeroU32::new(12_000).expect("test value must be non-zero"),
                window_seconds: NonZeroU32::new(60).expect("test value must be non-zero"),
            }
        );
        assert_eq!(policy.cors(), CorsPolicy::NotApplicable);
        assert_eq!(
            policy.websocket_origin(),
            WebSocketOriginPolicy::NotApplicable
        );
        assert_eq!(policy.streaming(), StreamingMode::None);
        assert_eq!(policy.audit(), AuditTraceClass::PublicCallback);
        assert_eq!(policy.effect_path(), &AllowedEffectPath::ProductWorkflow);
        // TODO(step): add a composition-side cross-check against
        // `slack_events_policy()` once this dormant registry is wired there.
    }

    #[tokio::test]
    async fn slack_events_manifest_parses_and_projects_valid_declaration() {
        let record = parse_slack(&slack_manifest("")).expect("Slack manifest must parse");
        let declarations = host_ingress_sections(&record).expect("Slack host ingress must project");
        assert_eq!(declarations.len(), 1);

        let declaration = &declarations[0];
        assert_eq!(declaration.route().route_id().as_str(), "slack.events");
        assert_eq!(declaration.route().method(), NetworkMethod::Post);
        assert_eq!(
            declaration.route().route_pattern().as_str(),
            "/webhooks/slack/events"
        );
        assert_eq!(
            declaration.target(),
            &HostIngressTarget::ProductAdapterInbound {
                capability_id: CapabilityId::new("slack.events")
                    .expect("test capability must be valid"),
                product_adapter_section: "product_adapter.inbound".to_string(),
            }
        );
        assert_eq!(declaration.auth().len(), 1);
        assert_eq!(declaration.auth()[0].scheme().as_str(), "slack_v0_hmac");
        assert_eq!(
            declaration.auth()[0].credential_handles()[0].as_str(),
            "slack_signing_secret"
        );
        assert_eq!(declaration.ack(), IngressAckMode::Immediate);
        assert_eq!(
            declaration.drain(),
            IngressDrainMode::DrainBeforeRuntimeShutdown
        );

        let store = InMemoryExtensionInstallationStore::default();
        store
            .upsert_manifest(record)
            .await
            .expect("upsert manifest");
        store
            .upsert_installation(enabled_slack_installation())
            .await
            .expect("upsert installation");
        let entries = list_enabled_host_ingress_entries(&store)
            .await
            .expect("runtime entries must project");
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].declaration().route().route_id().as_str(),
            "slack.events"
        );
    }

    #[test]
    fn unknown_profile_name_returns_error() {
        let raw = slack_manifest("").replace(
            r#"policy_profile = "slack_events""#,
            r#"policy_profile = "other_events""#,
        );
        let error = project_single_section(&raw).expect_err("unknown profile must reject");

        assert!(matches!(
            error,
            Error::UnknownPolicyProfile { ref profile } if profile == "other_events"
        ));
    }

    #[test]
    fn immediate_ack_with_no_drain_surfaces_host_api_validation_error() {
        let raw = slack_manifest("").replace(
            r#"drain = "drain_before_runtime_shutdown""#,
            r#"drain = "none""#,
        );
        let error =
            project_single_section(&raw).expect_err("immediate ack without drain must reject");

        assert!(matches!(
            error,
            Error::DeclarationValidation {
                source: HostIngressDeclarationError::ImmediateAckRequiresDrain,
                ..
            }
        ));
    }
}
