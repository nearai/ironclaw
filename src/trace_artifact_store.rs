//! Encrypted local artifact storage primitives for Trace Commons.
//!
//! The ingestion MVP currently writes JSON files directly. This module is the
//! storage building block for the production object-store path: serialize a
//! redacted artifact, encrypt it with the existing IronClaw secrets crypto, and
//! persist only ciphertext plus non-sensitive routing metadata.

use std::path::{Path, PathBuf};

use anyhow::Context;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

use crate::secrets::SecretsCrypto;

pub const TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_VERSION: &str = "ironclaw.trace_artifact_ciphertext.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedTraceArtifactReceipt {
    pub tenant_storage_ref: String,
    pub artifact_kind: TraceArtifactKind,
    pub object_key: String,
    pub ciphertext_sha256: String,
    pub encrypted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceArtifactKind {
    ContributionEnvelope,
    ReplayExportManifest,
    ReplayDatasetExport,
    BenchmarkConversion,
    RankerTrainingExport,
    VectorPayload,
    AuditSnapshot,
    Other,
}

impl TraceArtifactKind {
    fn as_path_segment(&self) -> &'static str {
        match self {
            Self::ContributionEnvelope => "contribution_envelope",
            Self::ReplayExportManifest => "replay_export_manifest",
            Self::ReplayDatasetExport => "replay_dataset_export",
            Self::BenchmarkConversion => "benchmark_conversion",
            Self::RankerTrainingExport => "ranker_training_export",
            Self::VectorPayload => "vector_payload",
            Self::AuditSnapshot => "audit_snapshot",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedTraceArtifact {
    pub schema_version: String,
    pub receipt: EncryptedTraceArtifactReceipt,
    pub salt_base64: String,
    pub ciphertext_base64: String,
}

pub trait TraceArtifactStore: Send + Sync {
    fn put_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt>;

    fn read_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<EncryptedTraceArtifact>;

    fn read_json(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<serde_json::Value>;

    fn read_json_by_object_key(
        &self,
        expected_tenant_storage_ref: &str,
        expected_artifact_kind: TraceArtifactKind,
        object_key: &str,
        expected_ciphertext_sha256: &str,
    ) -> anyhow::Result<serde_json::Value>;

    fn delete_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<bool>;
}

pub struct LocalEncryptedTraceArtifactStore {
    root: PathBuf,
    crypto: SecretsCrypto,
}

impl LocalEncryptedTraceArtifactStore {
    pub fn new(root: impl Into<PathBuf>, crypto: SecretsCrypto) -> Self {
        Self {
            root: root.into(),
            crypto,
        }
    }

    pub fn put_json<T: Serialize>(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        value: &T,
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        let plaintext = serde_json::to_vec(value).context("failed to serialize trace artifact")?;
        self.put_serialized_json(tenant_storage_ref, artifact_kind, object_id, &plaintext)
    }

    pub fn put_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        serde_json::from_slice::<serde_json::Value>(serialized_json)
            .context("failed to parse serialized trace artifact")?;
        let (ciphertext, salt) = self
            .crypto
            .encrypt(serialized_json)
            .context("failed to encrypt trace artifact")?;
        let ciphertext_sha256 = sha256_hex(&ciphertext);
        let object_key = artifact_object_key(tenant_storage_ref, &artifact_kind, object_id);
        let receipt = EncryptedTraceArtifactReceipt {
            tenant_storage_ref: tenant_storage_ref.to_string(),
            artifact_kind,
            object_key,
            ciphertext_sha256,
            encrypted_at: Utc::now(),
        };
        let artifact = EncryptedTraceArtifact {
            schema_version: TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_VERSION.to_string(),
            receipt: receipt.clone(),
            salt_base64: base64::engine::general_purpose::STANDARD.encode(salt),
            ciphertext_base64: base64::engine::general_purpose::STANDARD.encode(ciphertext),
        };
        write_json_file(
            &self.artifact_path(&receipt.tenant_storage_ref, &receipt.object_key)?,
            &artifact,
        )?;
        Ok(receipt)
    }

    pub fn get_json<T: DeserializeOwned>(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<T> {
        let artifact = self.read_artifact(expected_tenant_storage_ref, receipt)?;
        decrypt_artifact_json(&self.crypto, &artifact)
    }

    pub fn get_json_by_object_key<T: DeserializeOwned>(
        &self,
        expected_tenant_storage_ref: &str,
        expected_artifact_kind: TraceArtifactKind,
        object_key: &str,
        expected_ciphertext_sha256: &str,
    ) -> anyhow::Result<T> {
        let artifact = self.read_artifact_by_object_key(
            expected_tenant_storage_ref,
            expected_artifact_kind,
            object_key,
            expected_ciphertext_sha256,
        )?;
        decrypt_artifact_json(&self.crypto, &artifact)
    }

    pub fn read_artifact_by_object_key(
        &self,
        expected_tenant_storage_ref: &str,
        expected_artifact_kind: TraceArtifactKind,
        object_key: &str,
        expected_ciphertext_sha256: &str,
    ) -> anyhow::Result<EncryptedTraceArtifact> {
        let expected_ciphertext_sha256 = expected_ciphertext_sha256
            .strip_prefix("sha256:")
            .unwrap_or(expected_ciphertext_sha256);
        let path = self.artifact_path(expected_tenant_storage_ref, object_key)?;
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read trace artifact {}", path.display()))?;
        let artifact: EncryptedTraceArtifact =
            serde_json::from_str(&body).context("failed to parse encrypted trace artifact")?;
        anyhow::ensure!(
            artifact.schema_version == TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_VERSION,
            "unsupported encrypted trace artifact schema version"
        );
        anyhow::ensure!(
            artifact.receipt.tenant_storage_ref == expected_tenant_storage_ref,
            "encrypted trace artifact tenant mismatch"
        );
        anyhow::ensure!(
            artifact.receipt.artifact_kind == expected_artifact_kind,
            "encrypted trace artifact kind mismatch"
        );
        anyhow::ensure!(
            artifact.receipt.object_key == object_key,
            "encrypted trace artifact object key mismatch"
        );
        anyhow::ensure!(
            artifact.receipt.ciphertext_sha256 == expected_ciphertext_sha256,
            "encrypted trace artifact receipt hash mismatch"
        );
        let ciphertext = base64::engine::general_purpose::STANDARD
            .decode(artifact.ciphertext_base64.as_bytes())
            .context("failed to decode trace artifact ciphertext for hash check")?;
        anyhow::ensure!(
            sha256_hex(&ciphertext) == expected_ciphertext_sha256,
            "trace artifact ciphertext hash mismatch"
        );
        Ok(artifact)
    }

    pub fn read_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<EncryptedTraceArtifact> {
        anyhow::ensure!(
            receipt.tenant_storage_ref == expected_tenant_storage_ref,
            "trace artifact receipt tenant mismatch"
        );
        let path = self.artifact_path(expected_tenant_storage_ref, &receipt.object_key)?;
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read trace artifact {}", path.display()))?;
        let artifact: EncryptedTraceArtifact =
            serde_json::from_str(&body).context("failed to parse encrypted trace artifact")?;
        anyhow::ensure!(
            artifact.schema_version == TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_VERSION,
            "unsupported encrypted trace artifact schema version"
        );
        anyhow::ensure!(
            artifact.receipt.tenant_storage_ref == expected_tenant_storage_ref,
            "encrypted trace artifact tenant mismatch"
        );
        anyhow::ensure!(
            artifact.receipt == *receipt,
            "encrypted trace artifact receipt mismatch"
        );
        let ciphertext = base64::engine::general_purpose::STANDARD
            .decode(artifact.ciphertext_base64.as_bytes())
            .context("failed to decode trace artifact ciphertext for hash check")?;
        anyhow::ensure!(
            sha256_hex(&ciphertext) == receipt.ciphertext_sha256,
            "trace artifact ciphertext hash mismatch"
        );
        Ok(artifact)
    }

    pub fn delete_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            receipt.tenant_storage_ref == expected_tenant_storage_ref,
            "trace artifact receipt tenant mismatch"
        );
        let path = self.artifact_path(expected_tenant_storage_ref, &receipt.object_key)?;
        if !path.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&path)
            .with_context(|| format!("failed to delete trace artifact {}", path.display()))?;
        Ok(true)
    }

