use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, VerifyingKey};
use ironclaw_host_api::{NetworkPolicy, NetworkScheme, NetworkTargetPattern, sha256_digest_token};
use ironclaw_product_workflow::{LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef};

use super::errors::{catalog_error, invalid_input, product_error};
use super::model::{
    IronHubArtifact, IronHubCommandError, IronHubEntryKind, IronHubInstallOptions, IronHubManifest,
    IronHubProvenance, IronHubSkillEntry, IronHubToolEntry, SignedManifestEnvelope,
};

#[cfg(not(test))]
pub(super) fn verify_signed_manifest(envelope_bytes: &[u8]) -> Result<Vec<u8>, String> {
    verify_signed_manifest_with_keys(envelope_bytes, super::model::MANIFEST_VERIFY_KEYS)
}

pub(super) fn verify_signed_manifest_with_keys(
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

pub(super) fn classify_gate_and_digest(
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

pub(super) fn classify(
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

pub(super) fn tool_artifact_digest(entry: &IronHubToolEntry) -> String {
    sha256_digest_token(format!("{}:{}", entry.wasm.sha256, entry.capabilities.sha256).as_bytes())
}

fn skill_artifact_digest(entry: &IronHubSkillEntry) -> String {
    sha256_digest_token(entry.skill_md.sha256.as_bytes())
}

pub(super) fn validate_artifact(
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

pub(super) fn validate_artifact_url(
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

pub(super) fn network_policy_for_url(
    url: &str,
    max_bytes: u64,
) -> Result<NetworkPolicy, IronHubCommandError> {
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

pub(super) fn host_is_disallowed_target(host: &str) -> bool {
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

pub(super) fn validate_hub_name(name: &str) -> Result<(), IronHubCommandError> {
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

pub(super) fn tool_summary(
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

pub(super) fn skill_summary(
    entry: &IronHubSkillEntry,
) -> Result<ironclaw_product_workflow::LifecycleSkillSummary, IronHubCommandError> {
    Ok(ironclaw_product_workflow::LifecycleSkillSummary {
        name: LifecyclePackageId::new(entry.name.clone()).map_err(product_error)?,
        version: entry.version.clone(),
        description: format!("{} [{}]", entry.description, entry.provenance.trust_label()),
        source: ironclaw_product_workflow::LifecycleSkillSource::Registry,
        keywords: Vec::new(),
        tags: Vec::new(),
        requires_skills: Vec::new(),
    })
}

pub(super) fn entry_matches(name: &str, description: &str, query: &str) -> bool {
    query.is_empty()
        || name.to_ascii_lowercase().contains(query)
        || description.to_ascii_lowercase().contains(query)
}

pub(super) fn package_ref(
    kind: LifecyclePackageKind,
    id: &str,
) -> Result<LifecyclePackageRef, IronHubCommandError> {
    LifecyclePackageRef::new(kind, id).map_err(product_error)
}
