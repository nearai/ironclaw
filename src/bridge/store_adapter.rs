//! Hybrid store adapter — workspace-backed persistence for engine state.
//!
//! Knowledge docs use frontmatter+markdown for human readability.
//! Runtime state uses JSON under `runtime/` to stay out of the way.
//!
//! All v2 engine state lives under `.system/engine/` alongside other
//! machine-managed state (`.system/settings/`, `.system/extensions/`,
//! `.system/skills/`).
//!
//! ## Workspace layout
//!
//! ```text
//! .system/engine/
//! ├── README.md                                   (auto-generated index)
//! ├── knowledge/{type}/{slug}--{id8}.md           (frontmatter + content)
//! ├── orchestrator/v{N}.py                        (Python orchestrator versions)
//! ├── orchestrator/failures.json
//! ├── orchestrator/codeact-preamble-overlay.md    (runtime prompt patches)
//! ├── projects/{slug}--{id8}.json
//! ├── projects/{slug}/missions/{slug}--{id8}/mission.json
//! └── runtime/                                    (internal, not for browsing)
//!     ├── threads/active/{id}.json
//!     ├── threads/archive/{slug}.json             (compacted summaries)
//!     ├── conversations/{id}.json
//!     ├── leases/{id}.json
//!     ├── events/{thread_id}.json
//!     └── steps/{thread_id}.json
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use tokio::sync::RwLock;
use tracing::debug;

use ironclaw_engine::{
    CapabilityLease, ConversationId, ConversationSurface, DocId, DocType, EngineError, LeaseId,
    MemoryDoc, Project, ProjectId, Step, Store, Thread, ThreadEvent, ThreadId, ThreadState,
    types::mission::{Mission, MissionId, MissionStatus},
};

use crate::workspace::{Workspace, WorkspaceEntry};

// ── Path constants ──────────────────────────────────────────
//
// All v2 engine state lives under `.system/engine/` alongside other
// machine-managed state (`.system/settings/`, `.system/extensions/`,
// `.system/skills/`). The dot-prefix on `.system/` is the hidden marker;
// no inner dot is needed for `runtime/`.

const KNOWLEDGE_PREFIX: &str = ".system/engine/knowledge";
const ORCHESTRATOR_PREFIX: &str = ".system/engine/orchestrator";
/// Pre-#2049 orchestrator prefix; matched alongside the canonical prefix so
/// fresh writes targeted at the legacy path still hit the protection check.
const LEGACY_ORCHESTRATOR_PREFIX: &str = "engine/orchestrator";
const PROJECTS_PREFIX: &str = ".system/engine/projects";

const THREADS_PREFIX: &str = ".system/engine/runtime/threads/active";
const THREAD_ARCHIVE_PREFIX: &str = ".system/engine/runtime/threads/archive";
const STEPS_PREFIX: &str = ".system/engine/runtime/steps";
const EVENTS_PREFIX: &str = ".system/engine/runtime/events";
const LEASES_PREFIX: &str = ".system/engine/runtime/leases";
const CONVERSATIONS_PREFIX: &str = ".system/engine/runtime/conversations";

/// Legacy `engine/...` root used before #2049 moved engine state under
/// `.system/engine/...`. `migrate_legacy_engine_paths` rewrites any document
/// found under this prefix into the new location at startup. Kept indefinitely
/// so old workspaces upgrading after a long pause still get migrated.
const LEGACY_ENGINE_ROOT: &str = "engine";
const NEW_ENGINE_ROOT: &str = ".system/engine";

// Well-known titles for special-case routing (must match engine crate constants)
const ORCHESTRATOR_MAIN_TITLE: &str = "orchestrator:main";
const ORCHESTRATOR_FAILURES_TITLE: &str = "orchestrator:failures";
const PREAMBLE_OVERLAY_TITLE: &str = "prompt:codeact_preamble";
const ORCHESTRATOR_CODE_TAG: &str = "orchestrator_code";
const FIX_PATTERN_TITLE: &str = "fix_pattern_database";

/// Re-export the engine's process-wide self-modify snapshot so the store
/// gate reads the same value as the engine loop, the memory tool, and the
/// self-improvement mission.
fn self_modify_enabled() -> bool {
    ironclaw_engine::runtime::self_modify_enabled()
}

/// Workspace-backed engine store.
pub struct HybridStore {
    threads: RwLock<HashMap<ThreadId, Thread>>,
    steps: RwLock<HashMap<ThreadId, Vec<Step>>>,
    events: RwLock<HashMap<ThreadId, Vec<ThreadEvent>>>,
    projects: RwLock<HashMap<ProjectId, Project>>,
    conversations: RwLock<HashMap<ConversationId, ConversationSurface>>,
    leases: RwLock<HashMap<LeaseId, CapabilityLease>>,
    missions: RwLock<HashMap<MissionId, Mission>>,
    docs: RwLock<HashMap<DocId, MemoryDoc>>,
    /// Tracks current workspace path for each doc so renames can delete the old file.
    doc_paths: RwLock<HashMap<DocId, String>>,
    workspace: Option<Arc<Workspace>>,
}

impl HybridStore {
    pub fn new(workspace: Option<Arc<Workspace>>) -> Self {
        Self {
            threads: RwLock::new(HashMap::new()),
            steps: RwLock::new(HashMap::new()),
            events: RwLock::new(HashMap::new()),
            projects: RwLock::new(HashMap::new()),
            conversations: RwLock::new(HashMap::new()),
            leases: RwLock::new(HashMap::new()),
            missions: RwLock::new(HashMap::new()),
            docs: RwLock::new(HashMap::new()),
            doc_paths: RwLock::new(HashMap::new()),
            workspace,
        }
    }

    /// Load persisted engine state from the workspace on startup.
    pub async fn load_state_from_workspace(&self) {
        let Some(ws) = self.workspace.as_ref() else {
            return;
        };

        // Migrate any state still living under the legacy `engine/...`
        // prefix into `.system/engine/...` BEFORE the loaders run, so the
        // load below sees a single canonical location and orphaned legacy
        // documents don't accumulate.
        self.migrate_legacy_engine_paths(ws).await;

        self.load_knowledge_docs(ws).await;
        self.load_map(ws, PROJECTS_PREFIX, |project: Project| async {
            self.projects.write().await.insert(project.id, project);
        })
        .await;
        self.load_map(
            ws,
            CONVERSATIONS_PREFIX,
            |conversation: ConversationSurface| async {
                self.conversations
                    .write()
                    .await
                    .insert(conversation.id, conversation);
            },
        )
        .await;
        self.load_map(ws, THREADS_PREFIX, |thread: Thread| async {
            self.threads.write().await.insert(thread.id, thread);
        })
        .await;
        self.load_map(ws, STEPS_PREFIX, |steps: Vec<Step>| async {
            if let Some(thread_id) = steps.first().map(|step| step.thread_id) {
                self.steps.write().await.insert(thread_id, steps);
            }
        })
        .await;
        self.load_map(ws, EVENTS_PREFIX, |events: Vec<ThreadEvent>| async {
            if let Some(thread_id) = events.first().map(|event| event.thread_id) {
                self.events.write().await.insert(thread_id, events);
            }
        })
        .await;
        self.load_map(ws, LEASES_PREFIX, |lease: CapabilityLease| async {
            self.leases.write().await.insert(lease.id, lease);
        })
        .await;
        // Missions live under each project: .system/engine/projects/{slug}/missions/{slug}/mission.json
        self.load_missions_from_projects(ws).await;

        // Backfill archived threads referenced by missions but missing from the
        // active threads map (threads archived before the fix that preserves
        // stripped Thread objects in the active path).
        self.backfill_archived_threads(ws).await;

        let projects = self.projects.read().await.len();
        let conversations = self.conversations.read().await.len();
        let threads = self.threads.read().await.len();
        let steps = self.steps.read().await.len();
        let events = self.events.read().await.len();
        let leases = self.leases.read().await.len();
        let missions = self.missions.read().await.len();
        let docs = self.docs.read().await.len();

        debug!(
            projects,
            conversations,
            threads,
            steps,
            events,
            leases,
            missions,
            docs,
            "loaded engine state from workspace"
        );
    }

    /// One-shot startup migration: rewrite any documents stored under the
    /// legacy `engine/...` prefix into `.system/engine/...`.
    ///
    /// Pre-#2049 deployments persisted v2 engine state under `engine/...`.
    /// After the unification under `.system/`, the loaders only look at the
    /// new prefix — without this migration, all legacy state (projects,
    /// missions, threads, leases, conversations, knowledge docs) would be
    /// invisible after upgrade. The migration is idempotent: once nothing
    /// remains under `engine/`, the call is a single workspace listing.
    ///
    /// **Cheap preflight:** the steady-state startup case (post-migration)
    /// must not run a full workspace scan every time. We first check
    /// `ws.list("engine")` for any direct children. Only when that returns
    /// at least one entry do we fall back to `ws.list_all()` for the
    /// recursive discovery — most startups skip the full scan entirely.
    ///
    /// **Version history note:** the migration uses a read-write-delete
    /// pattern, not a path-rename. Because `memory_document_versions` has
    /// `ON DELETE CASCADE`, the legacy doc's version history is not
    /// preserved into the new doc — the new doc starts a fresh version
    /// chain. This is intentional and acceptable scope:
    ///
    /// - V2 engine state (projects, threads, leases, conversations,
    ///   knowledge docs) is *runtime state*, rewritten on every state
    ///   mutation. There is no curated user-edited version history at
    ///   risk.
    /// - V2 engine state was newly introduced in this PR. There is no
    ///   production deployment with months of accumulated v2 history.
    /// - Adding a path-preserving rename op to `Workspace`/`Database`
    ///   would require new trait methods on both backends and is
    ///   substantial scope creep for a fix-forward. If a future caller
    ///   needs version-history-preserving rename, that operation should
    ///   be added properly to the storage layer, not bolted onto the
    ///   migration here.
    ///
    /// Document `metadata` IS preserved via `ws.update_metadata` after
    /// the new doc is written.
    ///
    /// Failures on individual files are logged at `debug!` but do not abort
    /// the migration — the worst case is that the legacy file stays put and
    /// we retry on the next startup.
    async fn migrate_legacy_engine_paths(&self, ws: &Workspace) {
        // Cheap preflight: most startups have no legacy paths and must
        // not pay for a full `list_all()` traversal. A direct
        // `list("engine")` is one indexed lookup; if it returns nothing
        // we're done.
        match ws.list(LEGACY_ENGINE_ROOT).await {
            Ok(entries) if entries.is_empty() => return,
            Ok(_) => {}
            Err(e) => {
                debug!("legacy-engine migration: preflight list failed: {e}");
                return;
            }
        }

        // Preflight saw something — fall back to a full `list_all()` to
        // pick up nested paths. (`list` is single-level only, so we need
        // recursive enumeration to discover everything under `engine/`.)
        let all_paths = match ws.list_all().await {
            Ok(paths) => paths,
            Err(e) => {
                debug!("legacy-engine migration: list_all failed: {e}");
                return;
            }
        };

        let legacy: Vec<String> = all_paths
            .into_iter()
            .filter(|p| {
                // Match `engine/...` exactly (not `.system/engine/...` and not
                // some other path that happens to contain "engine"). Strip
                // any leading `/` first because some storages return absolute
                // paths and others don't.
                let trimmed = p.strip_prefix('/').unwrap_or(p);
                trimmed == LEGACY_ENGINE_ROOT
                    || trimmed.starts_with(&format!("{LEGACY_ENGINE_ROOT}/"))
            })
            .collect();

        if legacy.is_empty() {
            return;
        }

        debug!(
            count = legacy.len(),
            "migrating legacy engine paths to .system/engine/"
        );

        let mut migrated = 0usize;
        let mut failed = 0usize;
        for old_path in legacy {
            // Compute the new path by replacing the leading `engine` segment
            // with `.system/engine`. Preserves the rest of the path verbatim.
            let trimmed = old_path.strip_prefix('/').unwrap_or(&old_path);
            let suffix = trimmed
                .strip_prefix(LEGACY_ENGINE_ROOT)
                .and_then(|s| s.strip_prefix('/').or(Some(s)))
                .unwrap_or("");
            let new_path = if suffix.is_empty() {
                NEW_ENGINE_ROOT.to_string()
            } else {
                format!("{NEW_ENGINE_ROOT}/{suffix}")
            };

            // Read old, write new (preserving metadata), delete old. We
            // tolerate the case where the new path already exists by
            // skipping the rewrite (a partial previous migration may have
            // already moved this file).
            let doc = match ws.read(&old_path).await {
                Ok(doc) => doc,
                Err(e) => {
                    debug!(
                        old = %old_path,
                        "legacy-engine migration: read failed: {e}"
                    );
                    failed += 1;
                    continue;
                }
            };

            // Propagate `exists` errors instead of treating a transient
            // failure as "file absent" — that would cause the migrator to
            // overwrite an existing `.system/engine/...` doc when storage
            // hiccups.
            let already_present = match ws.exists(&new_path).await {
                Ok(present) => present,
                Err(e) => {
                    debug!(
                        old = %old_path,
                        new = %new_path,
                        "legacy-engine migration: exists check failed: {e}"
                    );
                    failed += 1;
                    continue;
                }
            };

            if !already_present {
                let new_doc = match ws.write(&new_path, &doc.content).await {
                    Ok(d) => d,
                    Err(e) => {
                        debug!(
                            old = %old_path,
                            new = %new_path,
                            "legacy-engine migration: write failed: {e}"
                        );
                        failed += 1;
                        continue;
                    }
                };
                // Preserve the legacy doc's metadata onto the new doc so
                // schema/skip_indexing/skip_versioning/hygiene flags
                // survive the migration. Logged-not-fatal because the
                // content has already been moved.
                if !doc.metadata.is_null()
                    && let Err(e) = ws.update_metadata(new_doc.id, &doc.metadata).await
                {
                    debug!(
                        old = %old_path,
                        new = %new_path,
                        "legacy-engine migration: metadata copy failed: {e}"
                    );
                }
            }

            if let Err(e) = ws.delete(&old_path).await {
                debug!(
                    old = %old_path,
                    "legacy-engine migration: delete failed: {e}"
                );
                failed += 1;
                continue;
            }
            // Always count successful path migrations — including the
            // already_present case where we skipped the rewrite but
            // still removed the legacy duplicate.
            migrated += 1;
        }

        debug!(migrated, failed, "legacy-engine migration: complete");
    }

