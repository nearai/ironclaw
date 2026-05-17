//! Universal `Entry` and related primitives for the unified storage surface.
//!
//! The kernel storage rework collapses the historical bytes plane
//! (read/write opaque files) and record plane (typed rows with queryable
//! columns) into a single shape: every put/get on the filesystem deals in an
//! [`Entry`] — a body of bytes plus optional schema metadata. A "file" is an
//! [`Entry`] with `kind = None` and an empty indexed projection. A "record"
//! is an [`Entry`] with `kind = Some(_)` and one or more declared indexed
//! projection values. There is no parallel API for the two cases; the same
//! put/get/query/CAS machinery serves both.
//!
//! Encryption-at-rest is applied by an
//! [`EncryptedBackend`](crate::EncryptedBackend) decorator over the
//! [`Entry::body`] and any `IndexValue::Bytes` projection so the indexed
//! scalar values (`scope`, `status`, …) remain queryable on the underlying
//! mount.
//!
//! Atomicity uses [`CasExpectation`] rather than closure-based transactions.
//! Stores work with compare-and-swap as the primitive; backends that support
//! richer multi-key transactions expose them through
//! [`StorageTxn`](crate::StorageTxn) but consumers must never depend on it.

use std::collections::BTreeMap;
use std::fmt;

use ironclaw_host_api::HostApiError;
use serde::{Deserialize, Serialize};

use crate::index::{IndexKey, IndexValue};

/// Schema family identifier for a record-shaped entry (e.g.
/// `credential_account`, `engine_thread`). Same validation as
/// [`IndexName`](crate::IndexName) / [`IndexKey`](crate::IndexKey): non-empty,
/// no separators, no whitespace, no control characters, ≤ 128 chars.
///
/// A [`None`] kind on an [`Entry`] marks the entry as an opaque file with no
/// schema; backends must accept these even if they otherwise serve records.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RecordKind(String);

