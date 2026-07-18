//! Filesystem-backed process and process-result stores.
//!
//! Records live under the `/processes` mount alias on a
//! [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem). The paths in
//! this module are alias-relative [`ScopedPath`] strings — at every op the
//! [`ScopedFilesystem`] resolves the alias against its caller-supplied
//! [`MountView`](ironclaw_host_api::MountView) and enforces per-grant ACL
//! before backend dispatch. The composition layer wires the alias to a
//! tenant/user-scoped [`VirtualPath`](ironclaw_host_api::VirtualPath), so
//! tenant isolation is structural rather than something this crate must
//! re-derive from `ResourceScope.tenant_id`/`user_id`.
//!
//! Within the alias, sub-scope (`agent_id`, `project_id`, `mission_id`,
//! `thread_id`) is still encoded in the path so a single tenant/user can
//! own multiple agent/project/mission/thread cells:
//!
//! ```text
//! /processes[/agents/<agent>][/projects/<project>][/missions/<mission>][/threads/<thread>]/records/<process_id>.json
//! /processes[/agents/<agent>][/projects/<project>][/missions/<mission>][/threads/<thread>]/results/<process_id>.json
//! /processes[/agents/<agent>][/projects/<project>][/missions/<mission>][/threads/<thread>]/outputs/<process_id>/output.json
//! ```
//!
//! All path/serde helpers are private to this module since they are tied to
//! the on-disk layout above.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_events::sanitize_error_kind;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, Filter, IndexKey,
    IndexKind, IndexName, IndexSpec, IndexValue, Page, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{ProcessId, ResourceScope, ScopedPath, VirtualPath};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::types::{
    ProcessError, ProcessRecord, ProcessResultRecord, ProcessResultStore, ProcessStart,
    ProcessStatus, ProcessStore, ensure_status_transition, invalid_path, same_scope_owner,
};

/// Filesystem-backed [`ProcessStore`].
///
/// Construct with a [`ScopedFilesystem`] over any [`RootFilesystem`] —
/// typically a composite root in production or the in-memory backend in
/// tests. The [`ScopedFilesystem`] enforces the caller's
/// [`MountView`](ironclaw_host_api::MountView) per-operation ACL and
/// resolves the `/processes` alias to a tenant-scoped
/// [`VirtualPath`](ironclaw_host_api::VirtualPath) before any backend
/// dispatch — so tenant isolation is structural, not a convention this
/// crate has to remember.
pub struct FilesystemProcessStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    transition_lock: AsyncMutex<()>,
}

