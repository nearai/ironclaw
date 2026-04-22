//! Capability — the unit of effect.
//!
//! A capability bundles actions (tools), knowledge (skills), and policies
//! (hooks) into a single installable/activatable unit. Capabilities are
//! granted to threads via leases.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::types::thread::ThreadId;

// ── Granted actions ────────────────────────────────────────

/// Which actions a lease grants access to.
///
/// `All` means the lease covers every action in the capability (wildcard).
/// `Specific` restricts the lease to the listed action names.
///
/// Serializes as a JSON array for backward compatibility: `[]` = All,
/// `["a","b"]` = Specific.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantedActions {
    /// Wildcard — covers all actions in the capability.
    All,
    /// Restricted to specific action names.
    Specific(Vec<String>),
}

impl GrantedActions {
    /// Check whether a specific action is covered.
    pub fn covers(&self, action_name: &str) -> bool {
        let hyphenated = action_name.replace('_', "-");
        let underscored = action_name.replace('-', "_");
        match self {
            GrantedActions::All => true,
            GrantedActions::Specific(actions) => actions.iter().any(|action| {
                action == action_name || action == &hyphenated || action == &underscored
            }),
        }
    }

    /// Returns true if this is a wildcard grant.
    pub fn is_all(&self) -> bool {
        matches!(self, GrantedActions::All)
    }

    /// Returns the specific actions, or an empty slice for wildcard.
    pub fn actions(&self) -> &[String] {
        match self {
            GrantedActions::All => &[],
            GrantedActions::Specific(actions) => actions,
        }
    }
}

impl Serialize for GrantedActions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            GrantedActions::All => Vec::<String>::new().serialize(serializer),
            GrantedActions::Specific(v) => v.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for GrantedActions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = Vec::<String>::deserialize(deserializer)?;
        if v.is_empty() {
            Ok(GrantedActions::All)
        } else {
            Ok(GrantedActions::Specific(v))
        }
    }
}

/// Strongly-typed lease identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LeaseId(pub Uuid);

impl LeaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for LeaseId {
    fn default() -> Self {
        Self::new()
    }
}

// ── Effect types ────────────────────────────────────────────

/// Classification of side effects that an action may produce.
/// Used by the policy engine for allow/deny decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectType {
    /// Read from local filesystem or workspace.
    ReadLocal,
    /// Read from external APIs (no mutation).
    ReadExternal,
    /// Write to local filesystem or workspace.
    WriteLocal,
    /// Write to external services (create PR, send email).
    WriteExternal,
    /// Authenticated API call requiring credentials.
    CredentialedNetwork,
    /// Code execution or shell access.
    Compute,
    /// Financial operations (payments, transfers).
    Financial,
}

// ── Action definition ───────────────────────────────────────

/// Definition of a single action within a capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionDef {
    /// Action name (e.g. "create_issue", "web_fetch").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for parameters.
    pub parameters_schema: serde_json::Value,
    /// Effect types this action may produce.
    pub effects: Vec<EffectType>,
    /// Whether this action requires user approval before execution.
    pub requires_approval: bool,
}

// ── Capability ──────────────────────────────────────────────

/// A capability — bundles actions, knowledge, and policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Capability name (e.g. "github", "deployment").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Executable actions (replaces tools).
    pub actions: Vec<ActionDef>,
    /// Domain knowledge blocks (replaces skills).
    pub knowledge: Vec<String>,
    /// Policy rules (replaces hooks).
    pub policies: Vec<PolicyRule>,
}

// ── Policy ──────────────────────────────────────────────────

/// A named policy rule within a capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub name: String,
    pub condition: PolicyCondition,
    pub effect: PolicyEffect,
}

/// When a policy rule applies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyCondition {
    /// Always applies.
    Always,
    /// Applies when the action name exactly matches the pattern.
    ActionMatches { pattern: String },
    /// Applies when the action has a specific effect type.
    EffectTypeIs(EffectType),
}

/// What the policy engine decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyEffect {
    Allow,
    Deny,
    RequireApproval,
}

// ── Capability lease ────────────────────────────────────────

