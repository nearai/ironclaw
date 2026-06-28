//! Multi-user capability policy: owner/admin/member roles, a persistent
//! user-directory port, and per-user capability grants (#5385).
//!
//! THE owner is established via env vars only (the `IRONCLAW_REBORN_WEBUI_USER_ID`
//! and `IRONCLAW_REBORN_WEBUI_TOKEN` pair). The owner is NEVER a directory row,
//! and REST mints no owners. Members are born default-DENY with an essential
//! capability allowlist; an admin GRANTS additional capabilities (an
//! `available` per-user delta) rather than hiding the rest.
//!
//! This crate owns only the port + the pure policy helpers; the
//! filesystem-backed adapter lives in host composition.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use ironclaw_host_api::{Timestamp, UserId, sha256_digest_token};
use serde::{Deserialize, Serialize};

/// Role rank, ascending: `Member < Admin < Owner`. The derived `Ord` follows
/// declaration order, so `>` is a true privilege comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Member,
    Admin,
    Owner,
}

impl UserRole {
    /// Owner and Admin may reach the admin command surface.
    pub fn is_admin(self) -> bool {
        matches!(self, UserRole::Admin | UserRole::Owner)
    }

    pub fn is_owner(self) -> bool {
        matches!(self, UserRole::Owner)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            UserRole::Member => "member",
            UserRole::Admin => "admin",
            UserRole::Owner => "owner",
        }
    }
}

/// Per-user availability delta over the role default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAvailability {
    Available,
    Hidden,
}

/// Essential capabilities every MEMBER is born with (default-DENY everything
/// else). Mirrors `ESSENTIAL_BUILTINS` in the xyzorg e2e validator; keep in
/// sync. `capability_info` is the always-present host meta-tool and is exempt
/// from the surface filter, so it is intentionally not listed here.
pub const ESSENTIAL_MEMBER_CAPABILITIES: &[&str] = &[
    "builtin.echo",
    "builtin.extension_activate",
    "builtin.extension_search",
    "builtin.json",
    "builtin.memory_read",
    "builtin.memory_search",
    "builtin.memory_write",
    "builtin.time",
];

/// A directory user. THE owner is env-configured and is never stored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDirectoryRecord {
    pub user_id: UserId,
    pub role: UserRole,
    /// `sha256(login_token)` hex. The raw token is returned once at creation
    /// and never stored.
    pub token_hash: String,
    /// Per-user capability availability deltas over the role default.
    #[serde(default)]
    pub grants: BTreeMap<String, CapabilityAvailability>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Backend errors for the user directory port. Kept coarse on purpose: the
/// facade maps these onto the sanitized `RebornServicesError` taxonomy.
#[derive(Debug, thiserror::Error)]
pub enum UserDirectoryError {
    #[error("user `{0}` already exists")]
    AlreadyExists(String),
    #[error("user `{0}` not found")]
    NotFound(String),
    #[error("user directory backend error: {0}")]
    Backend(String),
}

/// Persistent directory of users (roles + login-token hashes + per-user
/// capability grants). THE owner is NOT stored here. The adapter lives in host
/// composition (filesystem-backed); this crate owns only the port.
#[async_trait]
pub trait UserDirectoryStore: Send + Sync {
    async fn get(
        &self,
        user_id: &UserId,
    ) -> Result<Option<UserDirectoryRecord>, UserDirectoryError>;

    async fn list(&self) -> Result<Vec<UserDirectoryRecord>, UserDirectoryError>;

    async fn find_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<UserDirectoryRecord>, UserDirectoryError>;

    /// Insert a brand-new user. Errors [`UserDirectoryError::AlreadyExists`] if
    /// the id is taken.
    async fn insert(&self, record: UserDirectoryRecord) -> Result<(), UserDirectoryError>;

    /// Update an existing user's role. Errors [`UserDirectoryError::NotFound`]
    /// if absent.
    async fn set_role(
        &self,
        user_id: &UserId,
        role: UserRole,
    ) -> Result<UserDirectoryRecord, UserDirectoryError>;

    /// Set one per-user capability availability delta. Errors
    /// [`UserDirectoryError::NotFound`] if absent.
    async fn set_capability(
        &self,
        user_id: &UserId,
        capability_id: &str,
        availability: CapabilityAvailability,
    ) -> Result<(), UserDirectoryError>;

    /// Remove a user. Errors [`UserDirectoryError::NotFound`] if absent.
    async fn delete(&self, user_id: &UserId) -> Result<(), UserDirectoryError>;
}

/// Capabilities a MEMBER with these grants is allowed to use: the essential
/// baseline, plus any `Available` grant, minus any `Hidden` delta.
pub fn member_allowed_capability_ids(
    grants: &BTreeMap<String, CapabilityAvailability>,
) -> BTreeSet<String> {
    let mut allowed: BTreeSet<String> = ESSENTIAL_MEMBER_CAPABILITIES
        .iter()
        .map(|cap| (*cap).to_string())
        .collect();
    for (capability_id, availability) in grants {
        match availability {
            CapabilityAvailability::Available => {
                allowed.insert(capability_id.clone());
            }
            CapabilityAvailability::Hidden => {
                allowed.remove(capability_id);
            }
        }
    }
    allowed
}

