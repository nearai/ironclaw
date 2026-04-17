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

/// Stable v5 namespace for project IDs derived from `(user_id, slug)`.
/// Burning this value means every user's project IDs would rotate, so it
/// must never change once shipped.
const PROJECT_ID_NAMESPACE: Uuid = uuid::uuid!("6f1f3c5a-4f2e-4ba4-9f3a-1c7e3c4f5a10");

impl ProjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Derive a stable project ID from `(user_id, slug)`. Same inputs produce
    /// the same ID forever, so writing `projects/<slug>/...` in workspace
    /// always resolves to the same project.
    pub fn from_slug(user_id: &str, slug: &str) -> Self {
        let seed = format!("{user_id}:{slug}");
        Self(Uuid::new_v5(&PROJECT_ID_NAMESPACE, seed.as_bytes()))
    }
}

impl Default for ProjectId {
    fn default() -> Self {
        Self::new()
    }
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
    /// Create a new project with a deterministic ID derived from
    /// `(user_id, slugify(name))`. Calling `Project::new` twice with the
    /// same inputs returns the same project ID, which is what makes
    /// workspace-backed project storage idempotent: writing
    /// `projects/<slug>/AGENTS.md` creates the same project every time.
    ///
    /// Callers that need a throwaway project with a random UUID (tests,
    /// synthetic fixtures) should construct the struct directly with
    /// `ProjectId::new()`.
    pub fn new(
        user_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let user_id = user_id.into();
        let name = name.into();
        let slug = crate::types::slugify_simple(&name);
        let now = Utc::now();
        Self {
            id: ProjectId::from_slug(&user_id, &slug),
            user_id,
            name,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_uuid_is_stable() {
        // Rotating this UUID would reassign every user's project IDs.
        // If this test fails, you changed PROJECT_ID_NAMESPACE — don't.
        assert_eq!(
            PROJECT_ID_NAMESPACE.to_string(),
            "6f1f3c5a-4f2e-4ba4-9f3a-1c7e3c4f5a10"
        );
    }

    #[test]
    fn deterministic_project_id() {
        let p1 = Project::new("user-1", "commitments", "");
        let p2 = Project::new("user-1", "commitments", "");
        assert_eq!(p1.id, p2.id);
        // Different user same slug -> different ID
        let p3 = Project::new("user-2", "commitments", "");
        assert_ne!(p1.id, p3.id);
    }
}
