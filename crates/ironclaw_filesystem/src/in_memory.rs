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
use crate::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FileType, FilesystemError,
    FilesystemOperation, Filter, IndexCapability, IndexKey, IndexKind, IndexName, IndexSpec,
    IndexValue, Page, RecordVersion, RootFilesystem, SeqNo, TxnCapability, VersionedEntry,
};

#[derive(Clone)]
struct StoredEntry {
    entry: Entry,
    version: RecordVersion,
    modified: SystemTime,
}

struct State {
    entries: HashMap<String, StoredEntry>,
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
        BackendCapabilities {
            read: true,
            write: true,
            append: true,
            list: true,
            stat: true,
            delete: true,
            records: true,
            query: true,
            index: IndexCapability {
                exact: true,
                prefix: true,
                fts: false,
                vector: false,
            },
            txn: TxnCapability::Cas,
            events: true,
            indexed: false,
            embedded: false,
        }
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let mut state = self.state.lock().await;
        let key = path.as_str().to_string();
        let current_version = state.entries.get(&key).map(|stored| stored.version);
        check_cas(path, cas, current_version)?;

        let next_version = current_version
            .map(|v| v.next())
            .unwrap_or_else(|| RecordVersion::from_backend(1));
        state.entries.insert(
            key,
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
        Ok(state
            .entries
            .get(path.as_str())
            .map(|stored| VersionedEntry {
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
        if state.entries.remove(path.as_str()).is_some() {
            // Also sweep any descendants under the deleted path — a
            // record-shaped entry at /a/b plus byte entries at /a/b/c
            // should both be cleared on `delete("/a/b")`.
            let prefix = with_trailing_slash(path.as_str());
            state.entries.retain(|key, _| !key.starts_with(&prefix));
            return Ok(());
        }
        let prefix = with_trailing_slash(path.as_str());
        let before = state.entries.len();
        state.entries.retain(|key, _| !key.starts_with(&prefix));
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
            if let Some(suffix) = stored_path.strip_prefix(&prefix) {
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
        if let Some(stored) = state.entries.get(path.as_str()) {
            return Ok(FileStat {
                path: path.clone(),
                file_type: FileType::File,
                len: stored.entry.body.len() as u64,
                modified: Some(stored.modified),
                sensitive: stored.entry.kind.is_some(),
            });
        }
        let prefix = with_trailing_slash(path.as_str());
        if state.entries.keys().any(|key| key.starts_with(&prefix)) {
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
        let mut matched: Vec<(&String, &StoredEntry)> = state
            .entries
            .iter()
            .filter(|(key, _)| key.as_str() == path.as_str() || key.starts_with(&prefix))
            .filter(|(_, stored)| filter_matches(filter, &stored.entry.indexed))
            .collect();
        matched.sort_by(|a, b| a.0.cmp(b.0));
        let start = page.offset as usize;
        let end = start.saturating_add(page.limit as usize).min(matched.len());
        if start >= matched.len() {
            return Ok(Vec::new());
        }
        Ok(matched[start..end]
            .iter()
            .map(|(_, stored)| VersionedEntry {
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
        // Reject specs whose kind we don't claim to serve.
        match spec.kind {
            IndexKind::Exact | IndexKind::Prefix => {}
            IndexKind::Fts | IndexKind::Vector { .. } => {
                return Err(FilesystemError::Unsupported {
                    path: path.clone(),
                    operation: FilesystemOperation::EnsureIndex,
                });
            }
        }
        let bucket = state.indexes.entry(path.as_str().to_string()).or_default();
        if let Some(existing) = bucket.iter().find(|s| s.name == spec.name) {
            if existing != spec {
                return Err(FilesystemError::IndexConflict {
                    path: path.clone(),
                    name: existing.name.clone(),
                    reason: "spec already declared with different keys or kind".to_string(),
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
        let key = path.as_str().to_string();
        let existing = state.entries.get(&key).cloned();
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
            key,
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
                index_value_variant(lo) == index_value_variant(hi)
                    && index_value_variant(v) == index_value_variant(lo)
                    && v >= lo
                    && v <= hi
            }
            None => false,
        },
        Filter::And(children) => children.iter().all(|f| filter_matches(f, indexed)),
        Filter::Or(children) => children.iter().any(|f| filter_matches(f, indexed)),
    }
}

/// Identify the [`IndexValue`] variant for cross-variant Range matching
/// (see PR #3659 reviewer note in `filter_matches`).
fn index_value_variant(value: &IndexValue) -> u8 {
    match value {
        IndexValue::Bool(_) => 0,
        IndexValue::I64(_) => 1,
        IndexValue::Text(_) => 2,
        IndexValue::Bytes(_) => 3,
    }
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
    async fn ensure_index_rejects_unsupported_kind() {
        let fs = InMemoryBackend::new();
        let prefix = vpath("/memory");
        let spec = IndexSpec::new(
            IndexName::new("by_chunk").unwrap(),
            vec![key("chunk_id")],
            IndexKind::Fts,
        );
        let err = fs.ensure_index(&prefix, &spec).await.unwrap_err();
        assert!(matches!(err, FilesystemError::Unsupported { .. }));
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
    async fn legacy_read_write_file_default_routes_through_put_get() {
        let fs = InMemoryBackend::new();
        let path = vpath("/tmp/legacy.bin");
        // write_file default impl wraps in Entry::bytes + CAS::Any.
        fs.write_file(&path, b"payload").await.unwrap();
        let bytes = fs.read_file(&path).await.unwrap();
        assert_eq!(bytes, b"payload");
    }
}