    fn artifact_path(&self, tenant_storage_ref: &str, object_key: &str) -> anyhow::Result<PathBuf> {
        validate_object_key(object_key)?;
        let artifact_dir = self.tenant_artifact_dir(tenant_storage_ref)?;
        let path = artifact_dir.join(format!("{object_key}.json"));
        anyhow::ensure!(
            path.starts_with(&artifact_dir),
            "trace artifact path escapes tenant artifact directory"
        );
        Ok(path)
    }

    fn tenant_artifact_dir(&self, tenant_storage_ref: &str) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(
            !tenant_storage_ref.trim().is_empty(),
            "tenant storage ref must not be empty"
        );
        Ok(self
            .root
            .join("tenants")
            .join(sha256_text_hex(tenant_storage_ref))
            .join("artifacts"))
    }
}

impl TraceArtifactStore for LocalEncryptedTraceArtifactStore {
    fn put_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        Self::put_serialized_json(
            self,
            tenant_storage_ref,
            artifact_kind,
            object_id,
            serialized_json,
        )
    }

    fn read_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<EncryptedTraceArtifact> {
        Self::read_artifact(self, expected_tenant_storage_ref, receipt)
    }

    fn read_json(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<serde_json::Value> {
        Self::get_json(self, expected_tenant_storage_ref, receipt)
    }

    fn read_json_by_object_key(
        &self,
        expected_tenant_storage_ref: &str,
        expected_artifact_kind: TraceArtifactKind,
        object_key: &str,
        expected_ciphertext_sha256: &str,
    ) -> anyhow::Result<serde_json::Value> {
        Self::get_json_by_object_key(
            self,
            expected_tenant_storage_ref,
            expected_artifact_kind,
            object_key,
            expected_ciphertext_sha256,
        )
    }

    fn delete_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<bool> {
        Self::delete_artifact(self, expected_tenant_storage_ref, receipt)
    }
}

