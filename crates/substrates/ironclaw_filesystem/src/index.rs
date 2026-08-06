//! Index and query primitives for the universal `RootFilesystem` surface.
//!
//! Stores declare indexes once with [`IndexSpec`], then query with [`Filter`].
//! Backends translate to native machinery (Postgres `CREATE INDEX`, libSQL
//! `fts5` / `vector`, in-memory B-tree, …) — no SQL strings cross the
//! boundary. Indexed values are *projected* by the consumer; backends index
//! only what was declared, never the opaque payload.

use std::fmt;

use ironclaw_host_api::error::HostApiError;
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

/// Key of an indexed field within an [`Entry`](crate::Entry).
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
    // Audit finding F8: `bytes[0]` is safe here because the
    // `s.is_empty()` check above rules out a zero-length slice, but
    // indexing-by-position is fragile to refactors that reorder the
    // emptiness guard. `bytes.first()` makes the dependency explicit
    // and removes the panic path entirely.
    let Some(&first) = bytes.first() else {
        return Err(HostApiError::InvalidId {
            kind,
            value: s.to_string(),
            reason: "must not be empty".to_string(),
        });
    };
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

/// Typed value projected into the indexed map of an [`Entry`](crate::Entry).
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
    ///
    /// **Type constraint** (audit finding F3): `IndexKind::Prefix` is only
    /// meaningful against [`IndexValue::Text`] values. Backends accept this
    /// kind at `ensure_index` time without inspecting future
    /// [`IndexValue`] variants — but [`Filter::PrefixOn`] rejects every
    /// non-text variant at query time with
    /// [`FilesystemError::Unsupported`](crate::FilesystemError::Unsupported).
    /// Consumers that declare `IndexKind::Prefix` on a numeric or boolean
    /// projection will silently get an unused index and a query-time
    /// failure; declare `IndexKind::Exact` instead.
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
///
/// `Eq` is not derived because [`Filter::VectorNearest`] carries a `Vec<f32>`
/// embedding, and `f32` doesn't implement `Eq`. `PartialEq` is enough for the
/// places this type is compared (tests + serde round-trips).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Full-text search on a text-valued indexed `key`. Requires the index
    /// to be `IndexKind::Fts`. `query` is plain user text, never backend query
    /// language: punctuation separates terms, common English function words
    /// do not become required matches, and words such as `AND`/`OR`/`NOT`
    /// cannot become operators. Each backend translates those semantics to
    /// its native query language (FTS5 on libSQL, `plainto_tsquery` on
    /// PostgreSQL).
    Fts {
        key: IndexKey,
        query: String,
    },
    /// Vector-similarity search on a vector-valued indexed `key`. Requires
    /// the index to be `IndexKind::Vector { dim }` with a matching `dim`.
    /// `embedding` is the query vector; results are ranked by descending
    /// cosine similarity and the top `limit` returned.
    ///
    /// `limit` truncates after similarity ranking. This overrides the
    /// caller's [`Page::limit`] on the surrounding query — vector search
    /// is inherently a top-k operation and pagination through ranked
    /// results would require carrying a similarity cursor that the index
    /// surface doesn't expose.
    VectorNearest {
        key: IndexKey,
        embedding: Vec<f32>,
        limit: u32,
    },
    And(Vec<Filter>),
    Or(Vec<Filter>),
}

/// Normalize a plain-text FTS query into the required content terms shared by
/// the in-memory and libSQL backends. PostgreSQL's fixed `english`
/// `plainto_tsquery` path already performs the equivalent punctuation and
/// high-frequency function-word handling.
///
/// Keeping this parser outside the libSQL translator is important: the
/// reference backend and the shipping embedded backend must agree on whether
/// a query is empty and which terms are required. The returned strings contain
/// only Unicode alphanumeric characters, so the libSQL backend can quote every
/// term as an FTS5 literal without exposing caller text as FTS syntax.
pub(crate) fn plain_fts_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .filter(|term| !is_plain_fts_stop_word(term))
        .map(ToOwned::to_owned)
        .collect()
}

