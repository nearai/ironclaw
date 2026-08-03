use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, VerifyingKey};
use ironclaw_host_api::{
    action::{NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    approval::sha256_digest_token,
};
use sha2::{Digest, Sha256};

use crate::ironhub::{
    artifact_hosts::is_allowed_artifact_host,
    model::{
        IronHubArtifact, IronHubCommandError, IronHubEntryKind, IronHubEntrySummary,
        IronHubInstallOptions, IronHubManifest, IronHubProvenance, IronHubSkillEntry,
        IronHubToolEntry, MANIFEST_VERIFY_KEYS, MAX_METADATA_BYTES, MAX_TOOL_SCHEMA_ARTIFACTS,
        MAX_WASM_BYTES, SignedManifestEnvelope,
    },
};

const MAX_SEARCH_DESCRIPTION_BYTES: usize = 120;
const SEARCH_DESCRIPTION_ELLIPSIS: char = '…';

pub(crate) fn verify_signed_manifest(envelope_bytes: &[u8]) -> Result<Vec<u8>, String> {
    verify_signed_manifest_with_keys(envelope_bytes, MANIFEST_VERIFY_KEYS)
}

pub(crate) fn verify_signed_manifest_with_keys(
    envelope_bytes: &[u8],
    verify_keys: &[(&str, &str)],
) -> Result<Vec<u8>, String> {
    let envelope: SignedManifestEnvelope = serde_json::from_slice(envelope_bytes)
        .map_err(|error| format!("envelope parse failed: {error}"))?;
    if envelope.v != 1 {
        return Err(format!(
            "unsupported signed-manifest version {}",
            envelope.v
        ));
    }
    let key_hex = verify_keys
        .iter()
        .find(|(id, _)| *id == envelope.key_id)
        .map(|(_, key)| *key)
        .ok_or_else(|| format!("unknown manifest signing key_id '{}'", envelope.key_id))?;
    let verifying_key = verifying_key_from_hex(key_hex)?;
    let manifest_bytes = URL_SAFE_NO_PAD
        .decode(envelope.manifest_b64.as_bytes())
        .map_err(|error| format!("manifest_b64 decode failed: {error}"))?;
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(envelope.sig.as_bytes())
        .map_err(|error| format!("signature decode failed: {error}"))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|error| format!("signature malformed: {error}"))?;
    verifying_key
        .verify_strict(&manifest_bytes, &signature)
        .map_err(|_| "manifest signature verification failed".to_string())?;
    Ok(manifest_bytes)
}

fn verifying_key_from_hex(value: &str) -> Result<VerifyingKey, String> {
    let raw =
        hex::decode(value).map_err(|error| format!("verify key is not valid hex: {error}"))?;
    let raw: [u8; 32] = raw
        .try_into()
        .map_err(|_| "verify key must be 32 bytes".to_string())?;
    VerifyingKey::from_bytes(&raw).map_err(|error| format!("invalid verify key: {error}"))
}

