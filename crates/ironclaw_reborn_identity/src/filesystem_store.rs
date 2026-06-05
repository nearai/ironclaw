//! Filesystem-backed implementation of [`RebornIdentityResolver`](crate::RebornIdentityResolver).
//!
//! Identity records live behind the host [`RootFilesystem`] /
//! [`ScopedFilesystem`] abstraction — the same substrate boundary every other
//! durable Reborn store (run-state, approvals, threads, Slack host-state) sits
//! behind — so substrate choice, tenant scoping, and host ownership stay
//! centralized in the filesystem layer rather than this crate holding a raw
//! database handle. The relational guarantees the canonical key needs are
//! reconstructed on top of the filesystem's compare-and-swap primitive, the
//! same way [`FilesystemSlackHostState`](../../ironclaw_reborn_composition) does:
//!
//! - **Keyed lookup** — one record per `(tenant, surface, provider, instance,
//!   subject)`, addressed by a scoped path (key parts are opaque, separately
//!   path-segmented, never flattened so delimiter-like ids cannot collide).
//! - **Atomic resolve → link → create** — a per-identity-key async lock
//!   serializes concurrent first-contacts for one identity, and
//!   `CasExpectation::Absent` on every create is the cross-process backstop: a
//!   racing creator gets `VersionMismatch` and reconciles by re-reading.
//! - **Verified-email cross-provider linking** — a secondary index record
//!   `verified-email/<tenant>/<lower(email)>` → user id, so linking is a keyed
//!   read rather than a scan. Written only for verified emails; tenant-scoped.
//!
//! The store body stays focused on this resolve/link/create logic; the
//! persisted record shapes live in [`record`], path construction in [`paths`],
//! and the behavioral matrix in the `tests` submodule.

mod paths;
mod record;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, InvocationId, ProjectId, ResourceScope, ScopedPath, TenantId, UserId,
};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::{
    ExternalIdentityKey, RebornIdentityError, RebornIdentityResolver, ResolveExternalIdentity,
    SurfaceKind,
};
use paths::{identity_path, user_path, verified_email_path};
use record::{StoredExternalIdentity, StoredUser, StoredVerifiedEmailIndex};

/// Canonical identity store backed by a host scoped filesystem.
pub struct FilesystemRebornIdentityStore<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    /// Fixed host-caller scope for the filesystem API. Identity data is
    /// partitioned by tenant in the PATH (the store is multi-tenant); this
    /// scope is just the runtime-owner caller identity the host APIs require.
    scope: ResourceScope,
    locks: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl<F> FilesystemRebornIdentityStore<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            filesystem,
            scope: ResourceScope {
                tenant_id,
                user_id,
                agent_id: Some(agent_id),
                project_id,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn lock_for(&self, key: String) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    async fn read_record<T>(&self, path: &ScopedPath) -> Result<Option<T>, RebornIdentityError>
    where
        T: DeserializeOwned,
    {
        let Some(versioned) = self
            .filesystem
            .get(&self.scope, path)
            .await
            .map_err(backend)?
        else {
            return Ok(None);
        };
        let value = serde_json::from_slice(&versioned.entry.body)
            .map_err(|error| RebornIdentityError::Backend(error.to_string()))?;
        Ok(Some(value))
    }

    async fn write_record<T>(
        &self,
        path: &ScopedPath,
        value: &T,
        cas: CasExpectation,
    ) -> Result<(), FilesystemError>
    where
        T: Serialize,
    {
        let body =
            serde_json::to_vec(value).map_err(|error| FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::WriteFile,
                reason: format!("reborn-identity record could not be serialized: {error}"),
            })?;
        self.filesystem
            .put(
                &self.scope,
                path,
                Entry::bytes(body).with_content_type(ContentType::json()),
                cas,
            )
            .await
            .map(|_version| ())
    }

    /// Read the user already bound to an external identity, or `None`.
    async fn identity_user(
        &self,
        tenant: &str,
        surface: &str,
        provider: &str,
        instance: &str,
        subject: &str,
    ) -> Result<Option<UserId>, RebornIdentityError> {
        let path = identity_path(tenant, surface, provider, instance, subject)?;
        match self.read_record::<StoredExternalIdentity>(&path).await? {
            Some(record) => Ok(Some(to_user_id(record.user_id)?)),
            None => Ok(None),
        }
    }

    /// Write the identity record with `CasExpectation::Absent`; if a racing
    /// creator already wrote it, reconcile by returning the persisted user.
    async fn put_identity_reconciling(
        &self,
        path: &ScopedPath,
        user_id: &UserId,
        identity: &ResolveExternalIdentity,
        now: &str,
    ) -> Result<UserId, RebornIdentityError> {
        let record = StoredExternalIdentity {
            user_id: user_id.as_str().to_string(),
            email: identity.email.clone(),
            email_verified: identity.email_verified,
            created_at: now.to_string(),
        };
        match self
            .write_record(path, &record, CasExpectation::Absent)
            .await
        {
            Ok(()) => Ok(user_id.clone()),
            Err(FilesystemError::VersionMismatch { .. }) => {
                let Some(existing) = self.read_record::<StoredExternalIdentity>(path).await? else {
                    return Err(RebornIdentityError::Backend(
                        "identity record vanished during reconciliation".to_string(),
                    ));
                };
                to_user_id(existing.user_id)
            }
            Err(error) => Err(backend(error)),
        }
    }
}