fn is_plain_fts_stop_word(term: &str) -> bool {
    matches!(
        term.to_ascii_lowercase().as_str(),
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "been"
            | "being"
            | "but"
            | "by"
            | "can"
            | "could"
            | "did"
            | "do"
            | "does"
            | "for"
            | "from"
            | "had"
            | "has"
            | "have"
            | "how"
            | "i"
            | "in"
            | "is"
            | "it"
            | "me"
            | "my"
            | "not"
            | "of"
            | "on"
            | "or"
            | "please"
            | "should"
            | "tell"
            | "that"
            | "the"
            | "this"
            | "to"
            | "was"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "who"
            | "why"
            | "with"
            | "would"
            | "you"
            | "your"
    )
}

/// Pagination cursor for [`list_dir`](crate::RootFilesystem::list_dir) and
/// [`query`](crate::RootFilesystem::query).
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

/// Stable ordering for an indexed keyset query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// Last row returned by an indexed keyset query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedQueryCursor {
    pub value: IndexValue,
    pub tie_breaker: IndexValue,
}

/// Bounded keyset page over one declared indexed projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedPage {
    pub index: IndexName,
    pub key: IndexKey,
    pub tie_breaker: IndexKey,
    pub direction: SortDirection,
    pub after: Option<OrderedQueryCursor>,
    pub limit: u32,
}

impl OrderedPage {
    pub fn new(
        index: IndexName,
        key: IndexKey,
        tie_breaker: IndexKey,
        direction: SortDirection,
        limit: u32,
    ) -> Self {
        Self {
            index,
            key,
            tie_breaker,
            direction,
            after: None,
            limit: limit.clamp(1, Page::MAX_LIMIT),
        }
    }

    pub fn after(mut self, cursor: OrderedQueryCursor) -> Self {
        self.after = Some(cursor);
        self
    }
}

/// Number of projected key columns (`k0`..`k7`) an ordered index carries.
///
/// One definition for both SQL backends' projection DDL and the query-side
/// tie-breaker guard: a second copy that drifts silently stops projecting the
/// extra key.
pub(crate) const MAX_ORDERED_INDEX_KEYS: usize = 8;

/// All ancestor prefixes of `path`, **most specific first**, ending at `/`.
///
/// Index-spec resolution walks this chain so a caller may declare an index on
/// a higher prefix and query a child path (the "declare high, query low"
/// contract the FTS path already documented). The walk is bounded by path
/// depth — never a catalog scan — and "most specific first" is what gives
/// callers most-specific-spec-wins when several ancestors declare the same
/// index name.
pub(crate) fn ancestor_prefixes(path: &str) -> Vec<&str> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return vec!["/"];
    }
    let mut out = vec![trimmed];
    let mut end = trimmed.len();
    // `rfind('/')` can only return the offset of an ASCII '/', so every index
    // here is already a char boundary and neither `get` can yield `None`.
    // Going through `get` keeps this a total function — a path segment holding
    // multi-byte characters can never be split into a panic.
    while let Some(index) = trimmed.get(..end).and_then(|head| head.rfind('/')) {
        if index == 0 {
            out.push("/");
            break;
        }
        let Some(parent) = trimmed.get(..index) else {
            break;
        };
        out.push(parent);
        end = index;
    }
    out
}

/// Whether `candidate` is `prefix` itself or lies in its subtree.
///
/// Ordered-index rows are keyed by spec name and path only, with no record of
/// which declaration prefix projected them. Once resolution can match an
/// ancestor spec, every backend must re-apply this containment check to the
/// matched rows, or a query would return rows from sibling subtrees that the
/// caller's scope does not cover.
pub(crate) fn path_within_prefix(candidate: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return true;
    }
    candidate == prefix
        || (candidate.len() > prefix.len()
            && candidate.starts_with(prefix)
            && candidate.as_bytes()[prefix.len()] == b'/')
}

