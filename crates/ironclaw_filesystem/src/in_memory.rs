//! In-memory [`RootFilesystem`] implementing the full unified surface.
//!
//! Serves as:
//! - A reference implementation that shows how a backend should treat each op.
//! - The default test backend so tests don't need libSQL/Postgres running.
//! - The replacement for the N per-crate `InMemory*Store` implementations that
//!   each consumer used to maintain alongside their SQL backends.
//!
//! Semantics:
//! - Per-path monotonic versions. Each successful [`put`](RootFilesystem::put)
//!   increments the path's version; CAS rejects writes whose
//!   [`CasExpectation`] doesn't match the current version.
//! - Indexed projection is stored alongside the entry and used by
//!   [`query`](RootFilesystem::query). The current implementation evaluates
//!   filters by linear scan — that is fine for tests and small workloads; a
//!   production-grade backend would translate `ensure_index` declarations
//!   into native indexes.
//! - [`append`](RootFilesystem::append)/[`tail`](RootFilesystem::tail) keep
//!   one append log per path with monotonic [`SeqNo`].

use std::collections::HashMap;
use std::time::SystemTime;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;
use tokio::sync::Mutex;

use crate::backend::{EventRecord, StorageTxn};
use crate::vector::{cosine_similarity, decode_embedding_blob};
use crate::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FileType, FilesystemError,
    FilesystemOperation, Filter, IndexKey, IndexKind, IndexName, IndexSpec, IndexValue, Page,
    RecordVersion, RootFilesystem, SeqNo, VersionedEntry,
};

#[derive(Clone)]
struct StoredEntry {
    entry: Entry,
    version: RecordVersion,
    modified: SystemTime,
}

struct State {
    // Audit finding F2: keying on `VirtualPath` directly removes the
    // hot-path `VirtualPath::new(...).unwrap_or_else(unreachable!)` that
    // the prior `HashMap<String, _>` shape forced on every `query` /
    // `list_dir` result. Paths originate as `VirtualPath` on `put`, so
    // they're already validated — re-parsing them on every read was
    // both wasted work and a sloppy invariant to assert via panic.
    entries: HashMap<VirtualPath, StoredEntry>,
    indexes: HashMap<String, Vec<IndexSpec>>,
    event_logs: HashMap<String, Vec<EventRecord>>,
}

