//! Index and query primitives for the universal `StorageBackend` surface.
//!
//! Stores declare indexes once with [`IndexSpec`], then query with [`Filter`].
//! Backends translate to native machinery (Postgres `CREATE INDEX`, libSQL
//! `fts5` / `vector`, in-memory B-tree, …) — no SQL strings cross the
//! boundary. Indexed values are *projected* by the consumer; backends index
//! only what was declared, never the opaque payload.

use std::fmt;

use ironclaw_host_api::HostApiError;
use serde::{Deserialize, Serialize};

/// Name of an index registered on a mount prefix.
///
/// Validation matches the [`.claude/rules/types.md`] template: non-empty,
/// no path separators, no whitespace, no control characters. Constructing via
/// [`IndexName::new`] is the only way to obtain an instance; wire payloads are
/// validated on deserialize through `try_from = "String"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct IndexName(String);

/// Key of an indexed field within a [`Record`](crate::Record).
///
/// Same shape and validation rules as [`IndexName`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct IndexKey(String);

pub(crate) fn validate_simple_identifier(kind: &'static str, s: &str) -> Result<(), HostApiError> {
    if s.is_empty() {
        return Err(HostApiError::InvalidId {
            kind,
            value: s.to_string(),
            reason: "must not be empty".to_string(),
        });
    }
    if s.chars().count() > 128 {
        return Err(HostApiError::InvalidId {
            kind,
            value: s.to_string(),
            reason: "must be 128 characters or fewer".to_string(),
        });
    }
    // Tightened identifier shape after PR #3661 reviewer flag:
    //   `IndexKey::new("a.b")` used to pass validation but interact with
    //   `json_extract(indexed, '$.a.b')` as a nested-path traversal rather
    //   than the literal key `"a.b"`. Similarly, raw names used as DDL
    //   identifiers without SQL quoting allowed `-` / `.` / unicode through.
    //   Restrict to `[A-Za-z_][A-Za-z0-9_]*` so the same value is safe as a
    //   JSON path component, a SQL identifier, and a row key.
    let bytes = s.as_bytes();
    let first = bytes[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return Err(HostApiError::InvalidId {
            kind,
            value: s.to_string(),
            reason: "must start with an ASCII letter or underscore".to_string(),
        });
    }
    if bytes[1..]
        .iter()
        .any(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
    {
        return Err(HostApiError::InvalidId {
            kind,
            value: s.to_string(),
            reason: "must contain only ASCII letters, digits, and underscores".to_string(),
        });
    }
    // Legacy traversal/whitespace checks retained as belt-and-suspenders;
    // the ASCII alphanumeric rule above already rejects them.
    if s.contains('/')
        || s.contains('\\')
        || s.contains('\0')
        || s.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(HostApiError::InvalidId {
            kind,
            value: s.to_string(),
            reason: "must be a simple identifier with no path separators or whitespace".to_string(),
        });
    }
    Ok(())
}