pub(crate) fn ordered_query_prefix_values(
    spec: &IndexSpec,
    filter: &Filter,
    page: &OrderedPage,
) -> Option<Vec<IndexValue>> {
    if spec.name != page.index || !matches!(spec.kind, IndexKind::Exact | IndexKind::Prefix) {
        return None;
    }
    let sort_position = spec.keys.iter().position(|key| key == &page.key)?;
    if spec.keys.get(sort_position.saturating_add(1)) != Some(&page.tie_breaker) {
        return None;
    }
    let mut equality_values = std::collections::BTreeMap::new();
    if !collect_equality_values(filter, &mut equality_values) {
        return None;
    }
    let prefix = spec.keys.get(..sort_position)?;
    let prefix_keys = prefix
        .iter()
        .map(IndexKey::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    if equality_values
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        != prefix_keys
    {
        return None;
    }
    prefix
        .iter()
        .map(|key| equality_values.get(key.as_str()).cloned())
        .collect()
}

fn collect_equality_values<'a>(
    filter: &'a Filter,
    values: &mut std::collections::BTreeMap<&'a str, IndexValue>,
) -> bool {
    match filter {
        Filter::All => true,
        Filter::Eq { key, value } => values.insert(key.as_str(), value.clone()).is_none(),
        Filter::And(filters) => filters
            .iter()
            .all(|filter| collect_equality_values(filter, values)),
        Filter::PrefixOn { .. }
        | Filter::Range { .. }
        | Filter::Fts { .. }
        | Filter::VectorNearest { .. }
        | Filter::Or(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestor_prefixes_walk_from_most_to_least_specific() {
        // Order is the precedence rule: spec resolution takes the first match,
        // so the most specific declaration must come first.
        assert_eq!(
            ancestor_prefixes("/threads/agents/a/owners/u/threads/t-1/messages"),
            vec![
                "/threads/agents/a/owners/u/threads/t-1/messages",
                "/threads/agents/a/owners/u/threads/t-1",
                "/threads/agents/a/owners/u/threads",
                "/threads/agents/a/owners/u",
                "/threads/agents/a/owners",
                "/threads/agents/a",
                "/threads/agents",
                "/threads",
                "/",
            ]
        );
        assert_eq!(ancestor_prefixes("/threads"), vec!["/threads", "/"]);
        assert_eq!(ancestor_prefixes("/"), vec!["/"]);
        // A trailing separator must not produce a distinct candidate.
        assert_eq!(ancestor_prefixes("/threads/"), vec!["/threads", "/"]);
    }

    #[test]
    fn path_within_prefix_requires_a_segment_boundary() {
        assert!(path_within_prefix("/a/b", "/a/b"));
        assert!(path_within_prefix("/a/b/c", "/a/b"));
        // The classic prefix-matching bug: a sibling sharing a textual prefix
        // is not in the subtree, and must not be returned by a scoped query.
        assert!(!path_within_prefix("/a/bc", "/a/b"));
        assert!(!path_within_prefix("/a", "/a/b"));
        assert!(path_within_prefix("/anything", "/"));
    }

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
    fn plain_fts_terms_make_natural_language_backend_safe() {
        assert_eq!(
            plain_fts_terms("What is the launch-code-plum-42?"),
            ["launch", "code", "plum", "42"]
        );
        assert_eq!(plain_fts_terms("launch AND code"), ["launch", "code"]);
        assert_eq!(plain_fts_terms("launch OR code"), ["launch", "code"]);
        assert_eq!(plain_fts_terms("launch NOT code"), ["launch", "code"]);
        assert_eq!(plain_fts_terms("héllo, wörld!"), ["héllo", "wörld"]);
        assert!(plain_fts_terms("?! AND the").is_empty());
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
