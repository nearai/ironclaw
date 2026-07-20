//! [`RebornUserDirectory`] implementation for [`FilesystemRebornIdentityStore`].
//!
//! Admin CRUD over the canonical `users/` records. Enumeration lists the
//! (non-tenant-partitioned) users directory and filters by the record's own
//! `tenant_id`; mutations go through the shared, lock-free
//! [`cas_update`](ironclaw_filesystem::cas_update) helper (never a per-record
//! mutex — `ironclaw_filesystem/CLAUDE.md` invariant 2); delete cascades over
//! the tenant's external-identity subtree and the verified-email index.

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FileType, FilesystemError,
    RootFilesystem, cas_update,
};
use ironclaw_host_api::{TenantId, UserId};
use std::collections::BTreeMap;
use uuid::Uuid;

use super::paths::{
    child_path, external_tenant_dir_path, user_id_from_file_name, user_path, user_tombstone_path,
    users_dir_path, verified_email_path,
};
use super::record::{
    StoredExternalIdentity, StoredUser, StoredUserRole, StoredUserStatus, StoredUserTombstone,
    StoredVerifiedEmailIndex,
};
use super::{FilesystemRebornIdentityStore, backend, to_user_id};
use crate::RebornIdentityError;
use crate::user_directory::{
    RebornUser, RebornUserDirectory, RebornUserProfileUpdate, RebornUserRole, RebornUserStatus,
};

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// A missing directory is an empty subtree, not a failure — a fresh tenant has
/// no external identities and a fresh store has no users.
fn is_absent_dir(error: &FilesystemError) -> bool {
    matches!(
        error,
        FilesystemError::NotFound { .. } | FilesystemError::MountNotFound { .. }
    )
}

fn role_to_stored(role: RebornUserRole) -> StoredUserRole {
    match role {
        RebornUserRole::Owner => StoredUserRole::Owner,
        RebornUserRole::Admin => StoredUserRole::Admin,
        RebornUserRole::Member => StoredUserRole::Member,
    }
}

fn role_from_stored(role: StoredUserRole) -> RebornUserRole {
    match role {
        StoredUserRole::Owner => RebornUserRole::Owner,
        StoredUserRole::Admin => RebornUserRole::Admin,
        StoredUserRole::Member => RebornUserRole::Member,
    }
}

fn status_to_stored(status: RebornUserStatus) -> StoredUserStatus {
    match status {
        RebornUserStatus::Active => StoredUserStatus::Active,
        RebornUserStatus::Suspended => StoredUserStatus::Suspended,
    }
}

fn status_from_stored(status: StoredUserStatus) -> RebornUserStatus {
    match status {
        StoredUserStatus::Active => RebornUserStatus::Active,
        StoredUserStatus::Suspended => RebornUserStatus::Suspended,
    }
}

/// Map a persisted row to the public domain type, validating the persisted
/// `user_id` / `created_by` / `tenant_id` strings on the way out (a malformed
/// persisted id is a backend inconsistency, surfaced rather than dropped).
fn to_reborn_user(user_id: String, stored: StoredUser) -> Result<RebornUser, RebornIdentityError> {
    let created_by = match stored.created_by {
        Some(raw) => Some(to_user_id(raw)?),
        None => None,
    };
    let tenant_id = match stored.tenant_id {
        Some(raw) => Some(TenantId::new(raw).map_err(|error| {
            RebornIdentityError::Backend(format!("persisted tenant id is invalid: {error}"))
        })?),
        None => None,
    };
    Ok(RebornUser {
        user_id: to_user_id(user_id)?,
        email: stored.email,
        display_name: stored.display_name,
        status: status_from_stored(stored.status),
        role: role_from_stored(stored.role),
        created_at: stored.created_at,
        updated_at: stored.updated_at,
        created_by,
        last_login_at: stored.last_login_at,
        tenant_id,
        metadata: stored.metadata,
    })
}

