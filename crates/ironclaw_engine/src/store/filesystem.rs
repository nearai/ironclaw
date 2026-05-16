//! Filesystem-backed implementation of the engine [`Store`] trait.
//!
//! This is the kernel-storage migration target for the engine's persistence
//! surface: every CRUD operation routes through the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) plane
//! (`put`/`get`/`query`/`ensure_index`) instead of the legacy
//! `src/db/Database` composite trait.
//!
//! The legacy [`crate::Store`] consumer in the host crate
//! (`src/bridge/store_adapter.rs::HybridStore`) remains in place for the
//! migration window — once host wiring switches to this implementation,
//! `HybridStore` will be removed in task #17.
//!
//! Path layout, indexing, and concurrency model:
//!
//! - Path shape is documented in [`crate::store::paths`].
//! - Indexed projections (`thread_id`, `project_id`, `user_id`, `status`,
//!   `parent_thread_id`, `doc_type`, `revoked`) live in `Entry::indexed`.
//!   These are the queryable columns; backends never look inside
//!   `Entry::body`.
//! - All `Store` reads use scoped path queries (`thread_id` projection
//!   under `/engine/steps/<thread_id>`) where the path already restricts
//!   the result set; cross-thread / cross-project queries use the matching
//!   indexed projection over the per-prefix `query` op.
//! - State transitions (`update_thread_state`, `update_mission_status`,
//!   `revoke_lease`) are read-modify-write under a process-local per-key
//!   mutex, mirroring the secrets / authorization stores. Production
//!   shared roots additionally need a transactional backend or explicit
//!   CAS — see the crate guardrails for `ironclaw_authorization`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, Entry, FileType, FilesystemError, Filter, IndexKind, IndexSpec, Page,
    RecordKind, RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::VirtualPath;
use serde::{Serialize, de::DeserializeOwned};

use crate::store::paths::{
    conversation_path, conversations_prefix, event_path, events_prefix, host_api_to_engine_error,
    index_key_doc_type, index_key_parent_thread_id, index_key_project_id, index_key_revoked,
    index_key_status, index_key_thread_id, index_key_user_id, index_name, index_value_bool,
    index_value_text, lease_path, leases_prefix, memory_path, memory_prefix_all,
    memory_prefix_for_project, mission_path, missions_prefix, missions_prefix_for_project,
    project_path, projects_prefix, step_path, steps_prefix, thread_path, threads_prefix,
};
use crate::traits::store::Store;
use crate::types::capability::{CapabilityLease, LeaseId};
use crate::types::conversation::{ConversationId, ConversationSurface};
use crate::types::error::EngineError;
use crate::types::event::ThreadEvent;
use crate::types::memory::{DocId, MemoryDoc};
use crate::types::mission::{Mission, MissionId, MissionStatus};
use crate::types::project::{Project, ProjectId};
use crate::types::step::Step;
use crate::types::thread::{Thread, ThreadId, ThreadState};

// Schema-family record kinds. Building these names once at module init
// catches typos before any I/O.
fn record_kind(name: &str) -> RecordKind {
    RecordKind::new(name).unwrap_or_else(|_| panic!("invalid record kind literal: {name}"))
}

fn kind_thread() -> RecordKind {
    record_kind("engine_thread")
}
fn kind_step() -> RecordKind {
    record_kind("engine_step")
}
fn kind_event() -> RecordKind {
    record_kind("engine_event")
}
fn kind_project() -> RecordKind {
    record_kind("engine_project")
}
fn kind_conversation() -> RecordKind {
    record_kind("engine_conversation")
}
fn kind_memory_doc() -> RecordKind {
    record_kind("engine_memory_doc")
}
fn kind_lease() -> RecordKind {
    record_kind("engine_capability_lease")
}
fn kind_mission() -> RecordKind {
    record_kind("engine_mission")
}

/// Filesystem-backed [`Store`] implementation.
///
/// Construct with any [`RootFilesystem`] — typically a
/// [`CompositeRootFilesystem`](ironclaw_filesystem::CompositeRootFilesystem)
/// or the in-memory backend for tests. Indexes are declared lazily on the
/// first write that needs them (mirrors the secrets / authorization stores).
pub struct FilesystemStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

