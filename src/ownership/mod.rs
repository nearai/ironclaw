//! Centralized ownership types for IronClaw.
//!
//! [`UserId`] is the typed identifier carried from the channel boundary through
//! every scope constructor and authorization check. It collapses the previous
//! `OwnerId` + `Identity` split into a single value: a validated user id with
//! its attached [`UserRole`]. The [`Owned`] trait provides a uniform
//! `is_owned_by(user_id)` check across all resource types.
//!
//! # Type-safety invariants
//!
//! - No `From<String>` or `From<&str>` on [`UserId`]. Infallible conversion
//!   would silently bypass validation. Use [`UserId::new`] (validates) or
//!   [`UserId::from_trusted`] (documented opt-out for DB-sourced values).
//! - The string form only escapes through [`UserId::as_str`] / [`Display`] at
//!   explicit call sites.
//!
//! Known single-tenant assumptions still remain elsewhere in the app. In
//! particular, extension lifecycle/configuration, orchestrator secret injection,
//! some channel secret setup, and MCP session management still have owner-scoped
//! behavior that should not be mistaken for full multi-tenant isolation yet.
//! The ownership model here is the foundation for tightening those paths.

use std::fmt;

/// Role carried on every authenticated [`UserId`].
///
/// Three tiers:
///
/// - [`UserRole::Owner`] — deployment owner (super-admin). Implies admin.
/// - [`UserRole::Admin`] — administrative privileges (user management).
/// - [`UserRole::Regular`] — ordinary user. Default for safe, least-privilege
///   fallback when a DB value is missing or unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Owner,
    Admin,
    Regular,
}

impl UserRole {
    /// Parse a role string persisted in the users table.
    ///
    /// Unknown or missing values fall back to [`UserRole::Regular`] for a
    /// safe, least-privilege default. Historical `member` rows map to
    /// `Regular` transparently.
    pub fn from_db_role(role: &str) -> Self {
        match role.trim().to_ascii_lowercase().as_str() {
            "owner" => Self::Owner,
            "admin" => Self::Admin,
            // "member" (legacy) and any unknown value -> least-privilege default
            _ => Self::Regular,
        }
    }

    /// Returns the lowercase DB form of the role.
    pub fn as_db_role(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Regular => "regular",
        }
    }

    /// Returns `true` when the role has administrative privileges.
    /// Owners are admins too.
    pub fn is_admin(&self) -> bool {
        matches!(self, Self::Admin | Self::Owner)
    }

    /// Returns `true` for the deployment owner.
    pub fn is_owner(&self) -> bool {
        matches!(self, Self::Owner)
    }

    /// Returns `true` for ordinary (non-admin, non-owner) users.
    pub fn is_regular(&self) -> bool {
        matches!(self, Self::Regular)
    }

    /// Serde default when a persisted `UserId` is missing its role field.
    #[doc(hidden)]
    pub fn regular_default_if_missing() -> Self {
        Self::Regular
    }
}

/// Errors raised by validated [`UserId`] construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UserIdError {
    #[error("user id must not be empty")]
    Empty,
    #[error("user id must not be whitespace-only")]
    WhitespaceOnly,
}

/// Typed wrapper over `users.id` with its [`UserRole`].
///
/// Replaces all raw `&str`/`String` user_id params for values that flow
/// between internal modules. Constructed at the channel boundary via the
/// `OwnershipCache` after resolving `(channel, external_id)`, or via
/// [`UserId::from_trusted`] for DB-sourced values.
///
/// Deliberately omits `From<String>` / `From<&str>` so raw-string callers
/// must explicitly choose validation ([`UserId::new`]) or a documented
/// opt-out ([`UserId::from_trusted`]).
///
/// # Equality / hashing
///
/// `PartialEq`, `Eq`, and `Hash` are implemented manually over `id` only;
/// `role` is metadata that travels with the identity. Two `UserId` values
/// with the same `id` but different `role`s compare equal and hash
/// identically so that `HashMap`/`HashSet` lookups and cache keys remain
/// stable if the role is refreshed from the DB.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserId {
    id: String,
    #[serde(default = "UserRole::regular_default_if_missing")]
    role: UserRole,
}

impl PartialEq for UserId {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for UserId {}

impl std::hash::Hash for UserId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl UserId {
    /// Validated construction. Rejects empty or whitespace-only ids.
    pub fn new(id: impl AsRef<str>, role: UserRole) -> Result<Self, UserIdError> {
        let raw = id.as_ref();
        if raw.is_empty() {
            return Err(UserIdError::Empty);
        }
        if raw.trim().is_empty() {
            return Err(UserIdError::WhitespaceOnly);
        }
        Ok(Self {
            id: raw.to_string(),
            role,
        })
    }

    /// Opt-out for values sourced from a trusted upstream (DB row, registry
    /// entry, etc.) where the caller already trusts the shape.
    pub fn from_trusted(id: String, role: UserRole) -> Self {
        Self { id, role }
    }

    /// Borrow the raw user id string.
    pub fn as_str(&self) -> &str {
        &self.id
    }

    /// The attached role.
    pub fn role(&self) -> UserRole {
        self.role
    }

    pub fn is_owner(&self) -> bool {
        self.role.is_owner()
    }

    pub fn is_admin(&self) -> bool {
        self.role.is_admin()
    }

    pub fn is_regular(&self) -> bool {
        self.role.is_regular()
    }
}

impl fmt::Display for UserId {
    // Displays the id only. Role is intentionally not part of the Display
    // contract — logs/errors should carry the id, not the role.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id)
    }
}