/// In-memory backend serving the full unified [`RootFilesystem`] surface.
pub struct InMemoryBackend {
    state: Mutex<State>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                entries: HashMap::new(),
                indexes: HashMap::new(),
                event_logs: HashMap::new(),
            }),
        }
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RootFilesystem for InMemoryBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::in_memory_full()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let mut state = self.state.lock().await;
        // PR #3679 review fix: the SQL backends reject `put(/a)` when `/a/b`
        // already exists. Mirror the SQL contract so cross-backend tests
        // can't pass against impossible production state.
        let prefix = with_trailing_slash(path.as_str());
        if state
            .entries
            .keys()
            .any(|k| k.as_str().starts_with(&prefix))
        {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
                reason: "cannot overwrite a directory".to_string(),
            });
        }
        let current_version = state.entries.get(path).map(|stored| stored.version);
        check_cas(path, cas, current_version)?;

        let next_version = current_version
            .map(|v| v.next())
            .unwrap_or_else(|| RecordVersion::from_backend(1));
        state.entries.insert(
            path.clone(),
            StoredEntry {
                entry,
                version: next_version,
                modified: SystemTime::now(),
            },
        );
        Ok(next_version)
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let state = self.state.lock().await;
        Ok(state.entries.get(path).map(|stored| VersionedEntry {
            path: path.clone(),
            entry: stored.entry.clone(),
            version: stored.version,
        }))
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let mut state = self.state.lock().await;
        // PR #3659 reviewer fix: delete now matches the SQL backends'
        // subtree semantics. If an exact entry exists, remove it.
        // Otherwise, if the path has children (i.e. `stat` would call
        // this a directory), remove every entry under it. Returns
        // NotFound only when neither an exact entry nor any descendants
        // exist.
        if state.entries.remove(path).is_some() {
            // Also sweep any descendants under the deleted path — a
            // record-shaped entry at /a/b plus byte entries at /a/b/c
            // should both be cleared on `delete("/a/b")`.
            let prefix = with_trailing_slash(path.as_str());
            state
                .entries
                .retain(|key, _| !key.as_str().starts_with(&prefix));
            return Ok(());
        }
        let prefix = with_trailing_slash(path.as_str());
        let before = state.entries.len();
        state
            .entries
            .retain(|key, _| !key.as_str().starts_with(&prefix));
        if state.entries.len() == before {
            return Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::Delete,
            });
        }
        Ok(())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let state = self.state.lock().await;
        let prefix = with_trailing_slash(path.as_str());
        let mut seen: HashMap<String, FileType> = HashMap::new();
        for stored_path in state.entries.keys() {
            if let Some(suffix) = stored_path.as_str().strip_prefix(&prefix) {
                let (head, has_more) = first_segment(suffix);
                if head.is_empty() {
                    continue;
                }
                let file_type = if has_more {
                    FileType::Directory
                } else {
                    FileType::File
                };
                // PR #3659 reviewer fix: with `or_insert`, the first
                // discovery wins. If `/a/b` (file) is processed before
                // `/a/b/c` (under-`b` file), `b` would be listed as a
                // File even though it has children. Any path that
                // serves as a prefix for other entries is a Directory
                // in this listing — use `and_modify` to upgrade on a
                // later `has_more` discovery.
                seen.entry(head.to_string())
                    .and_modify(|existing| {
                        if has_more {
                            *existing = FileType::Directory;
                        }
                    })
                    .or_insert(file_type);
            }
        }
        let mut out = Vec::with_capacity(seen.len());
        for (name, file_type) in seen {
            let child_virtual = VirtualPath::new(join_path(path.as_str(), &name))?;
            out.push(DirEntry {
                name,
                path: child_virtual,
                file_type,
            });
        }
        out.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(out)
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let state = self.state.lock().await;
        if let Some(stored) = state.entries.get(path) {
            return Ok(FileStat {
                path: path.clone(),
                file_type: FileType::File,
                len: stored.entry.body.len() as u64,
                modified: Some(stored.modified),
                sensitive: stored.entry.kind.is_some(),
            });
        }
        let prefix = with_trailing_slash(path.as_str());
        if state
            .entries
            .keys()
            .any(|key| key.as_str().starts_with(&prefix))
        {
            return Ok(FileStat {
                path: path.clone(),
                file_type: FileType::Directory,
                len: 0,
                modified: None,
                sensitive: false,
            });
        }
        Err(FilesystemError::NotFound {
            path: path.clone(),
            operation: FilesystemOperation::Stat,
        })
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let state = self.state.lock().await;
        let prefix = with_trailing_slash(path.as_str());
        // Audit finding F2: `State::entries` now keys on `VirtualPath`
        // directly so the per-row `VirtualPath::new(...).unwrap_or_else(
        // unreachable!)` reparse on the hot path is gone. Candidates carry
        // borrowed `&VirtualPath` values that drop straight into
        // `VersionedEntry::path` on a cheap `clone()`.
        let candidates: Vec<(&VirtualPath, &StoredEntry)> = state
            .entries
            .iter()
            .filter(|(key, _)| key.as_str() == path.as_str() || key.as_str().starts_with(&prefix))
            .collect();
        if let Some((key, embedding, limit)) = top_level_vector_nearest(filter) {
            let mut ranked: Vec<(&VirtualPath, &StoredEntry, f32)> = candidates
                .into_iter()
                .filter_map(|(p, stored)| {
                    let stored_vec = match stored.entry.indexed.get(key) {
                        Some(IndexValue::Bytes(bytes)) => decode_embedding_blob(bytes)?,
                        _ => return None,
                    };
                    cosine_similarity(embedding, &stored_vec).map(|s| (p, stored, s))
                })
                .collect();
            ranked.sort_by(|a, b| {
                b.2.partial_cmp(&a.2)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.0.as_str().cmp(b.0.as_str()))
            });
            ranked.truncate(limit as usize);
            return Ok(ranked
                .into_iter()
                .map(|(matched_path, stored, _)| VersionedEntry {
                    path: matched_path.clone(),
                    entry: stored.entry.clone(),
                    version: stored.version,
                })
                .collect());
        }
        // Audit finding F5: a `Filter::VectorNearest` nested inside
        // `And`/`Or` is `Unsupported` on both SQL backends (the WHERE-
        // fragment translator refuses to inline a ranking op as a
        // predicate; the top of `query` only handles a top-level
        // `VectorNearest`). The in-memory backend previously treated a
        // nested `VectorNearest` as "any row with `IndexValue::Bytes` at
        // `key`", silently changing semantics across backends. Align by
        // surfacing the same `Unsupported` error before the scalar
        // filter loop runs.
        if contains_nested_vector_nearest(filter) {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::Query,
            });
        }
        let mut matched: Vec<(&VirtualPath, &StoredEntry)> = candidates
            .into_iter()
            .filter(|(_, stored)| filter_matches(filter, &stored.entry.indexed))
            .collect();
        matched.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        let start = page.offset as usize;
        let end = start.saturating_add(page.limit as usize).min(matched.len());
        if start >= matched.len() {
            return Ok(Vec::new());
        }
        Ok(matched[start..end]
            .iter()
            .map(|(matched_path, stored)| VersionedEntry {
                path: (*matched_path).clone(),
                entry: stored.entry.clone(),
                version: stored.version,
            })
            .collect())
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        let mut state = self.state.lock().await;
        // The in-memory backend serves Exact/Prefix natively, and serves
        // Fts/Vector as brute-force linear scans driven by the filter
        // translator. Reject only specs we genuinely can't materialize.
        match &spec.kind {
            IndexKind::Exact | IndexKind::Prefix | IndexKind::Fts => {}
            IndexKind::Vector { dim } => {
                if *dim == 0 {
                    return Err(FilesystemError::IndexConflict {
                        path: path.clone(),
                        name: spec.name.clone(),
                        reason: crate::IndexConflictReason::SpecMismatch,
                    });
                }
            }
        }
        let bucket = state.indexes.entry(path.as_str().to_string()).or_default();
        if let Some(existing) = bucket.iter().find(|s| s.name == spec.name) {
            if existing != spec {
                return Err(FilesystemError::IndexConflict {
                    path: path.clone(),
                    name: existing.name.clone(),
                    reason: crate::IndexConflictReason::SpecMismatch,
                });
            }
            return Ok(());
        }
        bucket.push(spec.clone());
        Ok(())
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        // In-memory backend supports CAS only; multi-key transactions would
        // require a separate state snapshot. Consumers must use CAS.
        Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::BeginTxn,
        })
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        let mut state = self.state.lock().await;
        let log = state
            .event_logs
            .entry(path.as_str().to_string())
            .or_default();
        let next = log
            .last()
            .map(|rec| rec.seq.next())
            .unwrap_or_else(|| SeqNo::ZERO.next());
        log.push(EventRecord { seq: next, payload });
        Ok(next)
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let state = self.state.lock().await;
        let Some(log) = state.event_logs.get(path.as_str()) else {
            return Ok(Vec::new());
        };
        Ok(log
            .iter()
            .filter(|record| record.seq > from)
            .cloned()
            .collect())
    }

    // Legacy bytes ops — default impls in the trait route them through put/get
    // and use our native implementations. The only one needing an explicit
    // impl is the required-method `list_dir`, which we already overrode above.
    // We provide `append_file` for byte-append symmetry.

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let mut state = self.state.lock().await;
        let existing = state.entries.get(path).cloned();
        let mut entry = existing
            .as_ref()
            .map(|s| s.entry.clone())
            .unwrap_or_else(|| Entry::bytes(Vec::new()));
        if entry.kind.is_some() {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::AppendFile,
                reason: "cannot append bytes to a record-shaped entry".to_string(),
            });
        }
        entry.body.extend_from_slice(bytes);
        let next_version = existing
            .map(|s| s.version.next())
            .unwrap_or_else(|| RecordVersion::from_backend(1));
        state.entries.insert(
            path.clone(),
            StoredEntry {
                entry,
                version: next_version,
                modified: SystemTime::now(),
            },
        );
        Ok(())
    }
}