pub(crate) fn classify_gate_and_digest(
    manifest: &IronHubManifest,
    name: &str,
    hint: Option<IronHubEntryKind>,
    options: &IronHubInstallOptions,
    source: IronHubManifestSource,
) -> Result<(IronHubEntryKind, IronHubProvenance, String), IronHubCommandError> {
    let kind = classify(manifest, name, hint)?;
    let (version, provenance, artifact_digest) = match kind {
        IronHubEntryKind::Tool => {
            let entry = manifest
                .find_tool(name)
                .ok_or_else(|| catalog("tool not found"))?;
            (
                entry.version.as_str(),
                entry.provenance,
                tool_artifact_digest(entry),
            )
        }
        IronHubEntryKind::Skill => {
            let entry = manifest
                .find_skill(name)
                .ok_or_else(|| catalog("skill not found"))?;
            (
                entry.version.as_str(),
                entry.provenance,
                skill_artifact_digest(entry),
            )
        }
    };
    let provenance = if source == IronHubManifestSource::Private {
        IronHubProvenance::Private
    } else if matches!(provenance, IronHubProvenance::Private) {
        return Err(invalid(format!(
            "catalog entry '{name}' claims private provenance but was not installed from a private manifest"
        )));
    } else {
        provenance
    };
    if let Some(expected) = &options.expected_version
        && expected != version
    {
        return Err(invalid(format!(
            "catalog version for '{name}' changed: expected {expected}, current {version}"
        )));
    }
    if let Some(expected) = &options.expected_artifact_digest
        && !expected.eq_ignore_ascii_case(&artifact_digest)
    {
        return Err(invalid(format!(
            "artifact digest for '{name}' changed: expected {expected}, current {artifact_digest}"
        )));
    }
    if provenance.is_community_unverified() && !options.acknowledge_unverified {
        return Err(invalid(format!(
            "'{name}' is UNVERIFIED community content (trust tier: {}). Re-run with explicit acknowledgement to install at your own risk.",
            provenance.as_wire()
        )));
    }
    Ok((kind, provenance, artifact_digest))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IronHubManifestSource {
    Public,
    Private,
}

pub(crate) fn classify(
    manifest: &IronHubManifest,
    name: &str,
    hint: Option<IronHubEntryKind>,
) -> Result<IronHubEntryKind, IronHubCommandError> {
    let in_tools = manifest.find_tool(name).is_some();
    let in_skills = manifest.find_skill(name).is_some();
    match (hint, in_tools, in_skills) {
        (Some(IronHubEntryKind::Tool), true, _) => Ok(IronHubEntryKind::Tool),
        (Some(IronHubEntryKind::Tool), false, _) => {
            Err(invalid(format!("'{name}' is not a tool in this catalog")))
        }
        (Some(IronHubEntryKind::Skill), _, true) => Ok(IronHubEntryKind::Skill),
        (Some(IronHubEntryKind::Skill), _, false) => {
            Err(invalid(format!("'{name}' is not a skill in this catalog")))
        }
        (None, true, false) => Ok(IronHubEntryKind::Tool),
        (None, false, true) => Ok(IronHubEntryKind::Skill),
        (None, true, true) => Err(invalid(format!(
            "'{name}' exists as both a tool and a skill; specify a kind"
        ))),
        (None, false, false) => Err(invalid(format!("'{name}' is not in this catalog"))),
    }
}

pub(crate) fn tool_summary(entry: &IronHubToolEntry) -> IronHubEntrySummary {
    IronHubEntrySummary {
        kind: IronHubEntryKind::Tool,
        name: entry.name.clone(),
        version: entry.version.clone(),
        description: entry.description.clone(),
        provenance: entry.provenance,
        artifact_digest: Some(tool_artifact_digest(entry)),
    }
}

pub(crate) fn skill_summary(entry: &IronHubSkillEntry) -> IronHubEntrySummary {
    IronHubEntrySummary {
        kind: IronHubEntryKind::Skill,
        name: entry.name.clone(),
        version: entry.version.clone(),
        description: entry.description.clone(),
        provenance: entry.provenance,
        artifact_digest: Some(skill_artifact_digest(entry)),
    }
}

pub(crate) fn compact_tool_summary(entry: &IronHubToolEntry) -> IronHubEntrySummary {
    IronHubEntrySummary {
        kind: IronHubEntryKind::Tool,
        name: entry.name.clone(),
        version: entry.version.clone(),
        description: compact_description(&entry.description),
        provenance: entry.provenance,
        artifact_digest: None,
    }
}

pub(crate) fn compact_skill_summary(entry: &IronHubSkillEntry) -> IronHubEntrySummary {
    IronHubEntrySummary {
        kind: IronHubEntryKind::Skill,
        name: entry.name.clone(),
        version: entry.version.clone(),
        description: compact_description(&entry.description),
        provenance: entry.provenance,
        artifact_digest: None,
    }
}

pub(crate) fn tool_artifact_digest(entry: &IronHubToolEntry) -> String {
    let mut digest_material = format!(
        "wasm:{}\0capabilities:{}\0",
        entry.wasm.sha256, entry.capabilities.sha256
    );
    if let Some(manifest) = &entry.manifest {
        digest_material.push_str("manifest:");
        digest_material.push_str(&manifest.sha256);
        digest_material.push('\0');
    }
    for (path, artifact) in &entry.schemas {
        digest_material.push_str("schema:");
        digest_material.push_str(path);
        digest_material.push('\0');
        digest_material.push_str(&artifact.sha256);
        digest_material.push('\0');
    }
    sha256_digest_token(digest_material.as_bytes())
}

fn skill_artifact_digest(entry: &IronHubSkillEntry) -> String {
    sha256_digest_token(entry.skill_md.sha256.as_bytes())
}

fn compact_description(description: &str) -> String {
    if description.len() <= MAX_SEARCH_DESCRIPTION_BYTES {
        return description.to_string();
    }

    let mut summary = String::new();
    for character in description.chars() {
        if summary.len() + character.len_utf8() + SEARCH_DESCRIPTION_ELLIPSIS.len_utf8()
            > MAX_SEARCH_DESCRIPTION_BYTES
        {
            break;
        }
        summary.push(character);
    }
    summary.push(SEARCH_DESCRIPTION_ELLIPSIS);
    summary
}

pub(crate) fn validate_manifest(manifest: &IronHubManifest) -> Result<(), IronHubCommandError> {
    validate_manifest_artifacts(manifest, None)
}

pub(crate) fn validate_private_manifest(
    manifest: &IronHubManifest,
    origin: &CatalogOrigin,
) -> Result<(), IronHubCommandError> {
    validate_manifest_artifacts(manifest, Some(origin))
}

fn validate_manifest_artifacts(
    manifest: &IronHubManifest,
    origin: Option<&CatalogOrigin>,
) -> Result<(), IronHubCommandError> {
    if manifest.version != "1" {
        return Err(catalog(format!(
            "unsupported IronHub manifest version {}",
            manifest.version
        )));
    }
    if manifest.release_tag.trim().is_empty() || manifest.repo.trim().is_empty() {
        return Err(catalog("manifest release_tag and repo must be non-empty"));
    }
    for entry in &manifest.tools {
        validate_hub_name(&entry.name)?;
        validate_artifact_for_origin(&entry.wasm, MAX_WASM_BYTES, origin)?;
        validate_artifact_for_origin(&entry.capabilities, MAX_METADATA_BYTES, origin)?;
        if let Some(extension_manifest) = &entry.manifest {
            validate_artifact_for_origin(extension_manifest, MAX_METADATA_BYTES, origin)?;
        }
        if entry.schemas.len() > MAX_TOOL_SCHEMA_ARTIFACTS {
            return Err(catalog(format!(
                "tool '{}' publishes more than {} schema artifacts",
                entry.name, MAX_TOOL_SCHEMA_ARTIFACTS
            )));
        }
        for (path, schema) in &entry.schemas {
            ironclaw_extensions::ExtensionAssetPath::new(path.clone()).map_err(|error| {
                catalog(format!(
                    "tool '{}' publishes an invalid schema path: {error}",
                    entry.name
                ))
            })?;
            validate_artifact_for_origin(schema, MAX_METADATA_BYTES, origin)?;
        }
    }
    for entry in &manifest.skills {
        validate_hub_name(&entry.name)?;
        validate_artifact_for_origin(&entry.skill_md, MAX_METADATA_BYTES, origin)?;
    }
    Ok(())
}

pub(crate) fn validate_artifact_for_origin(
    artifact: &IronHubArtifact,
    max_bytes: u64,
    origin: Option<&CatalogOrigin>,
) -> Result<(), IronHubCommandError> {
    validate_artifact_url_for_origin("artifact", "url", &artifact.url, origin)?;
    if artifact.size_bytes > max_bytes {
        return Err(catalog(format!("artifact exceeds {max_bytes} byte cap")));
    }
    if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(catalog("artifact sha256 must be 64 hex characters"));
    }
    Ok(())
}

