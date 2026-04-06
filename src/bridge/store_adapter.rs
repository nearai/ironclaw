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
const PROJECTS_PREFIX: &str = ".system/engine/projects";

const THREADS_PREFIX: &str = ".system/engine/runtime/threads/active";
const THREAD_ARCHIVE_PREFIX: &str = ".system/engine/runtime/threads/archive";
const STEPS_PREFIX: &str = ".system/engine/runtime/steps";
const EVENTS_PREFIX: &str = ".system/engine/runtime/events";
const LEASES_PREFIX: &str = ".system/engine/runtime/leases";
const CONVERSATIONS_PREFIX: &str = ".system/engine/runtime/conversations";

// Well-known titles for special-case routing (must match engine crate constants)
const ORCHESTRATOR_MAIN_TITLE: &str = "orchestrator:main";
const ORCHESTRATOR_FAILURES_TITLE: &str = "orchestrator:failures";
const PREAMBLE_OVERLAY_TITLE: &str = "prompt:codeact_preamble";
const ORCHESTRATOR_CODE_TAG: &str = "orchestrator_code";
const FIX_PATTERN_TITLE: &str = "fix_pattern_database";

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
        // Knowledge docs can be .md (frontmatter) or .json (legacy).
        // Also load special docs from orchestrator/ and prompts/ paths.
        let search_prefixes = [KNOWLEDGE_PREFIX, ORCHESTRATOR_PREFIX];

        for prefix in search_prefixes {
            for entry in self
                .file_entries(ws, prefix, &[".md", ".json", ".py"])
                .await
            {
                match ws.read(&entry.path).await {
                    Ok(doc) => {
                        // Try frontmatter format first, then JSON
                        let parsed = deserialize_knowledge_doc(&doc.content)
                            .or_else(|| serde_json::from_str::<MemoryDoc>(&doc.content).ok());
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
                let short = &project_id.0.to_string()[..8];
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
        } else if new_path.ends_with(".md") {
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

fn is_orchestrator_code_path(path: &str) -> bool {
    path.starts_with(ORCHESTRATOR_PREFIX) && path.ends_with(".py")
}

/// Check if a MemoryDoc is a protected orchestrator or prompt overlay document.
fn is_protected_orchestrator_doc(doc: &MemoryDoc) -> bool {
    doc.title.starts_with("orchestrator:") || doc.title.starts_with("prompt:")
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

    // Truncate slug to 60 chars, append 8-char ID suffix
    let max_slug = 60;
    let truncated = if collapsed.len() > max_slug {
        // Don't cut in the middle of a word — find last dash before limit
        match collapsed[..max_slug].rfind('-') {
            Some(pos) if pos > 20 => &collapsed[..pos],
            _ => &collapsed[..max_slug],
        }
    } else {
        collapsed
    };

    let short_id = if id.len() >= 8 { &id[..8] } else { id };
    format!("{truncated}--{short_id}")
}

// ── Frontmatter serialization ───────────────────────────────

/// Serialize a MemoryDoc as YAML frontmatter + markdown content.
fn serialize_knowledge_doc(doc: &MemoryDoc) -> String {
    let mut frontmatter = String::from("---\n");
    frontmatter.push_str(&format!("id: \"{}\"\n", doc.id.0));
    frontmatter.push_str(&format!("doc_type: \"{:?}\"\n", doc.doc_type));
    frontmatter.push_str(&format!("title: \"{}\"\n", doc.title.replace('"', "\\\"")));
    if !doc.tags.is_empty() {
        frontmatter.push_str(&format!(
            "tags: [{}]\n",
            doc.tags
                .iter()
                .map(|t| format!("\"{t}\""))
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

    // project_id is not in frontmatter — use nil UUID (assigned at load time)
    Some(MemoryDoc {
        id: DocId(id),
        project_id: ProjectId(uuid::Uuid::nil()),
        user_id: "legacy".to_string(),
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
        total_cost_usd: 0.0,
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
        // Defense-in-depth: block orchestrator/prompt writes when self-modify is
        // disabled, even if the caller bypassed tool-level checks.
        if is_protected_orchestrator_doc(doc) {
            let allow = std::env::var("ORCHESTRATOR_SELF_MODIFY")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false);
            if !allow {
                // Allow system-internal writes (v0 seeding, failure tracking) but
                // block LLM-authored patches (version > 0, non-meta titles).
                let is_system_internal = doc.title == "orchestrator:failures"
                    || doc
                        .metadata
                        .get("source")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| s == "compiled_in");
                if !is_system_internal {
                    return Err(EngineError::AccessDenied {
                        user_id: doc.user_id.clone(),
                        entity: format!(
                            "orchestrator doc '{}' (self-modification disabled)",
                            doc.title
                        ),
                    });
                }
            }
        }

        self.docs.write().await.insert(doc.id, doc.clone());
        self.persist_doc(doc).await;
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
