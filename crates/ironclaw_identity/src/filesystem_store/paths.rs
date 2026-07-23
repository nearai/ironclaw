//! Scoped-path construction for identity records.
//!
//! Key parts are opaque, so each is base64url-encoded into its own path
//! segment (never flattened into one delimiter-joined string) so a
//! delimiter-like id cannot collide with a key boundary. All identity data
//! lives under one tenant-shared root and is partitioned by tenant in the
//! PATH (the store is multi-tenant).

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ironclaw_host_api::ScopedPath;

use crate::{IronClawIdentityError, SurfaceKind};

const IDENTITY_ROOT: &str = "/tenant-shared/reborn-identity";

/// Path of the identity record for one
/// `(tenant, surface, provider, instance, subject)` key.
///
/// `surface` is the typed [`SurfaceKind`] rather than a `&str`: it is a closed,
/// trusted enum, so it crosses the boundary as a type (no transposition or
/// arbitrary-string risk) and is rendered via its stable `as_str()`. Unlike the
/// other key parts it needs no `segment()` encoding — the enum can never produce
/// a delimiter-like value.
pub(super) fn identity_path(
    tenant: &str,
    surface: SurfaceKind,
    provider: &str,
    instance: &str,
    subject: &str,
) -> Result<ScopedPath, IronClawIdentityError> {
    scoped_path(&format!(
        "{IDENTITY_ROOT}/external/{}/{}/{}/{}/{}.json",
        segment(tenant),
        surface.as_str(),
        segment(provider),
        segment(instance),
        segment(subject),
    ))
}

/// Path of the verified-email secondary index for one tenant + lowercased
/// email (the cross-provider linking record).
pub(super) fn verified_email_path(
    tenant: &str,
    lower_email: &str,
) -> Result<ScopedPath, IronClawIdentityError> {
    scoped_path(&format!(
        "{IDENTITY_ROOT}/verified-email/{}/{}.json",
        segment(tenant),
        segment(lower_email),
    ))
}

/// Path of a canonical user record.
pub(super) fn user_path(user_id: &str) -> Result<ScopedPath, IronClawIdentityError> {
    scoped_path(&format!("{IDENTITY_ROOT}/users/{}.json", segment(user_id)))
}

/// Path of a user's delete tombstone. Present only while a delete cascade is in
/// flight: it is written before the cascade and removed after it, so a
/// concurrent `resolve_or_create` can refuse to re-link an external identity to
/// a user that is being torn down (which would otherwise recreate a live
/// identity record for a soon-to-be-deleted id).
pub(super) fn user_tombstone_path(user_id: &str) -> Result<ScopedPath, IronClawIdentityError> {
    scoped_path(&format!(
        "{IDENTITY_ROOT}/tombstones/{}.json",
        segment(user_id)
    ))
}

/// Directory holding every canonical user record. User records are NOT
/// tenant-partitioned in the path (unlike identity/verified-email records), so
/// enumeration lists this one directory and filters by the record's own
/// `tenant_id` field.
pub(super) fn users_dir_path() -> Result<ScopedPath, IronClawIdentityError> {
    scoped_path(&format!("{IDENTITY_ROOT}/users"))
}

/// Root of one tenant's external-identity subtree
/// (`…/external/{tenant}/{surface}/{provider}/{instance}/{subject}.json`). The
/// delete cascade walks this subtree to remove every external login bound to a
/// deleted user, so a later re-login through that identity cannot resolve the
/// tombstoned user id back to life.
pub(super) fn external_tenant_dir_path(tenant: &str) -> Result<ScopedPath, IronClawIdentityError> {
    scoped_path(&format!("{IDENTITY_ROOT}/external/{}", segment(tenant)))
}

/// Build a child `ScopedPath` from a parent directory path and a listed entry
/// name. Used to turn a [`DirEntry`](ironclaw_filesystem::DirEntry) leaf name
/// back into a readable/deletable scoped path during enumeration and cascade.
pub(super) fn child_path(
    parent: &ScopedPath,
    name: &str,
) -> Result<ScopedPath, IronClawIdentityError> {
    scoped_path(&format!("{}/{}", parent.as_str(), name))
}

/// Recover the opaque `user_id` from a `users/` directory entry file name
/// (`{base64url(user_id)}.json`). Returns `None` for a name that is not a
/// `.json` record or whose stem is not valid base64url/UTF-8 — a foreign file
/// under the directory is skipped, not surfaced as an error.
pub(super) fn user_id_from_file_name(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".json")?;
    let bytes = URL_SAFE_NO_PAD.decode(stem).ok()?;
    String::from_utf8(bytes).ok()
}

/// URL-safe path segment for an opaque key part. Empty maps to `_` (a value
/// no base64 encoding produces, since encoding any non-empty input yields ≥2
/// chars) so an absent provider instance never collapses to an empty segment.
fn segment(value: &str) -> String {
    if value.is_empty() {
        "_".to_string()
    } else {
        URL_SAFE_NO_PAD.encode(value.as_bytes())
    }
}

fn scoped_path(raw: &str) -> Result<ScopedPath, IronClawIdentityError> {
    ScopedPath::new(raw).map_err(|error| {
        IronClawIdentityError::Backend(format!("invalid reborn-identity path: {error}"))
    })
}