pub(crate) fn validate_artifact_url(
    manifest_name: &str,
    field: &str,
    value: &str,
) -> Result<(), IronHubCommandError> {
    validate_artifact_url_for_origin(manifest_name, field, value, None)
}

fn validate_artifact_url_for_origin(
    manifest_name: &str,
    field: &str,
    value: &str,
    origin: Option<&CatalogOrigin>,
) -> Result<(), IronHubCommandError> {
    let parsed = url::Url::parse(value)
        .map_err(|error| catalog(format!("{manifest_name}.{field} invalid URL: {error}")))?;
    if parsed.scheme() != "https" {
        return Err(catalog(format!("{manifest_name}.{field} must use https")));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| catalog(format!("{manifest_name}.{field} host is missing")))?;
    let allowed = match origin {
        Some(origin) => origin.matches(&parsed),
        None => is_allowed_artifact_host(host),
    };
    if host_is_disallowed_target(host) || !allowed {
        return Err(catalog(format!(
            "{manifest_name}.{field} host '{host}' is not allowed"
        )));
    }
    Ok(())
}

pub(crate) fn network_policy_for_url_from_origin(
    value: &str,
    max_bytes: u64,
    origin: Option<&CatalogOrigin>,
) -> Result<NetworkPolicy, IronHubCommandError> {
    validate_artifact_url_for_origin("download", "url", value, origin)?;
    let parsed =
        url::Url::parse(value).map_err(|error| catalog(format!("invalid URL: {error}")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| catalog("URL host is missing"))?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CatalogOrigin {
    host: String,
    port: u16,
}

impl CatalogOrigin {
    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    pub(crate) fn redacted_source_url(&self) -> String {
        if self.port == 443 {
            format!("https://{}/", self.host)
        } else {
            format!("https://{}:{}/", self.host, self.port)
        }
    }

    fn from_url(value: &str, label: &str) -> Result<Self, IronHubCommandError> {
        let parsed = url::Url::parse(value)
            .map_err(|error| catalog(format!("{label} is not a valid URL: {error}")))?;
        if parsed.scheme() != "https" {
            return Err(catalog(format!("{label} must use https")));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(catalog(format!(
                "{label} must not contain user information"
            )));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| catalog(format!("{label} host is missing")))?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if host_is_disallowed_target(&host) {
            return Err(catalog(format!("{label} host is not allowed")));
        }
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| catalog(format!("{label} port is invalid")))?;
        Ok(Self { host, port })
    }

    fn matches(&self, parsed: &url::Url) -> bool {
        parsed.scheme() == "https"
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed
                .host_str()
                .map(|host| host.trim_end_matches('.').eq_ignore_ascii_case(&self.host))
                .unwrap_or(false)
            && parsed.port_or_known_default() == Some(self.port)
    }
}

pub(crate) fn validate_private_manifest_origin(
    configured_catalog_url: &str,
    private_manifest_url: &str,
) -> Result<CatalogOrigin, IronHubCommandError> {
    let configured = CatalogOrigin::from_url(configured_catalog_url, "configured catalog URL")?;
    let private = url::Url::parse(private_manifest_url)
        .map_err(|_| catalog("private manifest URL is invalid"))?;
    if !configured.matches(&private) {
        return Err(catalog(
            "private manifest URL must use the configured catalog origin",
        ));
    }
    Ok(configured)
}

pub(crate) fn host_is_disallowed_target(host: &str) -> bool {
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

pub(crate) fn validate_hub_name(name: &str) -> Result<(), IronHubCommandError> {
    let valid = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_');
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "name must be 1-128 bytes and contain only lowercase letters, digits, '-', '_'",
        ))
    }
}

pub(crate) fn entry_matches(name: &str, description: &str, query: &str) -> bool {
    query.is_empty()
        || name.to_ascii_lowercase().contains(query)
        || description.to_ascii_lowercase().contains(query)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn invalid(reason: impl Into<String>) -> IronHubCommandError {
    IronHubCommandError::InvalidInput {
        reason: reason.into(),
    }
}

pub(crate) fn catalog(reason: impl Into<String>) -> IronHubCommandError {
    IronHubCommandError::Catalog {
        reason: reason.into(),
    }
}
