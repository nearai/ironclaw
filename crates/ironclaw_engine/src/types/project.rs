//! Project — the unit of context.
//!
//! A project is a persistent domain of work that scopes memory documents,
//! threads, and missions. Examples: "IronClaw architecture", "deployment system".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{OwnerId, default_user_id};

/// Strongly-typed project identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub Uuid);

impl ProjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ProjectId {
    fn default() -> Self {
        Self::new()
    }
}

/// The global system project — a stable, well-known project where
/// admin-installed shared skills live. Using `Uuid::nil()` means it requires
/// no database row: every `Store` implementation can query it via the normal
/// `list_memory_docs(system_project_id(), user_id)` path.
///
/// Skills in this project are visible to all users regardless of which
/// per-user project their thread runs in.
pub fn system_project_id() -> ProjectId {
    ProjectId(Uuid::nil())
}

/// A project — the unit of context scoping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    /// Tenant isolation: the user who owns this project.
    #[serde(default = "default_user_id")]
    pub user_id: String,
    pub name: String,
    pub description: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Project {
    pub fn new(
        user_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: ProjectId::new(),
            user_id: user_id.into(),
            name: name.into(),
            description: description.into(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn owner_id(&self) -> OwnerId<'_> {
        OwnerId::from_user_id(&self.user_id)
    }

    pub fn is_owned_by(&self, user_id: &str) -> bool {
        self.owner_id().matches_user(user_id)
    }
}
