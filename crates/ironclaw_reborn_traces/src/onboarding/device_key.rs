//! Per-(scope, tenant) Ed25519 device keypairs (spec §2.2) and self-signed
//! workload JWTs (spec §2.4). Private keys never leave the machine.

use std::path::Path;

use base64::Engine as _;
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const WORKLOAD_JWT_TTL_SECS: i64 = 60;

#[derive(thiserror::Error, Debug)]
pub enum DeviceKeyError {
    #[error("device key io error: {reason}")]
    Io { reason: String },
    #[error("device key file is malformed: {reason}")]
    Malformed { reason: String },
}

/// On-disk JSON representation. Private to this module.
#[derive(Serialize, Deserialize)]
struct KeyFile {
    private_key: String, // base64-standard-padded of raw 32-byte secret
    public_key: String,  // base64-standard-padded of raw 32-byte pubkey
    device_key_id: String,
    tenant_id: Option<String>,
    created_at: chrono::DateTime<Utc>,
}

pub struct DeviceKeypair {
    signing_key: SigningKey,
    pub device_key_id: String,
    pub public_key_b64: String,
    pub tenant_id: Option<String>,
}

// Manual Debug that omits the signing key.
impl std::fmt::Debug for DeviceKeypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceKeypair")
            .field("device_key_id", &self.device_key_id)
            .field("public_key_b64", &self.public_key_b64)
            .field("tenant_id", &self.tenant_id)
            .field("signing_key", &"<redacted>")
            .finish()
    }
}

/// Full SHA-256 hex of tenant_id (64 hex chars). Not the same as scope_hash().
pub(crate) fn tenant_hash(tenant_id: &str) -> String {
    hex::encode(Sha256::digest(tenant_id.as_bytes()))
}

fn device_key_id_from_pubkey(pubkey_bytes: &[u8; 32]) -> String {
    let digest = Sha256::digest(pubkey_bytes);
    format!("sha256:{}", hex::encode(digest))
}

fn pending_path(base: &Path, invite_hash: &str) -> std::path::PathBuf {
    base.join("device_keys")
        .join("pending")
        .join(format!("{invite_hash}.json"))
}

fn tenant_path(base: &Path, tenant_id: &str) -> std::path::PathBuf {
    base.join("device_keys")
        .join(format!("{}.json", tenant_hash(tenant_id)))
}

/// Create `dir` (and ancestors) with mode 0700 on unix so pending invite
/// hashes / tenant hashes aren't enumerable by other local users. Falls back
/// to plain `create_dir_all` elsewhere. Idempotent.
fn ensure_private_dir(dir: &Path) -> Result<(), DeviceKeyError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(dir).map_err(|e| DeviceKeyError::Io {
            reason: format!("create_dir_all {}: {e}", dir.display()),
        })
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir).map_err(|e| DeviceKeyError::Io {
            reason: format!("create_dir_all {}: {e}", dir.display()),
        })
    }
}

fn load_from_path(path: &Path) -> Result<DeviceKeypair, DeviceKeyError> {
    let bytes = std::fs::read(path).map_err(|e| DeviceKeyError::Io {
        reason: format!("read {}: {e}", path.display()),
    })?;
    let kf: KeyFile = serde_json::from_slice(&bytes).map_err(|e| DeviceKeyError::Malformed {
        reason: format!("json parse {}: {e}", path.display()),
    })?;

    let secret_bytes = base64::engine::general_purpose::STANDARD
        .decode(&kf.private_key)
        .map_err(|e| DeviceKeyError::Malformed {
            reason: format!("base64 decode private_key: {e}"),
        })?;
    let secret_arr: [u8; 32] = secret_bytes
        .try_into()
        .map_err(|_| DeviceKeyError::Malformed {
            reason: "private_key is not 32 bytes".to_string(),
        })?;

    let signing_key = SigningKey::from_bytes(&secret_arr);
    Ok(DeviceKeypair {
        signing_key,
        device_key_id: kf.device_key_id,
        public_key_b64: kf.public_key,
        tenant_id: kf.tenant_id,
    })
}

