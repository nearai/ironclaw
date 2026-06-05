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
//! - **Atomic resolve → link → create** — a per-key async lock serializes
//!   concurrent first-contacts, and `CasExpectation::Absent` on every create
//!   is the cross-process backstop: a racing creator gets `VersionMismatch`
//!   and reconciles by re-reading.
//! - **Verified-email cross-provider linking** — a secondary index record
//!   `verified-email/<tenant>/<lower(email)>` → user id, so linking is a keyed
//!   read rather than a scan. Written only for verified emails; tenant-scoped.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
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
};

const IDENTITY_ROOT: &str = "/tenant-shared/reborn-identity";

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

    fn identity_path(
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

    fn verified_email_path(
        tenant: &str,
        lower_email: &str,
    ) -> Result<ScopedPath, RebornIdentityError> {
        scoped_path(&format!(
            "{IDENTITY_ROOT}/verified-email/{}/{}.json",
            segment(tenant),
            segment(lower_email),
        ))
    }

    fn user_path(user_id: &str) -> Result<ScopedPath, RebornIdentityError> {
        scoped_path(&format!("{IDENTITY_ROOT}/users/{}.json", segment(user_id)))
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
        let path = Self::identity_path(tenant, surface, provider, instance, subject)?;
        match self.read_record::<StoredExternalIdentity>(&path).await? {
            Some(record) => Ok(Some(to_user_id(record.user_id)?)),
            None => Ok(None),
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
        let id_path = Self::identity_path(tenant, surface, provider, instance, subject)?;

        // Fast path: a returning external identity resolves with a read only.
        if let Some(record) = self.read_record::<StoredExternalIdentity>(&id_path).await? {
            return to_user_id(record.user_id);
        }

        // Serialize the create/link race. Lock on the verified email when
        // present (so two providers asserting the same email converge) else on
        // the identity key itself.
        let lower_email = identity
            .email
            .as_deref()
            .filter(|_| identity.email_verified)
            .map(str::to_ascii_lowercase);
        let lock_key = match &lower_email {
            Some(email) => format!("email:{tenant}:{email}"),
            None => format!("identity:{}", id_path.as_str()),
        };
        let lock = self.lock_for(lock_key);
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
            let email_path = Self::verified_email_path(tenant, email)?;
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
        let new_user_id = to_user_id(Uuid::new_v4().to_string())?;
        self.write_record(
            &Self::user_path(new_user_id.as_str())?,
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
        //  1. The per-key/per-email lock is process-local, so a second
        //     runtime process can mint the canonical user for this email
        //     first. `CasExpectation::Absent` makes exactly one writer win
        //     the index; the loser adopts the winner's user (re-reading the
        //     index) instead of returning its own freshly-minted user and
        //     permanently splitting the principal.
        //  2. "A verified-email identity record exists" now always implies
        //     "its index exists" (index is written first), so the read-only
        //     fast path above never returns an identity whose email index is
        //     missing — a partial first write self-heals through the
        //     email-link branch on retry rather than minting a second user.
        let owner_user_id = match &lower_email {
            Some(email) => {
                let email_path = Self::verified_email_path(tenant, email)?;
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
        let path = Self::identity_path(
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
        let id_path = Self::identity_path(tenant, surface, provider, instance, subject)?;
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
        if identity.email_verified
            && let Some(email) = identity
                .email
                .as_deref()
                .map(str::to_ascii_lowercase)
                .filter(|email| !email.is_empty())
        {
            let email_path = Self::verified_email_path(tenant, &email)?;
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

impl<F> FilesystemRebornIdentityStore<F>
where
    F: RootFilesystem + 'static,
{
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

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredUser {
    email: Option<String>,
    display_name: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredExternalIdentity {
    user_id: String,
    email: Option<String>,
    email_verified: bool,
    created_at: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredVerifiedEmailIndex {
    user_id: String,
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

fn to_user_id(raw: String) -> Result<UserId, RebornIdentityError> {
    UserId::new(raw).map_err(|error| RebornIdentityError::InvalidUserId(error.to_string()))
}

fn backend(error: FilesystemError) -> RebornIdentityError {
    RebornIdentityError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExternalSubjectId, ProviderInstanceId, ProviderKind, SurfaceKind};
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    fn store_on(root: Arc<InMemoryBackend>) -> FilesystemRebornIdentityStore<InMemoryBackend> {
        let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
            root,
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/tenant-shared").unwrap(),
                VirtualPath::new("/tenants/host/shared").unwrap(),
                MountPermissions::read_write_list_delete(),
            )])
            .unwrap(),
        ));
        FilesystemRebornIdentityStore::new(
            scoped,
            TenantId::new("tenant-host").unwrap(),
            UserId::new("user:host").unwrap(),
            AgentId::new("agent:host").unwrap(),
            Some(ProjectId::new("project:host").unwrap()),
        )
    }

    fn store() -> FilesystemRebornIdentityStore<InMemoryBackend> {
        store_on(Arc::new(InMemoryBackend::default()))
    }

    /// Two stores over ONE shared backend with independent in-memory lock
    /// maps — the in-test stand-in for two runtime processes whose per-email
    /// locks do not serialize each other across the durable substrate.
    fn store_pair() -> (
        FilesystemRebornIdentityStore<InMemoryBackend>,
        FilesystemRebornIdentityStore<InMemoryBackend>,
    ) {
        let root = Arc::new(InMemoryBackend::default());
        (store_on(Arc::clone(&root)), store_on(root))
    }

    fn tenant(id: &str) -> TenantId {
        TenantId::new(id).expect("tenant")
    }

    fn oauth(
        tenant: &TenantId,
        provider: &str,
        sub: &str,
        email: Option<&str>,
        verified: bool,
    ) -> ResolveExternalIdentity {
        ResolveExternalIdentity {
            tenant_id: tenant.clone(),
            surface_kind: SurfaceKind::Oauth,
            provider_kind: ProviderKind::new(provider).expect("provider"),
            provider_instance_id: None,
            external_subject_id: ExternalSubjectId::new(sub).expect("subject"),
            email: email.map(str::to_string),
            email_verified: verified,
            display_name: None,
        }
    }

    fn channel_actor(
        tenant: &TenantId,
        provider: &str,
        instance: &str,
        actor: &str,
    ) -> ResolveExternalIdentity {
        ResolveExternalIdentity {
            tenant_id: tenant.clone(),
            surface_kind: SurfaceKind::ChannelActor,
            provider_kind: ProviderKind::new(provider).expect("provider"),
            provider_instance_id: Some(ProviderInstanceId::new(instance).expect("instance")),
            external_subject_id: ExternalSubjectId::new(actor).expect("actor"),
            email: None,
            email_verified: false,
            display_name: None,
        }
    }

    fn channel_key(tenant: &TenantId, provider: &str, actor: &str) -> ExternalIdentityKey {
        ExternalIdentityKey {
            tenant_id: tenant.clone(),
            surface_kind: SurfaceKind::ChannelActor,
            provider_kind: ProviderKind::new(provider).expect("provider"),
            provider_instance_id: None,
            external_subject_id: ExternalSubjectId::new(actor).expect("actor"),
        }
    }

    #[tokio::test]
    async fn same_identity_is_stable_across_logins() {
        let store = store();
        let t = tenant("t");
        let first = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        let second = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(first.as_str(), second.as_str());
    }

    #[tokio::test]
    async fn distinct_identities_get_distinct_users() {
        let store = store();
        let t = tenant("t");
        let a = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        let b = store
            .resolve_or_create(oauth(&t, "google", "g-2", Some("b@x.com"), true))
            .await
            .expect("resolve");
        assert_ne!(
            a.as_str(),
            b.as_str(),
            "different people are different users"
        );
    }

    #[tokio::test]
    async fn verified_email_links_across_oauth_providers() {
        let store = store();
        let t = tenant("t");
        let via_google = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        let via_github = store
            .resolve_or_create(oauth(&t, "github", "gh-9", Some("same@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(
            via_google.as_str(),
            via_github.as_str(),
            "a verified shared email links both provider identities to one user"
        );
    }

    #[tokio::test]
    async fn verified_email_link_is_case_insensitive() {
        let store = store();
        let t = tenant("t");
        let via_google = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("Alice@Example.COM"), true))
            .await
            .expect("resolve");
        let via_github = store
            .resolve_or_create(oauth(&t, "github", "gh-9", Some("alice@example.com"), true))
            .await
            .expect("resolve");
        assert_eq!(
            via_google.as_str(),
            via_github.as_str(),
            "verified-email linking must be case-insensitive across providers"
        );
    }

    #[tokio::test]
    async fn unverified_email_does_not_link() {
        let store = store();
        let t = tenant("t");
        let verified = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        let unverified = store
            .resolve_or_create(oauth(&t, "github", "gh-9", Some("same@x.com"), false))
            .await
            .expect("resolve");
        assert_ne!(
            verified.as_str(),
            unverified.as_str(),
            "an unverified email must never link to a verified account"
        );
    }

    #[tokio::test]
    async fn different_tenant_does_not_collide_on_same_subject() {
        let store = store();
        let (a, b) = (tenant("tenant-a"), tenant("tenant-b"));
        let in_a = store
            .resolve_or_create(oauth(&a, "google", "g-1", Some("u@x.com"), true))
            .await
            .expect("resolve");
        let in_b = store
            .resolve_or_create(oauth(&b, "google", "g-1", Some("u@x.com"), true))
            .await
            .expect("resolve");
        assert_ne!(
            in_a.as_str(),
            in_b.as_str(),
            "the same provider subject in two tenants must be two users"
        );
    }

    #[tokio::test]
    async fn verified_email_link_is_tenant_scoped() {
        let store = store();
        let (a, b) = (tenant("tenant-a"), tenant("tenant-b"));
        let in_a = store
            .resolve_or_create(oauth(&a, "google", "g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        let in_b = store
            .resolve_or_create(oauth(&b, "github", "gh-9", Some("same@x.com"), true))
            .await
            .expect("resolve");
        assert_ne!(
            in_a.as_str(),
            in_b.as_str(),
            "a shared verified email must not link accounts across tenants"
        );
    }

    #[tokio::test]
    async fn different_provider_instance_does_not_collide() {
        let store = store();
        let t = tenant("t");
        let i1 = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-1", "actor-7"))
            .await
            .expect("resolve");
        let i2 = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-2", "actor-7"))
            .await
            .expect("resolve");
        assert_ne!(
            i1.as_str(),
            i2.as_str(),
            "the same actor id under two installations must be two users"
        );
    }

    #[tokio::test]
    async fn channel_actor_without_email_is_stable_and_distinct() {
        let store = store();
        let t = tenant("t");
        let a1 = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-1", "actor-1"))
            .await
            .expect("resolve");
        let a1_again = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-1", "actor-1"))
            .await
            .expect("resolve");
        let a2 = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-1", "actor-2"))
            .await
            .expect("resolve");
        assert_eq!(a1.as_str(), a1_again.as_str(), "same actor is stable");
        assert_ne!(
            a1.as_str(),
            a2.as_str(),
            "distinct actors are distinct users"
        );
    }

    #[tokio::test]
    async fn concurrent_first_logins_for_one_email_resolve_to_one_user() {
        let store = Arc::new(store());
        let (a, b) = (store.clone(), store.clone());
        let (ra, rb) = tokio::join!(
            tokio::spawn(async move {
                let t = tenant("t");
                a.resolve_or_create(oauth(&t, "google", "g-1", Some("dup@x.com"), true))
                    .await
            }),
            tokio::spawn(async move {
                let t = tenant("t");
                b.resolve_or_create(oauth(&t, "github", "gh-1", Some("dup@x.com"), true))
                    .await
            }),
        );
        let user_a = ra.expect("join").expect("resolve");
        let user_b = rb.expect("join").expect("resolve");
        assert_eq!(
            user_a.as_str(),
            user_b.as_str(),
            "concurrent first-logins for one verified email must share a user"
        );
    }

    #[tokio::test]
    async fn concurrent_first_logins_for_same_identity_resolve_to_one_user() {
        // Same exact key (tenant, surface, provider, instance, subject) raced
        // twice: the in-lock re-check must let the loser observe the winner's
        // record instead of minting a second user.
        let store = Arc::new(store());
        let (a, b) = (store.clone(), store.clone());
        let (ra, rb) = tokio::join!(
            tokio::spawn(async move {
                let t = tenant("t");
                a.resolve_or_create(oauth(&t, "google", "same-sub", Some("a@x.com"), true))
                    .await
            }),
            tokio::spawn(async move {
                let t = tenant("t");
                b.resolve_or_create(oauth(&t, "google", "same-sub", Some("a@x.com"), true))
                    .await
            }),
        );
        let user_a = ra.expect("join").expect("resolve");
        let user_b = rb.expect("join").expect("resolve");
        assert_eq!(
            user_a.as_str(),
            user_b.as_str(),
            "concurrent first-logins for the same identity key must share a user"
        );
    }

    #[tokio::test]
    async fn cross_process_first_logins_for_one_email_resolve_to_one_user() {
        // Two processes (separate lock maps, shared substrate) race a first
        // login for the same verified email through different providers. The
        // per-email lock is process-local, so both may pass the index read and
        // reach the create path; the verified-email index CAS is the
        // cross-process arbiter, and the loser must adopt the winner's user
        // rather than returning its own freshly minted one (a permanent
        // split). Repeated rounds widen the race window this guards.
        for round in 0..16 {
            let (p1, p2) = store_pair();
            let (p1, p2) = (Arc::new(p1), Arc::new(p2));
            let email = format!("dup{round}@x.com");
            let (e1, e2) = (email.clone(), email);
            let (r1, r2) = tokio::join!(
                tokio::spawn(async move {
                    let t = tenant("t");
                    p1.resolve_or_create(oauth(&t, "google", "g-1", Some(&e1), true))
                        .await
                }),
                tokio::spawn(async move {
                    let t = tenant("t");
                    p2.resolve_or_create(oauth(&t, "github", "gh-1", Some(&e2), true))
                        .await
                }),
            );
            let user_1 = r1.expect("join").expect("resolve");
            let user_2 = r2.expect("join").expect("resolve");
            assert_eq!(
                user_1.as_str(),
                user_2.as_str(),
                "round {round}: cross-process first-logins for one verified email must not split"
            );
        }
    }

    #[tokio::test]
    async fn resolve_writes_verified_email_index_before_returning() {
        // The index is written before the identity record, so a verified
        // resolve always leaves a readable index — the invariant the fast
        // path relies on to never return an identity with a missing index.
        let store = store();
        let t = tenant("t");
        store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("Indexed@X.com"), true))
            .await
            .expect("resolve");
        let index = store
            .read_record::<StoredVerifiedEmailIndex>(
                &FilesystemRebornIdentityStore::<InMemoryBackend>::verified_email_path(
                    "t",
                    "indexed@x.com",
                )
                .unwrap(),
            )
            .await
            .expect("read index");
        assert!(
            index.is_some(),
            "a verified resolve must persist the canonical verified-email index"
        );
    }

    #[tokio::test]
    async fn adopt_migrated_identity_preserves_user_and_links_verified_email() {
        let store = store();
        let t = tenant("t");
        // A legacy verified Google identity migrated with its original user id.
        store
            .adopt_migrated_identity(
                oauth(&t, "google", "g-legacy", Some("Legacy@X.com"), true),
                &UserId::new("legacy-user").unwrap(),
            )
            .await
            .expect("adopt");

        // Returning through the SAME legacy identity keeps the original id.
        let returning = store
            .resolve_or_create(oauth(&t, "google", "g-legacy", Some("legacy@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(returning.as_str(), "legacy-user");

        // A LATER login through a different provider with the same verified
        // email links to the migrated user via the seeded canonical index.
        let via_github = store
            .resolve_or_create(oauth(&t, "github", "gh-9", Some("legacy@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(
            via_github.as_str(),
            "legacy-user",
            "a migrated verified email must link a later different-provider login"
        );
    }

    #[tokio::test]
    async fn adopt_migrated_identity_does_not_clobber_a_live_record() {
        let store = store();
        let t = tenant("t");
        // A user resolved live first, minting their canonical record.
        let live = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("live@x.com"), true))
            .await
            .expect("resolve");
        // A one-time fold then runs for the same key with a stale legacy id;
        // the live canonical record must win.
        store
            .adopt_migrated_identity(
                oauth(&t, "google", "g-1", Some("live@x.com"), true),
                &UserId::new("stale-legacy-user").unwrap(),
            )
            .await
            .expect("adopt");
        let again = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("live@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(
            again.as_str(),
            live.as_str(),
            "migration must not clobber a record a returning user already created"
        );
    }

    #[tokio::test]
    async fn lookup_unbound_actor_returns_none() {
        let store = store();
        let resolved = store
            .lookup(channel_key(&tenant("t"), "slack", "U-unbound"))
            .await
            .expect("lookup");
        assert!(resolved.is_none(), "an unbound actor must fail closed");
    }

    #[tokio::test]
    async fn bind_then_lookup_returns_bound_user() {
        let store = store();
        let t = tenant("t");
        let user = UserId::new("reborn-user-7").expect("user");
        store
            .bind(channel_key(&t, "slack", "U-1"), &user)
            .await
            .expect("bind");
        let resolved = store
            .lookup(channel_key(&t, "slack", "U-1"))
            .await
            .expect("lookup");
        assert_eq!(resolved.as_ref().map(UserId::as_str), Some("reborn-user-7"));
    }

    #[tokio::test]
    async fn rebind_repoints_to_new_user() {
        let store = store();
        let t = tenant("t");
        store
            .bind(
                channel_key(&t, "slack", "U-1"),
                &UserId::new("user-a").unwrap(),
            )
            .await
            .expect("first bind");
        store
            .bind(
                channel_key(&t, "slack", "U-1"),
                &UserId::new("user-b").unwrap(),
            )
            .await
            .expect("rebind");
        let resolved = store
            .lookup(channel_key(&t, "slack", "U-1"))
            .await
            .expect("lookup");
        assert_eq!(
            resolved.as_ref().map(UserId::as_str),
            Some("user-b"),
            "re-binding the same key re-points it"
        );
    }

    #[tokio::test]
    async fn bind_is_scoped_per_tenant() {
        let store = store();
        let user = UserId::new("user-a").expect("user");
        store
            .bind(channel_key(&tenant("tenant-a"), "slack", "U-1"), &user)
            .await
            .expect("bind");
        let other = store
            .lookup(channel_key(&tenant("tenant-b"), "slack", "U-1"))
            .await
            .expect("lookup");
        assert!(
            other.is_none(),
            "a binding in one tenant is invisible in another"
        );
    }
}
