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
use std::sync::Arc;

use ironclaw_extensions::{
    ExtensionInstallation, ExtensionInstallationError, ExtensionInstallationStore,
    ExtensionManifestRecord, ExtensionManifestV2, HostApiContractRegistry, HostApiId,
    HostApiManifestContext, HostApiManifestContract, HostApiMultiplicity, HostApiRefV2,
    ManifestSectionPath, ManifestSource, ManifestV2Error,
};
use ironclaw_host_api::{
    HostApiError, HostIngressDeclarationError, HostIngressRouteDeclaration, HostIngressTarget,
    HostPortCatalog, IngressAckMode, IngressAuthBinding, IngressAuthScheme,
    IngressCredentialHandle, IngressDrainMode, IngressPolicy, IngressRouteDescriptor,
    IngressRouteId, IngressRoutePattern, NetworkMethod,
};
use serde::Deserialize;
use thiserror::Error;

pub use ironclaw_extensions::ManifestHash;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const HOST_INGRESS_HOST_API_ID: &str = "ironclaw.host_ingress/v1";
pub const HOST_INGRESS_SECTION_PREFIX: &str = "host_ingress";

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
        project_host_ingress_section(host_api, section.clone())
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
    #[error("host ingress manifest section {section} parse failed: {reason}")]
    ManifestSectionParse {
        section: ManifestSectionPath,
        reason: String,
    },
    #[error("host ingress route descriptor failed to build for {section}: {source}")]
    RouteDescriptorBuild {
        section: ManifestSectionPath,
        source: HostApiError,
    },
    #[error("host ingress declaration validation failed for {section}: {source}")]
    DeclarationValidation {
        section: ManifestSectionPath,
        source: HostIngressDeclarationError,
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
        sections.push(project_host_ingress_section(
            host_api,
            section_value.clone(),
        )?);
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

fn project_host_ingress_section(
    host_api: &HostApiRefV2,
    value: toml::Value,
) -> Result<HostIngressRouteDeclaration, Error> {
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
    raw.into_declaration(&host_api.section)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHostIngressSection {
    transport: HostIngressTransport,
    policy: IngressPolicy,
    target: HostIngressTarget,
    auth: RawHostIngressAuth,
}

impl RawHostIngressSection {
    fn into_declaration(
        self,
        section: &ManifestSectionPath,
    ) -> Result<HostIngressRouteDeclaration, Error> {
        let auth = self.auth.into_binding(section)?;
        let HostIngressTransport::Webhook {
            route_id,
            method,
            path,
            ack,
            drain,
        } = self.transport;
        let descriptor =
            IngressRouteDescriptor::new(route_id.as_str(), method, path.as_str(), self.policy)
                .map_err(|source| Error::RouteDescriptorBuild {
                    section: section.clone(),
                    source,
                })?;
        HostIngressRouteDeclaration::new(descriptor, self.target, vec![auth], ack, drain).map_err(
            |source| Error::DeclarationValidation {
                section: section.clone(),
                source,
            },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum HostIngressTransport {
    Webhook {
        route_id: IngressRouteId,
        method: NetworkMethod,
        path: IngressRoutePattern,
        ack: IngressAckMode,
        drain: IngressDrainMode,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHostIngressAuth {
    verifier: IngressAuthScheme,
    credential_handles: Vec<IngressCredentialHandle>,
}

impl RawHostIngressAuth {
    fn into_binding(self, section: &ManifestSectionPath) -> Result<IngressAuthBinding, Error> {
        IngressAuthBinding::new(self.verifier, self.credential_handles).map_err(|source| {
            Error::DeclarationValidation {
                section: section.clone(),
                source,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU32, NonZeroU64};

    use chrono::Utc;
    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionCredentialBinding, ExtensionCredentialHandle,
        ExtensionInstallationId, ExtensionManifestRef, InMemoryExtensionInstallationStore,
        MANIFEST_SCHEMA_VERSION,
    };
    use ironclaw_host_api::{
        AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CapabilityId, CorsPolicy,
        IngressAckMode, IngressAuthPolicy, IngressAuthScheme, IngressDrainMode, IngressScopeSource,
        ListenerClass, NetworkMethod, RateLimitPolicy, RateLimitScope, SecretHandle, StreamingMode,
        WebSocketOriginPolicy,
    };

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

[host_ingress.events.transport]
kind = "webhook"
route_id = "slack.events"
method = "post"
path = "/webhooks/slack/events"
ack = "immediate"
drain = "drain_before_runtime_shutdown"

[host_ingress.events.policy]
listener_class = "public_webhook"
scope_source = "host_resolved"
cors = "not_applicable"
websocket_origin = "not_applicable"
streaming = "none"
audit = "public_callback"

[host_ingress.events.policy.auth]
type = "required"
schemes = ["webhook_signature"]

[host_ingress.events.policy.body_limit]
type = "limited"
max_bytes = 1048576

[host_ingress.events.policy.rate_limit]
type = "limited"
scope = "global"
max_requests = 12000
window_seconds = 60

[host_ingress.events.policy.effect_path]
type = "product_workflow"

[host_ingress.events.target]
type = "product_adapter_inbound"
capability_id = "slack.events"
product_adapter_section = "product_adapter.inbound"

[host_ingress.events.auth]
verifier = "webhook_signature"
credential_handles = ["slack_signing_secret"]

{extra}
"#,
            schema = MANIFEST_SCHEMA_VERSION,
        )
    }

    fn telegram_manifest(extra: &str) -> String {
        format!(
            r#"
schema_version = "{schema}"
id = "telegram"
name = "Telegram"
version = "0.1.0"
description = "Telegram product adapter"
trust = "third_party"

[runtime]
kind = "wasm"
module = "adapters/telegram.wasm"

[[host_api]]
id = "ironclaw.host_ingress/v1"
section = "host_ingress.updates"

[host_ingress.updates]

[host_ingress.updates.transport]
kind = "webhook"
route_id = "telegram.updates"
method = "post"
path = "/webhooks/telegram/updates"
ack = "immediate"
drain = "drain_before_runtime_shutdown"

[host_ingress.updates.policy]
listener_class = "public_webhook"
scope_source = "host_resolved"
cors = "not_applicable"
websocket_origin = "not_applicable"
streaming = "none"
audit = "public_callback"

[host_ingress.updates.policy.auth]
type = "required"
schemes = ["shared_secret_header"]

[host_ingress.updates.policy.body_limit]
type = "limited"
max_bytes = 1048576

[host_ingress.updates.policy.rate_limit]
type = "limited"
scope = "global"
max_requests = 12000
window_seconds = 60

[host_ingress.updates.policy.effect_path]
type = "product_workflow"

[host_ingress.updates.target]
type = "product_adapter_inbound"
capability_id = "telegram.updates"
product_adapter_section = "product_adapter.inbound"

[host_ingress.updates.auth]
verifier = "shared_secret_header"
credential_handles = ["telegram_webhook_secret"]

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

    fn host_ingress_ref(section: &str) -> HostApiRefV2 {
        HostApiRefV2 {
            id: HostApiId::new(HOST_INGRESS_HOST_API_ID).expect("test host API id must be valid"),
            section: ManifestSectionPath::new(section).expect("test section path must be valid"),
        }
    }

    fn project_single_section(
        raw: &str,
        section: &str,
    ) -> Result<HostIngressRouteDeclaration, Error> {
        let root: toml::Value = toml::from_str(raw).expect("test TOML must parse");
        let host_api = host_ingress_ref(section);
        let value = section_value(&root, &host_api.section)
            .expect("test host ingress section must exist")
            .clone();
        project_host_ingress_section(&host_api, value)
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

    fn assert_public_webhook_policy(
        policy: &IngressPolicy,
        expected_schemes: Vec<IngressAuthScheme>,
    ) {
        assert_eq!(policy.listener_class(), ListenerClass::PublicWebhook);
        assert_eq!(
            policy.auth(),
            &IngressAuthPolicy::Required {
                schemes: expected_schemes,
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
    }

    #[test]
    fn slack_events_manifest_projects_policy_from_manifest() {
        let declaration = project_single_section(&slack_manifest(""), "host_ingress.events")
            .expect("Slack events manifest policy must project");

        assert_eq!(
            declaration.route().route_pattern().as_str(),
            "/webhooks/slack/events"
        );
        assert_public_webhook_policy(
            declaration.route().policy(),
            vec![IngressAuthScheme::WebhookSignature],
        );
    }

    #[test]
    fn telegram_updates_manifest_projects_policy_from_manifest() {
        let declaration = project_single_section(&telegram_manifest(""), "host_ingress.updates")
            .expect("Telegram updates manifest policy must project");

        assert_eq!(
            declaration.route().route_pattern().as_str(),
            "/webhooks/telegram/updates"
        );
        assert_public_webhook_policy(
            declaration.route().policy(),
            vec![IngressAuthScheme::SharedSecretHeader],
        );
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
        assert_eq!(
            declaration.auth()[0].verifier(),
            IngressAuthScheme::WebhookSignature
        );
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
    fn unknown_policy_enum_value_is_rejected() {
        let raw = slack_manifest("").replace(
            r#"listener_class = "public_webhook""#,
            r#"listener_class = "public_webhooktypo""#,
        );
        let error = project_single_section(&raw, "host_ingress.events")
            .expect_err("unknown policy enum value must reject");

        assert!(matches!(
            error,
            Error::ManifestSectionParse { ref reason, .. } if reason.contains("public_webhooktypo")
        ));
    }

    #[test]
    fn unknown_auth_verifier_value_is_rejected_at_parse_time() {
        let raw = slack_manifest("").replace(
            r#"verifier = "webhook_signature""#,
            r#"verifier = "webhook_signature_typo""#,
        );
        let error = project_single_section(&raw, "host_ingress.events")
            .expect_err("unknown auth verifier enum value must reject");

        assert!(matches!(
            error,
            Error::ManifestSectionParse { ref reason, .. }
                if reason.contains("webhook_signature_typo")
        ));
    }

    #[test]
    fn auth_binding_verifier_must_be_allowed_by_policy() {
        let raw = slack_manifest("").replace(
            r#"verifier = "webhook_signature""#,
            r#"verifier = "shared_secret_header""#,
        );
        let error = project_single_section(&raw, "host_ingress.events")
            .expect_err("auth verifier outside policy scheme list must reject");

        assert!(matches!(
            error,
            Error::DeclarationValidation {
                source: HostIngressDeclarationError::AuthBindingVerifierNotDeclared {
                    verifier: IngressAuthScheme::SharedSecretHeader
                },
                ..
            }
        ));
    }

    #[test]
    fn telegram_auth_binding_verifier_must_not_fall_back_to_webhook_signature() {
        let raw = telegram_manifest("").replace(
            r#"verifier = "shared_secret_header""#,
            r#"verifier = "webhook_signature""#,
        );
        let error = project_single_section(&raw, "host_ingress.updates")
            .expect_err("telegram verifier outside policy scheme list must reject");

        assert!(matches!(
            error,
            Error::DeclarationValidation {
                source: HostIngressDeclarationError::AuthBindingVerifierNotDeclared {
                    verifier: IngressAuthScheme::WebhookSignature
                },
                ..
            }
        ));
    }

    #[test]
    fn unsupported_transport_kind_is_rejected_at_parse_time() {
        let raw = slack_manifest("").replace(r#"kind = "webhook""#, r#"kind = "websocket""#);
        let error = project_single_section(&raw, "host_ingress.events")
            .expect_err("unsupported transport kind must reject");

        assert!(matches!(
            error,
            Error::ManifestSectionParse { ref reason, .. } if reason.contains("websocket")
        ));
    }

    #[test]
    fn zero_policy_limit_is_rejected() {
        let raw = slack_manifest("").replace("max_requests = 12000", "max_requests = 0");
        let error = project_single_section(&raw, "host_ingress.events")
            .expect_err("zero rate limit must reject");

        assert!(matches!(
            error,
            Error::ManifestSectionParse { ref reason, .. } if reason.contains("nonzero")
        ));
    }

    #[test]
    fn immediate_ack_with_no_drain_surfaces_host_api_validation_error() {
        let raw = slack_manifest("").replace(
            r#"drain = "drain_before_runtime_shutdown""#,
            r#"drain = "none""#,
        );
        let error = project_single_section(&raw, "host_ingress.events")
            .expect_err("immediate ack without drain must reject");

        assert!(matches!(
            error,
            Error::DeclarationValidation {
                source: HostIngressDeclarationError::ImmediateAckRequiresDrain,
                ..
            }
        ));
    }
}