    /// Evict terminal (Done/Failed) threads from in-memory caches.
    ///
    /// Full thread data (messages, events, steps) is **always preserved on
    /// disk** — LLM output is never deleted.  This method only removes old
    /// terminal threads from the in-memory maps to keep RAM bounded.
    /// `load_thread()` will lazy-reload from disk on the next access.
    ///
    /// Also writes a compact archive summary for human-browsable indexing
    /// and cleans up expired/revoked leases (from memory only — lease files
    /// stay on disk).
    pub async fn cleanup_terminal_state(&self, min_age: chrono::Duration) -> usize {
        let mut cleaned = 0;
        let now = chrono::Utc::now();

        // 1. Evict terminal threads from in-memory maps (disk files stay)
        let terminal: Vec<Thread> = self
            .threads
            .read()
            .await
            .values()
            .filter(|t| {
                matches!(
                    t.state,
                    ThreadState::Done | ThreadState::Failed | ThreadState::Completed
                ) && t
                    .completed_at
                    .or(Some(t.updated_at))
                    .is_some_and(|at| (now - at) > min_age)
            })
            .cloned()
            .collect();

        for thread in &terminal {
            // Write compact archive summary (for human-readable browsing)
            let slug = slugify(&thread.goal, &thread.id.0.to_string());
            let archive_path = format!("{THREAD_ARCHIVE_PREFIX}/{slug}.json");
            let summary = compact_thread_summary(thread);
            self.persist_json(archive_path, &summary).await;

            // Evict from in-memory maps only — disk files are never deleted.
            self.threads.write().await.remove(&thread.id);
            self.events.write().await.remove(&thread.id);
            self.steps.write().await.remove(&thread.id);
            cleaned += 1;
        }

        // 2. Clean up revoked/expired leases from memory
        let dead_leases: Vec<LeaseId> = self
            .leases
            .read()
            .await
            .iter()
            .filter(|(_, l)| l.revoked || !l.is_valid())
            .map(|(id, _)| *id)
            .collect();
        for lid in &dead_leases {
            self.leases.write().await.remove(lid);
            cleaned += 1;
        }

        if cleaned > 0 {
            debug!(
                threads_evicted = terminal.len(),
                leases_cleaned = dead_leases.len(),
                "evicted terminal state from memory (disk preserved)"
            );
        }

        cleaned
    }

    /// Generate `.system/engine/README.md` with a summary of current engine state.
    pub async fn generate_engine_readme(&self) {
        let docs = self.docs.read().await;
        let threads = self.threads.read().await;
        let missions = self.missions.read().await;
        let leases = self.leases.read().await;

        let count_by_type = |dt: DocType| docs.values().filter(|d| d.doc_type == dt).count();
        let active_threads = threads
            .values()
            .filter(|t| !matches!(t.state, ThreadState::Done | ThreadState::Failed))
            .count();
        let active_leases = leases.values().filter(|l| l.is_valid()).count();

        // Count orchestrator versions
        let orch_versions = docs
            .values()
            .filter(|d| {
                d.title == ORCHESTRATOR_MAIN_TITLE
                    && d.tags.contains(&ORCHESTRATOR_CODE_TAG.to_string())
            })
            .count();

        let mut readme = format!(
            "# Engine State\n\n\
             Last updated: {}\n\n",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
        );

        readme.push_str("## Knowledge (`.system/engine/knowledge/`)\n\n");
        readme.push_str(&format!(
            "- **{} lessons** — learned rules\n",
            count_by_type(DocType::Lesson)
        ));
        readme.push_str(&format!(
            "- **{} skills** — extracted procedures\n",
            count_by_type(DocType::Skill)
        ));
        readme.push_str(&format!(
            "- **{} summaries** — thread completion records\n",
            count_by_type(DocType::Summary)
        ));
        readme.push_str(&format!(
            "- **{} specs** — specifications\n",
            count_by_type(DocType::Spec)
        ));
        readme.push_str(&format!(
            "- **{} issues** — known problems\n",
            count_by_type(DocType::Issue)
        ));

        readme.push_str(&format!(
            "\n## Orchestrator (`.system/engine/orchestrator/`)\n\n\
             - {} version(s) stored\n",
            orch_versions
        ));

        readme.push_str("\n## Missions (`.system/engine/projects/<project>/missions/`)\n\n");
        for m in missions.values() {
            readme.push_str(&format!(
                "- **{}** ({:?}) — {}\n",
                m.name,
                m.status,
                truncate_for_readme(&m.goal, 80)
            ));
        }

        readme.push_str(&format!(
            "\n## Runtime (`.system/engine/runtime/`)\n\n\
             - {} active thread(s)\n\
             - {} active lease(s)\n",
            active_threads, active_leases,
        ));

        self.persist_text(".system/engine/README.md".to_string(), &readme)
            .await;
    }

    // ── Internal helpers ────────────────────────────────────

    async fn load_knowledge_docs(&self, ws: &Workspace) {
        // Knowledge docs can be .md (frontmatter), .json (legacy/runtime),
        // or .py (orchestrator versions persisted as raw Python).
        let search_prefixes = [KNOWLEDGE_PREFIX, ORCHESTRATOR_PREFIX];

        for prefix in search_prefixes {
            for entry in self
                .file_entries(ws, prefix, &[".md", ".json", ".py"])
                .await
            {
                match ws.read(&entry.path).await {
                    Ok(doc) => {
                        // Try frontmatter format first, then JSON, then raw .py
                        let parsed = deserialize_knowledge_doc(&doc.content)
                            .or_else(|| serde_json::from_str::<MemoryDoc>(&doc.content).ok())
                            .or_else(|| {
                                synthesize_orchestrator_doc_from_py(&entry.path, &doc.content)
                            });
                        if let Some(memory_doc) = parsed {
                            self.doc_paths
                                .write()
                                .await
                                .insert(memory_doc.id, entry.path.clone());
                            self.docs.write().await.insert(memory_doc.id, memory_doc);
                        } else {
                            debug!(path = %entry.path, "skipped non-doc file in engine");
                        }
                    }
                    Err(e) => debug!(path = %entry.path, "failed to read engine doc: {e}"),
                }
            }
        }
    }