impl<F> FilesystemStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    pub fn filesystem(&self) -> &Arc<F> {
        &self.filesystem
    }

    // ── Index declaration ──────────────────────────────────────

    async fn ensure_threads_indexes(&self) -> Result<(), EngineError> {
        let prefix = threads_prefix()?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("threads_by_project"),
            index_key_project_id(),
        )
        .await?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("threads_by_user"),
            index_key_user_id(),
        )
        .await?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("threads_by_parent"),
            index_key_parent_thread_id(),
        )
        .await?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("threads_by_status"),
            index_key_status(),
        )
        .await?;
        Ok(())
    }

    async fn ensure_projects_indexes(&self) -> Result<(), EngineError> {
        let prefix = projects_prefix()?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("projects_by_user"),
            index_key_user_id(),
        )
        .await
    }

    async fn ensure_conversations_indexes(&self) -> Result<(), EngineError> {
        let prefix = conversations_prefix()?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("conversations_by_user"),
            index_key_user_id(),
        )
        .await
    }

    async fn ensure_memory_indexes(&self) -> Result<(), EngineError> {
        let prefix = memory_prefix_all()?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("memory_by_project"),
            index_key_project_id(),
        )
        .await?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("memory_by_user"),
            index_key_user_id(),
        )
        .await?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("memory_by_doc_type"),
            index_key_doc_type(),
        )
        .await?;
        Ok(())
    }

    async fn ensure_missions_indexes(&self) -> Result<(), EngineError> {
        let prefix = missions_prefix()?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("missions_by_project"),
            index_key_project_id(),
        )
        .await?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("missions_by_user"),
            index_key_user_id(),
        )
        .await?;
        ensure_exact_index(
            self.filesystem.as_ref(),
            &prefix,
            index_name("missions_by_status"),
            index_key_status(),
        )
        .await?;
        Ok(())
    }

    // ── Read helpers ───────────────────────────────────────────

    async fn read_one<T>(&self, path: &VirtualPath) -> Result<Option<T>, EngineError>
    where
        T: DeserializeOwned,
    {
        match self.filesystem.get(path).await {
            Ok(Some(versioned)) => deserialize(&versioned.entry.body).map(Some),
            Ok(None) => Ok(None),
            Err(error) => Err(fs_to_engine_error(error)),
        }
    }

    async fn read_versioned(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<VersionedEntry>, EngineError> {
        self.filesystem.get(path).await.map_err(fs_to_engine_error)
    }

    async fn query_all<T>(
        &self,
        prefix: &VirtualPath,
        filter: &Filter,
    ) -> Result<Vec<T>, EngineError>
    where
        T: DeserializeOwned,
    {
        let mut out = Vec::new();
        let mut offset: u64 = 0;
        loop {
            let page = Page::new(offset, Page::MAX_LIMIT);
            let entries = self
                .filesystem
                .query(prefix, filter, page)
                .await
                .map_err(fs_to_engine_error)?;
            let received = entries.len();
            for entry in entries {
                out.push(deserialize::<T>(&entry.entry.body)?);
            }
            if received < Page::MAX_LIMIT as usize {
                break;
            }
            offset = offset.saturating_add(received as u64);
        }
        Ok(out)
    }

    async fn list_subdir_paths(
        &self,
        prefix: &VirtualPath,
    ) -> Result<Vec<VirtualPath>, EngineError> {
        match self.filesystem.list_dir(prefix).await {
            Ok(entries) => Ok(entries
                .into_iter()
                .filter(|entry| entry.file_type == FileType::Directory)
                .map(|entry| entry.path)
                .collect()),
            Err(error) if is_not_found(&error) => Ok(Vec::new()),
            Err(error) => Err(fs_to_engine_error(error)),
        }
    }

    async fn read_all_files_under<T>(&self, prefix: &VirtualPath) -> Result<Vec<T>, EngineError>
    where
        T: DeserializeOwned,
    {
        let entries = match self.filesystem.list_dir(prefix).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_engine_error(error)),
        };
        let mut out = Vec::new();
        for entry in entries {
            if entry.file_type != FileType::File {
                continue;
            }
            if !entry.name.ends_with(".json") {
                continue;
            }
            if let Some(value) = self.read_one::<T>(&entry.path).await? {
                out.push(value);
            }
        }
        Ok(out)
    }

    // ── Write helpers ──────────────────────────────────────────

    async fn write_record<T>(
        &self,
        path: &VirtualPath,
        kind: RecordKind,
        value: &T,
        indexed: Vec<(
            ironclaw_filesystem::IndexKey,
            ironclaw_filesystem::IndexValue,
        )>,
    ) -> Result<(), EngineError>
    where
        T: Serialize,
    {
        let entry = build_record_entry(kind, value, indexed)?;
        self.filesystem
            .put(path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_engine_error)
    }
}

