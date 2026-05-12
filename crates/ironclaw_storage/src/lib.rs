//! Shared storage substrate primitives for Reborn persistence adapters.
//!
//! This crate owns reusable persistence mechanics only: backend identity,
//! redacted storage errors, JSON serialization helpers, pagination limits,
//! and migration descriptors. Domain crates still own their store traits,
//! schemas, validation, and query semantics.

use async_trait::async_trait;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, IgnoredAny},
};
use thiserror::Error;

/// Sentinel value for optional scope components stored in composite SQL keys.
///
/// Domain stores should continue to own which scope fields participate in a
/// key. This constant only keeps the storage representation consistent across
/// adapters when an optional scope component is absent.
pub const ABSENT_SCOPE_COMPONENT: &str = "";

/// Supported durable backend families known to the shared substrate.
///
/// This is an identity/support marker, not an authority grant and not a domain
/// repository selector. Composition code may use it for diagnostics and
/// migration routing after the owning domain has selected a store.
/// `Filesystem` identifies a blob/record store implementation backed by
/// filesystem mechanics; it does not grant file-shaped path authority or turn
/// this crate into a filesystem abstraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageBackendKind {
    Memory,
    LibSql,
    Postgres,
    Filesystem,
    Object,
}

impl StorageBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::LibSql => "libsql",
            Self::Postgres => "postgres",
            Self::Filesystem => "filesystem",
            Self::Object => "object",
        }
    }
}

/// Redacted storage-substrate error.
///
/// Variants intentionally avoid carrying raw backend messages. Domain stores can
/// map these into their own error types without leaking SQL details, host paths,
/// secret material, or provider/runtime payloads.
// Copy is intentional: variants must remain payload-free so redaction stays trivial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StorageError {
    #[error("storage backend operation failed")]
    Backend,
    #[error("storage serialization failed")]
    Serialization,
    #[error("storage migration failed")]
    Migration,
    #[error("storage record validation failed")]
    Validation,
    #[error("storage write conflict")]
    Conflict,
    #[error("storage operation is unsupported by backend")]
    Unsupported,
}

/// Convert any backend error into a redacted storage error.
///
/// The input is accepted so callers can pass concrete DB/client errors at the
/// call site, but it is deliberately discarded to prevent accidental leakage.
/// Callers that need operational diagnostics must log the raw error before
/// passing it here; this function is the redaction boundary.
pub fn redacted_backend_error(error: impl std::fmt::Display) -> StorageError {
    tracing::error!(%error, "storage backend operation failed");
    StorageError::Backend
}

/// Serialize a structured payload without exposing serializer internals.
pub fn encode_json<T: Serialize>(value: &T) -> Result<String, StorageError> {
    serde_json::to_string(value).map_err(|_| StorageError::Serialization)
}

/// Deserialize a structured payload without exposing raw payload snippets.
pub fn decode_json<T: DeserializeOwned>(value: &str) -> Result<T, StorageError> {
    serde_json::from_str(value).map_err(|_| StorageError::Serialization)
}

/// Return a stable SQL value for an optional scoped identifier.
pub fn optional_scope_component(value: Option<&str>) -> &str {
    value.unwrap_or(ABSENT_SCOPE_COMPONENT)
}

/// Backend-neutral storage key for primitive stores.
///
/// Keys may be path-like, but they are not authority-bearing filesystem paths.
/// Domain stores own key grammar and scope semantics before constructing one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct StorageKey(String);

