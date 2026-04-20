//! Project — the unit of context.
//!
//! A project is a persistent domain of work that scopes memory documents,
//! threads, and missions. Examples: "IronClaw architecture", "deployment system".

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{OwnerId, default_user_id};

/// A tracked metric within a project.
///
/// Metrics connect project goals to measurable numbers. The `evaluation` field
/// tells the agent *how* to obtain the current value (e.g., an API call, a shell
/// command, a file to read).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetric {
    /// Human-readable metric name (e.g., "Monthly Revenue").
    pub name: String,
    /// Unit of measurement (e.g., "USD", "users", "%").
    #[serde(default)]
    pub unit: String,
    /// Target value to reach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<f64>,
    /// Current measured value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<f64>,
    /// How to measure this metric — instructions the agent follows to obtain
    /// the current value (e.g., "Query Stripe API /v1/balance", "Run `wc -l`
    /// on the user database", "Read projects/acme/kpis.json").
    #[serde(default)]
    pub evaluation: String,
    /// When the `current` value was last updated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

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
    /// Top-line goals for this project (human-defined, agent can suggest).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<String>,
    /// Tracked metrics with evaluation instructions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<ProjectMetric>,
    pub metadata: serde_json::Value,
    /// Optional override for the host-filesystem directory bound into this
    /// project's sandbox at `/project/`. When `None`, the host computes a
    /// default path (see the bridge's `project_workspace_path` helper). The
    /// engine crate intentionally stores only the override and not the
    /// resolved default, because resolving the default depends on the host's
    /// base directory (`~/.ironclaw`) which lives outside this crate.
    #[serde(default)]
    pub workspace_path: Option<PathBuf>,
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
            goals: Vec::new(),
            metrics: Vec::new(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            workspace_path: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Set an explicit host-filesystem path for this project's `/project/`
    /// mount, returning `self` for chaining at construction sites.
    pub fn with_workspace_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace_path = Some(path.into());
        self
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

        // Pin the derived UUID for a known input so any drift in
        // `ProjectId::from_slug` (e.g. changing the seed format) is a
        // compile-checkable failure, not a silent re-ID of every
        // workspace-backed project in production.
        assert_eq!(
            Project::new("user-1", "commitments", "").id.0.to_string(),
            "aa38ce02-4359-5fa1-9d8b-efa8b573a353"
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