/// Build an `Entry` from a serializable value plus indexed projections.
/// Shared by `write_record` and the per-method CAS retry loops below so
/// the encode shape is identical regardless of the CAS expectation.
fn build_record_entry<T>(
    kind: RecordKind,
    value: &T,
    indexed: Vec<(
        ironclaw_filesystem::IndexKey,
        ironclaw_filesystem::IndexValue,
    )>,
) -> Result<Entry, EngineError>
where
    T: Serialize,
{
    let body = serialize_json(value)?;
    let mut entry = Entry::record(kind, &body).map_err(|error| EngineError::Store {
        reason: format!("filesystem engine store: failed to encode record: {error}"),
    })?;
    for (key, value) in indexed {
        entry = entry.with_indexed(key, value);
    }
    Ok(entry)
}

// ── Per-key mutation lock ──────────────────────────────────────
//
// Mirrors the `FILESYSTEM_RECORD_LOCKS` pattern from `ironclaw_secrets` and
// `ironclaw_authorization`: process-local locks keyed by virtual path so a
// `consume`/`revoke`/`status-transition` read-modify-write is atomic within
// a single instance. Multi-process callers must use a backend with
// transactional or CAS support — see the `ironclaw_authorization` guardrail.

type FilesystemRecordLock = Arc<tokio::sync::Mutex<()>>;

static FILESYSTEM_RECORD_LOCKS: OnceLock<Mutex<HashMap<String, FilesystemRecordLock>>> =
    OnceLock::new();

fn lock_for_path(path: &VirtualPath) -> FilesystemRecordLock {
    let locks = FILESYSTEM_RECORD_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = lock_or_recover(locks);
    Arc::clone(
        guard
            .entry(path.as_str().to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

fn lock_or_recover<T>(mutex: &Mutex<HashMap<String, T>>) -> MutexGuard<'_, HashMap<String, T>> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── Helpers ────────────────────────────────────────────────────

async fn ensure_exact_index<F>(
    filesystem: &F,
    prefix: &VirtualPath,
    name: ironclaw_filesystem::IndexName,
    key: ironclaw_filesystem::IndexKey,
) -> Result<(), EngineError>
where
    F: RootFilesystem + ?Sized,
{
    let spec = IndexSpec::new(name, vec![key], IndexKind::Exact);
    match filesystem.ensure_index(prefix, &spec).await {
        Ok(()) => Ok(()),
        // Backends without index support (e.g. byte-only LocalFilesystem) are
        // still usable for reads/writes; the engine never assumed SQL-style
        // indexes were available before, so degrade rather than fail closed.
        Err(ironclaw_filesystem::FilesystemError::Unsupported { .. }) => Ok(()),
        Err(error) => Err(fs_to_engine_error(error)),
    }
}

fn fs_to_engine_error(error: ironclaw_filesystem::FilesystemError) -> EngineError {
    // Tag the typed `Unsupported` variant so `is_engine_unsupported` can
    // recognize it discriminator-wise rather than by free-text matching.
    let tag = if matches!(
        error,
        ironclaw_filesystem::FilesystemError::Unsupported { .. }
    ) {
        FS_UNSUPPORTED_TAG
    } else {
        ""
    };
    EngineError::Store {
        reason: format!("filesystem engine store: {tag}{error}"),
    }
}

fn is_not_found(error: &ironclaw_filesystem::FilesystemError) -> bool {
    matches!(error, ironclaw_filesystem::FilesystemError::NotFound { .. })
}

// Discriminator-preserving check: the typed `FilesystemError::Unsupported`
// variant gets stringified into `EngineError::Store { reason }` at the
// boundary, so the reason carries a stable `[fs:unsupported]` tag we can
// match on without scanning the human-readable text. Substring-on-message
// would false-positive on any unrelated store error that happens to mention
// "unsupported" (see code review feedback on PR #3679).
const FS_UNSUPPORTED_TAG: &str = "[fs:unsupported]";

fn serialize_json<T: Serialize>(value: &T) -> Result<serde_json::Value, EngineError> {
    serde_json::to_value(value).map_err(|error| EngineError::Store {
        reason: format!("filesystem engine store: failed to encode payload: {error}"),
    })
}

fn deserialize<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, EngineError> {
    serde_json::from_slice(bytes).map_err(|error| EngineError::Store {
        reason: format!("filesystem engine store: failed to decode payload: {error}"),
    })
}

// ── Indexed projections ────────────────────────────────────────

fn thread_status_label(state: ThreadState) -> &'static str {
    match state {
        ThreadState::Created => "created",
        ThreadState::Running => "running",
        ThreadState::Waiting => "waiting",
        ThreadState::Suspended => "suspended",
        ThreadState::Completed => "completed",
        ThreadState::Done => "done",
        ThreadState::Failed => "failed",
    }
}

fn mission_status_label(status: MissionStatus) -> &'static str {
    match status {
        MissionStatus::Active => "active",
        MissionStatus::Paused => "paused",
        MissionStatus::Completed => "completed",
        MissionStatus::Failed => "failed",
    }
}

fn doc_type_label(doc_type: crate::types::memory::DocType) -> &'static str {
    use crate::types::memory::DocType;
    match doc_type {
        DocType::Summary => "summary",
        DocType::Lesson => "lesson",
        DocType::Issue => "issue",
        DocType::Spec => "spec",
        DocType::Note => "note",
        DocType::Skill => "skill",
        DocType::Plan => "plan",
    }
}

