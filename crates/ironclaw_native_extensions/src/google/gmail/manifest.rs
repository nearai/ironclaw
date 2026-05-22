//! HostBundled `ExtensionPackage` declaring the six Gmail capabilities.
//!
//! The manifest is descriptor-only: it carries the capability ids, effects,
//! and permission modes that the host authorization layer reads. The actual
//! handler implementations live in [`super::handlers`] and are registered as
//! first-party capability handlers separately.

use ironclaw_extensions::{
    CapabilityManifest, CapabilityVisibility, ExtensionError, ExtensionManifest, ExtensionPackage,
    ExtensionRuntime, MANIFEST_SCHEMA_VERSION, ManifestSource,
};
use ironclaw_host_api::{
    CapabilityId, CapabilityProfileSchemaRef, EffectKind, ExtensionId, PermissionMode,
    RequestedTrustClass, TrustClass, VirtualPath,
};

/// User-facing installed extension id. Capability ids must be prefixed with
/// `"{EXTENSION_ID}."` or [`ExtensionPackage::from_manifest`] rejects them.
pub const GMAIL_EXTENSION_ID: &str = "gmail";

/// First-party runtime service name carried in the manifest.
pub const GMAIL_SERVICE: &str = "gmail";

/// Effects of a read-only Gmail capability.
fn read_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
    ]
}

/// Effects of a write Gmail capability — read effects plus `ExternalWrite`.
fn write_effects() -> Vec<EffectKind> {
    let mut effects = read_effects();
    effects.push(EffectKind::ExternalWrite);
    effects
}

/// Stable kind tag for a capability descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GmailCapabilityKind {
    /// Read capability: `PermissionMode::Allow`, no `ExternalWrite` effect.
    Read,
    /// Write capability: `PermissionMode::Ask` (RequiresApproval) plus
    /// `ExternalWrite`.
    Write,
}

impl GmailCapabilityKind {
    fn permission(self) -> PermissionMode {
        match self {
            Self::Read => PermissionMode::Allow,
            Self::Write => PermissionMode::Ask,
        }
    }

    fn effects(self) -> Vec<EffectKind> {
        match self {
            Self::Read => read_effects(),
            Self::Write => write_effects(),
        }
    }
}

/// One row of the Gmail capability table: `(short_name, description, kind)`.
/// The fully-qualified id is `"{GMAIL_EXTENSION_ID}.{short_name}"`.
pub const GMAIL_CAPABILITIES: &[(&str, &str, GmailCapabilityKind)] = &[
    (
        "list_messages",
        "List messages in the user's Gmail mailbox, optionally filtered by query.",
        GmailCapabilityKind::Read,
    ),
    (
        "get_message",
        "Fetch a single Gmail message by id.",
        GmailCapabilityKind::Read,
    ),
    (
        "send_message",
        "Compose and send a new Gmail message.",
        GmailCapabilityKind::Write,
    ),
    (
        "create_draft",
        "Create a draft Gmail message without sending it.",
        GmailCapabilityKind::Write,
    ),
    (
        "reply_to_message",
        "Reply to an existing Gmail message, preserving its thread.",
        GmailCapabilityKind::Write,
    ),
    (
        "trash_message",
        "Move a Gmail message to the trash.",
        GmailCapabilityKind::Write,
    ),
];

/// Fully-qualified capability id for a Gmail capability short name.
pub fn capability_id(short_name: &str) -> String {
    format!("{GMAIL_EXTENSION_ID}.{short_name}")
}

fn capability_manifest(
    short_name: &str,
    description: &str,
    kind: GmailCapabilityKind,
) -> Result<CapabilityManifest, ExtensionError> {
    Ok(CapabilityManifest {
        id: CapabilityId::new(capability_id(short_name))?,
        implements: Vec::new(),
        description: description.to_string(),
        effects: kind.effects(),
        default_permission: kind.permission(),
        visibility: CapabilityVisibility::Model,
        input_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/gmail/{short_name}.input.v1.json"
        ))?,
        output_schema_ref: CapabilityProfileSchemaRef::new(format!(
            "schemas/gmail/{short_name}.output.v1.json"
        ))?,
        prompt_doc_ref: Some(CapabilityProfileSchemaRef::new(format!(
            "prompts/gmail/{short_name}.md"
        ))?),
        required_host_ports: Vec::new(),
        resource_profile: None,
    })
}

/// Build the HostBundled `ExtensionPackage` declaring all six Gmail
/// capabilities.
pub fn gmail_package() -> Result<ExtensionPackage, ExtensionError> {
    let capabilities = GMAIL_CAPABILITIES
        .iter()
        .map(|(short_name, description, kind)| capability_manifest(short_name, description, *kind))
        .collect::<Result<Vec<_>, ExtensionError>>()?;
    ExtensionPackage::from_manifest(
        ExtensionManifest {
            schema_version: MANIFEST_SCHEMA_VERSION.to_string(),
            id: ExtensionId::new(GMAIL_EXTENSION_ID)?,
            name: "Gmail".to_string(),
            version: "0.1.0".to_string(),
            description: "First-party Gmail capabilities for Reborn.".to_string(),
            source: ManifestSource::HostBundled,
            requested_trust: RequestedTrustClass::FirstPartyRequested,
            descriptor_trust_default: TrustClass::Sandbox,
            runtime: ExtensionRuntime::FirstParty {
                service: GMAIL_SERVICE.to_string(),
            },
            host_apis: Vec::new(),
            capabilities,
        },
        VirtualPath::new(format!("/system/extensions/{GMAIL_EXTENSION_ID}"))?,
    )
}
