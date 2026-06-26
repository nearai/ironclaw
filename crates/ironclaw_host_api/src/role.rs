//! User authority vocabulary: [`UserRole`] and [`UserStatus`].
//!
//! Shared identity enums (alongside `UserId` / `TenantId`) so any crate that
//! has a `UserId` can reason about a user's role without depending on the
//! identity store. Wire-stable snake_case; the string forms match the
//! `users.role` / `users.status` columns and must not drift
//! (see `.claude/rules/types.md`).

use serde::{Deserialize, Serialize};

/// Role a user holds on its tenant. Privilege order, highest first:
/// `Owner > Admin > Member`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    /// Deployment/tenant owner — implies admin.
    Owner,
    /// Administrative privileges (user + capability management).
    Admin,
    /// Ordinary user. Least-privilege default for unknown/missing values.
    #[default]
    Member,
}

impl UserRole {
    /// Stable wire/DB string (matches the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    /// Parse a persisted role. Unknown or missing values fall back to the
    /// least-privilege [`UserRole::Member`] — never escalate on a bad value.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "owner" => Self::Owner,
            "admin" => Self::Admin,
            _ => Self::Member,
        }
    }

    /// `true` for roles with administrative privileges (`Owner` and `Admin`).
    pub fn is_admin(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// `true` only for the tenant owner.
    pub fn is_owner(self) -> bool {
        matches!(self, Self::Owner)
    }
}

/// Lifecycle status of a user. Persisted as the `users.status` column; the
/// string form matches the serde representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    /// Normal, usable account. Default for unknown/missing values.
    #[default]
    Active,
    /// Temporarily blocked by an admin; identity retained.
    Suspended,
    /// Deactivated; retained for audit, not usable.
    Deactivated,
}

impl UserStatus {
    /// Stable wire/DB string (matches the serde representation).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deactivated => "deactivated",
        }
    }

    /// Parse a persisted status. Unknown or missing values fall back to
    /// [`UserStatus::Active`] (the DB-level default).
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "suspended" => Self::Suspended,
            "deactivated" => Self::Deactivated,
            _ => Self::Active,
        }
    }

    /// `true` for a normal, usable account.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[cfg(test)]
mod tests {
    use super::{UserRole, UserStatus};

    #[test]
    fn role_roundtrips_and_defaults_least_privilege() {
        for role in [UserRole::Owner, UserRole::Admin, UserRole::Member] {
            assert_eq!(UserRole::parse(role.as_str()), role);
        }
        assert_eq!(UserRole::parse("OWNER"), UserRole::Owner);
        assert_eq!(UserRole::parse("nonsense"), UserRole::Member);
        assert_eq!(UserRole::parse(""), UserRole::Member);
        assert_eq!(UserRole::default(), UserRole::Member);
    }

    #[test]
    fn is_admin_includes_owner() {
        assert!(UserRole::Owner.is_admin());
        assert!(UserRole::Admin.is_admin());
        assert!(!UserRole::Member.is_admin());
        assert!(UserRole::Owner.is_owner());
        assert!(!UserRole::Admin.is_owner());
    }

    #[test]
    fn status_roundtrips_and_defaults_active() {
        for status in [
            UserStatus::Active,
            UserStatus::Suspended,
            UserStatus::Deactivated,
        ] {
            assert_eq!(UserStatus::parse(status.as_str()), status);
        }
        assert_eq!(UserStatus::parse("unknown"), UserStatus::Active);
        assert_eq!(UserStatus::default(), UserStatus::Active);
        assert!(UserStatus::Active.is_active());
    }

    #[test]
    fn role_serde_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&UserRole::Member).expect("serialize"),
            "\"member\""
        );
        assert_eq!(
            serde_json::from_str::<UserRole>("\"owner\"").expect("deserialize"),
            UserRole::Owner
        );
    }
}