    async fn load_map<T, F, Fut>(&self, ws: &Workspace, directory: &str, on_value: F)
    where
        T: DeserializeOwned,
        F: Fn(T) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        for entry in self.file_entries(ws, directory, &[".json"]).await {
            match ws.read(&entry.path).await {
                Ok(doc) => match serde_json::from_str::<T>(&doc.content) {
                    Ok(value) => on_value(value).await,
                    Err(e) => debug!(path = %entry.path, "failed to parse engine state: {e}"),
                },
                Err(e) => debug!(path = %entry.path, "failed to read engine state: {e}"),
            }
        }
    }

    /// List files under a directory, recursing one level into subdirectories.
    async fn file_entries(
        &self,
        ws: &Workspace,
        directory: &str,
        extensions: &[&str],
    ) -> Vec<WorkspaceEntry> {
        let top = match ws.list(directory).await {
            Ok(entries) => entries,
            Err(_) => return Vec::new(),
        };

        let mut files = Vec::new();
        for entry in top {
            if entry.is_directory {
                if let Ok(children) = ws.list(&entry.path).await {
                    files.extend(children.into_iter().filter(|child| {
                        !child.is_directory
                            && extensions.iter().any(|ext| child.path.ends_with(ext))
                    }));
                }
            } else if extensions.iter().any(|ext| entry.path.ends_with(ext)) {
                files.push(entry);
            }
        }
        files
    }

    async fn persist_json<T: serde::Serialize>(&self, path: String, value: &T) {
        let Some(ws) = self.workspace.as_ref() else {
            return;
        };

        let json = match serde_json::to_string_pretty(value) {
            Ok(json) => json,
            Err(e) => {
                debug!(path = %path, "failed to serialize engine state: {e}");
                return;
            }
        };

        if let Err(e) = ws.write(&path, &json).await {
            debug!(path = %path, "failed to persist engine state: {e}");
        }
    }

    async fn persist_text(&self, path: String, content: &str) {
        let Some(ws) = self.workspace.as_ref() else {
            return;
        };
        if let Err(e) = ws.write(&path, content).await {
            debug!(path = %path, "failed to persist engine text: {e}");
        }
    }

    /// Load missions from within each project directory.
    ///
    /// Scans `.system/engine/projects/*/missions/*/mission.json`.
    async fn load_missions_from_projects(&self, ws: &Workspace) {
        let project_dirs = match ws.list(PROJECTS_PREFIX).await {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for proj_entry in project_dirs {
            if !proj_entry.is_directory {
                continue;
            }
            let missions_dir = format!("{}/missions", proj_entry.path);
            let mission_dirs = match ws.list(&missions_dir).await {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for mission_entry in mission_dirs {
                if !mission_entry.is_directory {
                    continue;
                }
                let mission_file = format!("{}/mission.json", mission_entry.path);
                if let Ok(doc) = ws.read(&mission_file).await {
                    match serde_json::from_str::<Mission>(&doc.content) {
                        Ok(mission) => {
                            self.missions.write().await.insert(mission.id, mission);
                        }
                        Err(e) => {
                            debug!(path = %mission_file, "failed to parse mission: {e}")
                        }
                    }
                }
            }
        }
    }

    /// Backfill threads referenced by missions but not yet in the in-memory map.
    ///
    /// Tries the full thread file first (active path in DB), then falls back to
    /// archive summaries for threads that were deleted before data-retention was
    /// fixed.
    async fn backfill_archived_threads(&self, ws: &Workspace) {
        // Collect thread IDs referenced by missions but missing from threads map
        let missions = self.missions.read().await.clone();
        let threads = self.threads.read().await;
        let missing: Vec<ThreadId> = missions
            .values()
            .flat_map(|m| m.thread_history.iter().copied())
            .filter(|tid| !threads.contains_key(tid))
            .collect();
        drop(threads);

        if missing.is_empty() {
            return;
        }

        let mut backfilled = 0usize;

        // First pass: try loading full Thread from active path in DB
        let mut still_missing = Vec::new();
        for tid in &missing {
            if let Ok(doc) = ws.read(&thread_path(*tid)).await
                && let Ok(thread) = serde_json::from_str::<Thread>(&doc.content)
            {
                self.threads.write().await.insert(thread.id, thread);
                backfilled += 1;
            } else {
                still_missing.push(tid.0.to_string());
            }
        }

        // Second pass: fall back to archive summaries for legacy-deleted threads
        if !still_missing.is_empty() {
            let missing_set: std::collections::HashSet<String> =
                still_missing.into_iter().collect();
            if let Ok(archive_entries) = ws.list(THREAD_ARCHIVE_PREFIX).await {
                for entry in archive_entries {
                    if entry.is_directory {
                        continue;
                    }
                    let Ok(doc) = ws.read(&entry.path).await else {
                        continue;
                    };
                    if let Ok(summary) = serde_json::from_str::<ThreadArchiveSummary>(&doc.content)
                        && missing_set.contains(&summary.thread_id)
                        && let Some(thread) = thread_from_archive(&summary)
                    {
                        self.threads.write().await.insert(thread.id, thread);
                        backfilled += 1;
                    }
                }
            }
        }

        if backfilled > 0 {
            debug!(backfilled, "backfilled mission threads from database");
        }
    }

    /// Get the project slug for a project_id, falling back to a short UUID.
    async fn project_slug(&self, project_id: ProjectId) -> String {
        self.projects
            .read()
            .await
            .get(&project_id)
            .map(|p| slugify(&p.name, &p.id.0.to_string()))
            .unwrap_or_else(|| {
                let short = &project_id.0.to_string()[..8]; // safety: UUID.to_string() is always 36 ASCII hex chars
                format!("unknown--{short}")
            })
    }

    async fn delete_workspace_file(&self, path: &str) {
        let Some(ws) = self.workspace.as_ref() else {
            return;
        };
        if let Err(e) = ws.delete(path).await {
            debug!(path = %path, "failed to delete engine file: {e}");
        }
    }

    /// Persist a MemoryDoc to workspace. Knowledge docs use frontmatter+markdown,
    /// special docs (orchestrator, prompts) use their native format, and internal
    /// docs use JSON.
    async fn persist_doc(&self, doc: &MemoryDoc) {
        let new_path = doc_workspace_path(doc);

        // If the doc previously existed at a different path, delete the old one
        if let Some(ref old) = self.doc_paths.read().await.get(&doc.id).cloned()
            && *old != new_path
        {
            self.delete_workspace_file(old).await;
        }

        // Choose serialization format based on path
        let content = if is_orchestrator_code_path(&new_path) {
            // Orchestrator Python: store raw content (the Python source code)
            doc.content.clone()
        } else if new_path.to_ascii_lowercase().ends_with(".md") {
            // safety: case-insensitive for macOS/Windows
            // Knowledge docs and prompt overlays: frontmatter + content
            serialize_knowledge_doc(doc)
        } else {
            // Everything else: JSON
            match serde_json::to_string_pretty(doc) {
                Ok(json) => json,
                Err(e) => {
                    debug!(path = %new_path, "failed to serialize doc: {e}");
                    return;
                }
            }
        };

        self.persist_text(new_path.clone(), &content).await;
        self.doc_paths.write().await.insert(doc.id, new_path);
    }
}

// ── Path helpers ────────────────────────────────────────────

/// Map a MemoryDoc to its workspace path based on title and type.
fn doc_workspace_path(doc: &MemoryDoc) -> String {
    let id_str = doc.id.0.to_string();

    // Orchestrator code versions → .system/engine/orchestrator/v{N}.py
    if doc.title == ORCHESTRATOR_MAIN_TITLE && doc.tags.contains(&ORCHESTRATOR_CODE_TAG.to_string())
    {
        let version = doc
            .metadata
            .get("version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        return format!("{ORCHESTRATOR_PREFIX}/v{version}.py");
    }

    // Orchestrator failure tracker → .system/engine/orchestrator/failures.json
    if doc.title == ORCHESTRATOR_FAILURES_TITLE {
        return format!("{ORCHESTRATOR_PREFIX}/failures.json");
    }

    // Prompt overlays → .system/engine/orchestrator/codeact-preamble-overlay.md
    if doc.title == PREAMBLE_OVERLAY_TITLE {
        return format!("{ORCHESTRATOR_PREFIX}/codeact-preamble-overlay.md");
    }

    // Fix pattern database → .system/engine/knowledge/notes/{slug}.md
    if doc.title == FIX_PATTERN_TITLE {
        let slug = slugify(&doc.title, &id_str);
        return format!("{KNOWLEDGE_PREFIX}/notes/{slug}.md");
    }

    // Knowledge docs → .system/engine/knowledge/{type}/{slug}.md
    let type_dir = match doc.doc_type {
        DocType::Summary => "summaries",
        DocType::Lesson => "lessons",
        DocType::Issue => "issues",
        DocType::Spec => "specs",
        DocType::Note => "notes",
        DocType::Skill => "skills",
        DocType::Plan => "plans",
    };
    let slug = slugify(&doc.title, &id_str);
    format!("{KNOWLEDGE_PREFIX}/{type_dir}/{slug}.md")
}

/// Check whether `path` resolves to an orchestrator `.py` version file.
///
/// Normalizes the path before matching so dot/double-slash/traversal
/// components cannot bypass the check (e.g. `engine/./orchestrator/v3.py`,
/// `.system/engine//orchestrator/v3.py`, `engine/knowledge/../orchestrator/v3.py`).
/// Traversal attempts (`..` segments) are conservatively rejected (return
/// `false`) — they cannot be a legitimate orchestrator code path.
fn is_orchestrator_code_path(path: &str) -> bool {
    let Some(canonical) = normalize_path(path) else {
        return false;
    };
    if !canonical.ends_with(".py") {
        return false;
    }
    canonical.starts_with(&format!("{ORCHESTRATOR_PREFIX}/"))
        || canonical.starts_with(&format!("{LEGACY_ORCHESTRATOR_PREFIX}/"))
}

/// Strip `.` segments and collapse `//`, returning `None` on `..` traversal.
///
/// Mirrors `normalize_workspace_path` in `tools::builtin::memory` — kept
/// local here so the store adapter has no dependency on the tool layer.
fn normalize_path(path: &str) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return None;
        }
        segments.push(seg);
    }
    Some(segments.join("/"))
}

/// Synthesize a MemoryDoc from a raw `.py` orchestrator file found on disk.
///
/// Orchestrator versions are persisted as
/// `.system/engine/orchestrator/v{N}.py` (raw Python). On restart, these
/// need to be reconstituted as MemoryDocs so `load_orchestrator_from_docs()`
/// can find them. The version number is extracted from the filename.
///
/// **Project scope**: orchestrator code is *physically global* — only one
/// `v{N}.py` exists per workspace, regardless of how many projects share
/// the workspace. Synthesized docs use `ProjectId::nil()` as the global
/// marker, and `HybridStore::list_shared_memory_docs` (overridden below)
/// surfaces them for any project query so the executor's per-project
/// `load_orchestrator(project_id)` always finds them after a restart.
fn synthesize_orchestrator_doc_from_py(path: &str, content: &str) -> Option<MemoryDoc> {
    if !is_orchestrator_code_path(path) {
        return None;
    }
    // Extract version from filename: .system/engine/orchestrator/v3.py → 3
    let filename = path.rsplit('/').next()?;
    let version: u64 = filename
        .strip_prefix('v')?
        .strip_suffix(".py")?
        .parse()
        .ok()?;

    Some(MemoryDoc {
        id: DocId(uuid::Uuid::new_v4()),
        project_id: ProjectId(uuid::Uuid::nil()),
        user_id: ironclaw_engine::types::shared_owner_id().to_string(),
        doc_type: DocType::Note,
        title: ORCHESTRATOR_MAIN_TITLE.to_string(),
        content: content.to_string(),
        source_thread_id: None,
        tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
        metadata: serde_json::json!({
            "version": version,
            "source": "persisted_py",
        }),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    })
}

/// Check if a MemoryDoc is a protected orchestrator or prompt overlay document.
fn is_protected_orchestrator_doc(doc: &MemoryDoc) -> bool {
    doc.title.starts_with("orchestrator:") || doc.title.starts_with("prompt:")
}

/// True for docs that are physically global (one file regardless of project).
///
/// These docs live at well-known workspace paths (e.g.
/// `.system/engine/orchestrator/v3.py`) and must surface for any project's
/// `list_shared_memory_docs` query — see the override on `HybridStore`.
fn is_globally_shared(doc: &MemoryDoc) -> bool {
    doc.title == ORCHESTRATOR_MAIN_TITLE
        || doc.title == ORCHESTRATOR_FAILURES_TITLE
        || doc.title == PREAMBLE_OVERLAY_TITLE
}

