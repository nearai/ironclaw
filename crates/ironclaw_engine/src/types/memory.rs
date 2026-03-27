//! Memory documents — the unit of durable knowledge.
//!
//! Memory docs are structured knowledge produced by reflection on completed
//! threads. They are project-scoped and used for context building (retrieval,
//! not replay of raw history).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::project::ProjectId;
use crate::types::thread::ThreadId;

/// Strongly-typed document identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocId(pub Uuid);

impl DocId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for DocId {
    fn default() -> Self {
        Self::new()
    }
}

/// The kind of knowledge a memory document captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocType {
    /// What a thread accomplished.
    Summary,
    /// Durable learning from experience.
    Lesson,
    /// Reusable multi-step procedure.
    Playbook,
    /// Detected problem for follow-up.
    Issue,
    /// Missing capability request.
    Spec,
    /// Working memory / scratch notes.
    Note,
    /// Reusable skill with activation metadata and optional code snippets.
    Skill,
}

/// A memory document — structured durable knowledge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDoc {
    pub id: DocId,
    pub project_id: ProjectId,
    pub doc_type: DocType,
    pub title: String,
    pub content: String,
    pub source_thread_id: Option<ThreadId>,
    pub tags: Vec<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MemoryDoc {
    pub fn new(
        project_id: ProjectId,
        doc_type: DocType,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: DocId::new(),
            project_id,
            doc_type,
            title: title.into(),
            content: content.into(),
            source_thread_id: None,
            tags: Vec::new(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_source_thread(mut self, thread_id: ThreadId) -> Self {
        self.source_thread_id = Some(thread_id);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}
