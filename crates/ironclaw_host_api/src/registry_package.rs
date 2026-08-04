//! Validated provenance for packages installed from a remote registry.
//!
//! The receipt is shared by extension and skill installation stores. It is
//! immutable package identity metadata, not execution authority: consumers
//! must still apply their owning lifecycle, trust, and authorization policy.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, de};

use crate::error::HostApiError;

// Keep the complete receipt below the skill sidecar's 4 KiB bound even when
// every free-form catalog identity field is at its maximum.
const REGISTRY_RECEIPT_FIELD_MAX_BYTES: usize = 512;

/// Host-validated provenance persisted with a package installed from a
/// registry. The receipt deliberately contains only immutable catalog
/// identity and digests; signed/private download URLs and credentials never
/// cross this boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegistryPackageProvenance {
    registry: String,
    repository: String,
    package_version: String,
    release_tag: String,
    catalog_origin: String,
    artifact_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest_digest: Option<String>,
    installed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryPackageProvenanceParts {
    pub registry: String,
    pub repository: String,
    pub package_version: String,
    pub release_tag: String,
    pub catalog_origin: String,
    pub artifact_digest: String,
    pub manifest_digest: Option<String>,
    pub installed_at: DateTime<Utc>,
}

impl RegistryPackageProvenance {
    pub fn new(parts: RegistryPackageProvenanceParts) -> Result<Self, HostApiError> {
        let registry = validate_registry_receipt_field("registry", parts.registry)?;
        let repository = validate_registry_receipt_field("repository", parts.repository)?;
        let package_version =
            validate_registry_receipt_field("package_version", parts.package_version)?;
        let release_tag = validate_registry_receipt_field("release_tag", parts.release_tag)?;
        let catalog_origin =
            validate_registry_receipt_field("catalog_origin", parts.catalog_origin)?;
        let catalog_authority = catalog_origin
            .strip_prefix("https://")
            .and_then(|value| value.strip_suffix('/').or(Some(value)));
        if !catalog_origin.starts_with("https://")
            || catalog_origin.contains('@')
            || catalog_origin.contains('?')
            || catalog_origin.contains('#')
            || catalog_authority.is_none_or(|authority| {
                authority.is_empty()
                    || authority.contains('/')
                    || !authority
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b".:-[]".contains(&byte))
            })
        {
            return Err(HostApiError::InvariantViolation {
                reason: "registry catalog origin must be a redacted HTTPS origin".to_string(),
            });
        }
        let artifact_digest = validate_registry_digest("artifact_digest", parts.artifact_digest)?;
        let manifest_digest = parts
            .manifest_digest
            .map(|digest| validate_registry_digest("manifest_digest", digest))
            .transpose()?;
        Ok(Self {
            registry,
            repository,
            package_version,
            release_tag,
            catalog_origin,
            artifact_digest,
            manifest_digest,
            installed_at: parts.installed_at,
        })
    }

    pub fn registry(&self) -> &str {
        &self.registry
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn package_version(&self) -> &str {
        &self.package_version
    }

    pub fn release_tag(&self) -> &str {
        &self.release_tag
    }

    pub fn catalog_origin(&self) -> &str {
        &self.catalog_origin
    }

    pub fn artifact_digest(&self) -> &str {
        &self.artifact_digest
    }

    pub fn manifest_digest(&self) -> Option<&str> {
        self.manifest_digest.as_deref()
    }

    pub fn installed_at(&self) -> DateTime<Utc> {
        self.installed_at
    }

    /// Whether two receipts identify the same registry package artifact.
    ///
    /// Installation time is lifecycle history, not package identity, so it is
    /// deliberately excluded from this comparison.
    pub fn same_package_identity(&self, other: &Self) -> bool {
        self.registry == other.registry
            && self.repository == other.repository
            && self.package_version == other.package_version
            && self.release_tag == other.release_tag
            && self.catalog_origin == other.catalog_origin
            && self
                .artifact_digest
                .eq_ignore_ascii_case(&other.artifact_digest)
            && self.manifest_digest == other.manifest_digest
    }
}

fn validate_registry_digest(field: &'static str, value: String) -> Result<String, HostApiError> {
    let value = validate_registry_receipt_field(field, value)?;
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(HostApiError::InvariantViolation {
            reason: format!("registry {field} must be a sha256 digest"),
        });
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(HostApiError::InvariantViolation {
            reason: format!("registry {field} must contain 64 hexadecimal characters"),
        });
    }
    Ok(format!("sha256:{}", hex.to_ascii_lowercase()))
}