impl TryFrom<(String, UserRole)> for UserId {
    type Error = UserIdError;

    fn try_from((id, role): (String, UserRole)) -> Result<Self, Self::Error> {
        Self::new(id, role)
    }
}

// Deliberately no `From<String>` / `From<&str>` / `Deref<Target = str>`.
// See module docs for rationale.

/// Scope of a tool or skill. Extension point — nothing sets `Global` yet.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResourceScope {
    User,
    Global,
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
    ///
    /// Returning `&str` here (rather than `&UserId`) is deliberate: the
    /// field on the impl side is almost always a raw DB column, and this is
    /// the leaf accessor — the boundary where the typed value terminates.
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
    fn user_id_new_rejects_empty() {
        assert_eq!(
            UserId::new("", UserRole::Regular).unwrap_err(),
            UserIdError::Empty
        );
    }

    #[test]
    fn user_id_new_rejects_whitespace_only() {
        assert_eq!(
            UserId::new("   \t\n", UserRole::Regular).unwrap_err(),
            UserIdError::WhitespaceOnly
        );
    }

    #[test]
    fn user_id_new_accepts_valid() {
        let id = UserId::new("alice", UserRole::Regular).unwrap();
        assert_eq!(id.as_str(), "alice");
        assert_eq!(id.role(), UserRole::Regular);
    }

    #[test]
    fn user_id_from_trusted_skips_validation() {
        // from_trusted is the documented opt-out. It does no validation.
        let id = UserId::from_trusted("".to_string(), UserRole::Regular);
        assert_eq!(id.as_str(), "");
    }

    #[test]
    fn user_id_display_shows_id_only() {
        let id = UserId::from_trusted("alice".into(), UserRole::Admin);
        assert_eq!(id.to_string(), "alice");
    }

    #[test]
    fn user_id_equality() {
        let a = UserId::from_trusted("alice".into(), UserRole::Regular);
        let b = UserId::from_trusted("alice".into(), UserRole::Regular);
        let c = UserId::from_trusted("bob".into(), UserRole::Regular);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn user_id_equality_ignores_role() {
        // Equality and hashing consider only the id. Role is metadata and
        // must not affect HashMap/HashSet lookup — otherwise a cache keyed
        // on UserId would miss when a role is refreshed from the DB.
        let regular = UserId::from_trusted("alice".into(), UserRole::Regular);
        let admin = UserId::from_trusted("alice".into(), UserRole::Admin);
        assert_eq!(regular, admin);
    }

    #[test]
    fn user_id_hashset_cross_role_membership() {
        use std::collections::HashSet;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Same id, different roles → same hash.
        let owner = UserId::new("alice", UserRole::Owner).unwrap();
        let regular = UserId::new("alice", UserRole::Regular).unwrap();
        assert_eq!(owner, regular);

        let mut h1 = DefaultHasher::new();
        let mut h2 = DefaultHasher::new();
        owner.hash(&mut h1);
        regular.hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());

        // A HashSet containing the Owner variant also "contains" the
        // Regular variant with the same id.
        let mut set: HashSet<UserId> = HashSet::new();
        set.insert(owner);
        assert!(set.contains(&regular));
    }

    #[test]
    fn user_id_try_from_tuple_validates() {
        assert!(UserId::try_from(("".to_string(), UserRole::Regular)).is_err());
        let id = UserId::try_from(("alice".to_string(), UserRole::Owner)).unwrap();
        assert!(id.is_owner());
    }

    #[test]
    fn user_id_role_predicates() {
        let owner = UserId::from_trusted("root".into(), UserRole::Owner);
        assert!(owner.is_owner());
        assert!(owner.is_admin()); // owner is admin too
        assert!(!owner.is_regular());

        let admin = UserId::from_trusted("a".into(), UserRole::Admin);
        assert!(!admin.is_owner());
        assert!(admin.is_admin());
        assert!(!admin.is_regular());

        let reg = UserId::from_trusted("r".into(), UserRole::Regular);
        assert!(!reg.is_owner());
        assert!(!reg.is_admin());
        assert!(reg.is_regular());
    }

    #[test]
    fn user_role_from_db_role_maps_three_variants() {
        assert_eq!(UserRole::from_db_role("owner"), UserRole::Owner);
        assert_eq!(UserRole::from_db_role("OWNER"), UserRole::Owner);
        assert_eq!(UserRole::from_db_role("admin"), UserRole::Admin);
        assert_eq!(UserRole::from_db_role("ADMIN"), UserRole::Admin);
        assert_eq!(UserRole::from_db_role("regular"), UserRole::Regular);
        // Legacy "member" and unknowns fall back to Regular.
        assert_eq!(UserRole::from_db_role("member"), UserRole::Regular);
        assert_eq!(UserRole::from_db_role("unknown"), UserRole::Regular);
        assert_eq!(UserRole::from_db_role(""), UserRole::Regular);
    }

    #[test]
    fn user_role_is_admin_includes_owner() {
        assert!(UserRole::Owner.is_admin());
        assert!(UserRole::Admin.is_admin());
        assert!(!UserRole::Regular.is_admin());
    }

    #[test]
    fn user_role_db_roundtrip() {
        for role in [UserRole::Owner, UserRole::Admin, UserRole::Regular] {
            assert_eq!(UserRole::from_db_role(role.as_db_role()), role);
        }
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