#[async_trait]
impl<F> RebornIdentityResolver for FilesystemRebornIdentityStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn resolve_or_create(
        &self,
        identity: ResolveExternalIdentity,
    ) -> Result<UserId, RebornIdentityError> {
        // Channel actors are never mint-capable: the resolver contract routes
        // them through lookup/bind so an unbound actor fails closed instead of
        // auto-provisioning. Only OAuth-surface identities (admission gated up
        // front by the email-domain allowlist) may mint here.
        if identity.surface_kind == SurfaceKind::ChannelActor {
            return Err(RebornIdentityError::ChannelActorNotMintable);
        }

        let tenant = identity.tenant_id.as_str();
        let surface = identity.surface_kind.as_str();
        let provider = identity.provider_kind.as_str();
        // No installation (browser OAuth) maps to "" so the key stays total.
        let instance = identity
            .provider_instance_id
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or("");
        let subject = identity.external_subject_id.as_str();
        let id_path = identity_path(tenant, surface, provider, instance, subject)?;

        // Fast path: a returning external identity resolves with a read only.
        if let Some(record) = self.read_record::<StoredExternalIdentity>(&id_path).await? {
            return to_user_id(record.user_id);
        }

        let lower_email = verified_email_key(&identity);

        // Serialize the create/link race on the IDENTITY KEY (not the email).
        // The in-lock re-check below then catches every same-key race —
        // including two first-logins for the same identity that present
        // divergent verified emails, which an email-scoped lock would let run
        // concurrently and publish an orphan verified-email index for the
        // loser of the identity CAS. Cross-provider linking for a SHARED email
        // across DIFFERENT identity keys is arbitrated by the verified-email
        // index CAS below, not this lock, so it still converges.
        let lock = self.lock_for(format!("identity:{}", id_path.as_str()));
        let _guard = lock.lock().await;

        // Re-check the identity key under the lock: a concurrent first-login
        // for the same key may have created it between the read above and the
        // lock, so the create path below must not mint a second user.
        if let Some(record) = self.read_record::<StoredExternalIdentity>(&id_path).await? {
            return to_user_id(record.user_id);
        }

        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

        // Link by a VERIFIED email to an existing user in the SAME tenant.
        if let Some(email) = &lower_email {
            let email_path = verified_email_path(tenant, email)?;
            if let Some(index) = self
                .read_record::<StoredVerifiedEmailIndex>(&email_path)
                .await?
            {
                let user_id = to_user_id(index.user_id)?;
                self.put_identity_reconciling(&id_path, &user_id, &identity, &now)
                    .await?;
                return Ok(user_id);
            }
        }

        // New user (or adopt the cross-process winner of this verified
        // email). Mint a candidate user record first so the verified-email
        // index and identity record below always point at a user that exists.
        // If this candidate then loses the verified-email index CAS (a
        // concurrent first-login for the same email through a DIFFERENT
        // provider won), it is left unreferenced — a benign orphan user, never
        // an orphan index: no identity and no index point at it, and the
        // principal still converges on the index winner below.
        //
        // The orphan is an accepted, bounded leak: it occurs only on a LOST
        // cold first-contact race (the returning-login fast path never mints),
        // the record is tiny, and there is no steady-state growth. We mint
        // first rather than deferring the write until ownership resolves
        // because writing the user last would, in the rarer divergent-email
        // cross-process race, leave the verified-email index pointing at an id
        // with no user record at all (a phantom) — strictly worse than an
        // unreferenced row. GC of unreferenced user rows is out of scope here.
        let new_user_id = to_user_id(Uuid::new_v4().to_string())?;
        self.write_record(
            &user_path(new_user_id.as_str())?,
            &StoredUser {
                email: identity.email.clone(),
                display_name: identity.display_name.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
            },
            CasExpectation::Absent,
        )
        .await
        .map_err(backend)?;

        // Establish the verified-email index BEFORE the identity record. Two
        // invariants follow, each closing a split-principal hole:
        //
        //  1. The per-identity-key lock is process-local, so a second runtime
        //     process can mint the canonical user for this email first.
        //     `CasExpectation::Absent` makes exactly one writer win the index;
        //     the loser adopts the winner's user (re-reading the index)
        //     instead of returning its own freshly-minted user and permanently
        //     splitting the principal.
        //  2. "A verified-email identity record exists" now always implies
        //     "its index exists" (index is written first), so the read-only
        //     fast path above never returns an identity whose email index is
        //     missing — a partial first write self-heals through the
        //     email-link branch on retry rather than minting a second user.
        let owner_user_id = match &lower_email {
            Some(email) => {
                let email_path = verified_email_path(tenant, email)?;
                match self
                    .write_record(
                        &email_path,
                        &StoredVerifiedEmailIndex {
                            user_id: new_user_id.as_str().to_string(),
                        },
                        CasExpectation::Absent,
                    )
                    .await
                {
                    Ok(()) => new_user_id.clone(),
                    Err(FilesystemError::VersionMismatch { .. }) => {
                        let Some(winner) = self
                            .read_record::<StoredVerifiedEmailIndex>(&email_path)
                            .await?
                        else {
                            return Err(RebornIdentityError::Backend(
                                "verified-email index vanished after CAS conflict".to_string(),
                            ));
                        };
                        to_user_id(winner.user_id)?
                    }
                    Err(error) => return Err(backend(error)),
                }
            }
            None => new_user_id.clone(),
        };

        // Identity record points at the resolved owner (ours, or the adopted
        // cross-process winner). Reconcile if a same-key racer beat us to it.
        self.put_identity_reconciling(&id_path, &owner_user_id, &identity, &now)
            .await
    }

    async fn lookup(
        &self,
        key: ExternalIdentityKey,
    ) -> Result<Option<UserId>, RebornIdentityError> {
        let instance = key
            .provider_instance_id
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or("");
        self.identity_user(
            key.tenant_id.as_str(),
            key.surface_kind.as_str(),
            key.provider_kind.as_str(),
            instance,
            key.external_subject_id.as_str(),
        )
        .await
    }

    async fn bind(
        &self,
        key: ExternalIdentityKey,
        user_id: &UserId,
    ) -> Result<(), RebornIdentityError> {
        let instance = key
            .provider_instance_id
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or("");
        let path = identity_path(
            key.tenant_id.as_str(),
            key.surface_kind.as_str(),
            key.provider_kind.as_str(),
            instance,
            key.external_subject_id.as_str(),
        )?;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let lock = self.lock_for(format!("identity:{}", path.as_str()));
        let _guard = lock.lock().await;
        // Re-binding the same key re-points it at `user_id` (upsert). Channel
        // actors carry no email, so the record stores none.
        let record = StoredExternalIdentity {
            user_id: user_id.as_str().to_string(),
            email: None,
            email_verified: false,
            created_at: now,
        };
        let cas = match self
            .filesystem
            .get(&self.scope, &path)
            .await
            .map_err(backend)?
        {
            Some(versioned) => CasExpectation::Version(versioned.version),
            None => CasExpectation::Absent,
        };
        match self.write_record(&path, &record, cas).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => {
                // Lost a concurrent write; overwrite to honor re-point semantics.
                self.write_record(&path, &record, CasExpectation::Any)
                    .await
                    .map_err(backend)
            }
            Err(error) => Err(backend(error)),
        }
    }

    async fn adopt_migrated_identity(
        &self,
        identity: ResolveExternalIdentity,
        user_id: &UserId,
    ) -> Result<(), RebornIdentityError> {
        let tenant = identity.tenant_id.as_str();
        let surface = identity.surface_kind.as_str();
        let provider = identity.provider_kind.as_str();
        let instance = identity
            .provider_instance_id
            .as_ref()
            .map(|value| value.as_str())
            .unwrap_or("");
        let subject = identity.external_subject_id.as_str();
        let id_path = identity_path(tenant, surface, provider, instance, subject)?;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

        // Idempotent: a returning user may have already resolved (creating the
        // canonical record) before the one-time fold ran. Never clobber an
        // existing identity — only seed the absent one.
        if self
            .read_record::<StoredExternalIdentity>(&id_path)
            .await?
            .is_none()
        {
            let record = StoredExternalIdentity {
                user_id: user_id.as_str().to_string(),
                email: identity.email.clone(),
                email_verified: identity.email_verified,
                created_at: now,
            };
            match self
                .write_record(&id_path, &record, CasExpectation::Absent)
                .await
            {
                // A concurrent writer (returning login) created it first; the
                // canonical record wins, migration leaves it untouched.
                Ok(()) | Err(FilesystemError::VersionMismatch { .. }) => {}
                Err(error) => return Err(backend(error)),
            }
        }

        // Seed the canonical verified-email index so a later login through a
        // DIFFERENT provider with the same verified email links to the
        // migrated user rather than minting a second one. First writer wins;
        // an already-present index (another migrated row sharing the email, or
        // a live resolve) is authoritative and left in place.
        if let Some(email) = verified_email_key(&identity) {
            let email_path = verified_email_path(tenant, &email)?;
            if self
                .read_record::<StoredVerifiedEmailIndex>(&email_path)
                .await?
                .is_none()
            {
                match self
                    .write_record(
                        &email_path,
                        &StoredVerifiedEmailIndex {
                            user_id: user_id.as_str().to_string(),
                        },
                        CasExpectation::Absent,
                    )
                    .await
                {
                    Ok(()) | Err(FilesystemError::VersionMismatch { .. }) => {}
                    Err(error) => return Err(backend(error)),
                }
            }
        }
        Ok(())
    }
}