fn write_keypair(
    path: &Path,
    signing_key: &SigningKey,
    device_key_id: &str,
    public_key_b64: &str,
    tenant_id: Option<&str>,
) -> Result<(), DeviceKeyError> {
    let secret_bytes = signing_key.to_bytes();
    let private_key = base64::engine::general_purpose::STANDARD.encode(secret_bytes);

    let kf = KeyFile {
        private_key,
        public_key: public_key_b64.to_string(),
        device_key_id: device_key_id.to_string(),
        tenant_id: tenant_id.map(str::to_string),
        created_at: Utc::now(),
    };

    // Ensure the containing dir exists at 0o700 before the write so the parent
    // isn't enumerable; the file itself is written via the crate's hardened
    // `write_json_file` (create_new + 0o600 + uuid temp name + sync_all +
    // best-effort parent-dir sync), which avoids the world-readable window and
    // fixed-temp-name race of a naive write-then-chmod.
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    crate::contribution::write_json_file(path, &kf, "device key").map_err(|e| DeviceKeyError::Io {
        reason: e.to_string(),
    })
}

impl DeviceKeypair {
    /// Load an existing pending keypair for `invite_hash`, or generate and stage a new one.
    pub fn load_or_generate_pending(
        base: &Path,
        invite_hash: &str,
    ) -> Result<Self, DeviceKeyError> {
        let path = pending_path(base, invite_hash);
        if path.exists() {
            return load_from_path(&path);
        }

        // Generate a new keypair.
        let signing_key = SigningKey::generate(&mut rand::rngs::OsRng);
        let verifying_key = signing_key.verifying_key();
        let pubkey_bytes: [u8; 32] = verifying_key.to_bytes();
        let public_key_b64 = base64::engine::general_purpose::STANDARD.encode(pubkey_bytes);
        let device_key_id = device_key_id_from_pubkey(&pubkey_bytes);

        write_keypair(&path, &signing_key, &device_key_id, &public_key_b64, None)?;

        Ok(Self {
            signing_key,
            device_key_id,
            public_key_b64,
            tenant_id: None,
        })
    }

    /// Atomically move the pending keypair to the tenant-keyed path, recording `tenant_id`.
    pub fn promote(
        self,
        base: &Path,
        invite_hash: &str,
        tenant_id: &str,
    ) -> Result<Self, DeviceKeyError> {
        let dest = tenant_path(base, tenant_id);
        let pubkey_bytes: [u8; 32] = self.signing_key.verifying_key().to_bytes();
        let public_key_b64 = base64::engine::general_purpose::STANDARD.encode(pubkey_bytes);

        write_keypair(
            &dest,
            &self.signing_key,
            &self.device_key_id,
            &public_key_b64,
            Some(tenant_id),
        )?;

        // Remove the pending file.
        let pending = pending_path(base, invite_hash);
        if pending.exists() {
            std::fs::remove_file(&pending).map_err(|e| DeviceKeyError::Io {
                reason: format!("remove pending {}: {e}", pending.display()),
            })?;
        }

        Ok(Self {
            signing_key: self.signing_key,
            device_key_id: self.device_key_id,
            public_key_b64,
            tenant_id: Some(tenant_id.to_string()),
        })
    }

    /// Load the keypair for a given tenant, returning `None` if it doesn't exist yet.
    pub fn load_for_tenant(base: &Path, tenant_id: &str) -> Result<Option<Self>, DeviceKeyError> {
        let path = tenant_path(base, tenant_id);
        if !path.exists() {
            return Ok(None);
        }
        load_from_path(&path).map(Some)
    }

