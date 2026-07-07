//! Persisted record shapes for the filesystem identity store.
//!
//! These are the on-disk JSON bodies behind each scoped path. They live in
//! their own module so the substrate's data layout is reviewable in one place,
//! separate from the resolve/link/create logic that reads and writes them.

/// The canonical user profile record, keyed by `UserId` at `…/users/{id}.json`.
///
/// `Clone`/`PartialEq` are required by the shared
/// [`cas_update`](ironclaw_filesystem::cas_update) read-modify-write helper the
/// admin mutation paths drive (it hands `apply` an owned snapshot and skips the
/// write when the snapshot is unchanged).
///
/// The admin fields (`status`, `role`, `created_by`, `last_login_at`,
/// `tenant_id`, `metadata`) are all `#[serde(default)]` so records written
/// before the admin surface existed — which carry only the first four fields —
/// still deserialize, defaulting to an `Active` `Member` with no tenant. That
/// back-compat is pinned by `legacy_stored_user_json_deserializes_with_defaults`
/// in the `tests` submodule.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct StoredUser {
    pub(super) email: Option<String>,
    pub(super) display_name: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    #[serde(default)]
    pub(super) status: StoredUserStatus,
    #[serde(default)]
    pub(super) role: StoredUserRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) created_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) last_login_at: Option<String>,
    /// Tenant that owns this user. `None` on records written before the admin
    /// surface existed; enumeration treats `None` as the deployment's single
    /// configured tenant (see `RebornUserDirectory::list_users`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(super) metadata: std::collections::BTreeMap<String, String>,
}

/// Account status. Wire-stable snake_case; persisted, so it must not drift.
#[derive(Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredUserStatus {
    #[default]
    Active,
    Suspended,
}

/// Account role. Wire-stable snake_case; persisted, so it must not drift.
/// `Member` is the default so a record written by `resolve_or_create` (a plain
/// SSO login) is never accidentally an admin.
#[derive(Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StoredUserRole {
    Owner,
    Admin,
    #[default]
    Member,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct StoredExternalIdentity {
    pub(super) user_id: String,
    pub(super) email: Option<String>,
    pub(super) email_verified: bool,
    pub(super) created_at: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct StoredVerifiedEmailIndex {
    pub(super) user_id: String,
}

/// In-flight delete marker, keyed by `UserId` at `…/tombstones/{id}.json`.
/// Written before a delete cascade and removed after it, so a concurrent
/// `resolve_or_create` can see that a user is being torn down and refuse to
/// re-link an external identity to it.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct StoredUserTombstone {
    pub(super) deleted_at: String,
}