/// The verified-email value an identity links and indexes on, normalized in
/// ONE place so `resolve_or_create` and `adopt_migrated_identity` cannot drift
/// (`.claude/rules/types.md` — one source of truth for the invariant). This is
/// the most security-load-bearing value in the crate: it is the
/// `verified-email/<tenant>/<lower(email)>` index key that decides whether a
/// later different-provider login collapses onto an existing `UserId`.
///
/// Returns `Some(lowercased email)` only when the email is present, non-empty,
/// provider-verified, AND on the OAuth surface. The surface gate matters
/// because the verified-email index carries no surface dimension: restricting
/// linking to the allowlist-gated browser-SSO surface stops a channel actor
/// that happens to assert a verified email from reading or overwriting an
/// OAuth user's index (a cross-surface account collapse). The empty-string
/// guard stops `Some("")` from indexing/locking on the `segment("") == "_"`
/// sentinel and linking unrelated future logins onto it.
fn verified_email_key(identity: &ResolveExternalIdentity) -> Option<String> {
    if identity.surface_kind != SurfaceKind::Oauth || !identity.email_verified {
        return None;
    }
    identity
        .email
        .as_deref()
        .map(str::to_ascii_lowercase)
        .filter(|email| !email.is_empty())
}

fn to_user_id(raw: String) -> Result<UserId, RebornIdentityError> {
    UserId::new(raw).map_err(|error| RebornIdentityError::InvalidUserId(error.to_string()))
}

fn backend(error: FilesystemError) -> RebornIdentityError {
    RebornIdentityError::Backend(error.to_string())
}
