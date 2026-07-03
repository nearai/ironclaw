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
}

impl InstalledSkillMetadata {
    pub fn installed_url(source_url: Option<&str>) -> Self {
        Self {
            source: Some(InstalledSkillMetadataSource::InstalledUrl),
            source_url: source_url.map(str::to_string),
            source_subdir: None,
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