fn decrypt_artifact_json<T: DeserializeOwned>(
    crypto: &SecretsCrypto,
    artifact: &EncryptedTraceArtifact,
) -> anyhow::Result<T> {
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(artifact.ciphertext_base64.as_bytes())
        .context("failed to decode trace artifact ciphertext")?;
    let salt = base64::engine::general_purpose::STANDARD
        .decode(artifact.salt_base64.as_bytes())
        .context("failed to decode trace artifact salt")?;
    let decrypted = crypto
        .decrypt(&ciphertext, &salt)
        .context("failed to decrypt trace artifact")?;
    let plaintext = decrypted.expose().as_bytes();
    serde_json::from_slice(plaintext).context("failed to deserialize trace artifact")
}

fn validate_object_key(object_key: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        object_key.len() == 64 && object_key.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "trace artifact object key must be a 64-character hex digest"
    );
    Ok(())
}

fn artifact_object_key(
    tenant_storage_ref: &str,
    artifact_kind: &TraceArtifactKind,
    object_id: &str,
) -> String {
    sha256_text_hex(&format!(
        "{}\n{}\n{}",
        tenant_storage_ref,
        artifact_kind.as_path_segment(),
        object_id
    ))
}

fn sha256_text_hex(input: &str) -> String {
    sha256_hex(input.as_bytes())
}

fn sha256_hex(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    hex::encode(digest)
}