fn thread_indexed(
    thread: &Thread,
) -> Vec<(
    ironclaw_filesystem::IndexKey,
    ironclaw_filesystem::IndexValue,
)> {
    let mut indexed = vec![
        (
            index_key_project_id(),
            index_value_text(thread.project_id.0.to_string()),
        ),
        (index_key_user_id(), index_value_text(&thread.user_id)),
        (
            index_key_status(),
            index_value_text(thread_status_label(thread.state)),
        ),
    ];
    if let Some(parent) = thread.parent_id {
        indexed.push((
            index_key_parent_thread_id(),
            index_value_text(parent.0.to_string()),
        ));
    }
    indexed
}

fn project_indexed(
    project: &Project,
) -> Vec<(
    ironclaw_filesystem::IndexKey,
    ironclaw_filesystem::IndexValue,
)> {
    vec![(index_key_user_id(), index_value_text(&project.user_id))]
}

fn conversation_indexed(
    conversation: &ConversationSurface,
) -> Vec<(
    ironclaw_filesystem::IndexKey,
    ironclaw_filesystem::IndexValue,
)> {
    vec![(index_key_user_id(), index_value_text(&conversation.user_id))]
}

fn memory_indexed(
    doc: &MemoryDoc,
) -> Vec<(
    ironclaw_filesystem::IndexKey,
    ironclaw_filesystem::IndexValue,
)> {
    vec![
        (
            index_key_project_id(),
            index_value_text(doc.project_id.0.to_string()),
        ),
        (index_key_user_id(), index_value_text(&doc.user_id)),
        (
            index_key_doc_type(),
            index_value_text(doc_type_label(doc.doc_type)),
        ),
    ]
}

fn lease_indexed(
    lease: &CapabilityLease,
) -> Vec<(
    ironclaw_filesystem::IndexKey,
    ironclaw_filesystem::IndexValue,
)> {
    vec![
        (
            index_key_thread_id(),
            index_value_text(lease.thread_id.0.to_string()),
        ),
        (index_key_revoked(), index_value_bool(lease.revoked)),
    ]
}

fn mission_indexed(
    mission: &Mission,
) -> Vec<(
    ironclaw_filesystem::IndexKey,
    ironclaw_filesystem::IndexValue,
)> {
    vec![
        (
            index_key_project_id(),
            index_value_text(mission.project_id.0.to_string()),
        ),
        (index_key_user_id(), index_value_text(&mission.user_id)),
        (
            index_key_status(),
            index_value_text(mission_status_label(mission.status)),
        ),
    ]
}

fn step_indexed(
    step: &Step,
) -> Vec<(
    ironclaw_filesystem::IndexKey,
    ironclaw_filesystem::IndexValue,
)> {
    vec![(
        index_key_thread_id(),
        index_value_text(step.thread_id.0.to_string()),
    )]
}

fn event_indexed(
    event: &ThreadEvent,
) -> Vec<(
    ironclaw_filesystem::IndexKey,
    ironclaw_filesystem::IndexValue,
)> {
    vec![(
        index_key_thread_id(),
        index_value_text(event.thread_id.0.to_string()),
    )]
}