impl RecordKind {
    pub fn new(raw: impl Into<String>) -> Result<Self, HostApiError> {
        let s = raw.into();
        crate::index::validate_simple_identifier("filesystem record kind", &s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for RecordKind {
    type Error = HostApiError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for RecordKind {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RecordKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<RecordKind> for String {
    fn from(value: RecordKind) -> Self {
        value.0
    }
}

/// MIME-style content-type hint for an [`Entry::body`].
///
/// Backends use this to choose a storage representation when it matters
/// (Postgres `JSONB` vs `BYTEA`, libSQL JSON vs BLOB). The hint is advisory:
/// every backend must accept any content type and store the bytes faithfully.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct ContentType(String);

impl ContentType {
    pub const OCTET_STREAM: &'static str = "application/octet-stream";
    pub const JSON: &'static str = "application/json";
    pub const MARKDOWN: &'static str = "text/markdown";

    pub fn new(raw: impl Into<String>) -> Result<Self, HostApiError> {
        let s = raw.into();
        if s.is_empty() {
            return Err(HostApiError::InvalidId {
                kind: "filesystem content type",
                value: s,
                reason: "must not be empty".to_string(),
            });
        }
        if s.chars().count() > 128 {
            return Err(HostApiError::InvalidId {
                kind: "filesystem content type",
                value: s,
                reason: "must be 128 characters or fewer".to_string(),
            });
        }
        // Allow a narrow MIME-style alphabet: letters, digits, and the
        // separators commonly seen in IANA media types.
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '+' | '-' | '.' | '_'))
        {
            return Err(HostApiError::InvalidId {
                kind: "filesystem content type",
                value: s,
                reason: "must contain only mime-style ascii characters".to_string(),
            });
        }
        Ok(Self(s))
    }

    pub fn octet_stream() -> Self {
        Self(Self::OCTET_STREAM.to_string())
    }

    pub fn json() -> Self {
        Self(Self::JSON.to_string())
    }

    pub fn markdown() -> Self {
        Self(Self::MARKDOWN.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for ContentType {
    type Error = HostApiError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Default for ContentType {
    fn default() -> Self {
        Self::octet_stream()
    }
}

impl AsRef<str> for ContentType {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<ContentType> for String {
    fn from(value: ContentType) -> Self {
        value.0
    }
}

/// Monotonically increasing version assigned by the backend to each successful
/// [`put`](crate::StorageBackend::put). Opaque to consumers: only compared for
/// equality via [`CasExpectation::Version`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecordVersion(u64);

impl RecordVersion {
    /// Internal constructor for backends. Consumers obtain versions only by
    /// reading existing entries — they cannot fabricate one.
    pub fn from_backend(raw: u64) -> Self {
        Self(raw)
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for RecordVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Monotonic sequence number used by the append/tail event plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SeqNo(u64);

impl SeqNo {
    pub const ZERO: Self = Self(0);

    pub fn from_backend(raw: u64) -> Self {
        Self(raw)
    }

    pub fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for SeqNo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Compare-and-swap precondition for [`put`](crate::StorageBackend::put).
///
/// All multi-step store operations (lease claim, lease consume, status
/// transitions) are implemented with `CasExpectation::Version` and retry on
/// [`FilesystemError::VersionMismatch`](crate::FilesystemError::VersionMismatch).
/// Closure-based transactions across async boundaries are intentionally absent
/// — backends that need atomic multi-key updates expose
/// [`StorageTxn`](crate::StorageTxn) separately, and consumers must continue
/// to work when only CAS is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "version")]
pub enum CasExpectation {
    /// Path must not currently hold an entry. Used for issue/create.
    Absent,
    /// Path must currently hold the named version. Used for claim/consume/
    /// status transitions.
    Version(RecordVersion),
    /// Overwrite regardless of current state. Used only by backfills / admin
    /// flows; domain code should default to one of the other variants.
    Any,
}

/// The universal "thing stored at a virtual path".
///
/// - **Opaque file**: `body` carries arbitrary bytes, `kind` is `None`,
///   `indexed` is empty. This is the shape used by project files, artifacts,
///   memory document Markdown, and anything that wouldn't have benefitted from
///   a SQL row.
/// - **Record**: `body` carries the serialized payload (typically JSON), `kind`
///   names the schema family (e.g. `credential_account`), and `indexed`
///   declares the projection that backends should expose to
///   [`query`](crate::StorageBackend::query).
///
/// Backends never look inside `body` for indexing; everything queryable lives
/// in `indexed`. This keeps the indexing contract small enough to be served
/// portably by libSQL, Postgres, local-file sidecar indexes, and HSM-backed
/// mounts without each backend having to parse the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub body: Vec<u8>,
    pub content_type: ContentType,
    pub kind: Option<RecordKind>,
    pub indexed: BTreeMap<IndexKey, IndexValue>,
}

impl Entry {
    /// Construct an opaque-file entry from bytes.
    pub fn bytes(body: Vec<u8>) -> Self {
        Self {
            body,
            content_type: ContentType::octet_stream(),
            kind: None,
            indexed: BTreeMap::new(),
        }
    }

    /// Construct an opaque-file entry from UTF-8 text with `text/markdown`.
    pub fn markdown(text: impl Into<String>) -> Self {
        Self {
            body: text.into().into_bytes(),
            content_type: ContentType::markdown(),
            kind: None,
            indexed: BTreeMap::new(),
        }
    }

    /// Construct a record-shaped entry by serializing `data` as JSON.
    pub fn record(kind: RecordKind, data: &serde_json::Value) -> Result<Self, serde_json::Error> {
        let body = serde_json::to_vec(data)?;
        Ok(Self {
            body,
            content_type: ContentType::json(),
            kind: Some(kind),
            indexed: BTreeMap::new(),
        })
    }

    /// Add or overwrite an indexed projection value.
    pub fn with_indexed(mut self, key: IndexKey, value: IndexValue) -> Self {
        self.indexed.insert(key, value);
        self
    }

    /// Replace the content-type hint.
    pub fn with_content_type(mut self, content_type: ContentType) -> Self {
        self.content_type = content_type;
        self
    }

    /// Convenience: is this entry an opaque file (`kind = None`)?
    pub fn is_opaque_file(&self) -> bool {
        self.kind.is_none()
    }

    /// Convenience: deserialize `body` as JSON. Returns an error if the entry
    /// is not record-shaped or the body is not valid JSON.
    pub fn parse_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }
}

/// Versioned read result returned by [`get`](crate::RootFilesystem::get) and
/// [`query`](crate::RootFilesystem::query).
///
/// `version` is the value to pass to [`CasExpectation::Version`] in a
/// subsequent write to avoid lost updates. `path` carries the addressable
/// [`VirtualPath`] of the record so query consumers can drive
/// `put`/`delete` workflows without re-deriving the path from
/// `entry.indexed` (PR #3659 review fix).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionedEntry {
    pub path: ironclaw_host_api::VirtualPath,
    pub entry: Entry,
    pub version: RecordVersion,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_kind_rejects_invalid() {
        assert!(RecordKind::new("").is_err());
        assert!(RecordKind::new("bad/kind").is_err());
        assert!(RecordKind::new("credential_account").is_ok());
    }

    #[test]
    fn content_type_rejects_invalid_and_accepts_mime() {
        assert!(ContentType::new("").is_err());
        assert!(ContentType::new("text with space").is_err());
        assert!(ContentType::new("application/json").is_ok());
        assert!(ContentType::new("application/vnd.api+json").is_ok());
        assert_eq!(ContentType::default().as_str(), ContentType::OCTET_STREAM);
    }

    #[test]
    fn record_version_orders_and_advances() {
        let v0 = RecordVersion::from_backend(0);
        let v1 = v0.next();
        assert!(v0 < v1);
        assert_eq!(v1.get(), 1);
    }

    #[test]
    fn entry_bytes_and_record_constructors() {
        let raw = Entry::bytes(vec![1, 2, 3]);
        assert!(raw.is_opaque_file());
        assert_eq!(raw.body, vec![1, 2, 3]);
        assert_eq!(raw.content_type.as_str(), ContentType::OCTET_STREAM);

        let kind = RecordKind::new("test").unwrap();
        let rec = Entry::record(kind.clone(), &serde_json::json!({"a": 1})).unwrap();
        assert_eq!(rec.kind.as_ref(), Some(&kind));
        assert_eq!(rec.content_type.as_str(), ContentType::JSON);
        let parsed: serde_json::Value = rec.parse_json().unwrap();
        assert_eq!(parsed, serde_json::json!({"a": 1}));
    }

    #[test]
    fn entry_indexed_projection_round_trip() {
        let kind = RecordKind::new("lease").unwrap();
        let scope = IndexKey::new("scope").unwrap();
        let status = IndexKey::new("status").unwrap();
        let entry = Entry::record(kind, &serde_json::json!({"hidden": true}))
            .unwrap()
            .with_indexed(scope.clone(), IndexValue::Text("acme".into()))
            .with_indexed(status.clone(), IndexValue::Text("active".into()));
        assert_eq!(entry.indexed.len(), 2);
        assert!(entry.indexed.contains_key(&scope));
        assert!(entry.indexed.contains_key(&status));
    }

    #[test]
    fn cas_expectation_serde_round_trip() {
        let cases = [
            CasExpectation::Absent,
            CasExpectation::Any,
            CasExpectation::Version(RecordVersion::from_backend(7)),
        ];
        for case in cases {
            let json = serde_json::to_string(&case).unwrap();
            let back: CasExpectation = serde_json::from_str(&json).unwrap();
            assert_eq!(case, back);
        }
    }
}