/// A time/use-limited grant of capability access to a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityLease {
    pub id: LeaseId,
    /// The thread this lease is granted to.
    pub thread_id: ThreadId,
    /// Which capability this lease covers.
    pub capability_name: String,
    /// Which actions from the capability are granted.
    pub granted_actions: GrantedActions,
    /// When the lease was granted.
    pub granted_at: DateTime<Utc>,
    /// When the lease expires (None = no expiry).
    pub expires_at: Option<DateTime<Utc>>,
    /// Maximum number of action invocations (None = unlimited).
    pub max_uses: Option<u32>,
    /// Remaining invocations (None = unlimited).
    pub uses_remaining: Option<u32>,
    /// Whether the lease has been explicitly revoked.
    pub revoked: bool,
    /// Why the lease was revoked (for audit trail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_reason: Option<String>,
}

impl CapabilityLease {
    /// Check whether this lease is currently valid.
    pub fn is_valid(&self) -> bool {
        if self.revoked {
            return false;
        }
        if let Some(expires_at) = self.expires_at
            && Utc::now() >= expires_at
        {
            return false;
        }
        if let Some(remaining) = self.uses_remaining
            && remaining == 0
        {
            return false;
        }
        true
    }

    /// Check whether a specific action is covered by this lease.
    pub fn covers_action(&self, action_name: &str) -> bool {
        self.granted_actions.covers(action_name)
    }

    /// Consume one use of this lease. Returns false if no uses remain.
    pub fn consume_use(&mut self) -> bool {
        if let Some(ref mut remaining) = self.uses_remaining {
            if *remaining == 0 {
                return false;
            }
            *remaining -= 1;
        }
        true
    }

    /// Refund one previously consumed use when execution was interrupted
    /// before the action actually completed.
    pub fn refund_use(&mut self) {
        if let (Some(max_uses), Some(remaining)) = (self.max_uses, self.uses_remaining.as_mut())
            && *remaining < max_uses
        {
            *remaining += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lease() -> CapabilityLease {
        CapabilityLease {
            id: LeaseId::new(),
            thread_id: ThreadId::new(),
            capability_name: "test".into(),
            granted_actions: GrantedActions::All,
            granted_at: Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        }
    }

    #[test]
    fn valid_lease() {
        let lease = make_lease();
        assert!(lease.is_valid());
    }

    #[test]
    fn revoked_lease_is_invalid() {
        let mut lease = make_lease();
        lease.revoked = true;
        assert!(!lease.is_valid());
    }

    #[test]
    fn expired_lease_is_invalid() {
        let mut lease = make_lease();
        lease.expires_at = Some(Utc::now() - chrono::Duration::seconds(10));
        assert!(!lease.is_valid());
    }

    #[test]
    fn exhausted_lease_is_invalid() {
        let mut lease = make_lease();
        lease.max_uses = Some(1);
        lease.uses_remaining = Some(0);
        assert!(!lease.is_valid());
    }

    #[test]
    fn consume_use_decrements() {
        let mut lease = make_lease();
        lease.max_uses = Some(2);
        lease.uses_remaining = Some(2);
        assert!(lease.consume_use());
        assert_eq!(lease.uses_remaining, Some(1));
        assert!(lease.consume_use());
        assert_eq!(lease.uses_remaining, Some(0));
        assert!(!lease.consume_use());
    }

    #[test]
    fn unlimited_consume_always_succeeds() {
        let mut lease = make_lease();
        for _ in 0..100 {
            assert!(lease.consume_use());
        }
    }

    #[test]
    fn refund_use_restores_budget_up_to_max() {
        let mut lease = make_lease();
        lease.max_uses = Some(2);
        lease.uses_remaining = Some(2);
        assert!(lease.consume_use());
        assert_eq!(lease.uses_remaining, Some(1));
        lease.refund_use();
        assert_eq!(lease.uses_remaining, Some(2));
        lease.refund_use();
        assert_eq!(lease.uses_remaining, Some(2));
    }

    #[test]
    fn covers_action_empty_grants_all() {
        let lease = make_lease();
        assert!(lease.covers_action("anything"));
    }

    #[test]
    fn covers_action_with_specific_grants() {
        let mut lease = make_lease();
        lease.granted_actions =
            GrantedActions::Specific(vec!["create_issue".into(), "list_prs".into()]);
        assert!(lease.covers_action("create_issue"));
        assert!(lease.covers_action("list_prs"));
        assert!(!lease.covers_action("delete_repo"));
    }

    #[test]
    fn covers_action_matches_hyphen_underscore_aliases() {
        let mut lease = make_lease();
        lease.granted_actions = GrantedActions::Specific(vec!["create_issue".into()]);
        assert!(lease.covers_action("create-issue"));

        lease.granted_actions = GrantedActions::Specific(vec!["list-prs".into()]);
        assert!(lease.covers_action("list_prs"));
    }
}