/// Whether `capability_id` is AVAILABLE to a caller with this role + grants.
/// Owner/Admin see everything; members see the essential baseline plus their
/// `Available` grants.
pub fn capability_available(
    role: UserRole,
    grants: &BTreeMap<String, CapabilityAvailability>,
    capability_id: &str,
) -> bool {
    if role.is_admin() {
        return true;
    }
    match grants.get(capability_id) {
        Some(CapabilityAvailability::Available) => true,
        Some(CapabilityAvailability::Hidden) => false,
        None => ESSENTIAL_MEMBER_CAPABILITIES.contains(&capability_id),
    }
}

/// Syntactic capability-id validation: `provider.capability`, lowercase
/// `[a-z0-9_-]` segments separated by dots. Rejects bare labels like `gdrive`
/// (no provider dot) while accepting `builtin.shell`, `nearai.web_search`, and
/// `google-drive.list_files`. Deliberately does NOT check catalog membership:
/// granting a not-yet-installed capability is a valid policy delta (it becomes
/// reachable once the extension is discovered via `extension_search`).
pub fn is_valid_capability_id(capability_id: &str) -> bool {
    if capability_id.is_empty() || capability_id.len() > 128 {
        return false;
    }
    let Some((provider, capability)) = capability_id.split_once('.') else {
        return false;
    };
    let segment_ok = |segment: &str| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    };
    // The capability part may itself contain dots (e.g. `builtin.trace_commons.status`).
    segment_ok(provider) && capability.split('.').all(segment_ok)
}

/// Mint a fresh, high-entropy login bearer (shown once at user creation, then
/// stored only as a hash).
pub fn generate_login_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Hash a login bearer for storage / comparison. Single source of truth shared
/// by user creation (store the hash) and the authenticator (match a bearer).
pub fn hash_login_token(token: &str) -> String {
    sha256_digest_token(token.as_bytes())
}

// ---- admin REST DTOs -------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct AdminCreateUserRequest {
    pub user_id: String,
    pub role: UserRole,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminUserResponse {
    pub user_id: String,
    pub role: UserRole,
    /// Present only in the create response (the one-time login bearer).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminUserSummary {
    pub user_id: String,
    pub role: UserRole,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminListUsersResponse {
    pub users: Vec<AdminUserSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdminSetRoleRequest {
    pub role: UserRole,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdminSetCapabilityRequest {
    pub availability: CapabilityAvailability,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_rank_is_owner_gt_admin_gt_member() {
        assert!(UserRole::Owner > UserRole::Admin);
        assert!(UserRole::Admin > UserRole::Member);
        assert!(UserRole::Owner.is_admin());
        assert!(UserRole::Admin.is_admin());
        assert!(!UserRole::Member.is_admin());
        assert!(UserRole::Owner.is_owner());
        assert!(!UserRole::Admin.is_owner());
    }

    #[test]
    fn member_sees_only_essentials_by_default() {
        let grants = BTreeMap::new();
        let allowed = member_allowed_capability_ids(&grants);
        assert_eq!(allowed.len(), ESSENTIAL_MEMBER_CAPABILITIES.len());
        assert!(allowed.contains("builtin.extension_search"));
        assert!(!allowed.contains("builtin.shell"));
        assert!(capability_available(
            UserRole::Member,
            &grants,
            "builtin.time"
        ));
        assert!(!capability_available(
            UserRole::Member,
            &grants,
            "builtin.shell"
        ));
    }

    #[test]
    fn granting_adds_to_member_surface() {
        let mut grants = BTreeMap::new();
        grants.insert(
            "builtin.shell".to_string(),
            CapabilityAvailability::Available,
        );
        grants.insert(
            "nearai.web_search".to_string(),
            CapabilityAvailability::Available,
        );
        let allowed = member_allowed_capability_ids(&grants);
        assert!(allowed.contains("builtin.shell"));
        assert!(allowed.contains("nearai.web_search"));
        assert!(allowed.contains("builtin.extension_search"));
        assert!(capability_available(
            UserRole::Member,
            &grants,
            "builtin.shell"
        ));
    }

    #[test]
    fn hiding_an_essential_removes_it_for_members() {
        let mut grants = BTreeMap::new();
        grants.insert("builtin.time".to_string(), CapabilityAvailability::Hidden);
        let allowed = member_allowed_capability_ids(&grants);
        assert!(!allowed.contains("builtin.time"));
        assert!(!capability_available(
            UserRole::Member,
            &grants,
            "builtin.time"
        ));
    }

    #[test]
    fn admin_sees_everything() {
        let grants = BTreeMap::new();
        assert!(capability_available(
            UserRole::Admin,
            &grants,
            "builtin.shell"
        ));
        assert!(capability_available(
            UserRole::Owner,
            &grants,
            "anything.at_all"
        ));
    }

    #[test]
    fn capability_id_validation_rejects_bare_labels() {
        assert!(is_valid_capability_id("builtin.shell"));
        assert!(is_valid_capability_id("nearai.web_search"));
        assert!(is_valid_capability_id("google-drive.list_files"));
        assert!(is_valid_capability_id("builtin.trace_commons.status"));
        assert!(!is_valid_capability_id("gdrive"));
        assert!(!is_valid_capability_id("google_drive"));
        assert!(!is_valid_capability_id(""));
        assert!(!is_valid_capability_id(".shell"));
        assert!(!is_valid_capability_id("builtin."));
        assert!(!is_valid_capability_id("Builtin.Shell"));
    }

    #[test]
    fn login_tokens_are_distinct_and_hash_stably() {
        let a = generate_login_token();
        let b = generate_login_token();
        assert_ne!(a, b);
        assert_eq!(hash_login_token(&a), hash_login_token(&a));
        assert_ne!(hash_login_token(&a), hash_login_token(&b));
    }
}
