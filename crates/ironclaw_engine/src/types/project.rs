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
