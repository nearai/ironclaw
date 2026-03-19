//! Filesystem persistence for instance DID state.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::bootstrap::ironclaw_base_dir;

use super::DidError;

/// On-disk representation of the instance identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredInstanceIdentity {
    pub version: u8,
    pub method: String,
    pub created_at: DateTime<Utc>,
    pub secret_key_hex: String,
}

impl StoredInstanceIdentity {
    pub fn new(method: &str, created_at: DateTime<Utc>, secret_key_hex: String) -> Self {
        Self {
            version: 1,
            method: method.to_string(),
            created_at,
            secret_key_hex,
        }
    }
}

/// Default on-disk path for the instance identity.
pub fn default_identity_path() -> PathBuf {
    ironclaw_base_dir().join("identity").join("instance.json")
}

pub(crate) fn load(path: &Path) -> Result<Option<StoredInstanceIdentity>, DidError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    let stored = serde_json::from_str(&content)?;
    Ok(Some(stored))
}

pub(crate) fn save(path: &Path, stored: &StoredInstanceIdentity) -> Result<(), DidError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = serde_json::to_vec_pretty(stored)?;
    std::fs::write(path, content)?;
    restrict_file_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("instance.json");
        let stored = StoredInstanceIdentity::new("did:key", Utc::now(), "11".repeat(32));

        save(&path, &stored).expect("save");
        let loaded = load(&path).expect("load").expect("stored identity");

        assert_eq!(loaded.method, "did:key");
        assert_eq!(loaded.secret_key_hex, "11".repeat(32));
    }

    #[cfg(unix)]
    #[test]
    fn stored_identity_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("instance.json");
        let stored = StoredInstanceIdentity::new("did:key", Utc::now(), "22".repeat(32));

        save(&path, &stored).expect("save");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
