//! The typed apply report: one [`Change`] per write the apply would make.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Per-domain reconcilers, named. Mirrors the typed Reborn repos a blueprint
/// reconciles into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    SystemPrompt,
    Providers,
    Runtime,
    AgentLoop,
    Extensions,
    Skills,
    Missions,
    Projects,
    CapabilitySurface,
    Harness,
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::SystemPrompt => "system_prompt",
            Self::Providers => "providers",
            Self::Runtime => "runtime",
            Self::AgentLoop => "agent_loop",
            Self::Extensions => "extensions",
            Self::Skills => "skills",
            Self::Missions => "missions",
            Self::Projects => "projects",
            Self::CapabilitySurface => "capability_surface",
            Self::Harness => "harness",
        };
        f.write_str(name)
    }
}

/// What a change does to one key in a domain repo.
///
/// `DeleteDeferred` records drift — a key present in the repo but absent from
/// the blueprint. Apply never performs the delete; it is opt-in via an explicit
/// admin `--prune`, per the epic's "do not delete user data on apply" rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Create,
    Update,
    NoOp,
    DeleteDeferred,
}

impl ChangeAction {
    /// Whether applying this change mutates the repo.
    pub fn is_write(self) -> bool {
        matches!(self, Self::Create | Self::Update)
    }
}

/// One planned change against a domain repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub domain: Domain,
    /// Stable key within the domain (e.g. a setting path, extension id).
    pub key: String,
    pub action: ChangeAction,
    /// SHA-256 of the repo's current value, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_hash: Option<String>,
    /// SHA-256 of the desired value, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_hash: Option<String>,
}

/// The result of an apply or dry-run: every change, in reconciler order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyReport {
    pub changes: Vec<Change>,
}

impl ApplyReport {
    pub fn push(&mut self, change: Change) {
        self.changes.push(change);
    }

    pub fn extend(&mut self, changes: impl IntoIterator<Item = Change>) {
        self.changes.extend(changes);
    }

    /// Count of changes that would mutate a repo (`Create`/`Update`).
    pub fn write_count(&self) -> usize {
        self.changes.iter().filter(|c| c.action.is_write()).count()
    }

    /// True when applying would change nothing — the idempotence signal.
    pub fn is_noop(&self) -> bool {
        self.write_count() == 0
    }

    /// Drift entries (keys present in the repo but absent from the blueprint).
    pub fn drift(&self) -> impl Iterator<Item = &Change> {
        self.changes
            .iter()
            .filter(|c| c.action == ChangeAction::DeleteDeferred)
    }
}
