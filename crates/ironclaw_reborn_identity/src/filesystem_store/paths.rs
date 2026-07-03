//! Scoped-path construction for identity records.
//!
//! Key parts are opaque, so each is base64url-encoded into its own path
//! segment (never flattened into one delimiter-joined string) so a
//! delimiter-like id cannot collide with a key boundary. All identity data
//! lives under one tenant-shared root and is partitioned by tenant in the
//! PATH (the store is multi-tenant).

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ironclaw_host_api::ScopedPath;

use crate::RebornIdentityError;

const IDENTITY_ROOT: &str = "/tenant-shared/reborn-identity";

/// Path of the identity record for one
/// `(tenant, surface, provider, instance, subject)` key.
pub(super) fn identity_path(
    tenant: &str,
    surface: &str,
    provider: &str,
    instance: &str,
    subject: &str,
) -> Result<ScopedPath, RebornIdentityError> {
    scoped_path(&format!(
        "{IDENTITY_ROOT}/external/{}/{surface}/{}/{}/{}.json",
        segment(tenant),
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
) -> Result<ScopedPath, RebornIdentityError> {
    scoped_path(&format!(
        "{IDENTITY_ROOT}/verified-email/{}/{}.json",
        segment(tenant),
        segment(lower_email),
    ))
}

/// Path of a canonical user record.
pub(super) fn user_path(user_id: &str) -> Result<ScopedPath, RebornIdentityError> {
    scoped_path(&format!("{IDENTITY_ROOT}/users/{}.json", segment(user_id)))
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

fn scoped_path(raw: &str) -> Result<ScopedPath, RebornIdentityError> {
    ScopedPath::new(raw).map_err(|error| {
        RebornIdentityError::Backend(format!("invalid reborn-identity path: {error}"))
    })
}