// ── Trait impl ─────────────────────────────────────────────────

#[async_trait]
impl<F> Store for FilesystemStore<F>
where
    F: RootFilesystem + 'static,
{
    // ── Thread ops ────────────────────────────────────────────

    async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
        self.ensure_threads_indexes().await?;
        let path = thread_path(thread.id)?;
        let lock = lock_for_path(&path);
        let _guard = lock.lock().await;
        self.write_record(&path, kind_thread(), thread, thread_indexed(thread))
            .await
    }

    async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
        let path = thread_path(id)?;
        self.read_one(&path).await
    }

    async fn list_threads(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<Thread>, EngineError> {
        self.ensure_threads_indexes().await?;
        let prefix = threads_prefix()?;
        let filter = Filter::And(vec![
            Filter::Eq {
                key: index_key_project_id(),
                value: index_value_text(project_id.0.to_string()),
            },
            Filter::Eq {
                key: index_key_user_id(),
                value: index_value_text(user_id),
            },
        ]);
        self.query_all(&prefix, &filter).await
    }

    async fn update_thread_state(
        &self,
        id: ThreadId,
        state: ThreadState,
    ) -> Result<(), EngineError> {
        let path = thread_path(id)?;
        let lock = lock_for_path(&path);
        let _guard = lock.lock().await;
        // The process-local mutex above only serializes writers in this
        // process; multi-process shared roots need CAS as the floor
        // (`crates/ironclaw_filesystem/CLAUDE.md` invariant 2). Re-read
        // and retry on `VersionMismatch` so a concurrent state transition
        // from another process doesn't silently disappear.
        loop {
            // silent-ok: HybridStore parity — update on unknown thread is a
            // tolerated no-op; the legacy store has the same behaviour.
            let Some(versioned) = self.read_versioned(&path).await? else {
                return Ok(());
            };
            let mut thread: Thread = deserialize(&versioned.entry.body)?;
            thread.state = state;
            let entry = build_record_entry(kind_thread(), &thread, thread_indexed(&thread))?;
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Version(versioned.version))
                .await
            {
                Ok(_) => return Ok(()),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_to_engine_error(error)),
            }
        }
    }

    // ── Step ops ──────────────────────────────────────────────

    async fn save_step(&self, step: &Step) -> Result<(), EngineError> {
        let path = step_path(step.thread_id, step.id)?;
        let lock = lock_for_path(&path);
        let _guard = lock.lock().await;
        self.write_record(&path, kind_step(), step, step_indexed(step))
            .await
    }

    async fn load_steps(&self, thread_id: ThreadId) -> Result<Vec<Step>, EngineError> {
        let prefix = steps_prefix(thread_id)?;
        let mut steps: Vec<Step> = self.read_all_files_under(&prefix).await?;
        steps.sort_by_key(|step| step.sequence);
        Ok(steps)
    }

    // ── Event ops ─────────────────────────────────────────────

    async fn append_events(&self, events: &[ThreadEvent]) -> Result<(), EngineError> {
        for event in events {
            let path = event_path(event.thread_id, event.id.0)?;
            // Events are append-only; if the same id is appended twice the
            // second write is a no-op duplicate. We don't take a per-event
            // lock because each event id is unique per emission.
            self.write_record(&path, kind_event(), event, event_indexed(event))
                .await?;
        }
        Ok(())
    }

    async fn load_events(&self, thread_id: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
        let prefix = events_prefix(thread_id)?;
        let mut events: Vec<ThreadEvent> = self.read_all_files_under(&prefix).await?;
        events.sort_by_key(|event| event.timestamp);
        Ok(events)
    }

    // ── Project ops ───────────────────────────────────────────

    async fn save_project(&self, project: &Project) -> Result<(), EngineError> {
        self.ensure_projects_indexes().await?;
        let path = project_path(project.id)?;
        let lock = lock_for_path(&path);
        let _guard = lock.lock().await;
        self.write_record(&path, kind_project(), project, project_indexed(project))
            .await
    }

    async fn load_project(&self, id: ProjectId) -> Result<Option<Project>, EngineError> {
        let path = project_path(id)?;
        self.read_one(&path).await
    }

    async fn list_projects(&self, user_id: &str) -> Result<Vec<Project>, EngineError> {
        self.ensure_projects_indexes().await?;
        let prefix = projects_prefix()?;
        let filter = Filter::Eq {
            key: index_key_user_id(),
            value: index_value_text(user_id),
        };
        self.query_all(&prefix, &filter).await
    }

    async fn list_all_projects(&self) -> Result<Vec<Project>, EngineError> {
        let prefix = projects_prefix()?;
        self.query_all(&prefix, &Filter::All).await
    }

    // ── Conversation ops ──────────────────────────────────────

    async fn save_conversation(
        &self,
        conversation: &ConversationSurface,
    ) -> Result<(), EngineError> {
        self.ensure_conversations_indexes().await?;
        let path = conversation_path(conversation.id)?;
        let lock = lock_for_path(&path);
        let _guard = lock.lock().await;
        self.write_record(
            &path,
            kind_conversation(),
            conversation,
            conversation_indexed(conversation),
        )
        .await
    }

    async fn load_conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<ConversationSurface>, EngineError> {
        let path = conversation_path(id)?;
        self.read_one(&path).await
    }

    async fn list_conversations(
        &self,
        user_id: &str,
    ) -> Result<Vec<ConversationSurface>, EngineError> {
        self.ensure_conversations_indexes().await?;
        let prefix = conversations_prefix()?;
        let filter = Filter::Eq {
            key: index_key_user_id(),
            value: index_value_text(user_id),
        };
        self.query_all(&prefix, &filter).await
    }

    // ── Memory doc ops ────────────────────────────────────────

    async fn save_memory_doc(&self, doc: &MemoryDoc) -> Result<(), EngineError> {
        self.ensure_memory_indexes().await?;
        let path = memory_path(doc.project_id, doc.id)?;
        let lock = lock_for_path(&path);
        let _guard = lock.lock().await;
        self.write_record(&path, kind_memory_doc(), doc, memory_indexed(doc))
            .await
    }

    async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
        // Without the project id in the path, we have to fan out over each
        // project subdirectory. This mirrors the legacy `HybridStore` shape
        // (in-memory `HashMap<DocId, MemoryDoc>`). Production traffic does
        // not hit this hot — most consumers call `list_memory_docs` with a
        // project scope first.
        let memory_root = memory_prefix_all()?;
        let project_dirs = self.list_subdir_paths(&memory_root).await?;
        for dir in project_dirs {
            let candidate = match dir.as_str().rsplit('/').next() {
                Some(slug) => slug,
                None => continue,
            };
            let project_id = match uuid::Uuid::parse_str(candidate) {
                Ok(uuid) => ProjectId(uuid),
                Err(_) => continue,
            };
            let path = memory_path(project_id, id)?;
            if let Some(doc) = self.read_one::<MemoryDoc>(&path).await? {
                return Ok(Some(doc));
            }
        }
        Ok(None)
    }

    async fn list_memory_docs(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        self.ensure_memory_indexes().await?;
        let prefix = memory_prefix_for_project(project_id)?;
        let filter = Filter::Eq {
            key: index_key_user_id(),
            value: index_value_text(user_id),
        };
        match self.query_all(&prefix, &filter).await {
            Ok(docs) => Ok(docs),
            // Falling back to a directory scan keeps behaviour usable on
            // byte-only mounts that report `Unsupported` for query. The
            // result is still correctly user-scoped because we filter in
            // Rust.
            Err(error) if is_engine_unsupported(&error) => {
                let mut docs: Vec<MemoryDoc> = self.read_all_files_under(&prefix).await?;
                docs.retain(|doc| doc.user_id == user_id);
                Ok(docs)
            }
            Err(error) => Err(error),
        }
    }

    async fn list_memory_docs_by_owner(
        &self,
        user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        self.ensure_memory_indexes().await?;
        let prefix = memory_prefix_all()?;
        let filter = Filter::Eq {
            key: index_key_user_id(),
            value: index_value_text(user_id),
        };
        match self.query_all(&prefix, &filter).await {
            Ok(docs) => Ok(docs),
            // Byte-only mounts: fall back to the cross-project directory
            // scan. The base `Store` default implementation would fan out
            // over `list_all_projects()`, which on a fresh filesystem is
            // empty — the directory scan is the right primitive here.
            Err(error) if is_engine_unsupported(&error) => {
                let project_dirs = self.list_subdir_paths(&prefix).await?;
                let mut docs = Vec::new();
                for dir in project_dirs {
                    docs.extend(self.read_all_files_under::<MemoryDoc>(&dir).await?);
                }
                docs.retain(|doc| doc.user_id == user_id);
                Ok(docs)
            }
            Err(error) => Err(error),
        }
    }

    // ── Capability lease ops ──────────────────────────────────

    async fn save_lease(&self, lease: &CapabilityLease) -> Result<(), EngineError> {
        let path = lease_path(lease.thread_id, lease.id)?;
        let lock = lock_for_path(&path);
        let _guard = lock.lock().await;
        self.write_record(&path, kind_lease(), lease, lease_indexed(lease))
            .await
    }

    async fn load_active_leases(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<CapabilityLease>, EngineError> {
        let prefix = leases_prefix(thread_id)?;
        let leases: Vec<CapabilityLease> = self.read_all_files_under(&prefix).await?;
        Ok(leases.into_iter().filter(|lease| !lease.revoked).collect())
    }

    async fn revoke_lease(&self, lease_id: LeaseId, reason: &str) -> Result<(), EngineError> {
        // We don't know the thread_id from the lease_id alone; scan all
        // lease subdirectories until we find it. Lease lookup is rare
        // (revoke + grant flows), and the directory cardinality is
        // bounded by active threads.
        let leases_root = leases_root_path()?;
        let thread_dirs = self.list_subdir_paths(&leases_root).await?;
        for dir in thread_dirs {
            let candidate = match dir.as_str().rsplit('/').next() {
                Some(slug) => slug,
                None => continue,
            };
            let thread_id = match uuid::Uuid::parse_str(candidate) {
                Ok(uuid) => ThreadId(uuid),
                Err(_) => continue,
            };
            let path = lease_path(thread_id, lease_id)?;
            let lock = lock_for_path(&path);
            let _guard = lock.lock().await;
            // CAS retry: a concurrent grant/consume on the same lease
            // from another process must not be silently overwritten by
            // this revoke.
            loop {
                // silent-ok: scanning thread directories for the lease's
                // owner — a directory that no longer contains this lease id
                // is a normal cold miss, not a failure (legacy LeaseManager
                // tolerates revoke on unknown lease ids the same way).
                let Some(versioned) = self.read_versioned(&path).await? else {
                    break;
                };
                let mut lease: CapabilityLease = deserialize(&versioned.entry.body)?;
                lease.revoked = true;
                lease.revoked_reason = Some(reason.to_string());
                let entry = build_record_entry(kind_lease(), &lease, lease_indexed(&lease))?;
                match self
                    .filesystem
                    .put(&path, entry, CasExpectation::Version(versioned.version))
                    .await
                {
                    Ok(_) => return Ok(()),
                    Err(FilesystemError::VersionMismatch { .. }) => continue,
                    Err(error) => return Err(fs_to_engine_error(error)),
                }
            }
        }
        // Unknown lease ids are tolerated — the engine's in-memory
        // `LeaseManager` already revokes by id and would never round-trip
        // an unknown id to the store except through misuse. Mirror the
        // `update_thread_state` behaviour and fail open here.
        Ok(())
    }

    // ── Mission ops ───────────────────────────────────────────

    async fn save_mission(&self, mission: &Mission) -> Result<(), EngineError> {
        self.ensure_missions_indexes().await?;
        let path = mission_path(mission.project_id, mission.id)?;
        let lock = lock_for_path(&path);
        let _guard = lock.lock().await;
        self.write_record(&path, kind_mission(), mission, mission_indexed(mission))
            .await
    }

    async fn load_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError> {
        // `id` is not unique across projects in path layout. Scan the
        // mission project subdirectories. Same approach as
        // `load_memory_doc`.
        let mission_root = missions_prefix()?;
        let project_dirs = self.list_subdir_paths(&mission_root).await?;
        for dir in project_dirs {
            let candidate = match dir.as_str().rsplit('/').next() {
                Some(slug) => slug,
                None => continue,
            };
            let project_id = match uuid::Uuid::parse_str(candidate) {
                Ok(uuid) => ProjectId(uuid),
                Err(_) => continue,
            };
            let path = mission_path(project_id, id)?;
            if let Some(mission) = self.read_one::<Mission>(&path).await? {
                return Ok(Some(mission));
            }
        }
        Ok(None)
    }

    async fn list_missions(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<Mission>, EngineError> {
        self.ensure_missions_indexes().await?;
        let prefix = missions_prefix_for_project(project_id)?;
        let filter = Filter::Eq {
            key: index_key_user_id(),
            value: index_value_text(user_id),
        };
        let mut missions: Vec<Mission> = match self.query_all(&prefix, &filter).await {
            Ok(missions) => missions,
            Err(error) if is_engine_unsupported(&error) => {
                let mut missions: Vec<Mission> = self.read_all_files_under(&prefix).await?;
                missions.retain(|mission| mission.user_id == user_id);
                missions
            }
            Err(error) => return Err(error),
        };
        // HybridStore parity (`src/bridge/store_adapter.rs:1913`):
        // sort by (name, id) so the `mission_list` tool sees a stable
        // order across runs — the underlying `query` / HashMap iteration
        // is otherwise non-deterministic.
        missions.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.0.cmp(&b.id.0)));
        Ok(missions)
    }

    async fn update_mission_status(
        &self,
        id: MissionId,
        status: MissionStatus,
    ) -> Result<(), EngineError> {
        let mission_root = missions_prefix()?;
        let project_dirs = self.list_subdir_paths(&mission_root).await?;
        for dir in project_dirs {
            let candidate = match dir.as_str().rsplit('/').next() {
                Some(slug) => slug,
                None => continue,
            };
            let project_id = match uuid::Uuid::parse_str(candidate) {
                Ok(uuid) => ProjectId(uuid),
                Err(_) => continue,
            };
            let path = mission_path(project_id, id)?;
            let lock = lock_for_path(&path);
            let _guard = lock.lock().await;
            // CAS retry: a concurrent mission edit from another process
            // (e.g. `save_mission` after a heartbeat tick) must not be
            // clobbered by this status transition.
            loop {
                // silent-ok: scanning project directories — a project
                // directory that doesn't contain this mission is a normal
                // miss; tolerate it the same way `update_thread_state` does.
                let Some(versioned) = self.read_versioned(&path).await? else {
                    break;
                };
                let mut mission: Mission = deserialize(&versioned.entry.body)?;
                mission.status = status;
                // HybridStore parity (`src/bridge/store_adapter.rs:1950`):
                // bump `updated_at` on every status transition so consumers
                // sorting by recency see the change.
                mission.updated_at = chrono::Utc::now();
                let entry =
                    build_record_entry(kind_mission(), &mission, mission_indexed(&mission))?;
                match self
                    .filesystem
                    .put(&path, entry, CasExpectation::Version(versioned.version))
                    .await
                {
                    Ok(_) => return Ok(()),
                    Err(FilesystemError::VersionMismatch { .. }) => continue,
                    Err(error) => return Err(fs_to_engine_error(error)),
                }
            }
        }
        // Unknown mission id: tolerate, matching `update_thread_state`.
        Ok(())
    }

    // ── Admin ops ─────────────────────────────────────────────

    async fn list_all_threads(&self, project_id: ProjectId) -> Result<Vec<Thread>, EngineError> {
        self.ensure_threads_indexes().await?;
        let prefix = threads_prefix()?;
        let filter = Filter::Eq {
            key: index_key_project_id(),
            value: index_value_text(project_id.0.to_string()),
        };
        self.query_all(&prefix, &filter).await
    }

    async fn list_all_missions(&self, project_id: ProjectId) -> Result<Vec<Mission>, EngineError> {
        self.ensure_missions_indexes().await?;
        let prefix = missions_prefix_for_project(project_id)?;
        let mut missions: Vec<Mission> = match self.query_all(&prefix, &Filter::All).await {
            Ok(missions) => missions,
            Err(error) if is_engine_unsupported(&error) => {
                self.read_all_files_under(&prefix).await?
            }
            Err(error) => return Err(error),
        };
        // HybridStore parity (`src/bridge/store_adapter.rs:1937`): admin
        // listing must also sort by (name, id) so dashboards see stable
        // output across runs.
        missions.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.0.cmp(&b.id.0)));
        Ok(missions)
    }
}

fn is_engine_unsupported(error: &EngineError) -> bool {
    // The typed `FilesystemError::Unsupported` discriminator is preserved
    // through `fs_to_engine_error` as a stable `[fs:unsupported]` tag in the
    // reason string. Match on that tag rather than free-text "Unsupported"
    // substrings, which would false-positive on unrelated errors that happen
    // to mention the word.
    match error {
        EngineError::Store { reason } => reason.contains(FS_UNSUPPORTED_TAG),
        _ => false,
    }
}

fn leases_root_path() -> Result<VirtualPath, EngineError> {
    VirtualPath::new("/engine/leases").map_err(host_api_to_engine_error)
}
