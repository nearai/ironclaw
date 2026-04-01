//! Capability — the unit of effect.
//!
//! A capability bundles actions (tools), knowledge (skills), and policies
//! (hooks) into a single installable/activatable unit. Capabilities are
//! granted to threads via leases.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::thread::ThreadId;

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
    /// Which actions from the capability are granted (empty = all).
    pub granted_actions: Vec<String>,
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
        self.granted_actions.is_empty() || self.granted_actions.iter().any(|a| a == action_name)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lease() -> CapabilityLease {
        CapabilityLease {
            id: LeaseId::new(),
            thread_id: ThreadId::new(),
            capability_name: "test".into(),
            granted_actions: vec![],
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
    fn covers_action_empty_grants_all() {
        let lease = make_lease();
        assert!(lease.covers_action("anything"));
    }

    #[test]
    fn covers_action_with_specific_grants() {
        let mut lease = make_lease();
        lease.granted_actions = vec!["create_issue".into(), "list_prs".into()];
        assert!(lease.covers_action("create_issue"));
        assert!(lease.covers_action("list_prs"));
        assert!(!lease.covers_action("delete_repo"));
    }
}