fn map_cas_error(error: CasUpdateError<RebornIdentityError>) -> RebornIdentityError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout => {
            RebornIdentityError::Backend("user record update timed out".to_string())
        }
        CasUpdateError::RetriesExhausted => {
            RebornIdentityError::Backend("user record update contended out".to_string())
        }
        CasUpdateError::CasUnsupported => RebornIdentityError::Backend(
            "user record backend does not support compare-and-swap".to_string(),
        ),
        CasUpdateError::Backend(inner) => RebornIdentityError::Backend(inner.to_string()),
    }
}

impl<F> FilesystemRebornIdentityStore<F>
where
    F: RootFilesystem + 'static,
{
    /// Lock-free read-modify-write of one user record through the shared
    /// `cas_update` helper. `mutate` is re-runnable (it may fire on every CAS
    /// retry), so it must only set fields. Returns the mutated record.
    pub(super) async fn mutate_user<M>(
        &self,
        user_id: &UserId,
        mutate: M,
    ) -> Result<RebornUser, RebornIdentityError>
    where
        M: Fn(&mut StoredUser) + Send + Sync,
    {
        let path = user_path(user_id.as_str())?;
        let user_id_str = user_id.as_str().to_string();
        let updated = cas_update(
            &self.filesystem,
            &self.scope,
            &path,
            |bytes| {
                serde_json::from_slice::<StoredUser>(bytes)
                    .map_err(|error| RebornIdentityError::Backend(error.to_string()))
            },
            |user: &StoredUser| {
                let body = serde_json::to_vec(user)
                    .map_err(|error| RebornIdentityError::Backend(error.to_string()))?;
                Ok(Entry::bytes(body).with_content_type(ContentType::json()))
            },
            |current: Option<StoredUser>| {
                let mutate = &mutate;
                let user_id_str = user_id_str.clone();
                async move {
                    let Some(mut user) = current else {
                        return Err(RebornIdentityError::UserNotFound(user_id_str));
                    };
                    mutate(&mut user);
                    Ok(CasApply::new(user.clone(), user))
                }
            },
        )
        .await
        .map_err(map_cas_error)?;
        to_reborn_user(user_id.as_str().to_string(), updated)
    }

    /// Delete every external-identity record in `tenant`'s subtree bound to
    /// `user_id`. Walks the fixed-depth `external/{tenant}/…/{subject}.json`
    /// tree iteratively (no async recursion) and deletes each matching leaf.
    async fn delete_external_identities_for_user(
        &self,
        tenant: &str,
        user_id: &str,
    ) -> Result<(), RebornIdentityError> {
        let mut worklist = vec![external_tenant_dir_path(tenant)?];
        while let Some(dir) = worklist.pop() {
            let entries = match self.filesystem.list_dir(&self.scope, &dir).await {
                Ok(entries) => entries,
                Err(error) if is_absent_dir(&error) => continue,
                Err(error) => return Err(backend(error)),
            };
            for entry in entries {
                let child = child_path(&dir, &entry.name)?;
                match entry.file_type {
                    FileType::Directory => worklist.push(child),
                    FileType::File => {
                        if !entry.name.ends_with(".json") {
                            continue;
                        }
                        if let Some(record) =
                            self.read_record::<StoredExternalIdentity>(&child).await?
                            && record.user_id == user_id
                        {
                            self.filesystem
                                .delete(&self.scope, &child)
                                .await
                                .map_err(backend)?;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Delete the user's verified-email index if one exists and points at them.
    /// Keyed by the user's own stored email — the common case, since a user's
    /// index is the verified email they first logged in with. Admin-created
    /// users have no index (no OAuth surface), so this is a no-op for them.
    async fn delete_verified_email_index_for_user(
        &self,
        tenant: &str,
        user_id: &str,
    ) -> Result<(), RebornIdentityError> {
        let Some(stored) = self.read_record::<StoredUser>(&user_path(user_id)?).await? else {
            return Ok(());
        };
        let Some(email) = stored.email.as_deref() else {
            return Ok(());
        };
        let lower = email.to_ascii_lowercase();
        if lower.is_empty() {
            return Ok(());
        }
        let index_path = verified_email_path(tenant, &lower)?;
        if let Some(index) = self
            .read_record::<StoredVerifiedEmailIndex>(&index_path)
            .await?
            && index.user_id == user_id
        {
            self.filesystem
                .delete(&self.scope, &index_path)
                .await
                .map_err(backend)?;
        }
        Ok(())
    }
}

#[async_trait]
impl<F> RebornUserDirectory for FilesystemRebornIdentityStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn list_users(
        &self,
        tenant_id: &TenantId,
        status: Option<RebornUserStatus>,
        after: Option<&UserId>,
        limit: usize,
    ) -> Result<Vec<RebornUser>, RebornIdentityError> {
        // A zero limit can never fit a record; short-circuit before any I/O.
        if limit == 0 {
            return Ok(Vec::new());
        }
        let dir = users_dir_path()?;
        let entries = match self.filesystem.list_dir(&self.scope, &dir).await {
            Ok(entries) => entries,
            Err(error) if is_absent_dir(&error) => return Ok(Vec::new()),
            Err(error) => return Err(backend(error)),
        };
        // Decode candidate user ids from the file names first — cheap, no
        // record reads. File names are base64url-encoded, so name order is NOT
        // id order; ordering by the DECODED id is what makes the `after` cursor
        // a stable, consistent seek point across calls.
        let mut candidates: Vec<(String, String)> = entries
            .into_iter()
            .filter(|entry| entry.file_type == FileType::File)
            .filter_map(|entry| user_id_from_file_name(&entry.name).map(|id| (id, entry.name)))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0));

        let after = after.map(UserId::as_str);
        let want_status = status.map(status_to_stored);
        // Only up to `limit` MATCHING records are read past the cursor — the
        // scan stops early instead of loading and allocating the whole tenant.
        let mut users = Vec::with_capacity(limit.min(candidates.len()));
        for (user_id, name) in candidates {
            // Skip forward to strictly after the cursor.
            if let Some(after) = after
                && user_id.as_str() <= after
            {
                continue;
            }
            let path = child_path(&dir, &name)?;
            let Some(stored) = self.read_record::<StoredUser>(&path).await? else {
                continue;
            };
            // A record with no persisted tenant belongs to the single
            // configured tenant (only single-tenant deployments have such
            // legacy records); otherwise it must match exactly.
            let belongs = match &stored.tenant_id {
                Some(tenant) => tenant == tenant_id.as_str(),
                None => true,
            };
            if !belongs {
                continue;
            }
            if let Some(want) = want_status
                && stored.status != want
            {
                continue;
            }
            users.push(to_reborn_user(user_id, stored)?);
            if users.len() >= limit {
                break;
            }
        }
        Ok(users)
    }

    async fn get_user(&self, user_id: &UserId) -> Result<Option<RebornUser>, RebornIdentityError> {
        let path = user_path(user_id.as_str())?;
        match self.read_record::<StoredUser>(&path).await? {
            Some(stored) => Ok(Some(to_reborn_user(user_id.as_str().to_string(), stored)?)),
            None => Ok(None),
        }
    }

    async fn create_user(
        &self,
        tenant_id: &TenantId,
        email: Option<String>,
        display_name: Option<String>,
        role: RebornUserRole,
        created_by: &UserId,
    ) -> Result<RebornUser, RebornIdentityError> {
        let new_user_id = to_user_id(Uuid::new_v4().to_string())?;
        let now = now_rfc3339();
        let record = StoredUser {
            email,
            display_name,
            created_at: now.clone(),
            updated_at: now,
            status: StoredUserStatus::Active,
            role: role_to_stored(role),
            created_by: Some(created_by.as_str().to_string()),
            last_login_at: None,
            tenant_id: Some(tenant_id.as_str().to_string()),
            metadata: BTreeMap::new(),
        };
        // A minted UUID collision would surface as VersionMismatch on the
        // Absent CAS; astronomically unlikely, and surfaced (not retried) so it
        // is never silently overwritten.
        self.write_record(
            &user_path(new_user_id.as_str())?,
            &record,
            CasExpectation::Absent,
        )
        .await
        .map_err(backend)?;
        to_reborn_user(new_user_id.as_str().to_string(), record)
    }

    async fn update_profile(
        &self,
        user_id: &UserId,
        update: RebornUserProfileUpdate,
    ) -> Result<RebornUser, RebornIdentityError> {
        let now = now_rfc3339();
        self.mutate_user(user_id, move |user| {
            if let Some(display_name) = &update.display_name {
                user.display_name = Some(display_name.clone());
            }
            if let Some(metadata) = &update.metadata {
                user.metadata = metadata.clone();
            }
            user.updated_at = now.clone();
        })
        .await
    }

    async fn update_status(
        &self,
        user_id: &UserId,
        status: RebornUserStatus,
    ) -> Result<RebornUser, RebornIdentityError> {
        let now = now_rfc3339();
        let stored = status_to_stored(status);
        self.mutate_user(user_id, move |user| {
            user.status = stored;
            user.updated_at = now.clone();
        })
        .await
    }

    async fn update_role(
        &self,
        user_id: &UserId,
        role: RebornUserRole,
    ) -> Result<RebornUser, RebornIdentityError> {
        let now = now_rfc3339();
        let stored = role_to_stored(role);
        self.mutate_user(user_id, move |user| {
            user.role = stored;
            user.updated_at = now.clone();
        })
        .await
    }

    async fn record_last_login(
        &self,
        user_id: &UserId,
        at: String,
    ) -> Result<(), RebornIdentityError> {
        // Deliberately does NOT bump updated_at (that tracks profile edits).
        self.mutate_user(user_id, move |user| {
            user.last_login_at = Some(at.clone());
        })
        .await
        .map(|_user| ())
    }

    async fn delete_user(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<(), RebornIdentityError> {
        // 0. Tombstone the user for the duration of the cascade. Deleting the
        //    external identities (step 1) before the verified-email index
        //    (step 2) opens a window where a concurrent `resolve_or_create`
        //    still sees the email index pointing at this user and would re-link
        //    a fresh identity record to an id about to be deleted (future
        //    logins would then fast-path to a ghost). The tombstone lets the
        //    resolver refuse that re-link; it is removed once the cascade
        //    completes (step 4).
        let tombstone = user_tombstone_path(user_id.as_str())?;
        self.write_record(
            &tombstone,
            &StoredUserTombstone {
                deleted_at: now_rfc3339(),
            },
            CasExpectation::Any,
        )
        .await
        .map_err(backend)?;

        // 1. External identities first: while the user record still exists we
        //    don't strictly need it, but doing identities first means a
        //    partial failure never leaves a deleted user whose logins still
        //    resolve.
        self.delete_external_identities_for_user(tenant_id.as_str(), user_id.as_str())
            .await?;

        // 2. Verified-email index, if the user carries an email and the index
        //    points at them.
        self.delete_verified_email_index_for_user(tenant_id.as_str(), user_id.as_str())
            .await?;

        // 3. The user record itself.
        self.filesystem
            .delete(&self.scope, &user_path(user_id.as_str())?)
            .await
            .map_err(backend)?;

        // 4. Cascade complete: drop the tombstone. A crash before this point
        //    leaves the tombstone in place, which fails safe — the resolver
        //    keeps refusing to re-link the half-deleted id until a re-run of
        //    the delete (or manual cleanup) clears it.
        self.filesystem
            .delete(&self.scope, &tombstone)
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn count_active_admins(
        &self,
        tenant_id: &TenantId,
    ) -> Result<usize, RebornIdentityError> {
        // Last-admin protection needs the TRUE active-admin count, so this scan
        // is deliberately unbounded (`usize::MAX`) rather than a bounded page.
        // Active admins are few, so reading them all is cheap.
        let active = self
            .list_users(tenant_id, Some(RebornUserStatus::Active), None, usize::MAX)
            .await?;
        Ok(active
            .into_iter()
            .filter(|user| user.role.is_admin())
            .count())
    }
}