/// Validate orchestrator content before persisting.
///
/// Only validates `orchestrator:*` documents — they contain Python code
/// executed by the Monty sandbox. `prompt:*` documents (e.g.
/// `prompt:codeact_preamble`) are markdown text injected into the system
/// prompt and are NOT code — validating them as Python would reject every
/// prompt overlay. If the engine ever supports Python-based prompt
/// overlays, this function must be updated to cover those titles too.
///
/// Checks Python syntax so a broken patch doesn't consume failure-budget
/// slots (3 failures trigger auto-rollback). Semantically dangerous
/// patterns (`exec(compile(...))`, `__import__('os')`) pass validation
/// because they are syntactically valid Python — all security enforcement
/// happens at runtime in the Monty sandbox (resource limits, host-function
/// gating, no filesystem/network access).
fn validate_orchestrator_content(doc: &MemoryDoc) -> Result<(), EngineError> {
    if doc.title.starts_with("orchestrator:")
        && doc.title != ORCHESTRATOR_FAILURES_TITLE
        && let Err(reason) = ironclaw_engine::executor::validate_python_syntax(&doc.content)
    {
        return Err(EngineError::InvalidInput {
            reason: format!(
                "orchestrator patch '{}' has invalid Python: {reason}",
                doc.title
            ),
        });
    }
    Ok(())
}

fn project_dir(name: &str, project_id: ProjectId) -> String {
    let slug = slugify(name, &project_id.0.to_string());
    format!("{PROJECTS_PREFIX}/{slug}")
}

fn project_path(name: &str, project_id: ProjectId) -> String {
    format!("{}/project.json", project_dir(name, project_id))
}

fn thread_path(thread_id: ThreadId) -> String {
    format!("{THREADS_PREFIX}/{}.json", thread_id.0)
}

fn conversation_path(conversation_id: ConversationId) -> String {
    format!("{CONVERSATIONS_PREFIX}/{}.json", conversation_id.0)
}

fn step_path(thread_id: ThreadId) -> String {
    format!("{STEPS_PREFIX}/{}.json", thread_id.0)
}

fn event_path(thread_id: ThreadId) -> String {
    format!("{EVENTS_PREFIX}/{}.json", thread_id.0)
}

fn lease_path(lease_id: LeaseId) -> String {
    format!("{LEASES_PREFIX}/{}.json", lease_id.0)
}

fn mission_dir(project_slug: &str, name: &str, mission_id: MissionId) -> String {
    let slug = slugify(name, &mission_id.0.to_string());
    format!("{PROJECTS_PREFIX}/{project_slug}/missions/{slug}")
}

fn mission_path(project_slug: &str, name: &str, mission_id: MissionId) -> String {
    format!(
        "{}/mission.json",
        mission_dir(project_slug, name, mission_id)
    )
}

// ── Slugify ─────────────────────────────────────────────────

/// Create a human-readable filename slug from a title with a short ID suffix.
///
/// `"Validate tool names before first call"` + `"65c9f5cd-..."` →
/// `"validate-tool-names-before-first-call--65c9f5cd"`
fn slugify(title: &str, id: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();

    // Collapse runs of dashes and trim
    let mut collapsed = String::with_capacity(slug.len());
    let mut prev_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_dash && !collapsed.is_empty() {
                collapsed.push('-');
            }
            prev_dash = true;
        } else {
            collapsed.push(c);
            prev_dash = false;
        }
    }
    let collapsed = collapsed.trim_end_matches('-');

    // Truncate slug to 60 chars, append 8-char ID suffix. `collapsed` is
    // ASCII-only because slugify() already replaced non-ASCII chars, so
    // byte-index slicing into it is safe.
    let max_slug = 60;
    let truncated = if collapsed.len() > max_slug {
        // Don't cut in the middle of a word — find last dash before limit.
        let window = &collapsed.as_bytes()[..max_slug]; // safety: ASCII-only input
        let dash_pos = window.iter().rposition(|&b| b == b'-');
        match dash_pos {
            Some(pos) if pos > 20 => &collapsed[..pos], // safety: ASCII-only input
            _ => &collapsed[..max_slug],                // safety: ASCII-only input
        }
    } else {
        collapsed
    };

    let short_id = if id.len() >= 8 { &id[..8] } else { id }; // safety: UUID string is always ASCII
    format!("{truncated}--{short_id}")
}

// ── Frontmatter serialization ───────────────────────────────

/// Serialize a MemoryDoc as YAML frontmatter + markdown content.
/// Escape a string for embedding inside a YAML double-quoted scalar.
///
/// YAML double-quoted scalars require `\`, `"`, and control characters to be
/// escaped. Newlines (`\n`, `\r`), tabs (`\t`), and backslashes are the most
/// common offenders in user-supplied identifiers (e.g. OIDC `sub` claims).
fn yaml_quoted_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn serialize_knowledge_doc(doc: &MemoryDoc) -> String {
    let mut frontmatter = String::from("---\n");
    frontmatter.push_str(&format!("id: \"{}\"\n", doc.id.0));
    frontmatter.push_str(&format!("project_id: \"{}\"\n", doc.project_id.0));
    frontmatter.push_str(&format!(
        "user_id: \"{}\"\n",
        yaml_quoted_escape(&doc.user_id)
    ));
    frontmatter.push_str(&format!("doc_type: \"{:?}\"\n", doc.doc_type));
    frontmatter.push_str(&format!("title: \"{}\"\n", yaml_quoted_escape(&doc.title)));
    if !doc.tags.is_empty() {
        frontmatter.push_str(&format!(
            "tags: [{}]\n",
            doc.tags
                .iter()
                .map(|t| format!("\"{}\"", yaml_quoted_escape(t)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(ref tid) = doc.source_thread_id {
        frontmatter.push_str(&format!("source_thread: \"{}\"\n", tid.0));
    }
    frontmatter.push_str(&format!("created: \"{}\"\n", doc.created_at.to_rfc3339()));
    frontmatter.push_str(&format!("updated: \"{}\"\n", doc.updated_at.to_rfc3339()));
    if doc.metadata != serde_json::json!({})
        && let Ok(meta_str) = serde_json::to_string(&doc.metadata)
    {
        frontmatter.push_str(&format!("metadata: {meta_str}\n"));
    }
    frontmatter.push_str("---\n\n");
    frontmatter.push_str(&doc.content);
    frontmatter
}

/// Deserialize a frontmatter+markdown string back to a MemoryDoc.
fn deserialize_knowledge_doc(content: &str) -> Option<MemoryDoc> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }

    // Find closing ---
    // All slice points are at ASCII boundaries (---, \n) so UTF-8 safe.
    let after_first = content.get(3..)?;
    let nl_pos = after_first.find('\n')?;
    let after_first_line = after_first.get(nl_pos + 1..)?;
    let yaml_end = after_first_line.find("\n---")?;
    let yaml_str = after_first_line.get(..yaml_end)?;
    let body_start = yaml_end + 4; // skip \n---
    let body = after_first_line.get(body_start..)?.trim_start_matches('\n');

    // Parse YAML frontmatter
    let yaml: serde_json::Value = serde_yml::from_str(yaml_str).ok()?;

    let id_str = yaml.get("id")?.as_str()?;
    let id = uuid::Uuid::parse_str(id_str).ok()?;
    let title = yaml.get("title")?.as_str()?.to_string();

    let doc_type_str = yaml
        .get("doc_type")
        .and_then(|v| v.as_str())
        .unwrap_or("Note");
    let doc_type = match doc_type_str {
        "Summary" => DocType::Summary,
        "Lesson" => DocType::Lesson,
        "Issue" => DocType::Issue,
        "Spec" => DocType::Spec,
        "Skill" => DocType::Skill,
        "Plan" => DocType::Plan,
        _ => DocType::Note,
    };

    let tags: Vec<String> = yaml
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let source_thread_id = yaml
        .get("source_thread")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(ThreadId);

    let created_at = yaml
        .get("created")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    let updated_at = yaml
        .get("updated")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    let metadata = yaml
        .get("metadata")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let project_id = yaml
        .get("project_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(ProjectId)
        .unwrap_or_else(|| {
            debug!(
                doc_id = %id,
                "knowledge doc missing project_id frontmatter; loading as nil — \
                 this indicates a doc serialized before project_id/user_id were \
                 persisted and it will not be visible to project-scoped queries"
            );
            ProjectId(uuid::Uuid::nil())
        });

    let user_id = yaml
        .get("user_id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| {
            debug!(
                doc_id = %id,
                "knowledge doc missing user_id frontmatter; loading as \"legacy\" — \
                 this indicates a doc serialized before project_id/user_id were \
                 persisted and it will not be visible to owner-scoped queries"
            );
            "legacy".to_string()
        });

    Some(MemoryDoc {
        id: DocId(id),
        project_id,
        user_id,
        doc_type,
        title,
        content: body.to_string(),
        source_thread_id,
        tags,
        metadata,
        created_at,
        updated_at,
    })
}

// ── Thread archival ─────────────────────────────────────────

/// Compact summary of a completed thread for archival.
#[derive(serde::Serialize, serde::Deserialize)]
struct ThreadArchiveSummary {
    thread_id: String,
    goal: String,
    state: String,
    created_at: String,
    completed_at: Option<String>,
    step_count: usize,
    total_tokens: u64,
    #[serde(default)]
    outcome_preview: String,
    // `#[serde(default)]` lets summaries written before this field existed
    // continue to deserialize as zero rather than failing.
    #[serde(default)]
    total_cost_usd: f64,
}

fn compact_thread_summary(thread: &Thread) -> ThreadArchiveSummary {
    // Extract last assistant message as outcome preview
    let outcome = thread
        .messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, ironclaw_engine::MessageRole::Assistant))
        .map(|m| truncate_for_readme(&m.content, 200))
        .unwrap_or_default();

    ThreadArchiveSummary {
        thread_id: thread.id.0.to_string(),
        goal: truncate_for_readme(&thread.goal, 200),
        state: format!("{:?}", thread.state),
        created_at: thread.created_at.to_rfc3339(),
        completed_at: thread.completed_at.map(|dt| dt.to_rfc3339()),
        step_count: thread.step_count,
        total_tokens: thread.total_tokens_used,
        outcome_preview: outcome,
        total_cost_usd: thread.total_cost_usd,
    }
}

/// Reconstruct a minimal Thread from an archive summary (for mission detail pages).
fn thread_from_archive(summary: &ThreadArchiveSummary) -> Option<Thread> {
    let id = uuid::Uuid::parse_str(&summary.thread_id).ok()?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&summary.created_at)
        .ok()?
        .with_timezone(&chrono::Utc);
    let completed_at = summary
        .completed_at
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));
    let state = match summary.state.as_str() {
        "Done" => ThreadState::Done,
        "Failed" => ThreadState::Failed,
        "Completed" => ThreadState::Completed,
        _ => ThreadState::Done,
    };
    Some(Thread {
        id: ThreadId(id),
        goal: summary.goal.clone(),
        thread_type: ironclaw_engine::ThreadType::Mission,
        state,
        project_id: ironclaw_engine::ProjectId(uuid::Uuid::nil()),
        user_id: "default".to_string(),
        parent_id: None,
        config: ironclaw_engine::ThreadConfig::default(),
        messages: Vec::new(),
        internal_messages: Vec::new(),
        events: Vec::new(),
        capability_leases: Vec::new(),
        metadata: serde_json::Value::Object(serde_json::Map::new()),
        created_at,
        updated_at: completed_at.unwrap_or(created_at),
        completed_at,
        step_count: summary.step_count,
        total_tokens_used: summary.total_tokens,
        total_cost_usd: summary.total_cost_usd,
    })
}

fn truncate_for_readme(s: &str, max: usize) -> String {
    let trimmed = s.trim().replace('\n', " ");
    if trimmed.chars().count() <= max {
        trimmed
    } else {
        let truncated: String = trimmed.chars().take(max).collect();
        format!("{truncated}...")
    }
}