impl StorageKey {
    pub fn new(value: impl Into<String>) -> Result<Self, StorageError> {
        let value = value.into();
        validate_storage_key(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for StorageKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(|_| serde::de::Error::custom("invalid storage key"))
    }
}

impl AsRef<str> for StorageKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for StorageKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Opaque backend version/fencing value for primitive compare-and-swap writes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct StorageVersion(String);

impl StorageVersion {
    pub fn new(value: impl Into<String>) -> Result<Self, StorageError> {
        let value = value.into();
        validate_storage_token(&value, "storage version", 128)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for StorageVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(|_| serde::de::Error::custom("invalid storage version"))
    }
}

impl AsRef<str> for StorageVersion {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for StorageVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Primitive write precondition shared by record/blob-like backends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PutCondition {
    #[default]
    Any,
    IfAbsent,
    IfVersion(StorageVersion),
}

impl PutCondition {
    pub fn allows(&self, current: Option<&StorageVersion>) -> bool {
        match self {
            Self::Any => true,
            Self::IfAbsent => current.is_none(),
            Self::IfVersion(expected) => current == Some(expected),
        }
    }
}

/// Blob payload returned by [`BlobStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    pub key: StorageKey,
    pub bytes: Vec<u8>,
    pub version: StorageVersion,
}

/// Validated structured JSON payload for [`RecordStore`] values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordPayloadJson(String);

impl RecordPayloadJson {
    pub fn new(value: impl Into<String>) -> Result<Self, StorageError> {
        let value = value.into();
        let _: IgnoredAny =
            serde_json::from_str(&value).map_err(|_| StorageError::Serialization)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for RecordPayloadJson {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for RecordPayloadJson {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Structured JSON record returned by [`RecordStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRecord {
    pub key: StorageKey,
    pub payload_json: RecordPayloadJson,
    pub version: StorageVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutBlobRequest {
    pub key: StorageKey,
    pub bytes: Vec<u8>,
    pub condition: PutCondition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutRecordRequest {
    pub key: StorageKey,
    pub payload_json: RecordPayloadJson,
    pub condition: PutCondition,
}

/// Primitive binary/object storage. Domain semantics live above this trait.
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put_blob(&self, request: PutBlobRequest) -> Result<StoredBlob, StorageError>;

    async fn get_blob(&self, key: &StorageKey) -> Result<Option<StoredBlob>, StorageError>;

    async fn delete_blob(
        &self,
        key: &StorageKey,
        condition: PutCondition,
    ) -> Result<(), StorageError>;
}

/// Primitive keyed structured-record storage. Domain stores own schemas.
#[async_trait]
pub trait RecordStore: Send + Sync {
    async fn put_record(&self, request: PutRecordRequest) -> Result<StoredRecord, StorageError>;

    async fn get_record(&self, key: &StorageKey) -> Result<Option<StoredRecord>, StorageError>;

    async fn delete_record(
        &self,
        key: &StorageKey,
        condition: PutCondition,
    ) -> Result<(), StorageError>;
}

fn validate_storage_key(value: &str) -> Result<(), StorageError> {
    validate_storage_token(value, "storage key", 512)?;
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.split('/').any(|segment| segment == "..")
        || looks_like_windows_absolute_path(value)
    {
        return Err(StorageError::Validation);
    }
    Ok(())
}

fn validate_storage_token(
    value: &str,
    _label: &'static str,
    max_bytes: usize,
) -> Result<(), StorageError> {
    if value.is_empty() || value.len() > max_bytes {
        return Err(StorageError::Validation);
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(StorageError::Validation);
    }
    Ok(())
}

fn looks_like_windows_absolute_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
        && bytes[0].is_ascii_alphabetic()
}

/// Bounded pagination helper for storage reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageLimit {
    value: usize,
}

impl PageLimit {
    pub fn new(requested: usize, default: usize, max: usize) -> Self {
        let max = max.max(1);
        let default = default.clamp(1, max);
        let value = if requested == 0 {
            default
        } else {
            requested.min(max)
        };
        Self { value }
    }

    pub fn get(self) -> usize {
        self.value
    }
}

/// Static migration descriptor used by storage adapters and composition code.
///
/// The SQL remains owned by the domain adapter; this descriptor gives shared
/// diagnostics and migration registries a common shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageMigration {
    pub id: &'static str,
    pub description: &'static str,
    pub backend: StorageBackendKind,
    pub sql: Option<&'static str>,
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicU64, Ordering},
        },
    };

    use super::*;

    #[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    struct Payload {
        value: String,
    }

    #[derive(Default)]
    struct InMemoryPrimitiveStore {
        blobs: Mutex<HashMap<StorageKey, StoredBlob>>,
        records: Mutex<HashMap<StorageKey, StoredRecord>>,
        next_version: AtomicU64,
    }

    impl InMemoryPrimitiveStore {
        fn next_storage_version(&self) -> StorageVersion {
            let version = self.next_version.fetch_add(1, Ordering::Relaxed) + 1;
            StorageVersion::new(format!("memory-{version}"))
                .expect("generated storage version is valid")
        }
    }

    #[async_trait::async_trait]
    impl BlobStore for InMemoryPrimitiveStore {
        async fn put_blob(&self, request: PutBlobRequest) -> Result<StoredBlob, StorageError> {
            let mut blobs = self.blobs.lock().map_err(|_| StorageError::Backend)?;
            let allowed = request
                .condition
                .allows(blobs.get(&request.key).map(|blob| &blob.version));
            if !allowed {
                return Err(StorageError::Conflict);
            }
            let stored = StoredBlob {
                key: request.key,
                bytes: request.bytes,
                version: self.next_storage_version(),
            };
            blobs.insert(stored.key.clone(), stored.clone());
            Ok(stored)
        }

        async fn get_blob(&self, key: &StorageKey) -> Result<Option<StoredBlob>, StorageError> {
            let blobs = self.blobs.lock().map_err(|_| StorageError::Backend)?;
            Ok(blobs.get(key).cloned())
        }

        async fn delete_blob(
            &self,
            key: &StorageKey,
            condition: PutCondition,
        ) -> Result<(), StorageError> {
            let mut blobs = self.blobs.lock().map_err(|_| StorageError::Backend)?;
            if !condition.allows(blobs.get(key).map(|blob| &blob.version)) {
                return Err(StorageError::Conflict);
            }
            blobs.remove(key);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl RecordStore for InMemoryPrimitiveStore {
        async fn put_record(
            &self,
            request: PutRecordRequest,
        ) -> Result<StoredRecord, StorageError> {
            let mut records = self.records.lock().map_err(|_| StorageError::Backend)?;
            let allowed = request
                .condition
                .allows(records.get(&request.key).map(|record| &record.version));
            if !allowed {
                return Err(StorageError::Conflict);
            }
            let stored = StoredRecord {
                key: request.key,
                payload_json: request.payload_json,
                version: self.next_storage_version(),
            };
            records.insert(stored.key.clone(), stored.clone());
            Ok(stored)
        }

        async fn get_record(&self, key: &StorageKey) -> Result<Option<StoredRecord>, StorageError> {
            let records = self.records.lock().map_err(|_| StorageError::Backend)?;
            Ok(records.get(key).cloned())
        }

        async fn delete_record(
            &self,
            key: &StorageKey,
            condition: PutCondition,
        ) -> Result<(), StorageError> {
            let mut records = self.records.lock().map_err(|_| StorageError::Backend)?;
            if !condition.allows(records.get(key).map(|record| &record.version)) {
                return Err(StorageError::Conflict);
            }
            records.remove(key);
            Ok(())
        }
    }

    #[test]
    fn json_helpers_round_trip_without_domain_semantics() {
        let payload = Payload {
            value: "hello".to_string(),
        };

        let encoded = encode_json(&payload).expect("test payload serializes");
        let decoded: Payload = decode_json(&encoded).expect("test payload deserializes");

        assert_eq!(decoded, payload);
    }

    #[test]
    fn json_decode_error_is_redacted() {
        let error = decode_json::<Payload>("{RAW_SECRET").unwrap_err();

        assert_eq!(error, StorageError::Serialization);
        assert!(!format!("{error:?}").contains("RAW_SECRET"));
        assert!(!error.to_string().contains("RAW_SECRET"));
    }

    #[test]
    fn backend_error_discards_raw_detail() {
        let error = redacted_backend_error("host path /tmp/secret.db failed");

        assert_eq!(error, StorageError::Backend);
        assert!(!format!("{error:?}").contains("/tmp/secret.db"));
        assert!(!error.to_string().contains("/tmp/secret.db"));
    }

    #[test]
    fn storage_key_and_version_reject_empty_control_and_oversized_values() {
        assert_eq!(
            StorageKey::new("thread/message").unwrap().as_str(),
            "thread/message"
        );
        assert_eq!(StorageKey::new("").unwrap_err(), StorageError::Validation);
        assert_eq!(
            StorageKey::new("bad\nkey").unwrap_err(),
            StorageError::Validation
        );
        assert_eq!(
            StorageKey::new("x".repeat(513)).unwrap_err(),
            StorageError::Validation
        );
        assert_eq!(StorageVersion::new("v1").unwrap().as_str(), "v1");
        assert_eq!(
            StorageVersion::new("v".repeat(129)).unwrap_err(),
            StorageError::Validation
        );
    }

    #[test]
    fn storage_key_and_version_reject_invalid_deserialized_values() {
        assert!(serde_json::from_str::<StorageKey>("\"\"").is_err());
        assert!(serde_json::from_str::<StorageKey>(&format!("\"{}\"", "x".repeat(513))).is_err());
        assert!(serde_json::from_str::<StorageKey>("\"bad\\nkey\"").is_err());
        assert!(serde_json::from_str::<StorageVersion>("\"\"").is_err());
        assert!(
            serde_json::from_str::<StorageVersion>(&format!("\"{}\"", "v".repeat(129))).is_err()
        );
    }

    #[test]
    fn storage_version_serializes_and_deserializes_transparently() {
        let version = StorageVersion::new("opaque-v1").unwrap();

        let encoded = serde_json::to_string(&version).unwrap();
        let decoded: StorageVersion = serde_json::from_str(&encoded).unwrap();

        assert_eq!(encoded, "\"opaque-v1\"");
        assert_eq!(decoded, version);
    }

    #[test]
    fn storage_key_rejects_path_traversal_and_platform_absolute_forms() {
        for invalid in ["../x", "/x", "a/../../b", "a\\..\\b", "C:\\secrets"] {
            assert_eq!(
                StorageKey::new(invalid).unwrap_err(),
                StorageError::Validation
            );
        }
        assert_eq!(
            StorageKey::new("version..2").unwrap().as_str(),
            "version..2"
        );
    }

    #[test]
    fn record_payload_json_rejects_malformed_payloads() {
        assert!(RecordPayloadJson::new("{\"ok\":true}").is_ok());
        assert_eq!(
            RecordPayloadJson::new("not json").unwrap_err(),
            StorageError::Serialization
        );
    }

    #[test]
    fn put_conditions_encode_cas_without_domain_semantics() {
        let current = StorageVersion::new("v1").unwrap();
        let other = StorageVersion::new("v2").unwrap();

        assert!(PutCondition::Any.allows(None));
        assert!(PutCondition::Any.allows(Some(&current)));
        assert!(PutCondition::IfAbsent.allows(None));
        assert!(!PutCondition::IfAbsent.allows(Some(&current)));
        assert!(PutCondition::IfVersion(current.clone()).allows(Some(&current)));
        assert!(!PutCondition::IfVersion(other).allows(Some(&current)));
        assert!(!PutCondition::IfVersion(current).allows(None));
    }

    #[tokio::test]
    async fn in_memory_store_exercises_blob_and_record_traits() {
        let store = InMemoryPrimitiveStore::default();
        let blob_key = StorageKey::new("blobs/example").unwrap();

        assert!(store.get_blob(&blob_key).await.unwrap().is_none());
        let first_blob = store
            .put_blob(PutBlobRequest {
                key: blob_key.clone(),
                bytes: b"first".to_vec(),
                condition: PutCondition::IfAbsent,
            })
            .await
            .unwrap();
        assert_eq!(first_blob.bytes, b"first");
        assert_eq!(
            store
                .put_blob(PutBlobRequest {
                    key: blob_key.clone(),
                    bytes: b"conflict".to_vec(),
                    condition: PutCondition::IfAbsent,
                })
                .await
                .unwrap_err(),
            StorageError::Conflict
        );
        let updated_blob = store
            .put_blob(PutBlobRequest {
                key: blob_key.clone(),
                bytes: b"second".to_vec(),
                condition: PutCondition::IfVersion(first_blob.version.clone()),
            })
            .await
            .unwrap();
        assert_eq!(updated_blob.bytes, b"second");
        assert_ne!(updated_blob.version, first_blob.version);
        assert_eq!(
            store.get_blob(&blob_key).await.unwrap().unwrap(),
            updated_blob
        );
        store
            .delete_blob(&blob_key, PutCondition::Any)
            .await
            .unwrap();
        assert!(store.get_blob(&blob_key).await.unwrap().is_none());

        let record_key = StorageKey::new("records/example").unwrap();
        let payload = RecordPayloadJson::new(r#"{"value":"hello"}"#).unwrap();
        let stored_record = store
            .put_record(PutRecordRequest {
                key: record_key.clone(),
                payload_json: payload.clone(),
                condition: PutCondition::IfAbsent,
            })
            .await
            .unwrap();
        assert_eq!(stored_record.payload_json, payload);
        assert_eq!(
            store.get_record(&record_key).await.unwrap().unwrap(),
            stored_record
        );
        store
            .delete_record(&record_key, PutCondition::Any)
            .await
            .unwrap();
        assert!(store.get_record(&record_key).await.unwrap().is_none());
    }

    #[test]
    fn page_limit_applies_default_and_max_bounds() {
        assert_eq!(PageLimit::new(0, 50, 100).get(), 50);
        assert_eq!(PageLimit::new(500, 50, 100).get(), 100);
        assert_eq!(PageLimit::new(10, 50, 100).get(), 10);
        assert_eq!(PageLimit::new(0, 0, 0).get(), 1);
    }
}