fn validate_registry_receipt_field(
    label: &'static str,
    value: String,
) -> Result<String, HostApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || value.len() > REGISTRY_RECEIPT_FIELD_MAX_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(HostApiError::InvariantViolation {
            reason: format!("registry package provenance contains an invalid {label}"),
        });
    }
    Ok(trimmed.to_string())
}

#[derive(Deserialize)]
struct RegistryPackageProvenanceWire {
    registry: String,
    repository: String,
    package_version: String,
    release_tag: String,
    catalog_origin: String,
    artifact_digest: String,
    #[serde(default)]
    manifest_digest: Option<String>,
    installed_at: DateTime<Utc>,
}

impl<'de> Deserialize<'de> for RegistryPackageProvenance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = RegistryPackageProvenanceWire::deserialize(deserializer)?;
        Self::new(RegistryPackageProvenanceParts {
            registry: wire.registry,
            repository: wire.repository,
            package_version: wire.package_version,
            release_tag: wire.release_tag,
            catalog_origin: wire.catalog_origin,
            artifact_digest: wire.artifact_digest,
            manifest_digest: wire.manifest_digest,
            installed_at: wire.installed_at,
        })
        .map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_or_non_https_origins() {
        for origin in [
            "http://hub.ironclaw.com/",
            "https://token@hub.ironclaw.com/",
            "https://hub.ironclaw.com/?token=secret",
            "https://hub.ironclaw.com/private/manifest",
        ] {
            let error = RegistryPackageProvenance::new(RegistryPackageProvenanceParts {
                registry: "ironhub".to_string(),
                repository: "nearai/ironhub".to_string(),
                package_version: "1.0.0".to_string(),
                release_tag: "v1".to_string(),
                catalog_origin: origin.to_string(),
                artifact_digest: "sha256:abc".to_string(),
                manifest_digest: None,
                installed_at: Utc::now(),
            })
            .expect_err("unsafe origin must be rejected");
            assert!(error.to_string().contains("redacted HTTPS origin"));
        }
    }

    #[test]
    fn rejects_non_sha256_digests() {
        let error = RegistryPackageProvenance::new(RegistryPackageProvenanceParts {
            registry: "ironhub".to_string(),
            repository: "nearai/ironhub".to_string(),
            package_version: "1.0.0".to_string(),
            release_tag: "v1".to_string(),
            catalog_origin: "https://hub.ironclaw.com/".to_string(),
            artifact_digest: "sha256:not-a-digest".to_string(),
            manifest_digest: None,
            installed_at: Utc::now(),
        })
        .expect_err("unusable digest must be rejected");

        assert!(error.to_string().contains("64 hexadecimal"));
    }

    #[test]
    fn deserialization_revalidates_hostile_json() {
        let valid = serde_json::json!({
            "registry": "ironhub",
            "repository": "nearai/ironhub",
            "package_version": "1.0.0",
            "release_tag": "v1",
            "catalog_origin": "https://hub.ironclaw.com/",
            "artifact_digest": format!("sha256:{}", "a".repeat(64)),
            "manifest_digest": format!("sha256:{}", "b".repeat(64)),
            "installed_at": Utc::now(),
        });
        for (field, hostile) in [
            ("registry", String::new()),
            ("registry", "iron\nHub".to_string()),
            (
                "repository",
                "x".repeat(REGISTRY_RECEIPT_FIELD_MAX_BYTES + 1),
            ),
            (
                "catalog_origin",
                "https://token@hub.ironclaw.com/".to_string(),
            ),
            ("manifest_digest", "sha256:bad".to_string()),
        ] {
            let mut candidate = valid.clone();
            candidate[field] = serde_json::Value::String(hostile);
            serde_json::from_value::<RegistryPackageProvenance>(candidate)
                .expect_err("hostile persisted receipt must be rejected on deserialize");
        }
    }

    #[test]
    fn legacy_receipt_without_manifest_digest_round_trips_with_stable_field_names() {
        let legacy = serde_json::json!({
            "registry": "ironhub",
            "repository": "nearai/ironhub",
            "package_version": "1.2.3",
            "release_tag": "v1.2.3",
            "catalog_origin": "https://hub.ironclaw.com",
            "artifact_digest": format!("sha256:{}", "a".repeat(64)),
            "installed_at": "2026-08-03T00:00:00Z",
        });
        let receipt: RegistryPackageProvenance =
            serde_json::from_value(legacy.clone()).expect("legacy receipt is accepted");
        let serialized = serde_json::to_value(receipt).expect("serialize receipt");

        assert_eq!(serialized, legacy);
        assert!(serialized.get("manifest_digest").is_none());
    }
}
