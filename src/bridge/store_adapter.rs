//! Hybrid store adapter — workspace-backed persistence for engine state.
//!
//! Reflection docs, projects, threads, steps, events, leases, and missions are
//! cached in memory and mirrored to the workspace as JSON. This keeps the
//! engine restart-safe without introducing dedicated DB tables yet.

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

const ENGINE_DOCS_PREFIX: &str = "engine/docs";
const PROJECTS_PREFIX: &str = "engine/state/projects";
const CONVERSATIONS_PREFIX: &str = "engine/state/conversations";
const THREADS_PREFIX: &str = "engine/state/threads";
const STEPS_PREFIX: &str = "engine/state/steps";
const EVENTS_PREFIX: &str = "engine/state/events";
const LEASES_PREFIX: &str = "engine/state/leases";
const MISSIONS_PREFIX: &str = "engine/state/missions";

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
            workspace,
        }
    }

    /// Load persisted engine state from the workspace on startup.
    pub async fn load_state_from_workspace(&self) {
        let Some(ws) = self.workspace.as_ref() else {
            return;
        };

        self.load_docs(ws).await;
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
        self.load_map(ws, MISSIONS_PREFIX, |mission: Mission| async {
            self.missions.write().await.insert(mission.id, mission);
        })
        .await;

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

    async fn load_docs(&self, ws: &Workspace) {
        for entry in self.json_entries(ws, ENGINE_DOCS_PREFIX).await {
            match ws.read(&entry.path).await {
                Ok(doc) => match serde_json::from_str::<MemoryDoc>(&doc.content) {
                    Ok(memory_doc) => {
                        self.docs.write().await.insert(memory_doc.id, memory_doc);
                    }
                    Err(e) => debug!(path = %entry.path, "failed to parse engine doc: {e}"),
                },
                Err(e) => debug!(path = %entry.path, "failed to read engine doc: {e}"),
            }
        }
    }

    async fn load_map<T, F, Fut>(&self, ws: &Workspace, directory: &str, on_value: F)
    where
        T: DeserializeOwned,
        F: Fn(T) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        for entry in self.json_entries(ws, directory).await {
            match ws.read(&entry.path).await {
                Ok(doc) => match serde_json::from_str::<T>(&doc.content) {
                    Ok(value) => on_value(value).await,
                    Err(e) => debug!(path = %entry.path, "failed to parse engine state: {e}"),
                },
                Err(e) => debug!(path = %entry.path, "failed to read engine state: {e}"),
            }
        }
    }

    async fn json_entries(&self, ws: &Workspace, directory: &str) -> Vec<WorkspaceEntry> {
        let top = match ws.list(directory).await {
            Ok(entries) => entries,
            Err(_) => return Vec::new(),
        };

        let mut files = Vec::new();
        for entry in top {
            if entry.is_directory {
                if let Ok(children) = ws.list(&entry.path).await {
                    files.extend(
                        children
                            .into_iter()
                            .filter(|child| !child.is_directory && child.path.ends_with(".json")),
                    );
                }
            } else if entry.path.ends_with(".json") {
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
}

fn doc_workspace_path(doc: &MemoryDoc) -> String {
    let type_dir = match doc.doc_type {
        DocType::Summary => "summaries",
        DocType::Lesson => "lessons",
        DocType::Playbook => "playbooks",
        DocType::Issue => "issues",
        DocType::Spec => "specs",
        DocType::Note => "notes",
    };
    format!("{ENGINE_DOCS_PREFIX}/{type_dir}/{}.json", doc.id.0)
}

fn project_path(project_id: ProjectId) -> String {
    format!("{PROJECTS_PREFIX}/{}.json", project_id.0)
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

fn mission_path(mission_id: MissionId) -> String {
    format!("{MISSIONS_PREFIX}/{}.json", mission_id.0)
}

#[async_trait::async_trait]
impl Store for HybridStore {
    async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
        self.threads.write().await.insert(thread.id, thread.clone());
        self.persist_json(thread_path(thread.id), thread).await;
        Ok(())
    }

    async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
        Ok(self.threads.read().await.get(&id).cloned())
    }

    async fn list_threads(&self, project_id: ProjectId) -> Result<Vec<Thread>, EngineError> {
        Ok(self
            .threads
            .read()
            .await
            .values()
            .filter(|thread| thread.project_id == project_id)
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
        Ok(self
            .steps
            .read()
            .await
            .get(&thread_id)
            .cloned()
            .unwrap_or_default())
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
        Ok(self
            .events
            .read()
            .await
            .get(&thread_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn save_project(&self, project: &Project) -> Result<(), EngineError> {
        self.projects
            .write()
            .await
            .insert(project.id, project.clone());
        self.persist_json(project_path(project.id), project).await;
        Ok(())
    }

    async fn load_project(&self, id: ProjectId) -> Result<Option<Project>, EngineError> {
        Ok(self.projects.read().await.get(&id).cloned())
    }

    async fn list_projects(&self) -> Result<Vec<Project>, EngineError> {
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
        self.docs.write().await.insert(doc.id, doc.clone());
        self.persist_json(doc_workspace_path(doc), doc).await;
        Ok(())
    }

    async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
        Ok(self.docs.read().await.get(&id).cloned())
    }

    async fn list_memory_docs(&self, project_id: ProjectId) -> Result<Vec<MemoryDoc>, EngineError> {
        Ok(self
            .docs
            .read()
            .await
            .values()
            .filter(|doc| doc.project_id == project_id)
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
        self.missions
            .write()
            .await
            .insert(mission.id, mission.clone());
        self.persist_json(mission_path(mission.id), mission).await;
        Ok(())
    }

    async fn load_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError> {
        Ok(self.missions.read().await.get(&id).cloned())
    }

    async fn list_missions(&self, project_id: ProjectId) -> Result<Vec<Mission>, EngineError> {
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
            self.persist_json(mission_path(id), mission).await;
        }
        Ok(())
    }
}
