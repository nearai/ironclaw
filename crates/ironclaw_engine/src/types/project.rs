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
    /// Project with deterministic ID from `(user_id, slugify(name))` —
    /// same inputs yield the same ID, so `memory_write` is idempotent.
    /// For a random-UUID throwaway (tests), build the struct directly.
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

    /// Every path that normalizes to the same slug must land on the
    /// same `ProjectId`. If this breaks, workspace paths stop round-tripping
    /// and auto-registered projects silently fork.
    #[test]
    fn slug_variants_collapse_to_one_project_id() {
        use crate::types::slugify_simple;
        let user = "user-1";
        let canonical = Project::new(user, "my-project", "").id;
        // Each input below must slugify to `my-project`.
        let variants = [
            "my-project",
            "My Project",
            "MY PROJECT",
            "  my-project  ",
            "my--project",
            "my___project",
            "my.project",
            "my/project",
            "my@project!",
            "-my-project-",
            "---my---project---",
            "My\tProject",
        ];
        for name in variants {
            assert_eq!(
                slugify_simple(name),
                "my-project",
                "slugify_simple({name:?}) should produce `my-project`"
            );
            assert_eq!(
                Project::new(user, name, "").id,
                canonical,
                "Project::new({name:?}) must share canonical ID"
            );
        }
    }

    /// Unicode names must slugify deterministically (folded to lowercase
    /// ASCII by dropping non-alphanumerics), not panic or silently drop
    /// characters in a way that changes the ID across platforms.
    #[test]
    fn unicode_names_slugify_consistently() {
        use crate::types::slugify_simple;
        // Non-ASCII letters become dashes (they are not ASCII alphanumeric).
        assert_eq!(slugify_simple("Café"), "caf");
        assert_eq!(slugify_simple("日本語"), "");
        assert_eq!(slugify_simple("emoji 🚀 project"), "emoji-project");
        // Accented but followed by ASCII — still deterministic.
        let a = Project::new("u", "Café au Lait", "").id;
        let b = Project::new("u", "Café au Lait", "").id;
        assert_eq!(a, b);
    }

    /// A name that slugifies to an empty string still produces a valid
    /// `ProjectId` — it shouldn't panic. Different empty-after-slugify
    /// names share the same ID (they all map to the empty-slug bucket).
    #[test]
    fn empty_slug_produces_stable_id() {
        use crate::types::slugify_simple;
        assert_eq!(slugify_simple(""), "");
        assert_eq!(slugify_simple("---"), "");
        assert_eq!(slugify_simple("!@#$%"), "");
        let a = Project::new("u", "", "").id;
        let b = Project::new("u", "---", "").id;
        let c = Project::new("u", "!@#$%", "").id;
        assert_eq!(a, b);
        assert_eq!(b, c);
        // But a different user under the same empty slug gets a different ID.
        let d = Project::new("other", "", "").id;
        assert_ne!(a, d);
    }

    /// `slugify_simple` must trim trailing dashes and collapse runs —
    /// it's the contract synth_bare_project and auto-registration both rely on.
    #[test]
    fn slugify_simple_normalizes_runs_and_edges() {
        use crate::types::slugify_simple;
        assert_eq!(slugify_simple("a"), "a");
        assert_eq!(slugify_simple("-a-"), "a");
        assert_eq!(slugify_simple("a-b"), "a-b");
        assert_eq!(slugify_simple("a   b"), "a-b");
        assert_eq!(slugify_simple("a!!!b"), "a-b");
        // Digits preserved.
        assert_eq!(slugify_simple("q4-2026-plan"), "q4-2026-plan");
        // Very long input — slug length not truncated here (that's
        // slugify()'s job, not slugify_simple's), but stays well-formed.
        let long = "a".repeat(500);
        assert_eq!(slugify_simple(&long), long);
    }
}
