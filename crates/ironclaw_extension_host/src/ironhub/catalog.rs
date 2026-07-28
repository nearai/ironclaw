use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, VerifyingKey};
use ironclaw_host_api::{NetworkPolicy, NetworkScheme, NetworkTargetPattern};
use sha2::{Digest, Sha256};

use super::model::{
    IronHubArtifact, IronHubCommandError, IronHubEntryKind, IronHubEntrySummary,
    IronHubInstallOptions, IronHubManifest, IronHubProvenance, IronHubSkillEntry, IronHubToolEntry,
    SignedManifestEnvelope,
};

const MAX_SEARCH_DESCRIPTION_BYTES: usize = 120;
const SEARCH_DESCRIPTION_ELLIPSIS: char = '…';

pub(crate) fn verify_signed_manifest(envelope_bytes: &[u8]) -> Result<Vec<u8>, String> {
    verify_signed_manifest_with_keys(envelope_bytes, super::model::MANIFEST_VERIFY_KEYS)
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
    sha256_hex(format!("{}:{}", entry.wasm.sha256, entry.capabilities.sha256).as_bytes())
}

fn skill_artifact_digest(entry: &IronHubSkillEntry) -> String {
    sha256_hex(entry.skill_md.sha256.as_bytes())
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
        validate_artifact(&entry.wasm, super::model::MAX_WASM_BYTES)?;
        validate_artifact(&entry.capabilities, super::model::MAX_METADATA_BYTES)?;
    }
    for entry in &manifest.skills {
        validate_hub_name(&entry.name)?;
        validate_artifact(&entry.skill_md, super::model::MAX_METADATA_BYTES)?;
    }
    Ok(())
}

pub(crate) fn validate_artifact(
    artifact: &IronHubArtifact,
    max_bytes: u64,
) -> Result<(), IronHubCommandError> {
    validate_artifact_url("artifact", "url", &artifact.url)?;
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
    let parsed = url::Url::parse(value)
        .map_err(|error| catalog(format!("{manifest_name}.{field} invalid URL: {error}")))?;
    if parsed.scheme() != "https" {
        return Err(catalog(format!("{manifest_name}.{field} must use https")));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| catalog(format!("{manifest_name}.{field} host is missing")))?;
    if host_is_disallowed_target(host) || !is_allowed_artifact_host(host) {
        return Err(catalog(format!(
            "{manifest_name}.{field} host '{host}' is not allowed"
        )));
    }
    Ok(())
}

pub(crate) fn network_policy_for_url(
    value: &str,
    max_bytes: u64,
) -> Result<NetworkPolicy, IronHubCommandError> {
    validate_artifact_url("download", "url", value)?;
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

fn is_allowed_artifact_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("hub.ironclaw.com")
        || ironclaw_host_runtime::is_allowed_code_artifact_host(host)
        || extra_artifact_hosts()
            .iter()
            .any(|allowed| host.eq_ignore_ascii_case(allowed))
}

fn extra_artifact_hosts() -> Vec<String> {
    std::env::var("IRONHUB_EXTRA_ARTIFACT_HOSTS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|host| !host.is_empty() && !host_is_disallowed_target(host))
        .collect()
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
