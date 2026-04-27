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
pub struct TraceArtifactProviderConfig {
    pub kind: TraceArtifactProviderKind,
    pub object_store: String,
}

impl TraceArtifactProviderConfig {
    pub fn local_encrypted(object_store: impl Into<String>) -> anyhow::Result<Self> {
        Self::new(TraceArtifactProviderKind::LocalEncrypted, object_store)
    }

    pub fn service_owned_remote(object_store: impl Into<String>) -> anyhow::Result<Self> {
        Self::new(TraceArtifactProviderKind::ServiceOwnedRemote, object_store)
    }

    fn new(
        kind: TraceArtifactProviderKind,
        object_store: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let object_store = object_store.into();
        validate_non_empty_ref("trace artifact object store", &object_store)?;
        anyhow::ensure!(
            object_store
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
            "trace artifact object store contains unsupported characters"
        );
        Ok(Self { kind, object_store })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceArtifactProviderKind {
    LocalEncrypted,
    ServiceOwnedRemote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceArtifactScope {
    pub tenant_storage_ref: String,
    pub submission_storage_ref: String,
}

impl TraceArtifactScope {
    pub fn new(
        tenant_storage_ref: impl Into<String>,
        submission_storage_ref: impl Into<String>,
    ) -> Self {
        Self {
            tenant_storage_ref: tenant_storage_ref.into(),
            submission_storage_ref: submission_storage_ref.into(),
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        validate_non_empty_ref("tenant storage ref", &self.tenant_storage_ref)?;
        validate_non_empty_ref("submission storage ref", &self.submission_storage_ref)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceArtifactObjectRef {
    pub provider_kind: TraceArtifactProviderKind,
    pub object_store: String,
    pub tenant_storage_ref: String,
    pub submission_storage_ref: String,
    pub artifact_kind: TraceArtifactKind,
    pub object_key: String,
    pub ciphertext_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceArtifactPutReceipt {
    pub object_ref: TraceArtifactObjectRef,
    pub encrypted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceArtifactDeleteReceipt {
    pub object_ref: TraceArtifactObjectRef,
    pub deleted: bool,
    pub deleted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceArtifactInvalidationReceipt {
    pub object_ref: TraceArtifactObjectRef,
    pub reason: TraceArtifactInvalidationReason,
    pub invalidated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceArtifactInvalidationReason {
    Revoked,
    RetentionExpired,
    Replaced,
    OperatorRequested,
    Other,
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

#[derive(Debug, Clone)]
pub struct RemoteTraceArtifactRecord {
    pub object_ref: TraceArtifactObjectRef,
    pub artifact: EncryptedTraceArtifact,
    pub invalidated_at: Option<DateTime<Utc>>,
}

pub trait RemoteTraceArtifactProvider: Send + Sync {
    fn put_encrypted_artifact(
        &self,
        object_ref: TraceArtifactObjectRef,
        artifact: EncryptedTraceArtifact,
    ) -> anyhow::Result<()>;

    fn read_encrypted_artifact(
        &self,
        object_ref: &TraceArtifactObjectRef,
    ) -> anyhow::Result<RemoteTraceArtifactRecord>;

    fn invalidate_encrypted_artifact(
        &self,
        object_ref: &TraceArtifactObjectRef,
        reason: TraceArtifactInvalidationReason,
        invalidated_at: DateTime<Utc>,
    ) -> anyhow::Result<()>;

    fn delete_encrypted_artifact(
        &self,
        object_ref: &TraceArtifactObjectRef,
        deleted_at: DateTime<Utc>,
    ) -> anyhow::Result<bool>;
}

const TRACE_ARTIFACT_STORE_LEGACY_SUBMISSION_STORAGE_REF: &str = "trace-artifact-store-legacy";

pub struct ServiceOwnedTraceArtifactStore<P> {
    config: TraceArtifactProviderConfig,
    crypto: SecretsCrypto,
    provider: P,
}

impl<P: RemoteTraceArtifactProvider> ServiceOwnedTraceArtifactStore<P> {
    pub fn new(config: TraceArtifactProviderConfig, crypto: SecretsCrypto, provider: P) -> Self {
        Self {
            config,
            crypto,
            provider,
        }
    }

    pub fn put_scoped_json<T: Serialize>(
        &self,
        scope: &TraceArtifactScope,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        value: &T,
    ) -> anyhow::Result<TraceArtifactPutReceipt> {
        let plaintext = serde_json::to_vec(value).context("failed to serialize trace artifact")?;
        self.put_scoped_serialized_json(scope, artifact_kind, object_id, &plaintext)
    }

    pub fn put_scoped_serialized_json(
        &self,
        scope: &TraceArtifactScope,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<TraceArtifactPutReceipt> {
        self.validate_remote_config()?;
        scope.validate()?;
        validate_non_empty_ref("trace artifact object id", object_id)?;
        serde_json::from_slice::<serde_json::Value>(serialized_json)
            .context("failed to parse serialized trace artifact")?;
        let (ciphertext, salt) = self
            .crypto
            .encrypt(serialized_json)
            .context("failed to encrypt trace artifact")?;
        let ciphertext_sha256 = sha256_hex(&ciphertext);
        let object_key = remote_artifact_object_key(&self.config, scope, &artifact_kind, object_id);
        let encrypted_at = Utc::now();
        let legacy_receipt = EncryptedTraceArtifactReceipt {
            tenant_storage_ref: scope.tenant_storage_ref.clone(),
            artifact_kind: artifact_kind.clone(),
            object_key: object_key.clone(),
            ciphertext_sha256: ciphertext_sha256.clone(),
            encrypted_at,
        };
        let artifact = EncryptedTraceArtifact {
            schema_version: TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_VERSION.to_string(),
            receipt: legacy_receipt,
            salt_base64: base64::engine::general_purpose::STANDARD.encode(salt),
            ciphertext_base64: base64::engine::general_purpose::STANDARD.encode(ciphertext),
        };
        let object_ref = TraceArtifactObjectRef {
            provider_kind: self.config.kind,
            object_store: self.config.object_store.clone(),
            tenant_storage_ref: scope.tenant_storage_ref.clone(),
            submission_storage_ref: scope.submission_storage_ref.clone(),
            artifact_kind,
            object_key,
            ciphertext_sha256,
        };
        validate_remote_object_ref(scope, &self.config, &object_ref)?;
        self.provider
            .put_encrypted_artifact(object_ref.clone(), artifact)?;
        Ok(TraceArtifactPutReceipt {
            object_ref,
            encrypted_at,
        })
    }

    pub fn read_scoped_artifact(
        &self,
        expected_scope: &TraceArtifactScope,
        object_ref: &TraceArtifactObjectRef,
    ) -> anyhow::Result<EncryptedTraceArtifact> {
        validate_remote_object_ref(expected_scope, &self.config, object_ref)?;
        let record = self.provider.read_encrypted_artifact(object_ref)?;
        anyhow::ensure!(
            record.object_ref == *object_ref,
            "remote trace artifact object ref mismatch"
        );
        anyhow::ensure!(
            record.invalidated_at.is_none(),
            "remote trace artifact object ref invalidated"
        );
        verify_encrypted_artifact(
            &record.artifact,
            expected_scope.tenant_storage_ref.as_str(),
            &object_ref.artifact_kind,
            object_ref.object_key.as_str(),
            object_ref.ciphertext_sha256.as_str(),
        )?;
        Ok(record.artifact)
    }

    pub fn read_scoped_json<T: DeserializeOwned>(
        &self,
        expected_scope: &TraceArtifactScope,
        object_ref: &TraceArtifactObjectRef,
    ) -> anyhow::Result<T> {
        let artifact = self.read_scoped_artifact(expected_scope, object_ref)?;
        decrypt_artifact_json(&self.crypto, &artifact)
    }

    pub fn invalidate_scoped_artifact(
        &self,
        expected_scope: &TraceArtifactScope,
        object_ref: &TraceArtifactObjectRef,
        reason: TraceArtifactInvalidationReason,
    ) -> anyhow::Result<TraceArtifactInvalidationReceipt> {
        validate_remote_object_ref(expected_scope, &self.config, object_ref)?;
        let invalidated_at = Utc::now();
        self.provider
            .invalidate_encrypted_artifact(object_ref, reason, invalidated_at)?;
        Ok(TraceArtifactInvalidationReceipt {
            object_ref: object_ref.clone(),
            reason,
            invalidated_at,
        })
    }

    pub fn delete_scoped_artifact(
        &self,
        expected_scope: &TraceArtifactScope,
        object_ref: &TraceArtifactObjectRef,
    ) -> anyhow::Result<TraceArtifactDeleteReceipt> {
        validate_remote_object_ref(expected_scope, &self.config, object_ref)?;
        let deleted_at = Utc::now();
        let deleted = self
            .provider
            .delete_encrypted_artifact(object_ref, deleted_at)?;
        Ok(TraceArtifactDeleteReceipt {
            object_ref: object_ref.clone(),
            deleted,
            deleted_at,
        })
    }

    fn validate_remote_config(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.config.kind == TraceArtifactProviderKind::ServiceOwnedRemote,
            "trace artifact provider config is not remote service-owned"
        );
        validate_non_empty_ref("trace artifact object store", &self.config.object_store)
    }
}

impl<P: RemoteTraceArtifactProvider> TraceArtifactStore for ServiceOwnedTraceArtifactStore<P> {
    fn put_serialized_json(
        &self,
        tenant_storage_ref: &str,
        artifact_kind: TraceArtifactKind,
        object_id: &str,
        serialized_json: &[u8],
    ) -> anyhow::Result<EncryptedTraceArtifactReceipt> {
        let scope = legacy_trace_artifact_scope(tenant_storage_ref);
        let receipt =
            self.put_scoped_serialized_json(&scope, artifact_kind, object_id, serialized_json)?;
        Ok(EncryptedTraceArtifactReceipt {
            tenant_storage_ref: receipt.object_ref.tenant_storage_ref,
            artifact_kind: receipt.object_ref.artifact_kind,
            object_key: receipt.object_ref.object_key,
            ciphertext_sha256: receipt.object_ref.ciphertext_sha256,
            encrypted_at: receipt.encrypted_at,
        })
    }

    fn read_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<EncryptedTraceArtifact> {
        anyhow::ensure!(
            receipt.tenant_storage_ref == expected_tenant_storage_ref,
            "trace artifact receipt tenant mismatch"
        );
        let scope = legacy_trace_artifact_scope(expected_tenant_storage_ref);
        let object_ref = legacy_remote_object_ref_from_receipt(&self.config, &scope, receipt)?;
        let artifact = self.read_scoped_artifact(&scope, &object_ref)?;
        anyhow::ensure!(
            artifact.receipt == *receipt,
            "encrypted trace artifact receipt mismatch"
        );
        Ok(artifact)
    }

    fn read_json(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<serde_json::Value> {
        let artifact =
            TraceArtifactStore::read_artifact(self, expected_tenant_storage_ref, receipt)?;
        decrypt_artifact_json(&self.crypto, &artifact)
    }

    fn read_json_by_object_key(
        &self,
        expected_tenant_storage_ref: &str,
        expected_artifact_kind: TraceArtifactKind,
        object_key: &str,
        expected_ciphertext_sha256: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let scope = legacy_trace_artifact_scope(expected_tenant_storage_ref);
        let object_ref = legacy_remote_object_ref_from_object_key(
            &self.config,
            &scope,
            expected_artifact_kind,
            object_key,
            expected_ciphertext_sha256,
        )?;
        self.read_scoped_json(&scope, &object_ref)
    }

    fn delete_artifact(
        &self,
        expected_tenant_storage_ref: &str,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            receipt.tenant_storage_ref == expected_tenant_storage_ref,
            "trace artifact receipt tenant mismatch"
        );
        let scope = legacy_trace_artifact_scope(expected_tenant_storage_ref);
        let object_ref = legacy_remote_object_ref_from_receipt(&self.config, &scope, receipt)?;
        let delete_receipt = self.delete_scoped_artifact(&scope, &object_ref)?;
        Ok(delete_receipt.deleted)
    }
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

fn verify_encrypted_artifact(
    artifact: &EncryptedTraceArtifact,
    expected_tenant_storage_ref: &str,
    expected_artifact_kind: &TraceArtifactKind,
    expected_object_key: &str,
    expected_ciphertext_sha256: &str,
) -> anyhow::Result<()> {
    let expected_ciphertext_sha256 = expected_ciphertext_sha256
        .strip_prefix("sha256:")
        .unwrap_or(expected_ciphertext_sha256);
    anyhow::ensure!(
        artifact.schema_version == TRACE_ARTIFACT_CIPHERTEXT_SCHEMA_VERSION,
        "unsupported encrypted trace artifact schema version"
    );
    anyhow::ensure!(
        artifact.receipt.tenant_storage_ref == expected_tenant_storage_ref,
        "encrypted trace artifact tenant mismatch"
    );
    anyhow::ensure!(
        artifact.receipt.artifact_kind == *expected_artifact_kind,
        "encrypted trace artifact kind mismatch"
    );
    anyhow::ensure!(
        artifact.receipt.object_key == expected_object_key,
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
    Ok(())
}

fn validate_remote_object_ref(
    expected_scope: &TraceArtifactScope,
    expected_config: &TraceArtifactProviderConfig,
    object_ref: &TraceArtifactObjectRef,
) -> anyhow::Result<()> {
    expected_scope.validate()?;
    validate_remote_object_key(&object_ref.object_key)?;
    validate_object_hash(&object_ref.ciphertext_sha256)?;
    anyhow::ensure!(
        object_ref.provider_kind == TraceArtifactProviderKind::ServiceOwnedRemote,
        "remote trace artifact provider kind mismatch"
    );
    anyhow::ensure!(
        object_ref.provider_kind == expected_config.kind,
        "remote trace artifact config kind mismatch"
    );
    anyhow::ensure!(
        object_ref.object_store == expected_config.object_store,
        "remote trace artifact object store mismatch"
    );
    anyhow::ensure!(
        object_ref.tenant_storage_ref == expected_scope.tenant_storage_ref,
        "remote trace artifact tenant mismatch"
    );
    anyhow::ensure!(
        object_ref.submission_storage_ref == expected_scope.submission_storage_ref,
        "remote trace artifact submission mismatch"
    );
    validate_remote_object_key_scope(expected_scope, &object_ref.object_key)?;
    Ok(())
}

fn validate_remote_object_key_scope(
    expected_scope: &TraceArtifactScope,
    object_key: &str,
) -> anyhow::Result<()> {
    let segments: Vec<&str> = object_key.split('/').collect();
    anyhow::ensure!(
        segments.len() >= 7
            && segments[0] == "v1"
            && segments[1] == "tenants"
            && segments[3] == "submissions",
        "remote trace artifact object key has unsupported partition layout"
    );
    anyhow::ensure!(
        segments[2] == sha256_text_hex(&expected_scope.tenant_storage_ref),
        "remote trace artifact tenant mismatch: object key partition"
    );
    anyhow::ensure!(
        segments[4] == sha256_text_hex(&expected_scope.submission_storage_ref),
        "remote trace artifact submission mismatch: object key partition"
    );
    Ok(())
}

fn legacy_trace_artifact_scope(tenant_storage_ref: &str) -> TraceArtifactScope {
    TraceArtifactScope::new(
        tenant_storage_ref,
        TRACE_ARTIFACT_STORE_LEGACY_SUBMISSION_STORAGE_REF,
    )
}

fn legacy_remote_object_ref_from_receipt(
    config: &TraceArtifactProviderConfig,
    scope: &TraceArtifactScope,
    receipt: &EncryptedTraceArtifactReceipt,
) -> anyhow::Result<TraceArtifactObjectRef> {
    anyhow::ensure!(
        receipt.tenant_storage_ref == scope.tenant_storage_ref,
        "trace artifact receipt tenant mismatch"
    );
    legacy_remote_object_ref_from_object_key(
        config,
        scope,
        receipt.artifact_kind.clone(),
        &receipt.object_key,
        &receipt.ciphertext_sha256,
    )
}

fn legacy_remote_object_ref_from_object_key(
    config: &TraceArtifactProviderConfig,
    scope: &TraceArtifactScope,
    artifact_kind: TraceArtifactKind,
    object_key: &str,
    ciphertext_sha256: &str,
) -> anyhow::Result<TraceArtifactObjectRef> {
    let object_ref = TraceArtifactObjectRef {
        provider_kind: TraceArtifactProviderKind::ServiceOwnedRemote,
        object_store: config.object_store.clone(),
        tenant_storage_ref: scope.tenant_storage_ref.clone(),
        submission_storage_ref: scope.submission_storage_ref.clone(),
        artifact_kind,
        object_key: object_key.to_string(),
        ciphertext_sha256: ciphertext_sha256
            .strip_prefix("sha256:")
            .unwrap_or(ciphertext_sha256)
            .to_string(),
    };
    validate_remote_object_ref(scope, config, &object_ref)?;
    Ok(object_ref)
}

fn validate_non_empty_ref(label: &str, value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "{label} must not be empty");
    anyhow::ensure!(
        !value.bytes().any(|byte| byte.is_ascii_control()),
        "{label} must not contain control characters"
    );
    Ok(())
}

fn validate_object_hash(value: &str) -> anyhow::Result<()> {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    anyhow::ensure!(
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "trace artifact ciphertext hash must be a 64-character hex digest"
    );
    Ok(())
}

fn validate_object_key(object_key: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        object_key.len() == 64 && object_key.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "trace artifact object key must be a 64-character hex digest"
    );
    Ok(())
}

fn validate_remote_object_key(object_key: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        object_key.starts_with("v1/tenants/"),
        "remote trace artifact object key must use the v1 tenant partition"
    );
    anyhow::ensure!(
        !object_key
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | "..")),
        "remote trace artifact object key contains unsafe path components"
    );
    anyhow::ensure!(
        object_key.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.')
        }),
        "remote trace artifact object key contains unsupported characters"
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

fn remote_artifact_object_key(
    config: &TraceArtifactProviderConfig,
    scope: &TraceArtifactScope,
    artifact_kind: &TraceArtifactKind,
    object_id: &str,
) -> String {
    let tenant_partition = sha256_text_hex(scope.tenant_storage_ref.as_str());
    let submission_partition = sha256_text_hex(scope.submission_storage_ref.as_str());
    let object_digest = sha256_text_hex(&format!(
        "{}\n{}\n{}\n{}\n{}",
        config.object_store,
        scope.tenant_storage_ref,
        scope.submission_storage_ref,
        artifact_kind.as_path_segment(),
        object_id
    ));
    format!(
        "v1/tenants/{tenant_partition}/submissions/{submission_partition}/{}/{}.json",
        artifact_kind.as_path_segment(),
        object_digest
    )
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
#[derive(Default)]
pub struct InMemoryRemoteTraceArtifactProvider {
    objects:
        std::sync::RwLock<std::collections::HashMap<String, InMemoryRemoteTraceArtifactRecord>>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct InMemoryRemoteTraceArtifactRecord {
    object_ref: TraceArtifactObjectRef,
    artifact: EncryptedTraceArtifact,
    invalidated_at: Option<DateTime<Utc>>,
    invalidation_reason: Option<TraceArtifactInvalidationReason>,
}

#[cfg(test)]
impl RemoteTraceArtifactProvider for InMemoryRemoteTraceArtifactProvider {
    fn put_encrypted_artifact(
        &self,
        object_ref: TraceArtifactObjectRef,
        artifact: EncryptedTraceArtifact,
    ) -> anyhow::Result<()> {
        let mut objects = self
            .objects
            .write()
            .map_err(|_| anyhow::anyhow!("remote trace artifact provider lock poisoned"))?;
        objects.insert(
            object_ref.object_key.clone(),
            InMemoryRemoteTraceArtifactRecord {
                object_ref,
                artifact,
                invalidated_at: None,
                invalidation_reason: None,
            },
        );
        Ok(())
    }

    fn read_encrypted_artifact(
        &self,
        object_ref: &TraceArtifactObjectRef,
    ) -> anyhow::Result<RemoteTraceArtifactRecord> {
        let objects = self
            .objects
            .read()
            .map_err(|_| anyhow::anyhow!("remote trace artifact provider lock poisoned"))?;
        let record = objects.get(&object_ref.object_key).with_context(|| {
            format!(
                "remote trace artifact object not found: {}",
                object_ref.object_key
            )
        })?;
        anyhow::ensure!(
            record.object_ref == *object_ref,
            "remote trace artifact object ref mismatch"
        );
        let _ = record.invalidation_reason;
        Ok(RemoteTraceArtifactRecord {
            object_ref: record.object_ref.clone(),
            artifact: record.artifact.clone(),
            invalidated_at: record.invalidated_at,
        })
    }

    fn invalidate_encrypted_artifact(
        &self,
        object_ref: &TraceArtifactObjectRef,
        reason: TraceArtifactInvalidationReason,
        invalidated_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let mut objects = self
            .objects
            .write()
            .map_err(|_| anyhow::anyhow!("remote trace artifact provider lock poisoned"))?;
        let record = objects.get_mut(&object_ref.object_key).with_context(|| {
            format!(
                "remote trace artifact object not found: {}",
                object_ref.object_key
            )
        })?;
        anyhow::ensure!(
            record.object_ref == *object_ref,
            "remote trace artifact object ref mismatch"
        );
        record.invalidated_at = Some(invalidated_at);
        record.invalidation_reason = Some(reason);
        Ok(())
    }

    fn delete_encrypted_artifact(
        &self,
        object_ref: &TraceArtifactObjectRef,
        _deleted_at: DateTime<Utc>,
    ) -> anyhow::Result<bool> {
        let mut objects = self
            .objects
            .write()
            .map_err(|_| anyhow::anyhow!("remote trace artifact provider lock poisoned"))?;
        Ok(objects.remove(&object_ref.object_key).is_some())
    }
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

    fn test_remote_store() -> ServiceOwnedTraceArtifactStore<InMemoryRemoteTraceArtifactProvider> {
        let key = crate::secrets::keychain::generate_master_key_hex();
        let crypto = SecretsCrypto::new(SecretString::from(key)).expect("test crypto");
        let config = TraceArtifactProviderConfig::service_owned_remote("trace-commons-prod")
            .expect("remote provider config");
        ServiceOwnedTraceArtifactStore::new(
            config,
            crypto,
            InMemoryRemoteTraceArtifactProvider::default(),
        )
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
    fn service_owned_remote_store_satisfies_trace_artifact_store_contract() {
        let store = test_remote_store();

        assert_trace_artifact_store_contract(&store);
    }

    #[test]
    fn service_owned_remote_trace_artifact_store_trait_rejects_cross_tenant_access() {
        let store = test_remote_store();
        let payload = json!({"safe": true});
        let serialized_payload = serde_json::to_vec(&payload).expect("payload serializes");
        let receipt = TraceArtifactStore::put_serialized_json(
            &store,
            "tenant:sha256:alpha",
            TraceArtifactKind::AuditSnapshot,
            "legacy-trait-audit",
            &serialized_payload,
        )
        .expect("artifact writes through trait");

        let read_error = TraceArtifactStore::read_json(&store, "tenant:sha256:beta", &receipt)
            .expect_err("cross-tenant remote trait receipt read must fail");
        assert!(read_error.to_string().contains("tenant mismatch"));

        let object_key_error = TraceArtifactStore::read_json_by_object_key(
            &store,
            "tenant:sha256:beta",
            TraceArtifactKind::AuditSnapshot,
            &receipt.object_key,
            &receipt.ciphertext_sha256,
        )
        .expect_err("cross-tenant remote trait object-key read must fail");
        assert!(object_key_error.to_string().contains("tenant mismatch"));

        let delete_error =
            TraceArtifactStore::delete_artifact(&store, "tenant:sha256:beta", &receipt)
                .expect_err("cross-tenant remote trait delete must fail");
        assert!(delete_error.to_string().contains("tenant mismatch"));
    }

    #[test]
    fn service_owned_remote_store_binds_refs_to_tenant_and_submission() {
        let key = crate::secrets::keychain::generate_master_key_hex();
        let crypto = SecretsCrypto::new(SecretString::from(key)).expect("test crypto");
        let config = TraceArtifactProviderConfig::service_owned_remote("trace-commons-prod")
            .expect("remote provider config");
        let store = ServiceOwnedTraceArtifactStore::new(
            config,
            crypto,
            InMemoryRemoteTraceArtifactProvider::default(),
        );
        let scope = TraceArtifactScope::new("tenant:sha256:alpha", "submission-alpha");
        let payload = json!({"safe": true, "summary": "<redacted>"});

        let receipt = store
            .put_scoped_json(
                &scope,
                TraceArtifactKind::ContributionEnvelope,
                "submitted-envelope",
                &payload,
            )
            .expect("remote artifact writes");

        assert_eq!(
            receipt.object_ref.provider_kind,
            TraceArtifactProviderKind::ServiceOwnedRemote
        );
        assert_eq!(receipt.object_ref.object_store, "trace-commons-prod");
        assert_eq!(receipt.object_ref.tenant_storage_ref, "tenant:sha256:alpha");
        assert_eq!(
            receipt.object_ref.submission_storage_ref,
            "submission-alpha"
        );
        assert!(
            !receipt
                .object_ref
                .object_key
                .contains("tenant:sha256:alpha")
        );
        assert!(!receipt.object_ref.object_key.contains("submission-alpha"));

        let round_trip: serde_json::Value = store
            .read_scoped_json(&scope, &receipt.object_ref)
            .expect("remote artifact reads with matching scope");
        assert_eq!(round_trip, payload);

        let wrong_tenant = TraceArtifactScope::new("tenant:sha256:beta", "submission-alpha");
        let error = store
            .read_scoped_json::<serde_json::Value>(&wrong_tenant, &receipt.object_ref)
            .expect_err("remote refs must not cross tenants");
        assert!(error.to_string().contains("tenant mismatch"));

        let wrong_submission = TraceArtifactScope::new("tenant:sha256:alpha", "submission-beta");
        let error = store
            .read_scoped_json::<serde_json::Value>(&wrong_submission, &receipt.object_ref)
            .expect_err("remote refs must not cross submissions");
        assert!(error.to_string().contains("submission mismatch"));
    }

    #[test]
    fn service_owned_remote_store_returns_invalidate_and_delete_receipts() {
        let key = crate::secrets::keychain::generate_master_key_hex();
        let crypto = SecretsCrypto::new(SecretString::from(key)).expect("test crypto");
        let config = TraceArtifactProviderConfig::service_owned_remote("trace-commons-prod")
            .expect("remote provider config");
        let store = ServiceOwnedTraceArtifactStore::new(
            config,
            crypto,
            InMemoryRemoteTraceArtifactProvider::default(),
        );
        let scope = TraceArtifactScope::new("tenant:sha256:alpha", "submission-alpha");
        let payload = json!({"safe": true});
        let receipt = store
            .put_scoped_json(
                &scope,
                TraceArtifactKind::BenchmarkConversion,
                "conversion-artifact",
                &payload,
            )
            .expect("remote artifact writes");

        let invalidation = store
            .invalidate_scoped_artifact(
                &scope,
                &receipt.object_ref,
                TraceArtifactInvalidationReason::Revoked,
            )
            .expect("remote artifact invalidates");
        assert_eq!(invalidation.object_ref, receipt.object_ref);
        assert_eq!(
            invalidation.reason,
            TraceArtifactInvalidationReason::Revoked
        );

        let error = store
            .read_scoped_json::<serde_json::Value>(&scope, &receipt.object_ref)
            .expect_err("invalidated remote artifact must fail closed");
        assert!(error.to_string().contains("invalidated"));

        let delete_receipt = store
            .delete_scoped_artifact(&scope, &receipt.object_ref)
            .expect("remote artifact deletes");
        assert_eq!(delete_receipt.object_ref, receipt.object_ref);
        assert!(delete_receipt.deleted);

        let repeat_delete = store
            .delete_scoped_artifact(&scope, &receipt.object_ref)
            .expect("remote artifact delete is idempotent");
        assert_eq!(repeat_delete.object_ref, receipt.object_ref);
        assert!(!repeat_delete.deleted);
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