    /// Delete the pending keypair for `invite_hash` on terminal failure.
    pub fn discard_pending(base: &Path, invite_hash: &str) -> Result<(), DeviceKeyError> {
        let path = pending_path(base, invite_hash);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| DeviceKeyError::Io {
                reason: format!("discard pending {}: {e}", path.display()),
            })?;
        }
        Ok(())
    }

    pub fn verifying_key(&self) -> Result<ed25519_dalek::VerifyingKey, DeviceKeyError> {
        Ok(self.signing_key.verifying_key())
    }

    /// Produce a self-signed workload JWT for the given audience.
    /// Errors if this keypair has no tenant binding.
    pub fn sign_workload_jwt(&self, audience: &str) -> Result<String, DeviceKeyError> {
        let tenant_id = self
            .tenant_id
            .as_deref()
            .ok_or_else(|| DeviceKeyError::Malformed {
                reason: "device key has no tenant binding".to_string(),
            })?;

        let iat = Utc::now().timestamp();
        let exp = iat + WORKLOAD_JWT_TTL_SECS;

        // Header: {"alg":"EdDSA","typ":"JWT","kid":"<device_key_id>"}
        let header = serde_json::json!({
            "alg": "EdDSA",
            "typ": "JWT",
            "kid": self.device_key_id,
        });
        // Claims
        let claims = serde_json::json!({
            "tenant_id": tenant_id,
            "aud": audience,
            "iat": iat,
            "exp": exp,
        });

        let b64url = base64::engine::general_purpose::URL_SAFE_NO_PAD;

        let header_b64 =
            b64url.encode(serde_json::to_vec(&header).map_err(|e| DeviceKeyError::Io {
                reason: format!("serialize header: {e}"),
            })?);
        let claims_b64 =
            b64url.encode(serde_json::to_vec(&claims).map_err(|e| DeviceKeyError::Io {
                reason: format!("serialize claims: {e}"),
            })?);

        let signing_input = format!("{header_b64}.{claims_b64}");
        let sig: ed25519_dalek::Signature = self.signing_key.sign(signing_input.as_bytes());
        let sig_b64 = b64url.encode(sig.to_bytes());

        Ok(format!("{signing_input}.{sig_b64}"))
    }

    /// Exposed only in tests for the debug-redaction assertion.
    #[cfg(test)]
    pub fn private_key_b64_for_test(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.signing_key.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn generates_and_stages_pending_keypair_with_0600() {
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "abc123hash").unwrap();
        let pending = dir.path().join("device_keys/pending/abc123hash.json");
        assert!(pending.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&pending).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(kp.device_key_id.starts_with("sha256:"));
    }

    #[test]
    fn reloads_same_pending_keypair_on_retry() {
        let dir = tmp_dir();
        let a = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let b = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        assert_eq!(a.device_key_id, b.device_key_id);
    }

    #[test]
    fn promote_moves_pending_to_tenant_path_and_records_tenant() {
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let promoted = kp.promote(dir.path(), "h1", "tenant-a").unwrap();
        assert!(!dir.path().join("device_keys/pending/h1.json").exists());
        let tenant_file = dir
            .path()
            .join(format!("device_keys/{}.json", tenant_hash("tenant-a")));
        assert!(tenant_file.exists());
        assert_eq!(promoted.tenant_id.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn load_for_tenant_finds_promoted_key() {
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let promoted = kp.promote(dir.path(), "h1", "tenant-a").unwrap();
        let loaded = DeviceKeypair::load_for_tenant(dir.path(), "tenant-a")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.device_key_id, promoted.device_key_id);
    }

    #[test]
    fn discard_pending_removes_file() {
        let dir = tmp_dir();
        DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        DeviceKeypair::discard_pending(dir.path(), "h1").unwrap();
        assert!(!dir.path().join("device_keys/pending/h1.json").exists());
    }

    #[test]
    fn self_signed_workload_jwt_has_correct_shape_and_verifies() {
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let kp = kp.promote(dir.path(), "h1", "tenant-a").unwrap();
        let jwt = kp.sign_workload_jwt("trace-commons-ingest").unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);

        let header = jsonwebtoken::decode_header(&jwt).unwrap();
        assert_eq!(header.alg, jsonwebtoken::Algorithm::EdDSA);
        assert_eq!(header.kid.as_deref(), Some(kp.device_key_id.as_str()));

        use base64::Engine as _;
        let payload: serde_json::Value = serde_json::from_slice(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(parts[1])
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["tenant_id"], "tenant-a");
        assert_eq!(payload["aud"], "trace-commons-ingest");
        let iat = payload["iat"].as_i64().unwrap();
        let exp = payload["exp"].as_i64().unwrap();
        assert_eq!(exp - iat, 60);

        use ed25519_dalek::Verifier as _;
        let signing_input = format!("{}.{}", parts[0], parts[1]);
        let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[2])
            .unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&sig_bytes).unwrap();
        kp.verifying_key()
            .unwrap()
            .verify(signing_input.as_bytes(), &sig)
            .unwrap();
    }

    #[test]
    fn debug_impl_redacts_private_key() {
        let dir = tmp_dir();
        let kp = DeviceKeypair::load_or_generate_pending(dir.path(), "h1").unwrap();
        let dbg = format!("{kp:?}");
        assert!(!dbg.contains(&kp.private_key_b64_for_test()));
    }
}
