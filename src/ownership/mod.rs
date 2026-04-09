//! Centralized ownership types for IronClaw.
//!
//! `Identity` is the single struct that flows from the channel boundary through
//! every scope constructor and authorization check. The [`Owned`] trait provides
//! a uniform `is_owned_by(user_id)` check across all resource types.
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

impl UserRole {
    /// Parse a role string persisted in the users table.
    ///
    /// Unknown values are treated as `Member` for a safe, least-privilege
    /// fallback.
    pub fn from_db_role(role: &str) -> Self {
        if role.eq_ignore_ascii_case("admin") {
            Self::Admin
        } else {
            Self::Member
        }
    }
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

/// Trait for types that have a user owner.
///
/// Provides a uniform `is_owned_by(user_id)` check across all resource types
/// (jobs, routines, etc.). Engine types (Mission, Thread, Project) have their
/// own inherent `is_owned_by` that additionally handles shared ownership
/// (`__shared__`); those are left as-is.
///
/// **Do NOT implement on engine types** (`Mission`, `Thread`, `Project`,
/// `MemoryDoc`). They have inherent `is_owned_by()` methods with
/// shared-ownership semantics that differ from this trait's default.
pub trait Owned {
    /// Returns the raw `user_id` string identifying the owner.
    fn owner_user_id(&self) -> &str;

    /// Returns true if `user_id` owns this resource.
    fn is_owned_by(&self, user_id: &str) -> bool {
        self.owner_user_id() == user_id
    }
}

pub mod cache;
pub use cache::OwnershipCache;

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_user_role_from_db_role() {
        assert_eq!(UserRole::from_db_role("admin"), UserRole::Admin);
        assert_eq!(UserRole::from_db_role("ADMIN"), UserRole::Admin);
        assert_eq!(UserRole::from_db_role("member"), UserRole::Member);
        assert_eq!(UserRole::from_db_role("owner"), UserRole::Member);
    }

    // --- Owned trait tests ---

    struct FakeResource {
        user_id: String,
    }

    impl Owned for FakeResource {
        fn owner_user_id(&self) -> &str {
            &self.user_id
        }
    }

    #[test]
    fn test_owned_is_owned_by_own_user() {
        let r = FakeResource {
            user_id: "alice".to_string(),
        };
        assert!(r.is_owned_by("alice"));
    }

    #[test]
    fn test_owned_is_not_owned_by_other_user() {
        let r = FakeResource {
            user_id: "alice".to_string(),
        };
        assert!(!r.is_owned_by("bob"));
    }

    #[test]
    fn test_owned_owner_user_id() {
        let r = FakeResource {
            user_id: "henry".to_string(),
        };
        assert_eq!(r.owner_user_id(), "henry");
    }
}
