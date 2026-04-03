//! Centralized ownership types for IronClaw.
//!
//! `Identity` is the single struct that flows from the channel boundary through
//! every scope constructor and authorization check. `can_act_on` is the sole
//! place that decides whether an actor may mutate a resource.
//!
//! Known single-tenant assumptions still remain elsewhere in the app. In
//! particular, extension lifecycle/configuration, orchestrator secret injection,
//! some channel secret setup, and MCP session management still have owner-scoped
//! behavior that should not be mistaken for full multi-tenant isolation yet.
//! The ownership model here is the foundation for tightening those paths.

/// Typed wrapper over `users.id`. Replaces all raw `&str`/`String` user_id params.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct OwnerId(String);

impl OwnerId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for OwnerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for OwnerId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for OwnerId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// Role carried on every authenticated `Identity`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UserRole {
    Admin,
    Member,
}

/// Scope of a tool or skill. Extension point — nothing sets `Global` yet.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResourceScope {
    User,
    Global,
}

/// Single identity struct passed to every scope constructor and authorization check.
///
/// Constructed from the `OwnershipCache` at the channel boundary after resolving
/// `(channel, external_id)` → `OwnerId` + `UserRole`. Never constructed from
/// raw user-supplied strings at call sites.
#[derive(Debug, Clone)]
pub struct Identity {
    pub owner_id: OwnerId,
    pub role: UserRole,
}

impl Identity {
    pub fn new(owner_id: impl Into<OwnerId>, role: UserRole) -> Self {
        Self {
            owner_id: owner_id.into(),
            role,
        }
    }
}

/// Central authorization check: returns true if the actor owns the resource.
///
/// Ownership is strict equality — role has no effect here.
/// Admin-only operations are gated by a separate scope type; do not add
/// role-based bypasses to this function.
pub fn can_act_on(actor: &Identity, resource_owner: &OwnerId) -> bool {
    actor.owner_id == *resource_owner
}

pub mod cache;
pub use cache::OwnershipCache;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_can_act_on_own_resource() {
        let actor = Identity {
            owner_id: OwnerId::from("alice"),
            role: UserRole::Member,
        };
        assert!(can_act_on(&actor, &OwnerId::from("alice")));
    }

    #[test]
    fn test_cannot_act_on_others_resource() {
        let actor = Identity {
            owner_id: OwnerId::from("alice"),
            role: UserRole::Member,
        };
        assert!(!can_act_on(&actor, &OwnerId::from("bob")));
    }

    #[test]
    fn test_admin_cannot_act_on_others_resource() {
        // Admin role does NOT bypass ownership in can_act_on
        let actor = Identity {
            owner_id: OwnerId::from("alice"),
            role: UserRole::Admin,
        };
        assert!(!can_act_on(&actor, &OwnerId::from("bob")));
    }

    #[test]
    fn test_owner_id_display() {
        let id = OwnerId::from("alice");
        assert_eq!(id.to_string(), "alice");
        assert_eq!(id.as_str(), "alice");
    }

    #[test]
    fn test_owner_id_equality() {
        assert_eq!(OwnerId::from("alice"), OwnerId::from("alice"));
        assert_ne!(OwnerId::from("alice"), OwnerId::from("bob"));
    }

    #[test]
    fn test_owner_id_from_string() {
        let s = "henry".to_string();
        let id = OwnerId::from(s);
        assert_eq!(id.as_str(), "henry");
    }

    #[test]
    fn test_identity_new() {
        let id = Identity::new("alice", UserRole::Admin);
        assert_eq!(id.owner_id.as_str(), "alice");
        assert_eq!(id.role, UserRole::Admin);
    }
}
