use ironclaw_host_api::registry_package::RegistryPackageProvenance;
use serde::{Deserialize, Serialize};

pub const INSTALL_METADATA_FILE_NAME: &str = ".ironclaw-install.json";
pub const MAX_INSTALL_METADATA_BYTES: usize = 4096;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledSkillMetadata {
    #[serde(default)]
    pub source: Option<InstalledSkillMetadataSource>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub source_subdir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_provenance: Option<RegistryPackageProvenance>,
}

impl InstalledSkillMetadata {
    pub fn installed_url(source_url: Option<&str>) -> Self {
        Self {
            source: Some(InstalledSkillMetadataSource::InstalledUrl),
            source_url: source_url.map(str::to_string),
            source_subdir: None,
            registry_provenance: None,
        }
    }

    pub fn installed_registry(
        source_url: Option<&str>,
        provenance: RegistryPackageProvenance,
    ) -> Self {
        Self {
            source: Some(InstalledSkillMetadataSource::InstalledUrl),
            source_url: source_url.map(str::to_string),
            source_subdir: None,
            registry_provenance: Some(provenance),
        }
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }

    pub fn sidecar_bytes_mark_installed(bytes: &[u8]) -> bool {
        let Ok(metadata) = serde_json::from_slice::<Self>(bytes) else {
            return true;
        };
        match metadata.source {
            Some(InstalledSkillMetadataSource::InstalledUrl) | None => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstalledSkillMetadataSource {
    InstalledUrl,
}

impl InstalledSkillMetadataSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InstalledUrl => "installed_url",
        }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::registry_package::RegistryPackageProvenance;

    use super::InstalledSkillMetadata;

    #[test]
    fn registry_receipt_round_trips_in_the_install_sidecar() {
        let provenance: RegistryPackageProvenance = serde_json::from_value(serde_json::json!({
            "registry": "ironhub",
            "repository": "nearai/ironhub",
            "package_version": "1.2.3",
            "release_tag": "v1.2.3",
            "catalog_origin": "https://hub.ironclaw.com",
            "artifact_digest": format!("sha256:{}", "c".repeat(64)),
            "manifest_digest": null,
            "installed_at": "2026-08-03T00:00:00Z",
        }))
        .expect("valid provenance");
        let metadata = InstalledSkillMetadata::installed_registry(
            Some("https://hub.ironclaw.com"),
            provenance,
        );
        let bytes = metadata.to_pretty_json().expect("serialize sidecar");
        let restored: InstalledSkillMetadata =
            serde_json::from_slice(&bytes).expect("deserialize sidecar");

        assert_eq!(restored, metadata);
    }
}