fn write_json_file<T: Serialize + ?Sized>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create trace artifact dir {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(value)
        .context("failed to serialize encrypted trace artifact")?;
    std::fs::write(path, body).with_context(|| {
        format!(
            "failed to write encrypted trace artifact {}",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;
    use serde_json::json;

    use super::*;

    fn test_store(temp: &tempfile::TempDir) -> LocalEncryptedTraceArtifactStore {
        let key = crate::secrets::keychain::generate_master_key_hex();
        let crypto = SecretsCrypto::new(SecretString::from(key)).expect("test crypto");
        LocalEncryptedTraceArtifactStore::new(temp.path(), crypto)
    }

    fn assert_trace_artifact_store_contract(store: &dyn TraceArtifactStore) {
        let payload = json!({"safe": true, "summary": "<redacted>"});
        let serialized_payload = serde_json::to_vec(&payload).expect("payload serializes");
        let receipt = store
            .put_serialized_json(
                "tenant:sha256:trait",
                TraceArtifactKind::ContributionEnvelope,
                "trait-contract",
                &serialized_payload,
            )
            .expect("artifact writes through trait");

        let artifact = store
            .read_artifact("tenant:sha256:trait", &receipt)
            .expect("artifact envelope reads through trait");
        assert_eq!(artifact.receipt, receipt);

        let receipt_round_trip: serde_json::Value = store
            .read_json("tenant:sha256:trait", &receipt)
            .expect("artifact JSON reads by receipt through trait");
        assert_eq!(receipt_round_trip, payload);

        let round_trip: serde_json::Value = store
            .read_json_by_object_key(
                "tenant:sha256:trait",
                TraceArtifactKind::ContributionEnvelope,
                &receipt.object_key,
                &receipt.ciphertext_sha256,
            )
            .expect("artifact JSON reads through trait");
        assert_eq!(round_trip, payload);

        assert!(
            store
                .delete_artifact("tenant:sha256:trait", &receipt)
                .expect("artifact deletes through trait")
        );
    }

    #[test]
    fn encrypted_artifact_round_trips_without_plaintext_on_disk() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = test_store(&temp);
        let payload = json!({
            "submission_id": "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
            "canonical_summary": "User asked about <PRIVATE_DATE>",
        });

        let receipt = store
            .put_json(
                "tenant:sha256:abc123",
                TraceArtifactKind::ContributionEnvelope,
                "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
                &payload,
            )
            .expect("artifact writes");
        let round_trip: serde_json::Value = store
            .get_json("tenant:sha256:abc123", &receipt)
            .expect("artifact reads");
        assert_eq!(round_trip, payload);

        let artifact = store
            .read_artifact("tenant:sha256:abc123", &receipt)
            .expect("ciphertext reads");
        let serialized = serde_json::to_string(&artifact).expect("artifact serializes");
        assert!(!serialized.contains("plaintext_sha256"));
        assert!(!serialized.contains("<PRIVATE_DATE>"));
        assert!(!serialized.contains("User asked"));
        assert_eq!(artifact.receipt.tenant_storage_ref, "tenant:sha256:abc123");
    }

    #[test]
    fn local_store_satisfies_trace_artifact_store_contract() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = test_store(&temp);

        assert_trace_artifact_store_contract(&store);
    }

    #[test]
    fn encrypted_artifact_rejects_cross_tenant_receipts() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = test_store(&temp);
        let payload = json!({"safe": true});
        let receipt = store
            .put_json(
                "tenant:sha256:abc123",
                TraceArtifactKind::BenchmarkConversion,
                "conversion-1",
                &payload,
            )
            .expect("artifact writes");

        let error = store
            .get_json::<serde_json::Value>("tenant:sha256:other", &receipt)
            .expect_err("cross-tenant receipt must fail");
        assert!(error.to_string().contains("tenant mismatch"));
    }

    #[test]
    fn encrypted_artifact_reads_by_object_key_and_hash() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = test_store(&temp);
        let payload = json!({"safe": true, "summary": "<redacted>"});
        let receipt = store
            .put_json(
                "tenant:sha256:abc123",
                TraceArtifactKind::ContributionEnvelope,
                "object-ref-read",
                &payload,
            )
            .expect("artifact writes");

        let round_trip: serde_json::Value = store
            .get_json_by_object_key(
                "tenant:sha256:abc123",
                TraceArtifactKind::ContributionEnvelope,
                &receipt.object_key,
                &format!("sha256:{}", receipt.ciphertext_sha256),
            )
            .expect("artifact reads by object key and hash");
        assert_eq!(round_trip, payload);

        let error = store
            .get_json_by_object_key::<serde_json::Value>(
                "tenant:sha256:abc123",
                TraceArtifactKind::ContributionEnvelope,
                &receipt.object_key,
                "sha256:wrong",
            )
            .expect_err("wrong object hash must fail");
        assert!(error.to_string().contains("receipt hash mismatch"));
    }

    #[test]
    fn encrypted_artifact_detects_ciphertext_receipt_tampering() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = test_store(&temp);
        let payload = json!({"safe": true});
        let mut receipt = store
            .put_json(
                "tenant:sha256:abc123",
                TraceArtifactKind::BenchmarkConversion,
                "conversion-1",
                &payload,
            )
            .expect("artifact writes");

        receipt.ciphertext_sha256 = "sha256:wrong".to_string();
        let error = store
            .get_json::<serde_json::Value>("tenant:sha256:abc123", &receipt)
            .expect_err("tampered receipt must fail");
        assert!(error.to_string().contains("receipt mismatch"));
    }

    #[test]
    fn encrypted_artifact_rejects_path_shaped_object_keys() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = test_store(&temp);
        let receipt = EncryptedTraceArtifactReceipt {
            tenant_storage_ref: "tenant:sha256:abc123".to_string(),
            artifact_kind: TraceArtifactKind::Other,
            object_key: "../escape".to_string(),
            ciphertext_sha256: "sha256:unused".to_string(),
            encrypted_at: Utc::now(),
        };

        let error = store
            .read_artifact("tenant:sha256:abc123", &receipt)
            .expect_err("path-shaped object keys must fail");
        assert!(error.to_string().contains("64-character hex digest"));
    }

    #[test]
    fn encrypted_artifact_delete_removes_ciphertext_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = test_store(&temp);
        let payload = json!({"safe": true});
        let receipt = store
            .put_json(
                "tenant:sha256:abc123",
                TraceArtifactKind::ContributionEnvelope,
                "delete-me",
                &payload,
            )
            .expect("artifact writes");

        assert!(
            store
                .delete_artifact("tenant:sha256:abc123", &receipt)
                .expect("artifact deletes")
        );
        assert!(
            !store
                .delete_artifact("tenant:sha256:abc123", &receipt)
                .expect("missing artifact delete is idempotent")
        );
        let error = store
            .read_artifact("tenant:sha256:abc123", &receipt)
            .expect_err("deleted artifact should not read");
        assert!(error.to_string().contains("failed to read trace artifact"));
    }
}