impl<F> FilesystemProcessStore<F>
where
    F: RootFilesystem,
{
    /// Construct a filesystem-backed process store.
    ///
    /// **Single-instance invariant**: the `transition_lock` only serializes
    /// `start` and `update_status` (i.e. `complete`/`fail`/`kill`) within a
    /// single `FilesystemProcessStore` instance. Operating multiple instances
    /// concurrently against the same on-disk root is unsupported and will
    /// race on the JSON record files. Construct the store once and share via
    /// `Arc` (see [`from_arc`](Self::from_arc)).
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            transition_lock: AsyncMutex::new(()),
        }
    }

    /// Convenience constructor mirroring [`new`](Self::new) — kept so call
    /// sites that previously held an [`Arc<ScopedFilesystem<F>>`] separately
    /// from a borrow can continue to use the same shape.
    ///
    /// The same single-instance invariant from [`new`](Self::new) applies:
    /// share the resulting store via `Arc` rather than constructing multiple
    /// instances pointed at the same root.
    pub fn from_arc(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self::new(filesystem)
    }

    async fn write_record(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        let path = process_record_path(&record.scope, record.process_id)?;
        let body = serialize_pretty(record)?;
        self.ensure_indexes(&record.scope).await?;
        let entry = process_record_entry(body, record);
        put_with_byte_fallback(
            &self.filesystem,
            &record.scope,
            &path,
            entry,
            CasExpectation::Any,
        )
        .await?;
        Ok(())
    }

    /// Declare the indexed-projection fields on the per-owner `records/`
    /// prefix so `records_for_scope` can use a native `query` filter.
    /// Tolerates `Unsupported` for byte-only backends (e.g. DiskFilesystem)
    /// so the existing list+get fallback path is still reachable.
    async fn ensure_indexes(&self, scope: &ResourceScope) -> Result<(), ProcessError> {
        let prefix = process_records_root(scope)?;
        ensure_exact_index(
            &self.filesystem,
            scope,
            &prefix,
            index_name("processes_by_tenant"),
            index_key_tenant_id(),
        )
        .await?;
        ensure_exact_index(
            &self.filesystem,
            scope,
            &prefix,
            index_name("processes_by_user"),
            index_key_user_id(),
        )
        .await?;
        ensure_exact_index(
            &self.filesystem,
            scope,
            &prefix,
            index_name("processes_by_status"),
            index_key_status(),
        )
        .await?;
        ensure_exact_index(
            &self.filesystem,
            scope,
            &prefix,
            index_name("processes_by_extension"),
            index_key_extension_id(),
        )
        .await?;
        ensure_exact_index(
            &self.filesystem,
            scope,
            &prefix,
            index_name("processes_by_parent"),
            index_key_parent_process_id(),
        )
        .await?;
        Ok(())
    }

    /// Read the current record, validate the requested transition,
    /// then write it back with `CasExpectation::Version` so a concurrent
    /// writer from another process is rejected at the backend instead of
    /// silently overwriting our status flip. The bounded retry loop
    /// handles the legitimate race where another caller transitioned the
    /// same record between our read and write — we re-read, re-validate,
    /// and try again until either the CAS succeeds or
    /// [`MAX_CAS_RETRIES`] is exhausted.
    ///
    /// Backends without versioning (DiskFilesystem) return version `0`
    /// for every read and reject `CasExpectation::Version` with
    /// `Unsupported`; for those, [`put_with_byte_fallback`] falls
    /// through to `CasExpectation::Any` so the existing single-instance
    /// guarantee from `transition_lock` carries the safety invariant.
    async fn update_status(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        to: ProcessStatus,
        error_kind: Option<String>,
    ) -> Result<ProcessRecord, ProcessError> {
        let _guard = self.transition_lock.lock().await;
        let path = process_record_path(scope, process_id)?;
        for _ in 0..MAX_CAS_RETRIES {
            let Some(versioned) = self.filesystem.get(scope, &path).await? else {
                return Err(ProcessError::UnknownProcess { process_id });
            };
            let mut record = deserialize::<ProcessRecord>(&versioned.entry.body)?;
            ensure_process_record_matches(&record, process_id)?;
            if !same_scope_owner(&record.scope, scope) {
                return Err(ProcessError::UnknownProcess { process_id });
            }
            ensure_status_transition(process_id, record.status, to)?;
            record.status = to;
            record.error_kind = error_kind.clone();
            self.ensure_indexes(&record.scope).await?;
            let body = serialize_pretty(&record)?;
            let entry = process_record_entry(body, &record);
            match put_with_byte_fallback(
                &self.filesystem,
                scope,
                &path,
                entry,
                CasExpectation::Version(versioned.version),
            )
            .await
            {
                Ok(()) => return Ok(record),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        let virtual_path = self.filesystem.resolve(scope, &path)?;
        Err(ProcessError::Filesystem(FilesystemError::Backend {
            path: virtual_path,
            operation: FilesystemOperation::WriteFile,
            reason: format!(
                "process {process_id} status transition exhausted {MAX_CAS_RETRIES} CAS retries"
            ),
        }))
    }
}

/// Maximum number of compare-and-swap retries before
/// [`FilesystemProcessStore::update_status`] returns a `Filesystem`
/// error. Five attempts mirrors the retry budget used by the secrets and
/// authorization stores and is enough to absorb common contention while
/// failing loudly on pathological loops.
const MAX_CAS_RETRIES: usize = 5;

#[async_trait]
impl<F> ProcessStore for FilesystemProcessStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let _guard = self.transition_lock.lock().await;
        let path = process_record_path(&start.scope, start.process_id)?;
        // Existence check uses `get` (unified read) so it works regardless of
        // whether the backend has native put. Atomicity is provided by the
        // transition_lock per the single-instance invariant in this struct's
        // docstring. A future migration can switch to `CasExpectation::Absent`
        // once every backend in production exposes native put.
        if self.filesystem.get(&start.scope, &path).await?.is_some() {
            return Err(ProcessError::ProcessAlreadyExists {
                process_id: start.process_id,
            });
        }
        let record = ProcessRecord {
            process_id: start.process_id,
            parent_process_id: start.parent_process_id,
            invocation_id: start.invocation_id,
            scope: start.scope,
            authenticated_actor_user_id: start.authenticated_actor_user_id,
            extension_id: start.extension_id,
            capability_id: start.capability_id,
            runtime: start.runtime,
            status: ProcessStatus::Running,
            grants: start.grants,
            mounts: start.mounts,
            estimated_resources: start.estimated_resources,
            resource_reservation_id: start.resource_reservation_id,
            error_kind: None,
        };
        self.write_record(&record).await?;
        Ok(record)
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Completed, None)
            .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(
            scope,
            process_id,
            ProcessStatus::Failed,
            Some(sanitize_error_kind(error_kind)),
        )
        .await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        self.update_status(scope, process_id, ProcessStatus::Killed, None)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        let path = process_record_path(scope, process_id)?;
        let Some(versioned) = self.filesystem.get(scope, &path).await? else {
            return Ok(None);
        };
        let record = deserialize::<ProcessRecord>(&versioned.entry.body)?;
        ensure_process_record_matches(&record, process_id)?;
        if same_scope_owner(&record.scope, scope) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        let root = process_records_root(scope)?;
        // Try the indexed query path first. The `tenant_id` + `user_id`
        // pair is still projected onto each record so a backend serving
        // a shared root (e.g. tests reusing one InMemoryBackend across
        // mount views) can distinguish records. With the ScopedFilesystem
        // refactor the path itself already encodes tenant/user via the
        // MountView, but the indexed projection stays as belt-and-braces
        // — backends that share storage across MountViews must still
        // produce the right rows. The post-query `same_scope_owner`
        // check guards the remaining sub-scope (agent/project/mission/
        // thread) axes that are not in the index spec yet. Backends
        // without index support (DiskFilesystem) return `Unsupported`
        // and we fall back to the legacy list+get scan.
        self.ensure_indexes(scope).await?;
        let filter = Filter::And(vec![
            Filter::Eq {
                key: index_key_tenant_id(),
                value: IndexValue::Text(scope.tenant_id.as_str().to_string()),
            },
            Filter::Eq {
                key: index_key_user_id(),
                value: IndexValue::Text(scope.user_id.as_str().to_string()),
            },
        ]);
        match query_all_records(&self.filesystem, scope, &root, &filter).await {
            Ok(records) => {
                let mut filtered = records
                    .into_iter()
                    .filter(|record| same_scope_owner(&record.scope, scope))
                    .collect::<Vec<_>>();
                filtered.sort_by_key(|record| record.process_id.as_uuid());
                Ok(filtered)
            }
            Err(error) if is_unsupported(&error) => {
                self.records_for_scope_via_list(scope, &root).await
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl<F> FilesystemProcessStore<F>
where
    F: RootFilesystem,
{
    /// Legacy list+get scan used as the fallback for byte-only backends
    /// (DiskFilesystem) that cannot serve `query` over indexed
    /// projections. Production deployments on libSQL / Postgres / the
    /// in-memory backend take the indexed path in [`records_for_scope`].
    async fn records_for_scope_via_list(
        &self,
        scope: &ResourceScope,
        root: &ScopedPath,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        let entries = match self.filesystem.list_dir(scope, root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            // `list_dir` returns `VirtualPath`s because resolution has
            // already happened. We reconstruct the child as a
            // [`ScopedPath`] under the same alias-relative prefix so the
            // per-op ACL still runs on the follow-up `get` (mirrors the
            // engine store's `list_subdir_names` shape).
            let scoped_child = join_scoped(root, &entry.name)?;
            // Reviewer (PR #3666) flagged: a `get` returning `None` after
            // `list_dir` enumerated the path indicates a race or backend
            // inconsistency. Returning a partial process list silently
            // hides this; surface it as a filesystem error so callers see
            // the same failure shape they got with the legacy `read_file`
            // path.
            let Some(versioned) = self.filesystem.get(scope, &scoped_child).await? else {
                return Err(ProcessError::Filesystem(FilesystemError::Backend {
                    path: entry.path,
                    operation: FilesystemOperation::ReadFile,
                    reason: format!(
                        "process record listed but missing at {}",
                        scoped_child.as_str()
                    ),
                }));
            };
            let record = deserialize::<ProcessRecord>(&versioned.entry.body)?;
            if same_scope_owner(&record.scope, scope) {
                records.push(record);
            }
        }
        records.sort_by_key(|record| record.process_id.as_uuid());
        Ok(records)
    }
}

pub struct FilesystemProcessResultStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemProcessResultStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    /// Convenience constructor mirroring [`new`](Self::new); preserved so
    /// existing call sites (composition factories, tests) keep their shape.
    pub fn from_arc(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self::new(filesystem)
    }

    async fn write_result(&self, record: &ProcessResultRecord) -> Result<(), ProcessError> {
        let path = process_result_path(&record.scope, record.process_id)?;
        let body = serialize_pretty(record)?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(&record.scope, &path, entry, CasExpectation::Any)
            .await?;
        Ok(())
    }

    async fn write_output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: &Value,
    ) -> Result<VirtualPath, ProcessError> {
        let path = process_output_path(scope, process_id)?;
        let body = serialize_pretty(output)?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(scope, &path, entry, CasExpectation::Any)
            .await?;
        let virtual_path = self
            .filesystem
            .resolve(scope, &path)
            .map_err(ProcessError::Filesystem)?;
        Ok(virtual_path)
    }

    async fn store_result(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        status: ProcessStatus,
        output: Option<Value>,
        output_ref: Option<VirtualPath>,
        error_kind: Option<String>,
    ) -> Result<ProcessResultRecord, ProcessError> {
        let record = ProcessResultRecord {
            process_id,
            scope: scope.clone(),
            status,
            output,
            output_ref,
            error_kind,
        };
        self.write_result(&record).await?;
        Ok(record)
    }
}