impl IndexName {
    pub fn new(raw: impl Into<String>) -> Result<Self, HostApiError> {
        let s = raw.into();
        validate_simple_identifier("filesystem index name", &s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for IndexName {
    type Error = HostApiError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for IndexName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IndexName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<IndexName> for String {
    fn from(value: IndexName) -> Self {
        value.0
    }
}

impl IndexKey {
    pub fn new(raw: impl Into<String>) -> Result<Self, HostApiError> {
        let s = raw.into();
        validate_simple_identifier("filesystem index key", &s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl TryFrom<String> for IndexKey {
    type Error = HostApiError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for IndexKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IndexKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<IndexKey> for String {
    fn from(value: IndexKey) -> Self {
        value.0
    }
}

/// Typed value projected into the indexed map of a [`Record`](crate::Record).
///
/// Variants are intentionally narrow — backends translate to their native
/// column type. New variants require coordinated backend updates and a wire
/// migration; do not extend casually.
///
/// Serialization is untagged so SQL backends storing the indexed map as
/// JSON can run native predicates against it (`indexed->>'scope' = 'acme'`
/// in Postgres, `json_extract(indexed, '$.scope') = 'acme'` in libSQL).
/// Bool is listed first so JSON booleans don't accidentally match `I64`,
/// and `Bytes` is last because a JSON array could otherwise be mis-typed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IndexValue {
    Bool(bool),
    I64(i64),
    Text(String),
    Bytes(Vec<u8>),
}

impl fmt::Display for IndexValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(s) => f.write_str(s),
            Self::I64(n) => write!(f, "{n}"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Bytes(b) => write!(f, "<{}B>", b.len()),
        }
    }
}

/// Kind of index a backend should materialize.
///
/// Backends may decline to support some kinds; mount-time capability checks
/// (see [`BackendCapabilities`](crate::BackendCapabilities)) catch a typed
/// store demanding a kind its mount cannot serve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum IndexKind {
    /// Equality lookup on the indexed key(s).
    Exact,
    /// Prefix lookup on a text key (e.g. `scope LIKE 'tenant:acme/%'`).
    Prefix,
    /// Full-text search on a text key. Backends translate to `fts5` /
    /// `tsvector` / equivalent.
    Fts,
    /// Vector similarity index. `dim` is the embedding dimension.
    Vector { dim: u32 },
}

/// Declaration of an index on a mount prefix.
///
/// `keys` is ordered. Backends that support composite indexes use the order;
/// backends that only support single-key indexes accept `keys.len() == 1`
/// and reject otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexSpec {
    pub name: IndexName,
    pub keys: Vec<IndexKey>,
    pub kind: IndexKind,
}

impl IndexSpec {
    /// Construct an index spec from a name, one or more keys, and a kind.
    pub fn new(name: IndexName, keys: Vec<IndexKey>, kind: IndexKind) -> Self {
        Self { name, keys, kind }
    }
}

/// Predicate against indexed values.
///
/// Deliberately narrow: every variant maps cleanly to all supported backends.
/// Backends that cannot serve a particular variant on a given index (e.g. a
/// `Range` on an FTS index) fail with [`FilesystemError::Unsupported`](
/// crate::FilesystemError::Unsupported).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Filter {
    /// Match every record under the queried prefix.
    All,
    /// Match records whose indexed `key` equals `value`.
    Eq {
        key: IndexKey,
        value: IndexValue,
    },
    /// Match records whose indexed `key` starts with `value`. Requires the
    /// index to be `IndexKind::Prefix`.
    PrefixOn {
        key: IndexKey,
        value: IndexValue,
    },
    /// Match records whose indexed `key` falls in `[lo, hi]`.
    Range {
        key: IndexKey,
        lo: IndexValue,
        hi: IndexValue,
    },
    And(Vec<Filter>),
    Or(Vec<Filter>),
}

/// Pagination cursor for [`list`](crate::StorageBackend::list) and
/// [`query`](crate::StorageBackend::query).
///
/// `offset` is 0-based; `limit` is bounded by [`Page::MAX_LIMIT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub offset: u64,
    pub limit: u32,
}

impl Page {
    pub const MAX_LIMIT: u32 = 1024;
    pub const DEFAULT_LIMIT: u32 = 100;

    pub fn new(offset: u64, limit: u32) -> Self {
        Self {
            offset,
            limit: limit.min(Self::MAX_LIMIT),
        }
    }

    pub fn first(limit: u32) -> Self {
        Self::new(0, limit)
    }
}

impl Default for Page {
    fn default() -> Self {
        Self::first(Self::DEFAULT_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_name_rejects_empty_and_separators() {
        assert!(IndexName::new("").is_err());
        assert!(IndexName::new("scope/leases").is_err());
        assert!(IndexName::new("with space").is_err());
        assert!(IndexName::new("ok_name_1").is_ok());
    }

    #[test]
    fn index_key_rejects_chars_that_break_sql_or_json_paths() {
        // Reviewer (PR #3661) flagged that allowing `.` lets
        // `json_extract(indexed, '$.a.b')` traverse rather than match the
        // literal key `"a.b"`, and that other punctuation can break DDL.
        // After tightening, IndexKey/Name accept `[A-Za-z_][A-Za-z0-9_]*`
        // only.
        assert!(IndexKey::new("a.b").is_err());
        assert!(IndexKey::new("a-b").is_err());
        assert!(IndexKey::new("1abc").is_err()); // can't start with digit
        assert!(IndexKey::new("").is_err());
        assert!(IndexKey::new("scope").is_ok());
        assert!(IndexKey::new("_internal").is_ok());
        assert!(IndexKey::new("scope_v2").is_ok());
    }

    #[test]
    fn index_value_orders_within_variant() {
        assert!(IndexValue::I64(1) < IndexValue::I64(2));
        assert!(IndexValue::Text("a".into()) < IndexValue::Text("b".into()));
    }

    #[test]
    fn page_clamps_to_max_limit() {
        let page = Page::new(0, u32::MAX);
        assert_eq!(page.limit, Page::MAX_LIMIT);
    }

    #[test]
    fn index_name_serde_round_trip_validates() {
        let name = IndexName::new("by_scope_status").unwrap();
        let json = serde_json::to_string(&name).unwrap();
        let back: IndexName = serde_json::from_str(&json).unwrap();
        assert_eq!(name, back);
        assert!(serde_json::from_str::<IndexName>("\"bad/name\"").is_err());
    }
}