// ── Store trait implementation ───────────────────────────────

#[async_trait::async_trait]
impl Store for HybridStore {
    async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
        self.threads.write().await.insert(thread.id, thread.clone());
        self.persist_json(thread_path(thread.id), thread).await;
        Ok(())
    }

    async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
        // Fast path: check in-memory cache
        if let Some(thread) = self.threads.read().await.get(&id).cloned() {
            return Ok(Some(thread));
        }
        // Slow path: reload from database (thread may have been evicted from memory)
        if let Some(ws) = self.workspace.as_ref()
            && let Ok(doc) = ws.read(&thread_path(id)).await
            && let Ok(thread) = serde_json::from_str::<Thread>(&doc.content)
        {
            self.threads.write().await.insert(thread.id, thread.clone());
            return Ok(Some(thread));
        }
        Ok(None)
    }

    async fn list_threads(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<Thread>, EngineError> {
        Ok(self
            .threads
            .read()
            .await
            .values()
            .filter(|thread| thread.project_id == project_id && thread.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn update_thread_state(
        &self,
        id: ThreadId,
        state: ThreadState,
    ) -> Result<(), EngineError> {
        let updated = {
            let mut threads = self.threads.write().await;
            if let Some(thread) = threads.get_mut(&id) {
                thread.state = state;
                Some(thread.clone())
            } else {
                None
            }
        };
        if let Some(thread) = updated.as_ref() {
            self.persist_json(thread_path(id), thread).await;
        }
        Ok(())
    }

    async fn save_step(&self, step: &Step) -> Result<(), EngineError> {
        let snapshot = {
            let mut steps = self.steps.write().await;
            let thread_steps = steps.entry(step.thread_id).or_default();
            if let Some(existing) = thread_steps
                .iter_mut()
                .find(|existing| existing.id == step.id)
            {
                *existing = step.clone();
            } else {
                thread_steps.push(step.clone());
                thread_steps.sort_by_key(|saved| saved.sequence);
            }
            thread_steps.clone()
        };
        self.persist_json(step_path(step.thread_id), &snapshot)
            .await;
        Ok(())
    }

    async fn load_steps(&self, thread_id: ThreadId) -> Result<Vec<Step>, EngineError> {
        if let Some(steps) = self.steps.read().await.get(&thread_id).cloned() {
            return Ok(steps);
        }
        // Reload from database (may have been evicted from memory)
        if let Some(ws) = self.workspace.as_ref()
            && let Ok(doc) = ws.read(&step_path(thread_id)).await
            && let Ok(steps) = serde_json::from_str::<Vec<Step>>(&doc.content)
        {
            self.steps.write().await.insert(thread_id, steps.clone());
            return Ok(steps);
        }
        Ok(Vec::new())
    }

    async fn append_events(&self, events: &[ThreadEvent]) -> Result<(), EngineError> {
        let mut grouped: HashMap<ThreadId, Vec<ThreadEvent>> = HashMap::new();
        for event in events {
            grouped
                .entry(event.thread_id)
                .or_default()
                .push(event.clone());
        }

        for (thread_id, new_events) in grouped {
            let snapshot = {
                let mut stored = self.events.write().await;
                let thread_events = stored.entry(thread_id).or_default();
                for event in new_events {
                    if !thread_events.iter().any(|existing| existing.id == event.id) {
                        thread_events.push(event);
                    }
                }
                thread_events.sort_by_key(|event| event.timestamp);
                thread_events.clone()
            };
            self.persist_json(event_path(thread_id), &snapshot).await;
        }
        Ok(())
    }

    async fn load_events(&self, thread_id: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
        if let Some(events) = self.events.read().await.get(&thread_id).cloned() {
            return Ok(events);
        }
        // Reload from database (may have been evicted from memory)
        if let Some(ws) = self.workspace.as_ref()
            && let Ok(doc) = ws.read(&event_path(thread_id)).await
            && let Ok(events) = serde_json::from_str::<Vec<ThreadEvent>>(&doc.content)
        {
            self.events.write().await.insert(thread_id, events.clone());
            return Ok(events);
        }
        Ok(Vec::new())
    }

    async fn save_project(&self, project: &Project) -> Result<(), EngineError> {
        self.projects
            .write()
            .await
            .insert(project.id, project.clone());
        self.persist_json(project_path(&project.name, project.id), project)
            .await;
        Ok(())
    }

    async fn load_project(&self, id: ProjectId) -> Result<Option<Project>, EngineError> {
        Ok(self.projects.read().await.get(&id).cloned())
    }

    async fn list_projects(&self, user_id: &str) -> Result<Vec<Project>, EngineError> {
        Ok(self
            .projects
            .read()
            .await
            .values()
            .filter(|p| p.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn list_all_projects(&self) -> Result<Vec<Project>, EngineError> {
        Ok(self.projects.read().await.values().cloned().collect())
    }

    async fn save_conversation(
        &self,
        conversation: &ConversationSurface,
    ) -> Result<(), EngineError> {
        self.conversations
            .write()
            .await
            .insert(conversation.id, conversation.clone());
        self.persist_json(conversation_path(conversation.id), conversation)
            .await;
        Ok(())
    }

    async fn load_conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<ConversationSurface>, EngineError> {
        Ok(self.conversations.read().await.get(&id).cloned())
    }

    async fn list_conversations(
        &self,
        user_id: &str,
    ) -> Result<Vec<ConversationSurface>, EngineError> {
        Ok(self
            .conversations
            .read()
            .await
            .values()
            .filter(|conversation| conversation.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn save_memory_doc(&self, doc: &MemoryDoc) -> Result<(), EngineError> {
        // Defense-in-depth: gate orchestrator/prompt writes even if a caller
        // bypassed tool-level checks. The "trusted internal" exemption is
        // keyed off a tokio task-local flag set by `with_trusted_internal_writes`,
        // not off caller-supplied metadata or title — an LLM tool call cannot
        // enter that scope, so it cannot forge the system-internal marker.
        // The failure-tracker writes (`record_orchestrator_failure`,
        // `reset_orchestrator_failures`) enter the scope at the call site.
        if is_protected_orchestrator_doc(doc) {
            let trusted = ironclaw_engine::runtime::is_trusted_internal_write_active();

            // The failure tracker is a *system-internal* accounting doc — no
            // LLM-reachable code path should ever write it. Reject untrusted
            // writes to it regardless of self-modify state, so an attacker
            // can't manipulate the auto-rollback budget even when patching
            // is turned on.
            if !trusted && doc.title == ORCHESTRATOR_FAILURES_TITLE {
                return Err(EngineError::AccessDenied {
                    user_id: doc.user_id.clone(),
                    entity: format!("orchestrator doc '{}' (system-internal tracker)", doc.title),
                });
            }

            if !self_modify_enabled() {
                if !trusted {
                    return Err(EngineError::AccessDenied {
                        user_id: doc.user_id.clone(),
                        entity: format!(
                            "orchestrator doc '{}' (self-modification disabled)",
                            doc.title
                        ),
                    });
                }
            } else if !trusted {
                // Self-modification is enabled — validate untrusted (LLM-written)
                // patches before persisting so a broken patch doesn't consume
                // failure-budget slots (3 failures trigger auto-rollback).
                validate_orchestrator_content(doc)?;
            }
        }

        let mut stamped = doc.clone();
        // Normalize project_id for physically global docs so they surface
        // from any project's `list_shared_memory_docs` query immediately,
        // not just after restart where synthesize_orchestrator_doc_from_py
        // creates them with nil. Without this, a fresh seed from
        // MissionManager carries the writing project's id and is invisible
        // to other projects until the workspace is reloaded.
        if is_globally_shared(&stamped)
            && ironclaw_engine::types::is_shared_owner(&stamped.user_id)
            && !stamped.project_id.0.is_nil()
        {
            stamped.project_id = ProjectId(uuid::Uuid::nil());
        }
        // Stamp a content hash for audit trail on all protected docs. This
        // is **write-time only** — `load_knowledge_docs` does not verify
        // the hash on read because the workspace is the trust boundary
        // (anyone with workspace access can edit files directly). The hash
        // gives operators a "what did the LLM persist on this write" record
        // for incident review, not a runtime integrity guarantee.
        if is_protected_orchestrator_doc(doc) {
            use sha2::{Digest, Sha256};
            let hash = format!("{:x}", Sha256::digest(doc.content.as_bytes()));
            if !stamped.metadata.is_object() {
                stamped.metadata = serde_json::json!({});
            }
            if let Some(map) = stamped.metadata.as_object_mut() {
                map.insert("content_hash".into(), serde_json::Value::String(hash));
            }
        }

        self.docs.write().await.insert(stamped.id, stamped.clone());
        self.persist_doc(&stamped).await;
        Ok(())
    }

    async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
        Ok(self.docs.read().await.get(&id).cloned())
    }

    async fn list_memory_docs(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        Ok(self
            .docs
            .read()
            .await
            .values()
            .filter(|doc| doc.project_id == project_id && doc.user_id == user_id)
            .cloned()
            .collect())
    }

    /// List shared docs visible to *any* project query.
    ///
    /// The default trait impl filters by `(project_id, shared_owner)`. We
    /// override here so that **physically global** docs — orchestrator
    /// versions, the failure tracker, and prompt overlays, all of which
    /// live at one well-known workspace path regardless of project — also
    /// surface for any project that asks. Without this, orchestrator docs
    /// rehydrated from disk on restart (which use `ProjectId::nil()` as a
    /// global marker) would be invisible to project-scoped executor calls
    /// such as `load_orchestrator(project_id)`, and self-modify state
    /// would silently revert to compiled-in defaults.
    async fn list_shared_memory_docs(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        let docs = self.docs.read().await;
        let mut out: Vec<MemoryDoc> = docs
            .values()
            .filter(|doc| {
                let owner_shared = ironclaw_engine::types::is_shared_owner(&doc.user_id);
                if !owner_shared {
                    return false;
                }
                // Match the project filter, OR surface global docs (those
                // saved with `ProjectId::nil()`) for every project.
                doc.project_id == project_id
                    || (doc.project_id.0.is_nil() && is_globally_shared(doc))
            })
            .cloned()
            .collect();
        out.sort_by_key(|doc| doc.id.0);
        out.dedup_by_key(|doc| doc.id);
        Ok(out)
    }

    async fn list_memory_docs_by_owner(
        &self,
        user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        Ok(self
            .docs
            .read()
            .await
            .values()
            .filter(|doc| doc.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn save_lease(&self, lease: &CapabilityLease) -> Result<(), EngineError> {
        self.leases.write().await.insert(lease.id, lease.clone());
        self.persist_json(lease_path(lease.id), lease).await;
        Ok(())
    }

    async fn load_active_leases(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<CapabilityLease>, EngineError> {
        Ok(self
            .leases
            .read()
            .await
            .values()
            .filter(|lease| lease.thread_id == thread_id && lease.is_valid())
            .cloned()
            .collect())
    }

    async fn revoke_lease(&self, lease_id: LeaseId, _reason: &str) -> Result<(), EngineError> {
        let updated = {
            let mut leases = self.leases.write().await;
            if let Some(lease) = leases.get_mut(&lease_id) {
                lease.revoked = true;
                Some(lease.clone())
            } else {
                None
            }
        };
        if let Some(lease) = updated.as_ref() {
            self.persist_json(lease_path(lease_id), lease).await;
        }
        Ok(())
    }

    async fn save_mission(&self, mission: &Mission) -> Result<(), EngineError> {
        let proj_slug = self.project_slug(mission.project_id).await;
        self.missions
            .write()
            .await
            .insert(mission.id, mission.clone());
        self.persist_json(mission_path(&proj_slug, &mission.name, mission.id), mission)
            .await;
        Ok(())
    }

    async fn load_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError> {
        Ok(self.missions.read().await.get(&id).cloned())
    }

    async fn list_missions(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<Mission>, EngineError> {
        Ok(self
            .missions
            .read()
            .await
            .values()
            .filter(|mission| mission.project_id == project_id && mission.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn list_all_threads(&self, project_id: ProjectId) -> Result<Vec<Thread>, EngineError> {
        Ok(self
            .threads
            .read()
            .await
            .values()
            .filter(|thread| thread.project_id == project_id)
            .cloned()
            .collect())
    }

    async fn list_all_missions(&self, project_id: ProjectId) -> Result<Vec<Mission>, EngineError> {
        Ok(self
            .missions
            .read()
            .await
            .values()
            .filter(|mission| mission.project_id == project_id)
            .cloned()
            .collect())
    }

    async fn update_mission_status(
        &self,
        id: MissionId,
        status: MissionStatus,
    ) -> Result<(), EngineError> {
        let updated = {
            let mut missions = self.missions.write().await;
            if let Some(mission) = missions.get_mut(&id) {
                mission.status = status;
                mission.updated_at = chrono::Utc::now();
                Some(mission.clone())
            } else {
                None
            }
        };
        if let Some(mission) = updated.as_ref() {
            let proj_slug = self.project_slug(mission.project_id).await;
            self.persist_json(mission_path(&proj_slug, &mission.name, id), mission)
                .await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_engine::types::shared_owner_id;

    // ── normalize_path / is_orchestrator_code_path ─────────────

    #[test]
    fn normalize_strips_dot_and_double_slash() {
        assert_eq!(
            normalize_path(".system/engine/./orchestrator/v3.py").as_deref(),
            Some(".system/engine/orchestrator/v3.py")
        );
        assert_eq!(
            normalize_path(".system/engine//orchestrator//v3.py").as_deref(),
            Some(".system/engine/orchestrator/v3.py")
        );
    }

    #[test]
    fn normalize_rejects_traversal() {
        assert!(normalize_path(".system/engine/knowledge/../orchestrator/v3.py").is_none());
        assert!(normalize_path("../escape").is_none());
    }

    #[test]
    fn orchestrator_code_path_canonical() {
        assert!(is_orchestrator_code_path(
            ".system/engine/orchestrator/v3.py"
        ));
        assert!(is_orchestrator_code_path(
            ".system/engine/orchestrator/v0.py"
        ));
    }

    #[test]
    fn orchestrator_code_path_legacy() {
        assert!(is_orchestrator_code_path("engine/orchestrator/v3.py"));
    }

    #[test]
    fn orchestrator_code_path_blocks_dot_segment_bypass() {
        // The reviewer-flagged bypass: dot/double-slash segments resolved
        // to the protected location but the raw `starts_with` missed them,
        // letting the LLM persist a `.py` file that skipped syntax validation.
        assert!(is_orchestrator_code_path(
            ".system/engine/./orchestrator/v3.py"
        ));
        assert!(is_orchestrator_code_path(
            ".system/engine//orchestrator/v3.py"
        ));
        assert!(is_orchestrator_code_path("engine/./orchestrator/v3.py"));
    }

    #[test]
    fn orchestrator_code_path_rejects_traversal() {
        // Traversal attempts can't be a legitimate code path; conservatively
        // return false so the synthesis & write paths bail.
        assert!(!is_orchestrator_code_path(
            ".system/engine/knowledge/../orchestrator/v3.py"
        ));
        assert!(!is_orchestrator_code_path("../engine/orchestrator/v3.py"));
    }

    #[test]
    fn orchestrator_code_path_rejects_unrelated_paths() {
        assert!(!is_orchestrator_code_path(
            ".system/engine/orchestrator/v3.md"
        ));
        assert!(!is_orchestrator_code_path(
            ".system/engine/knowledge/notes/foo.py"
        ));
        assert!(!is_orchestrator_code_path(
            "engine_other/orchestrator/v3.py"
        ));
        assert!(!is_orchestrator_code_path(""));
    }

    /// Parity test: the store adapter's `normalize_path` and the memory
    /// tool's `normalize_workspace_path` both sit on the orchestrator
    /// self-modify security boundary. They are independent copies of the
    /// same normalization logic (one lives in the `ironclaw` bridge layer,
    /// the other in a tool module) — if they ever diverge, a path-traversal
    /// or dot-segment bypass reopens on one side.
    ///
    /// Reviewer concern (PR #1958 round 4): extract into a shared helper OR
    /// add a cross-check test. Shared extraction would pull the store
    /// adapter into the tools tree or vice versa; a parity test is the
    /// lighter, more local guard.
    #[test]
    fn normalize_path_parity_with_memory_tool() {
        use crate::tools::builtin::memory::normalize_workspace_path;

        // Canonical input set covering every transformation we care about:
        // pass-through, dot segments, double slashes, leading `./`, trailing
        // slash, traversal (must reject), bare `..`, empty input, logical
        // aliases (which are passed through unchanged).
        let cases = [
            "engine/orchestrator/v3.py",
            ".system/engine/orchestrator/v0.py",
            "engine/./orchestrator/v3.py",
            "engine//orchestrator//v3.py",
            "./engine/orchestrator/v3.py",
            "engine/orchestrator/",
            "engine/knowledge/../orchestrator/v3.py",
            "../escape",
            "..",
            "",
            "orchestrator:main",
            "prompt:codeact_preamble",
            "daily/2026-04-14.md",
        ];

        for input in cases {
            assert_eq!(
                normalize_path(input),
                normalize_workspace_path(input),
                "normalize_path and normalize_workspace_path must agree on {input:?}"
            );
        }
    }

    // ── synthesize_orchestrator_doc_from_py ────────────────────

    #[test]
    fn synthesize_extracts_version_from_filename() {
        let doc = synthesize_orchestrator_doc_from_py(
            ".system/engine/orchestrator/v7.py",
            "def run_loop(): pass\n",
        )
        .expect("synthesizes a doc");
        assert_eq!(doc.title, ORCHESTRATOR_MAIN_TITLE);
        assert_eq!(doc.user_id, shared_owner_id());
        assert!(doc.tags.contains(&ORCHESTRATOR_CODE_TAG.to_string()));
        assert_eq!(
            doc.metadata.get("version").and_then(|v| v.as_u64()),
            Some(7)
        );
        assert!(doc.content.contains("run_loop"));
    }

    #[test]
    fn synthesize_returns_none_for_non_orchestrator_paths() {
        assert!(synthesize_orchestrator_doc_from_py("MEMORY.md", "x").is_none());
        assert!(
            synthesize_orchestrator_doc_from_py(".system/engine/knowledge/foo.py", "x").is_none()
        );
        assert!(
            synthesize_orchestrator_doc_from_py(
                ".system/engine/orchestrator/missing-prefix.py",
                "x"
            )
            .is_none(),
        );
    }

    #[test]
    fn synthesize_returns_none_for_traversal() {
        // Traversal in the path should never produce a synthesized doc.
        assert!(
            synthesize_orchestrator_doc_from_py(
                ".system/engine/knowledge/../orchestrator/v3.py",
                "x"
            )
            .is_none()
        );
    }

    #[test]
    fn synthesize_uses_global_project_id_marker() {
        let doc = synthesize_orchestrator_doc_from_py(
            ".system/engine/orchestrator/v0.py",
            "def run_loop(): pass\n",
        )
        .expect("synthesizes a doc");
        assert!(
            doc.project_id.0.is_nil(),
            "synthesized doc must use the nil ProjectId global marker so the \
             list_shared_memory_docs override surfaces it for any project query"
        );
    }

    // ── validate_orchestrator_content ──────────────────────────

    #[test]
    fn validate_accepts_well_formed_python() {
        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId(uuid::Uuid::nil()),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: ORCHESTRATOR_MAIN_TITLE.to_string(),
            content: "def run_loop():\n    return 1\n".to_string(),
            source_thread_id: None,
            tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
            metadata: serde_json::json!({"version": 1}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert!(validate_orchestrator_content(&doc).is_ok());
    }

    #[test]
    fn validate_rejects_broken_python() {
        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId(uuid::Uuid::nil()),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: ORCHESTRATOR_MAIN_TITLE.to_string(),
            content: "def f(\n".to_string(),
            source_thread_id: None,
            tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
            metadata: serde_json::json!({"version": 1}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let err = validate_orchestrator_content(&doc).expect_err("broken Python should fail");
        let msg = format!("{err:?}");
        assert!(msg.contains("invalid Python") || msg.contains("syntax"));
    }

    #[test]
    fn validate_skips_failure_tracker_doc() {
        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId(uuid::Uuid::nil()),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            // The failure tracker is JSON, not Python — validator must skip it.
            title: ORCHESTRATOR_FAILURES_TITLE.to_string(),
            content: r#"{"version": 1, "count": 2}"#.to_string(),
            source_thread_id: None,
            tags: vec!["orchestrator_meta".to_string()],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert!(validate_orchestrator_content(&doc).is_ok());
    }

    #[test]
    fn validate_skips_non_orchestrator_titles() {
        // Prompt overlays are markdown — validator only runs on `orchestrator:*`.
        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId(uuid::Uuid::nil()),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: PREAMBLE_OVERLAY_TITLE.to_string(),
            content: "Some markdown overlay text.\n".to_string(),
            source_thread_id: None,
            tags: vec![],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert!(validate_orchestrator_content(&doc).is_ok());
    }

    // ── is_protected_orchestrator_doc / is_globally_shared ─────

    #[test]
    fn protected_doc_predicate_matches_titles() {
        let make = |title: &str| MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId(uuid::Uuid::nil()),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: title.to_string(),
            content: String::new(),
            source_thread_id: None,
            tags: vec![],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert!(is_protected_orchestrator_doc(&make("orchestrator:main")));
        assert!(is_protected_orchestrator_doc(&make(
            "orchestrator:failures"
        )));
        assert!(is_protected_orchestrator_doc(&make(
            "prompt:codeact_preamble"
        )));
        assert!(!is_protected_orchestrator_doc(&make("MEMORY.md")));
        assert!(!is_protected_orchestrator_doc(&make("daily/2026-04-11.md")));
    }

    #[test]
    fn global_doc_predicate_matches_titles() {
        let make = |title: &str| MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId(uuid::Uuid::nil()),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: title.to_string(),
            content: String::new(),
            source_thread_id: None,
            tags: vec![],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert!(is_globally_shared(&make(ORCHESTRATOR_MAIN_TITLE)));
        assert!(is_globally_shared(&make(ORCHESTRATOR_FAILURES_TITLE)));
        assert!(is_globally_shared(&make(PREAMBLE_OVERLAY_TITLE)));
        assert!(!is_globally_shared(&make("MEMORY.md")));
    }

    // ── HybridStore::list_shared_memory_docs override ──────────

    #[tokio::test]
    async fn list_shared_surfaces_global_orchestrator_for_any_project() {
        use ironclaw_engine::Store;

        let store = HybridStore::new(None);

        // Insert a global synthesized orchestrator doc (project_id::nil()).
        let synthesized = synthesize_orchestrator_doc_from_py(
            ".system/engine/orchestrator/v2.py",
            "def run_loop(): pass\n",
        )
        .expect("synthesize");
        store
            .docs
            .write()
            .await
            .insert(synthesized.id, synthesized.clone());

        // A query scoped to *any* concrete project must return it.
        let project = ProjectId::new();
        let docs = store
            .list_shared_memory_docs(project)
            .await
            .expect("list shared");
        assert!(
            docs.iter().any(|d| d.id == synthesized.id),
            "global orchestrator must be visible from any project query"
        );
    }

    #[tokio::test]
    async fn list_shared_surfaces_project_scoped_orchestrator_for_matching_project() {
        use ironclaw_engine::Store;

        let store = HybridStore::new(None);
        let project = ProjectId::new();

        // A non-global orchestrator written under a specific project must
        // appear for that project but NOT for unrelated ones.
        let scoped = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: project,
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: ORCHESTRATOR_MAIN_TITLE.to_string(),
            content: "def run_loop(): pass\n".to_string(),
            source_thread_id: None,
            tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
            metadata: serde_json::json!({"version": 1}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        store.docs.write().await.insert(scoped.id, scoped.clone());

        let matching = store.list_shared_memory_docs(project).await.unwrap();
        assert!(matching.iter().any(|d| d.id == scoped.id));

        let other_project = ProjectId::new();
        let other = store.list_shared_memory_docs(other_project).await.unwrap();
        assert!(
            !other.iter().any(|d| d.id == scoped.id),
            "project-scoped orchestrator must not leak to unrelated projects"
        );
    }

    // ── save_memory_doc gate (forgeable metadata regression) ───

    #[tokio::test]
    async fn save_rejects_llm_orchestrator_when_self_modify_disabled() {
        use ironclaw_engine::Store;

        let _guard = ironclaw_engine::runtime::SelfModifyTestGuard::disable();
        let store = HybridStore::new(None);

        // The reviewer-flagged forgeable bypass: previously, an LLM-authored
        // doc with `metadata.source = "compiled_in"` was treated as system
        // internal. The new gate keys off the trusted-write task-local —
        // since this call is NOT inside `with_trusted_internal_writes`,
        // the metadata claim must NOT bypass the gate.
        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId::new(),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: ORCHESTRATOR_MAIN_TITLE.to_string(),
            content: "def run_loop(): pass\n".to_string(),
            source_thread_id: None,
            tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
            metadata: serde_json::json!({"version": 5, "source": "compiled_in"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let err = store
            .save_memory_doc(&doc)
            .await
            .expect_err("forgeable metadata must not bypass the self-modify gate");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("self-modification disabled"),
            "expected denial reason; got: {msg}"
        );
    }

    #[tokio::test]
    async fn save_allows_trusted_seed_when_self_modify_disabled() {
        use ironclaw_engine::Store;
        use ironclaw_engine::runtime::with_trusted_internal_writes;

        let _guard = ironclaw_engine::runtime::SelfModifyTestGuard::disable();
        let store = HybridStore::new(None);

        // System-internal seed inside the trusted-write scope must succeed
        // even when self-modify is off (this is exactly what
        // MissionManager::seed_orchestrator_v0 does at bootstrap).
        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId::new(),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: ORCHESTRATOR_MAIN_TITLE.to_string(),
            content: "def run_loop(): pass\n".to_string(),
            source_thread_id: None,
            tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
            metadata: serde_json::json!({"version": 0}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        with_trusted_internal_writes(store.save_memory_doc(&doc))
            .await
            .expect("trusted seed write must succeed");
    }

    #[tokio::test]
    async fn save_validates_python_when_self_modify_enabled() {
        use ironclaw_engine::Store;

        let _guard = ironclaw_engine::runtime::SelfModifyTestGuard::enable();
        let store = HybridStore::new(None);

        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId::new(),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: ORCHESTRATOR_MAIN_TITLE.to_string(),
            // Broken Python — validator must reject before persisting.
            content: "def f(\n".to_string(),
            source_thread_id: None,
            tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
            metadata: serde_json::json!({"version": 1}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let err = store
            .save_memory_doc(&doc)
            .await
            .expect_err("invalid Python must be rejected at write time");
        let msg = format!("{err:?}");
        assert!(msg.contains("invalid Python") || msg.contains("syntax"));
    }

    /// Regression test (PR #1958 round-4 review):
    ///
    /// Previously the self-modify gate contained a title-based exemption:
    /// `trusted = is_trusted_internal_write_active() || doc.title ==
    /// ORCHESTRATOR_FAILURES_TITLE`. Any code path that called
    /// `save_memory_doc` with that title — including LLM-reachable ones
    /// through the self-improvement mission — bypassed the gate and
    /// could corrupt the failure counter (which governs auto-rollback).
    /// The exemption now lives at the two real call sites inside
    /// `with_trusted_internal_writes`; untrusted writes to the failure
    /// tracker must be rejected.
    #[tokio::test]
    async fn save_rejects_untrusted_failure_tracker_writes() {
        use ironclaw_engine::Store;

        // Self-modify *enabled* is the interesting case — under the old
        // title-based exemption the write would bypass even the
        // `validate_orchestrator_content` step (the validator also skipped
        // the failures title). With the fix, untrusted callers are blocked
        // regardless of self-modify state.
        let _guard = ironclaw_engine::runtime::SelfModifyTestGuard::enable();
        let store = HybridStore::new(None);

        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId::new(),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: "orchestrator:failures".to_string(),
            content: r#"{"version":99,"count":0}"#.to_string(),
            source_thread_id: None,
            tags: vec![],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let err = store
            .save_memory_doc(&doc)
            .await
            .expect_err("untrusted failure-tracker write must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("system-internal tracker"),
            "expected system-internal denial; got: {msg}"
        );
    }

    /// The system-initiated failure-tracker writes from
    /// `record_orchestrator_failure` and `reset_orchestrator_failures` must
    /// still succeed — they enter the trusted-write scope at the call site.
    #[tokio::test]
    async fn save_allows_trusted_failure_tracker_writes() {
        use ironclaw_engine::Store;
        use ironclaw_engine::runtime::with_trusted_internal_writes;

        let _guard = ironclaw_engine::runtime::SelfModifyTestGuard::disable();
        let store = HybridStore::new(None);

        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId::new(),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: "orchestrator:failures".to_string(),
            content: r#"{"version":3,"count":1}"#.to_string(),
            source_thread_id: None,
            tags: vec![],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        with_trusted_internal_writes(store.save_memory_doc(&doc))
            .await
            .expect("trusted failure-tracker write must succeed");
    }

    #[tokio::test]
    async fn save_stamps_content_hash_on_protected_docs() {
        use ironclaw_engine::Store;

        let _guard = ironclaw_engine::runtime::SelfModifyTestGuard::enable();
        let store = HybridStore::new(None);

        let doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: ProjectId::new(),
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: ORCHESTRATOR_MAIN_TITLE.to_string(),
            content: "def run_loop(): return 1\n".to_string(),
            source_thread_id: None,
            tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
            metadata: serde_json::json!({"version": 1}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let id = doc.id;
        store.save_memory_doc(&doc).await.unwrap();

        let stored = store.docs.read().await.get(&id).cloned().expect("stored");
        let hash = stored
            .metadata
            .get("content_hash")
            .and_then(|v| v.as_str())
            .expect("content_hash stamped");
        // Sha256 hex digest is 64 chars
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

#[cfg(all(test, feature = "libsql"))]
mod migration_tests {
    use super::*;
    use crate::db::Database;
    use crate::db::libsql::LibSqlBackend;

    /// Build a fresh in-memory libsql-backed workspace for migration tests.
    async fn fresh_workspace() -> (Arc<Workspace>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");
        let db: Arc<dyn Database> = Arc::new(backend);
        let ws = Arc::new(Workspace::new_with_db("test_user", db));
        (ws, dir)
    }

    #[tokio::test]
    async fn legacy_engine_paths_are_migrated_to_system_engine() {
        // Regression: pre-#2049, engine state was persisted under `engine/...`.
        // Without an explicit migration the loaders only see `.system/engine/...`,
        // and existing deployments would silently lose their engine state on
        // upgrade. The HybridStore must rewrite legacy paths at startup.
        let (ws, _dir) = fresh_workspace().await;

        // Seed three legacy-shaped documents covering the directory roots
        // that the engine actually uses.
        ws.write("engine/projects/sample.json", r#"{"hello": "world"}"#)
            .await
            .expect("seed legacy projects file");
        ws.write(
            "engine/runtime/threads/active/abc.json",
            r#"{"thread": "data"}"#,
        )
        .await
        .expect("seed legacy thread file");
        ws.write("engine/orchestrator/v1.py", "# legacy orchestrator")
            .await
            .expect("seed legacy orchestrator file");

        // Run the migration.
        let store = HybridStore::new(Some(Arc::clone(&ws)));
        store.migrate_legacy_engine_paths(&ws).await;

        // Files should now exist under the new prefix with the same content.
        let new_proj = ws
            .read(".system/engine/projects/sample.json")
            .await
            .expect("new projects file");
        assert_eq!(new_proj.content, r#"{"hello": "world"}"#);
        let new_thread = ws
            .read(".system/engine/runtime/threads/active/abc.json")
            .await
            .expect("new thread file");
        assert_eq!(new_thread.content, r#"{"thread": "data"}"#);
        let new_orch = ws
            .read(".system/engine/orchestrator/v1.py")
            .await
            .expect("new orchestrator file");
        assert_eq!(new_orch.content, "# legacy orchestrator");

        // Old paths should be gone (delete may leave the doc with empty
        // content depending on storage; either is acceptable, but
        // exists() must return false).
        assert!(
            !ws.exists("engine/projects/sample.json")
                .await
                .unwrap_or(true),
            "legacy projects file should be removed"
        );
        assert!(
            !ws.exists("engine/runtime/threads/active/abc.json")
                .await
                .unwrap_or(true),
            "legacy thread file should be removed"
        );
        assert!(
            !ws.exists("engine/orchestrator/v1.py").await.unwrap_or(true),
            "legacy orchestrator file should be removed"
        );

        // Idempotent — running again on a clean workspace must not panic
        // and must not resurrect anything.
        store.migrate_legacy_engine_paths(&ws).await;
    }

    #[tokio::test]
    async fn migration_skips_when_target_already_present() {
        // If a partial previous migration left the new path populated, the
        // migrator must NOT overwrite it — but it should still delete the
        // legacy duplicate so it doesn't keep showing up.
        let (ws, _dir) = fresh_workspace().await;

        ws.write(".system/engine/projects/x.json", "new content")
            .await
            .expect("seed new path");
        ws.write("engine/projects/x.json", "stale legacy content")
            .await
            .expect("seed legacy path");

        let store = HybridStore::new(Some(Arc::clone(&ws)));
        store.migrate_legacy_engine_paths(&ws).await;

        // New path content preserved.
        let new = ws
            .read(".system/engine/projects/x.json")
            .await
            .expect("new path readable");
        assert_eq!(new.content, "new content");
        // Legacy duplicate cleared.
        assert!(
            !ws.exists("engine/projects/x.json").await.unwrap_or(true),
            "legacy duplicate should be removed even when new path exists"
        );
    }

    #[tokio::test]
    async fn migration_preserves_document_metadata() {
        // Regression: the migration originally only copied `doc.content`
        // and silently dropped the `metadata` column. Custom metadata
        // (e.g. schema, skip_indexing, hygiene flags) must survive the
        // engine/ → .system/engine/ rewrite.
        let (ws, _dir) = fresh_workspace().await;

        let original = ws
            .write("engine/projects/with_meta.json", r#"{"hello": "world"}"#)
            .await
            .expect("seed legacy doc");
        let metadata = serde_json::json!({
            "skip_indexing": true,
            "skip_versioning": false,
            "custom_marker": "engine-state"
        });
        ws.update_metadata(original.id, &metadata)
            .await
            .expect("seed metadata");

        let store = HybridStore::new(Some(Arc::clone(&ws)));
        store.migrate_legacy_engine_paths(&ws).await;

        let migrated = ws
            .read(".system/engine/projects/with_meta.json")
            .await
            .expect("migrated doc readable");
        assert_eq!(migrated.content, r#"{"hello": "world"}"#);
        // Metadata should be carried forward verbatim.
        assert_eq!(
            migrated
                .metadata
                .get("skip_indexing")
                .and_then(|v| v.as_bool()),
            Some(true),
            "skip_indexing should be preserved: {:?}",
            migrated.metadata
        );
        assert_eq!(
            migrated
                .metadata
                .get("custom_marker")
                .and_then(|v| v.as_str()),
            Some("engine-state"),
            "custom metadata fields should be preserved: {:?}",
            migrated.metadata
        );
    }

    #[tokio::test]
    async fn migration_preflight_skips_full_scan_when_no_legacy_paths() {
        // The cheap preflight is the dominant code path on every
        // post-migration startup. We can't easily measure that
        // `list_all` was skipped from outside, but we can at least pin
        // down the observable behavior: a workspace with zero legacy
        // paths must produce zero migrations and zero failures, and
        // unrelated content must remain untouched.
        let (ws, _dir) = fresh_workspace().await;
        ws.write("notes/clean.md", "untouched")
            .await
            .expect("seed unrelated doc");
        ws.write(".system/engine/projects/already.json", "{}")
            .await
            .expect("seed already-migrated doc");

        let store = HybridStore::new(Some(Arc::clone(&ws)));
        store.migrate_legacy_engine_paths(&ws).await;

        // Unrelated doc untouched.
        let unrelated = ws.read("notes/clean.md").await.expect("unrelated readable");
        assert_eq!(unrelated.content, "untouched");
        // Already-migrated doc untouched.
        let already = ws
            .read(".system/engine/projects/already.json")
            .await
            .expect("already-migrated readable");
        assert_eq!(already.content, "{}");
    }

    #[tokio::test]
    async fn migration_is_noop_with_no_legacy_paths() {
        let (ws, _dir) = fresh_workspace().await;
        // Only `.system/engine/...` content present.
        ws.write(".system/engine/projects/clean.json", "ok")
            .await
            .expect("seed");

        let store = HybridStore::new(Some(Arc::clone(&ws)));
        store.migrate_legacy_engine_paths(&ws).await;

        // Untouched.
        let doc = ws
            .read(".system/engine/projects/clean.json")
            .await
            .expect("readable");
        assert_eq!(doc.content, "ok");
    }

    // ── Orchestrator persistence end-to-end ────────────────────
    //
    // These tests simulate the full save → restart → load → query path
    // for orchestrator code persisted as raw `.py`. They were the focus
    // of three high-severity reviewer findings on PR #1958:
    //
    // 1. The synthesized doc used `ProjectId::nil()` so project-scoped
    //    queries returned nothing after restart (orchestrator silently
    //    reverted to compiled-in defaults).
    // 2. The path normalization bypass let an attacker persist a `.py`
    //    file outside the load path, so the runtime + audit picture
    //    diverged from disk.
    // 3. Frontmatter `project_id` / `user_id` were dropped on
    //    deserialization, so .md docs disappeared after restart.

    #[tokio::test]
    async fn orchestrator_py_round_trips_through_restart() {
        use ironclaw_engine::Store;
        use ironclaw_engine::runtime::with_trusted_internal_writes;
        use ironclaw_engine::types::shared_owner_id;

        let _guard = ironclaw_engine::runtime::SelfModifyTestGuard::enable();
        let (ws, _dir) = fresh_workspace().await;

        // ── Phase 1: write an orchestrator v3 through the trusted path
        let project = ProjectId::new();
        let store_a = HybridStore::new(Some(Arc::clone(&ws)));
        let mut doc = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: project,
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: ORCHESTRATOR_MAIN_TITLE.to_string(),
            content: "def run_loop():\n    return 42\n".to_string(),
            source_thread_id: None,
            tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
            metadata: serde_json::json!({"version": 3}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        // Use a non-trusted save path: self_modify is enabled, so the
        // store gate validates content but does not require the trusted
        // scope. (`with_trusted_internal_writes` is also fine here; either
        // codepath should round-trip.)
        store_a.save_memory_doc(&doc).await.expect("write v3");

        // The physical file should now exist as raw .py
        let on_disk = ws
            .read(".system/engine/orchestrator/v3.py")
            .await
            .expect("v3.py persisted");
        assert!(
            on_disk.content.contains("return 42"),
            "raw Python content must round-trip to disk"
        );

        // ── Phase 2: simulate process restart with a fresh HybridStore
        let store_b = HybridStore::new(Some(Arc::clone(&ws)));
        store_b.load_state_from_workspace().await;

        // The orchestrator must be visible to a project-scoped shared query
        // — even when the synthesized doc was saved with ProjectId::nil().
        let docs = store_b
            .list_shared_memory_docs(project)
            .await
            .expect("list shared post-restart");
        let restored = docs
            .iter()
            .find(|d| {
                d.title == ORCHESTRATOR_MAIN_TITLE
                    && d.tags.contains(&ORCHESTRATOR_CODE_TAG.to_string())
                    && d.metadata.get("version").and_then(|v| v.as_u64()) == Some(3)
            })
            .expect("orchestrator v3 must be visible after restart");
        assert!(restored.content.contains("return 42"));

        // And it must be visible from an unrelated project too — the
        // global override surfaces it everywhere, so the executor's
        // load_orchestrator(any_project_id) call always finds the latest.
        let other_project = ProjectId::new();
        let other_docs = store_b
            .list_shared_memory_docs(other_project)
            .await
            .expect("list shared for other project");
        assert!(
            other_docs.iter().any(|d| {
                d.title == ORCHESTRATOR_MAIN_TITLE
                    && d.metadata.get("version").and_then(|v| v.as_u64()) == Some(3)
            }),
            "global orchestrator must be visible to ALL project queries after restart"
        );

        // ── Phase 3: also verify v0 trusted-seed bypass survives restart
        doc.id = DocId(uuid::Uuid::new_v4());
        doc.metadata = serde_json::json!({"version": 0});
        doc.content = "def run_loop():\n    return 0\n".to_string();
        with_trusted_internal_writes(store_a.save_memory_doc(&doc))
            .await
            .expect("write v0 via trusted path");

        let store_c = HybridStore::new(Some(Arc::clone(&ws)));
        store_c.load_state_from_workspace().await;
        let docs = store_c.list_shared_memory_docs(project).await.unwrap();
        let v0 = docs
            .iter()
            .find(|d| d.metadata.get("version").and_then(|v| v.as_u64()) == Some(0));
        assert!(v0.is_some(), "v0 must be visible after restart");
        let v3 = docs
            .iter()
            .find(|d| d.metadata.get("version").and_then(|v| v.as_u64()) == Some(3));
        assert!(v3.is_some(), "v3 must still be visible after restart");
    }

    #[tokio::test]
    async fn knowledge_md_doc_round_trips_project_id_and_user_id() {
        // Reviewer finding: `serialize_knowledge_doc` persisted neither
        // `project_id` nor `user_id`, so `deserialize_knowledge_doc`
        // restored them as `nil`/`"legacy"` and `list_memory_docs` (which
        // filters by exact pair) returned nothing after restart.
        use ironclaw_engine::Store;

        let _guard = ironclaw_engine::runtime::SelfModifyTestGuard::disable();
        let (ws, _dir) = fresh_workspace().await;

        let project = ProjectId::new();
        let user = "alice";

        let store_a = HybridStore::new(Some(Arc::clone(&ws)));
        let lesson = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: project,
            user_id: user.to_string(),
            doc_type: DocType::Lesson,
            title: "Always validate input".to_string(),
            content: "Lesson body.".to_string(),
            source_thread_id: None,
            tags: vec!["safety".to_string()],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        store_a.save_memory_doc(&lesson).await.expect("save lesson");

        // Restart and query: the doc must reappear under the same
        // (project, user) pair via list_memory_docs.
        let store_b = HybridStore::new(Some(Arc::clone(&ws)));
        store_b.load_state_from_workspace().await;
        let docs = store_b
            .list_memory_docs(project, user)
            .await
            .expect("list project-scoped");
        assert!(
            docs.iter().any(|d| d.title == "Always validate input"),
            "lesson must be visible to its original project + user pair after restart"
        );

        // Other (project, user) pair must NOT see it.
        let other_project = ProjectId::new();
        let docs_other = store_b
            .list_memory_docs(other_project, user)
            .await
            .expect("list other project");
        assert!(
            !docs_other
                .iter()
                .any(|d| d.title == "Always validate input")
        );
        let docs_other_user = store_b
            .list_memory_docs(project, "bob")
            .await
            .expect("list other user");
        assert!(
            !docs_other_user
                .iter()
                .any(|d| d.title == "Always validate input")
        );
    }

    #[tokio::test]
    async fn invalid_python_orchestrator_is_rejected_at_write_time() {
        // Reviewer finding: the validator must run *before* the doc is
        // persisted, otherwise broken patches consume failure-budget slots
        // (3 failures trigger auto-rollback) and corrupt the version chain.
        use ironclaw_engine::Store;
        use ironclaw_engine::types::shared_owner_id;

        let _guard = ironclaw_engine::runtime::SelfModifyTestGuard::enable();
        let (ws, _dir) = fresh_workspace().await;

        let store = HybridStore::new(Some(Arc::clone(&ws)));
        let project = ProjectId::new();
        let bad = MemoryDoc {
            id: DocId(uuid::Uuid::new_v4()),
            project_id: project,
            user_id: shared_owner_id().to_string(),
            doc_type: DocType::Note,
            title: ORCHESTRATOR_MAIN_TITLE.to_string(),
            content: "def f(\n".to_string(), // unclosed paren
            source_thread_id: None,
            tags: vec![ORCHESTRATOR_CODE_TAG.to_string()],
            metadata: serde_json::json!({"version": 1}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let err = store
            .save_memory_doc(&bad)
            .await
            .expect_err("invalid Python must be rejected");
        assert!(format!("{err:?}").contains("invalid Python"));

        // No file was persisted to disk.
        assert!(
            !ws.exists(".system/engine/orchestrator/v1.py")
                .await
                .unwrap_or(true),
            "rejected patch must not be persisted to disk"
        );
    }
}