fn check_cas(
    path: &VirtualPath,
    cas: CasExpectation,
    current: Option<RecordVersion>,
) -> Result<(), FilesystemError> {
    match (cas, current) {
        (CasExpectation::Any, _) => Ok(()),
        (CasExpectation::Absent, None) => Ok(()),
        (CasExpectation::Absent, found @ Some(_)) => Err(FilesystemError::VersionMismatch {
            path: path.clone(),
            expected: None,
            found,
        }),
        (CasExpectation::Version(expected), Some(found)) if expected == found => Ok(()),
        (CasExpectation::Version(expected), found) => Err(FilesystemError::VersionMismatch {
            path: path.clone(),
            expected: Some(expected),
            found,
        }),
    }
}

fn filter_matches(
    filter: &Filter,
    indexed: &std::collections::BTreeMap<IndexKey, IndexValue>,
) -> bool {
    match filter {
        Filter::All => true,
        Filter::Eq { key, value } => indexed.get(key) == Some(value),
        Filter::PrefixOn { key, value } => match (indexed.get(key), value) {
            (Some(IndexValue::Text(stored)), IndexValue::Text(prefix)) => {
                stored.starts_with(prefix)
            }
            _ => false,
        },
        Filter::Range { key, lo, hi } => match indexed.get(key) {
            Some(v) => {
                // PR #3659 reviewer fix: Filter::Range previously used
                // the derived IndexValue Ord, which orders across
                // variants by their declared position. That meant a
                // numeric `lo` and a Bytes `hi` could match Bool/Text
                // values purely on enum-variant ordering rather than
                // domain ordering. Require all three sides to share a
                // variant; mismatched bound/value variants don't match.
                let lo_d = std::mem::discriminant(lo);
                let hi_d = std::mem::discriminant(hi);
                let v_d = std::mem::discriminant(v);
                lo_d == hi_d && v_d == lo_d && v >= lo && v <= hi
            }
            None => false,
        },
        Filter::Fts { key, query } => match indexed.get(key) {
            Some(IndexValue::Text(stored)) => fts_naive_matches(stored, query),
            _ => false,
        },
        // Audit finding F5: `Filter::VectorNearest` is a ranking operation
        // and is only meaningful at the top level of a `query` filter.
        // The top of `query` extracts a top-level `VectorNearest` before
        // any scalar `filter_matches` call, and a `contains_nested_vector_nearest`
        // pre-check rejects nested occurrences with `Unsupported`. Reaching
        // this arm therefore indicates that pre-check was bypassed; return
        // `false` (the conservative answer; the caller already errored) so
        // we don't fall through to "match any row with a bytes value at
        // key" the way prior versions did.
        Filter::VectorNearest { .. } => false,
        Filter::And(children) => children.iter().all(|f| filter_matches(f, indexed)),
        Filter::Or(children) => children.iter().any(|f| filter_matches(f, indexed)),
    }
}