#[async_trait]
impl<F> ProcessResultStore for FilesystemProcessResultStore<F>
where
    F: RootFilesystem + 'static,
{
    /// Persist a successful terminal record and its output blob.
    ///
    /// Writes happen in two steps (`write_output` then `write_result`); if
    /// the second write fails, the output blob at
    /// `outputs/<process_id>/output.json` is left on disk as an
    /// orphan. Cleanup of orphaned output blobs is the caller's responsibility
    /// (typically swept during operator-initiated reconciliation rather than
    /// inline, since orphans are observable via missing
    /// `results/<process_id>.json`).
    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        output: Value,
    ) -> Result<ProcessResultRecord, ProcessError> {
        let output_ref = self.write_output(scope, process_id, &output).await?;
        self.store_result(
            scope,
            process_id,
            ProcessStatus::Completed,
            None,
            Some(output_ref),
            None,
        )
        .await
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.store_result(
            scope,
            process_id,
            ProcessStatus::Failed,
            None,
            None,
            Some(sanitize_error_kind(error_kind)),
        )
        .await
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessResultRecord, ProcessError> {
        self.store_result(scope, process_id, ProcessStatus::Killed, None, None, None)
            .await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessResultRecord>, ProcessError> {
        let path = process_result_path(scope, process_id)?;
        let Some(versioned) = self.filesystem.get(scope, &path).await? else {
            return Ok(None);
        };
        let record = deserialize::<ProcessResultRecord>(&versioned.entry.body)?;
        ensure_result_record_matches(&record, process_id)?;
        if same_scope_owner(&record.scope, scope) {
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    async fn output(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<Value>, ProcessError> {
        let Some(record) = self.get(scope, process_id).await? else {
            return Ok(None);
        };
        if let Some(output) = record.output {
            return Ok(Some(output));
        }
        let Some(output_ref) = record.output_ref else {
            return Ok(None);
        };
        // The stored `output_ref` is a tenant-scoped [`VirtualPath`]; we
        // compare it against the resolved view of the path the current
        // scope would produce, so a forged record whose `output_ref`
        // points at a sibling tenant/scope's blob is rejected before any
        // read. After the match passes, we read the blob through the
        // scoped path (going through the per-op ACL) rather than the raw
        // `VirtualPath` so backends with stricter scopes still apply
        // their checks.
        let expected_scoped = process_output_path(scope, process_id)?;
        let expected_virtual = self
            .filesystem
            .resolve(scope, &expected_scoped)
            .map_err(ProcessError::Filesystem)?;
        if output_ref != expected_virtual {
            return Err(invalid_stored_record(format!(
                "process result output ref {} does not match expected {}",
                output_ref.as_str(),
                expected_virtual.as_str()
            )));
        }
        let Some(versioned) = self.filesystem.get(scope, &expected_scoped).await? else {
            return Ok(None);
        };
        deserialize::<Value>(&versioned.entry.body).map(Some)
    }
}

// ── Paths ──────────────────────────────────────────────────────
//
// Every path returned here is alias-relative to the `/processes` mount
// alias on the caller's [`ScopedFilesystem`]. The leading tenant/user
// segment that the legacy implementation hand-formatted into the path
// is gone: the MountView's `/processes -> /tenants/<tenant>/users/<user>/processes`
// grant supplies it at every op. Sub-scope axes (agent/project/mission/
// thread) remain in the alias-relative path because they are *within*-
// tenant scoping and are not covered by the per-tenant MountAlias.

const PROCESSES_PREFIX: &str = "/processes";

fn process_record_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<ScopedPath, ProcessError> {
    scoped_path(&format!(
        "{}/{process_id}.json",
        process_records_root_string(scope)
    ))
}

fn process_records_root(scope: &ResourceScope) -> Result<ScopedPath, ProcessError> {
    scoped_path(&process_records_root_string(scope))
}

fn process_records_root_string(scope: &ResourceScope) -> String {
    format!("{}/records", scope_owner_root_string(scope))
}

fn process_result_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<ScopedPath, ProcessError> {
    scoped_path(&format!(
        "{}/results/{process_id}.json",
        scope_owner_root_string(scope)
    ))
}

fn process_output_path(
    scope: &ResourceScope,
    process_id: ProcessId,
) -> Result<ScopedPath, ProcessError> {
    scoped_path(&format!(
        "{}/outputs/{process_id}/output.json",
        scope_owner_root_string(scope)
    ))
}

/// Build the alias-relative prefix for a given sub-scope under
/// `/processes`. The tenant/user prefix is supplied by the caller's
/// MountView at op time and intentionally absent here.
fn scope_owner_root_string(scope: &ResourceScope) -> String {
    let mut base = String::from(PROCESSES_PREFIX);
    if let Some(agent_id) = &scope.agent_id {
        base.push_str("/agents/");
        base.push_str(agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    if let Some(mission_id) = &scope.mission_id {
        base.push_str("/missions/");
        base.push_str(mission_id.as_str());
    }
    if let Some(thread_id) = &scope.thread_id {
        base.push_str("/threads/");
        base.push_str(thread_id.as_str());
    }
    base
}

fn scoped_path(raw: &str) -> Result<ScopedPath, ProcessError> {
    ScopedPath::new(raw).map_err(invalid_path)
}

/// Join a leaf segment onto a [`ScopedPath`] prefix. Used when
/// reconstructing a child path after `list_dir` (which returns
/// [`VirtualPath`]s) so the per-op ACL check still runs on the follow-up
/// `get` — mirrors the engine store's `join_scoped` helper.
fn join_scoped(prefix: &ScopedPath, leaf: &str) -> Result<ScopedPath, ProcessError> {
    let joined = format!("{}/{}", prefix.as_str().trim_end_matches('/'), leaf);
    ScopedPath::new(joined).map_err(invalid_path)
}

fn ensure_process_record_matches(
    record: &ProcessRecord,
    process_id: ProcessId,
) -> Result<(), ProcessError> {
    if record.process_id != process_id {
        return Err(invalid_stored_record(format!(
            "stored process id {} does not match requested {}",
            record.process_id, process_id
        )));
    }
    Ok(())
}

fn ensure_result_record_matches(
    record: &ProcessResultRecord,
    process_id: ProcessId,
) -> Result<(), ProcessError> {
    if record.process_id != process_id {
        return Err(invalid_stored_record(format!(
            "stored process result id {} does not match requested {}",
            record.process_id, process_id
        )));
    }
    Ok(())
}

fn invalid_stored_record(reason: impl Into<String>) -> ProcessError {
    ProcessError::InvalidStoredRecord {
        reason: reason.into(),
    }
}

fn serialize_pretty<T>(value: &T) -> Result<Vec<u8>, ProcessError>
where
    T: Serialize,
{
    serde_json::to_vec_pretty(value).map_err(|error| ProcessError::Serialization(error.to_string()))
}

fn deserialize<T>(bytes: &[u8]) -> Result<T, ProcessError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| ProcessError::Deserialization(error.to_string()))
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

fn is_unsupported(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::Unsupported { .. })
}

/// Construct the [`Entry`] persisted for a process lifecycle record.
///
/// The on-disk JSON body is unchanged from the migration commit; we only
/// decorate the entry with indexed projections so backends that support
/// records can answer [`ProcessStore::records_for_scope`] through
/// [`RootFilesystem::query`] instead of an N+1 list+get scan.
fn process_record_entry(body: Vec<u8>, record: &ProcessRecord) -> Entry {
    let mut entry = Entry::bytes(body)
        .with_content_type(ContentType::json())
        .with_indexed(
            index_key_tenant_id(),
            IndexValue::Text(record.scope.tenant_id.as_str().to_string()),
        )
        .with_indexed(
            index_key_user_id(),
            IndexValue::Text(record.scope.user_id.as_str().to_string()),
        )
        .with_indexed(
            index_key_status(),
            IndexValue::Text(process_status_label(record.status).to_string()),
        )
        .with_indexed(
            index_key_extension_id(),
            IndexValue::Text(record.extension_id.as_str().to_string()),
        );
    if let Some(parent) = record.parent_process_id {
        entry = entry.with_indexed(
            index_key_parent_process_id(),
            IndexValue::Text(parent.to_string()),
        );
    }
    entry
}

fn process_status_label(status: ProcessStatus) -> &'static str {
    // Match the snake_case serde rename on [`ProcessStatus`]. Used as the
    // indexed projection text value so filters can match the same wire
    // form callers would serialize.
    match status {
        ProcessStatus::Running => "running",
        ProcessStatus::Completed => "completed",
        ProcessStatus::Failed => "failed",
        ProcessStatus::Killed => "killed",
    }
}

/// `put` with a fallback to an opaque (byte-only) entry on `Unsupported`.
///
/// Backends that don't yet implement records (DiskFilesystem with no
/// sidecar metadata) reject `kind = Some(_)` or any non-`Any` CAS
/// expectation. We try the indexed write first so SQL and in-memory
/// backends get the projection, then retry with the same body stripped
/// of metadata and the CAS downgraded to `Any` so the legacy byte-only
/// path keeps working during the consumer migration. The single-instance
/// `transition_lock` on the caller carries the ordering safety
/// invariant that CAS would otherwise provide.
async fn put_with_byte_fallback<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<(), FilesystemError>
where
    F: RootFilesystem,
{
    match filesystem.put(scope, path, entry.clone(), cas).await {
        Ok(_) => Ok(()),
        Err(error) if is_unsupported(&error) => {
            let opaque = Entry::bytes(entry.body).with_content_type(entry.content_type);
            filesystem
                .put(scope, path, opaque, CasExpectation::Any)
                .await
                .map(|_| ())
        }
        Err(error) => Err(error),
    }
}

/// Declare a single-key `Exact` index on `prefix`, tolerating backends
/// that don't support indexes. Mirrors the engine store's
/// `ensure_exact_index` shape so backends without index support degrade
/// to the list+get fallback path instead of failing closed.
async fn ensure_exact_index<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    prefix: &ScopedPath,
    name: IndexName,
    key: IndexKey,
) -> Result<(), ProcessError>
where
    F: RootFilesystem,
{
    let spec = IndexSpec::new(name, vec![key], IndexKind::Exact);
    match filesystem.ensure_index(scope, prefix, &spec).await {
        Ok(()) => Ok(()),
        Err(FilesystemError::Unsupported { .. }) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Drain a paginated `query` against `prefix` with `filter`, materializing
/// every matched [`ProcessRecord`].
async fn query_all_records<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    prefix: &ScopedPath,
    filter: &Filter,
) -> Result<Vec<ProcessRecord>, FilesystemError>
where
    F: RootFilesystem,
{
    let mut out = Vec::new();
    let mut offset: u64 = 0;
    loop {
        let page = Page::new(offset, Page::MAX_LIMIT);
        let entries = filesystem.query(scope, prefix, filter, page).await?;
        let received = entries.len();
        for entry in entries {
            let record: ProcessRecord =
                serde_json::from_slice(&entry.entry.body).map_err(|error| {
                    FilesystemError::Backend {
                        path: entry.path.clone(),
                        operation: ironclaw_filesystem::FilesystemOperation::Query,
                        reason: format!("process record deserialization failed: {error}"),
                    }
                })?;
            out.push(record);
        }
        if received < Page::MAX_LIMIT as usize {
            break;
        }
        offset = offset.saturating_add(received as u64);
    }
    Ok(out)
}

// ── Index identifiers ──────────────────────────────────────────
//
// `IndexName` / `IndexKey` validate as `[A-Za-z_][A-Za-z0-9_]*`. The
// literals below all satisfy that shape, so construction cannot fail at
// runtime — but we still route through the validating constructor and
// `unwrap_or_else(unreachable!())` so a future rename catches the typo
// at the test site rather than silently producing an empty filter.

fn index_name(value: &'static str) -> IndexName {
    IndexName::new(value)
        .unwrap_or_else(|_| unreachable!("process index name {value} must be a simple identifier"))
}

fn index_key(value: &'static str) -> IndexKey {
    IndexKey::new(value)
        .unwrap_or_else(|_| unreachable!("process index key {value} must be a simple identifier"))
}

fn index_key_tenant_id() -> IndexKey {
    index_key("tenant_id")
}

fn index_key_user_id() -> IndexKey {
    index_key("user_id")
}

fn index_key_status() -> IndexKey {
    index_key("status")
}

fn index_key_extension_id() -> IndexKey {
    index_key("extension_id")
}

fn index_key_parent_process_id() -> IndexKey {
    index_key("parent_process_id")
}