/// Coarse FTS approximation: tokenize the query on whitespace and require
/// every token to appear (case-insensitively) in the stored text. This
/// matches FTS5's default `AND`-of-terms behavior closely enough for the
/// in-memory reference; the SQL backends use the real engines.
fn fts_naive_matches(stored: &str, query: &str) -> bool {
    let stored_lower = stored.to_lowercase();
    query
        .split_whitespace()
        .all(|token| stored_lower.contains(&token.to_lowercase()))
}

/// If `filter` is a top-level `VectorNearest` (the only shape the SQL
/// backends and this reference implementation evaluate by ranking rather
/// than predication), return its components.
fn top_level_vector_nearest(filter: &Filter) -> Option<(&IndexKey, &[f32], u32)> {
    if let Filter::VectorNearest {
        key,
        embedding,
        limit,
    } = filter
    {
        return Some((key, embedding, *limit));
    }
    None
}

/// Walk `filter` and report whether any `VectorNearest` occurs strictly
/// inside an `And`/`Or` compound. A top-level `VectorNearest` is handled
/// by the query method's ranking path; nested occurrences are rejected
/// with `Unsupported` to match the SQL backends (audit finding F5).
fn contains_nested_vector_nearest(filter: &Filter) -> bool {
    fn walk(filter: &Filter, inside_compound: bool) -> bool {
        match filter {
            Filter::VectorNearest { .. } => inside_compound,
            Filter::And(children) | Filter::Or(children) => {
                children.iter().any(|child| walk(child, true))
            }
            _ => false,
        }
    }
    walk(filter, false)
}

fn with_trailing_slash(s: &str) -> String {
    if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

fn first_segment(s: &str) -> (&str, bool) {
    match s.find('/') {
        Some(idx) => (&s[..idx], true),
        None => (s, false),
    }
}

fn join_path(parent: &str, child: &str) -> String {
    if parent.ends_with('/') {
        format!("{parent}{child}")
    } else {
        format!("{parent}/{child}")
    }
}

// Silence unused warning on `IndexName` import while keeping the symbol
// available for documentation/test crates that re-export from this module.
#[allow(dead_code)]
const _: fn() -> Option<IndexName> = || None;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RecordKind, VersionedEntry};

    fn vpath(s: &str) -> VirtualPath {
        VirtualPath::new(s).unwrap()
    }

    fn key(s: &str) -> IndexKey {
        IndexKey::new(s).unwrap()
    }

    #[tokio::test]
    async fn put_get_round_trip_for_opaque_file() {
        let fs = InMemoryBackend::new();
        let path = vpath("/projects/notes.md");
        let body = b"hello world".to_vec();
        let version = fs
            .put(&path, Entry::bytes(body.clone()), CasExpectation::Absent)
            .await
            .unwrap();
        let got: VersionedEntry = fs.get(&path).await.unwrap().unwrap();
        assert_eq!(got.entry.body, body);
        assert!(got.entry.is_opaque_file());
        assert_eq!(got.version, version);
    }

    #[tokio::test]
    async fn cas_absent_rejects_when_present() {
        let fs = InMemoryBackend::new();
        let path = vpath("/secrets/leases/L1");
        fs.put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();
        let err = fs
            .put(&path, Entry::bytes(vec![2]), CasExpectation::Absent)
            .await
            .unwrap_err();
        assert!(matches!(err, FilesystemError::VersionMismatch { .. }));
    }

    #[tokio::test]
    async fn cas_version_advances_monotonically() {
        let fs = InMemoryBackend::new();
        let path = vpath("/secrets/leases/L2");
        let v1 = fs
            .put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();
        let v2 = fs
            .put(&path, Entry::bytes(vec![2]), CasExpectation::Version(v1))
            .await
            .unwrap();
        assert!(v2 > v1);
        // Stale version is rejected.
        let err = fs
            .put(&path, Entry::bytes(vec![3]), CasExpectation::Version(v1))
            .await
            .unwrap_err();
        assert!(matches!(err, FilesystemError::VersionMismatch { .. }));
    }

    #[tokio::test]
    async fn query_filters_on_indexed_projection() {
        let fs = InMemoryBackend::new();
        let kind = RecordKind::new("lease").unwrap();
        for (path, scope, status) in [
            ("/secrets/leases/A", "acme", "active"),
            ("/secrets/leases/B", "acme", "revoked"),
            ("/secrets/leases/C", "globex", "active"),
        ] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(key("scope"), IndexValue::Text(scope.into()))
                .with_indexed(key("status"), IndexValue::Text(status.into()));
            fs.put(&vpath(path), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        let results = fs
            .query(
                &vpath("/secrets/leases"),
                &Filter::And(vec![
                    Filter::Eq {
                        key: key("scope"),
                        value: IndexValue::Text("acme".into()),
                    },
                    Filter::Eq {
                        key: key("status"),
                        value: IndexValue::Text("active".into()),
                    },
                ]),
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn ensure_index_rejects_kind_conflict() {
        let fs = InMemoryBackend::new();
        let prefix = vpath("/secrets/leases");
        let name = IndexName::new("by_scope").unwrap();
        let spec_a = IndexSpec::new(name.clone(), vec![key("scope")], IndexKind::Exact);
        let spec_b = IndexSpec::new(name, vec![key("scope")], IndexKind::Prefix);
        fs.ensure_index(&prefix, &spec_a).await.unwrap();
        // Re-declaring same spec is idempotent.
        fs.ensure_index(&prefix, &spec_a).await.unwrap();
        // Conflicting kind on same name fails.
        let err = fs.ensure_index(&prefix, &spec_b).await.unwrap_err();
        assert!(matches!(err, FilesystemError::IndexConflict { .. }));
    }

    #[tokio::test]
    async fn ensure_index_accepts_fts_and_vector_kinds() {
        // The in-memory reference now serves FTS as a substring scan and
        // Vector as a brute-force cosine rank; both are accepted at
        // declaration time. Backend implementations may still decline
        // (e.g. a backend without pgvector); the trait-level capability
        // declaration is what gates real backends.
        let fs = InMemoryBackend::new();
        let capabilities = fs.capabilities();
        assert!(capabilities.has(crate::Capability::IndexFts));
        assert!(capabilities.has(crate::Capability::IndexVector));
        let prefix = vpath("/memory");
        let fts = IndexSpec::new(
            IndexName::new("by_chunk").unwrap(),
            vec![key("chunk_id")],
            IndexKind::Fts,
        );
        fs.ensure_index(&prefix, &fts).await.unwrap();
        let vector = IndexSpec::new(
            IndexName::new("by_vec").unwrap(),
            vec![key("embedding")],
            IndexKind::Vector { dim: 384 },
        );
        fs.ensure_index(&prefix, &vector).await.unwrap();
    }

    #[tokio::test]
    async fn fts_filter_matches_naive_substring_tokens() {
        let fs = InMemoryBackend::new();
        let kind = RecordKind::new("chunk").unwrap();
        for (path, text) in [
            ("/memory/A", "the quick brown fox"),
            ("/memory/B", "lazy dogs sleep"),
            ("/memory/C", "the fox jumps over"),
        ] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(key("content"), IndexValue::Text(text.into()));
            fs.put(&vpath(path), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        let results = fs
            .query(
                &vpath("/memory"),
                &Filter::Fts {
                    key: key("content"),
                    query: "fox".into(),
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn vector_nearest_returns_top_k_by_cosine() {
        let fs = InMemoryBackend::new();
        let kind = RecordKind::new("chunk").unwrap();
        let blob = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|f| f.to_le_bytes()).collect() };
        for (path, vec) in [
            ("/memory/A", vec![1.0_f32, 0.0, 0.0]),
            ("/memory/B", vec![0.9_f32, 0.1, 0.0]),
            ("/memory/C", vec![0.0_f32, 0.0, 1.0]),
        ] {
            let entry = Entry::record(kind.clone(), &serde_json::json!({}))
                .unwrap()
                .with_indexed(key("embedding"), IndexValue::Bytes(blob(&vec)));
            fs.put(&vpath(path), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        let results = fs
            .query(
                &vpath("/memory"),
                &Filter::VectorNearest {
                    key: key("embedding"),
                    embedding: vec![1.0_f32, 0.0, 0.0],
                    limit: 2,
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        // /memory/A is the closest match (identical vector).
        assert_eq!(
            results[0].entry.indexed.get(&key("embedding")),
            Some(&IndexValue::Bytes(blob(&[1.0, 0.0, 0.0])))
        );
    }

    #[tokio::test]
    async fn append_and_tail_assigns_monotonic_seqno() {
        let fs = InMemoryBackend::new();
        let log = vpath("/events/engine");
        let s1 = fs.append(&log, b"a".to_vec()).await.unwrap();
        let s2 = fs.append(&log, b"b".to_vec()).await.unwrap();
        let s3 = fs.append(&log, b"c".to_vec()).await.unwrap();
        assert!(s1 < s2 && s2 < s3);
        let tail = fs.tail(&log, SeqNo::ZERO).await.unwrap();
        assert_eq!(tail.len(), 3);
        let tail_after_first = fs.tail(&log, s1).await.unwrap();
        assert_eq!(tail_after_first.len(), 2);
    }

    #[tokio::test]
    async fn delete_removes_entry() {
        let fs = InMemoryBackend::new();
        let path = vpath("/tmp/x");
        fs.put(&path, Entry::bytes(vec![1]), CasExpectation::Absent)
            .await
            .unwrap();
        assert!(fs.get(&path).await.unwrap().is_some());
        fs.delete(&path).await.unwrap();
        assert!(fs.get(&path).await.unwrap().is_none());
        let err = fs.delete(&path).await.unwrap_err();
        assert!(matches!(err, FilesystemError::NotFound { .. }));
    }

    #[tokio::test]
    async fn delete_directory_removes_subtree() {
        // PR #3659 reviewer fix: deleting a directory path (no exact
        // entry, but children exist) used to return NotFound and leave
        // the subtree behind, diverging from the SQL backends'
        // subtree-delete semantics.
        let fs = InMemoryBackend::new();
        for p in ["/projects/dir/a", "/projects/dir/b", "/projects/dir/sub/c"] {
            fs.put(&vpath(p), Entry::bytes(vec![1]), CasExpectation::Absent)
                .await
                .unwrap();
        }
        let dir = vpath("/projects/dir");
        // No exact entry at /projects/dir, but children exist → treat
        // as a directory and remove the subtree.
        fs.delete(&dir).await.unwrap();
        for p in ["/projects/dir/a", "/projects/dir/b", "/projects/dir/sub/c"] {
            assert!(fs.get(&vpath(p)).await.unwrap().is_none());
        }
        // Now NotFound — the subtree is gone.
        let err = fs.delete(&dir).await.unwrap_err();
        assert!(matches!(err, FilesystemError::NotFound { .. }));
    }

    #[tokio::test]
    async fn list_dir_upgrades_to_directory_on_later_child_discovery() {
        // PR #3659 reviewer fix: with `or_insert`, the first discovery
        // of a name decided its FileType. Path /a/b inserted first as
        // a File could shadow /a/b/c arriving later, leaving `b`
        // listed as a File even though it has children.
        let fs = InMemoryBackend::new();
        // Insert /projects/x as a leaf file, then /projects/x/y as a
        // file under x — `x` should now list as Directory because it
        // has children, regardless of insertion order.
        fs.put(
            &vpath("/projects/x"),
            Entry::bytes(vec![1]),
            CasExpectation::Absent,
        )
        .await
        .unwrap();
        fs.put(
            &vpath("/projects/x/y"),
            Entry::bytes(vec![2]),
            CasExpectation::Absent,
        )
        .await
        .unwrap();
        let entries = fs.list_dir(&vpath("/projects")).await.unwrap();
        let x = entries
            .iter()
            .find(|e| e.name == "x")
            .expect("/projects/x should be listed");
        assert_eq!(x.file_type, FileType::Directory);
    }

    #[tokio::test]
    async fn filter_range_rejects_cross_variant_bounds() {
        // PR #3659 reviewer fix: Filter::Range used to rely on derived
        // IndexValue Ord, which orders across variants by their
        // declared position. That meant numeric lo + Bytes hi could
        // include Bool/Text values purely on enum ordering. We now
        // require all three sides (lo, hi, stored) to share a variant.
        let fs = InMemoryBackend::new();
        let kind = RecordKind::new("widget").unwrap();
        let key = IndexKey::new("size").unwrap();
        let entry = Entry::record(kind, &serde_json::json!({}))
            .unwrap()
            .with_indexed(key.clone(), IndexValue::Text("medium".into()));
        fs.put(&vpath("/projects/W1"), entry, CasExpectation::Absent)
            .await
            .unwrap();
        // Numeric lo + Bytes hi over a Text-valued indexed field must
        // not match.
        let results = fs
            .query(
                &vpath("/projects"),
                &Filter::Range {
                    key,
                    lo: IndexValue::I64(0),
                    hi: IndexValue::Bytes(vec![0xff]),
                },
                Page::default(),
            )
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn list_dir_returns_direct_children_only() {
        let fs = InMemoryBackend::new();
        for p in [
            "/projects/a.md",
            "/projects/sub/b.md",
            "/projects/sub/c.md",
            "/projects/d.md",
        ] {
            fs.put(&vpath(p), Entry::bytes(vec![]), CasExpectation::Absent)
                .await
                .unwrap();
        }
        let mut names: Vec<String> = fs
            .list_dir(&vpath("/projects"))
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.md", "d.md", "sub"]);
    }

    #[tokio::test]
    async fn list_dir_bounded_uses_trait_default_truncation() {
        let fs = InMemoryBackend::new();
        for p in ["/projects/a.md", "/projects/b.md", "/projects/c.md"] {
            fs.put(&vpath(p), Entry::bytes(vec![]), CasExpectation::Absent)
                .await
                .unwrap();
        }

        let entries = fs.list_dir_bounded(&vpath("/projects"), 2).await.unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a.md");
        assert_eq!(entries[1].name, "b.md");
    }

    #[tokio::test]
    async fn legacy_read_write_file_default_routes_through_put_get() {
        let fs = InMemoryBackend::new();
        let path = vpath("/tmp/legacy.bin");
        // write_file default impl wraps in Entry::bytes + CAS::Any.
        fs.write_file(&path, b"payload").await.unwrap();
        let bytes = fs.read_file(&path).await.unwrap();
        assert_eq!(bytes, b"payload");
    }

    #[tokio::test]
    async fn bounded_read_default_impl_routes_through_stat_and_get() {
        let fs = InMemoryBackend::new();
        let path = vpath("/tmp/bounded.bin");
        fs.put(
            &path,
            Entry::bytes(b"bounded payload".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .unwrap();

        assert_eq!(
            fs.read_file_bounded(&path, 15).await.unwrap(),
            Some(b"bounded payload".to_vec())
        );
        assert_eq!(fs.read_file_bounded(&path, 14).await.unwrap(), None);
    }

    #[tokio::test]
    async fn get_and_query_populate_versioned_entry_path() {
        // PR #3659 review fix: `VersionedEntry` now carries the
        // [`VirtualPath`] of the record so query consumers can drive
        // `put`/`delete` workflows directly off the result.
        let fs = InMemoryBackend::new();
        let path = vpath("/memory/a");
        let kind = crate::RecordKind::new("test").unwrap();
        let entry = crate::Entry::record(kind.clone(), &serde_json::json!({"k": 1}))
            .unwrap()
            .with_indexed(
                crate::IndexKey::new("k").unwrap(),
                crate::IndexValue::I64(1),
            );
        fs.put(&path, entry, CasExpectation::Absent).await.unwrap();

        let got = fs.get(&path).await.unwrap().expect("get returns Some");
        assert_eq!(got.path, path, "get must populate VersionedEntry.path");

        let results = fs
            .query(
                &vpath("/memory"),
                &crate::Filter::Eq {
                    key: crate::IndexKey::new("k").unwrap(),
                    value: crate::IndexValue::I64(1),
                },
                crate::Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].path, path,
            "query must populate VersionedEntry.path for every row"
        );
    }

    #[tokio::test]
    async fn range_filter_rejects_cross_variant_stored_values() {
        // PR #3659 review fix: a numeric Range must not match rows that
        // happen to have a text value under the same indexed key.
        // (The in-memory backend already enforced this via the
        // `std::mem::discriminant` fix; this regression test locks it in.)
        let fs = InMemoryBackend::new();
        let kind = crate::RecordKind::new("test").unwrap();
        let key = crate::IndexKey::new("v").unwrap();
        for (path_str, value) in [
            ("/memory/numeric", crate::IndexValue::I64(5)),
            ("/memory/text", crate::IndexValue::Text("5".into())),
        ] {
            let entry = crate::Entry::record(kind.clone(), &serde_json::json!({"v": 5}))
                .unwrap()
                .with_indexed(key.clone(), value);
            fs.put(&vpath(path_str), entry, CasExpectation::Absent)
                .await
                .unwrap();
        }
        let results = fs
            .query(
                &vpath("/memory"),
                &crate::Filter::Range {
                    key,
                    lo: crate::IndexValue::I64(1),
                    hi: crate::IndexValue::I64(10),
                },
                crate::Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1, "only the numeric row should match");
        assert_eq!(results[0].path.as_str(), "/memory/numeric");
    }

    #[tokio::test]
    async fn vector_nearest_nested_in_and_or_returns_unsupported() {
        // Audit finding F5: cross-backend semantic alignment. SQL backends
        // reject `VectorNearest` nested inside `And`/`Or` with
        // `Unsupported` because ranking can't be expressed as a WHERE
        // fragment. Previously the in-memory backend silently treated
        // such a filter as "match any row whose `key` is an
        // `IndexValue::Bytes`", which is semantically nothing like the
        // SQL result. The in-memory backend must now surface
        // `Unsupported` too.
        let fs = InMemoryBackend::new();
        let kind = crate::RecordKind::new("chunk").unwrap();
        let blob = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|f| f.to_le_bytes()).collect() };
        let entry = crate::Entry::record(kind, &serde_json::json!({}))
            .unwrap()
            .with_indexed(key("embedding"), IndexValue::Bytes(blob(&[1.0_f32, 0.0])));
        fs.put(&vpath("/memory/A"), entry, CasExpectation::Absent)
            .await
            .unwrap();

        let nested_and = crate::Filter::And(vec![crate::Filter::VectorNearest {
            key: key("embedding"),
            embedding: vec![1.0_f32, 0.0],
            limit: 5,
        }]);
        let err = fs
            .query(&vpath("/memory"), &nested_and, crate::Page::default())
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                FilesystemError::Unsupported {
                    operation: FilesystemOperation::Query,
                    ..
                }
            ),
            "VectorNearest nested in And must error Unsupported, got {err:?}"
        );

        let nested_or = crate::Filter::Or(vec![
            crate::Filter::Eq {
                key: key("embedding"),
                value: IndexValue::Bytes(blob(&[1.0_f32, 0.0])),
            },
            crate::Filter::VectorNearest {
                key: key("embedding"),
                embedding: vec![1.0_f32, 0.0],
                limit: 5,
            },
        ]);
        let err = fs
            .query(&vpath("/memory"), &nested_or, crate::Page::default())
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                FilesystemError::Unsupported {
                    operation: FilesystemOperation::Query,
                    ..
                }
            ),
            "VectorNearest nested in Or must error Unsupported, got {err:?}"
        );

        // Top-level VectorNearest still works.
        let top = crate::Filter::VectorNearest {
            key: key("embedding"),
            embedding: vec![1.0_f32, 0.0],
            limit: 5,
        };
        let ok = fs
            .query(&vpath("/memory"), &top, crate::Page::default())
            .await;
        assert!(ok.is_ok(), "top-level VectorNearest must still work");
    }
}
