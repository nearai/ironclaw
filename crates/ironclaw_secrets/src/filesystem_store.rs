// arch-exempt: large_file, mechanical LocalFilesystem->DiskFilesystem Bucket-2 rename (arch-simplification §4.4), no logic change, plan #6168
//! Filesystem-backed implementations of the scoped secret and credential stores.
//!
//! Routes persistence through the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) surface so secrets,
//! credential accounts, and credential sessions all share the same dispatch
//! fabric as the rest of Reborn (`ironclaw_processes`, `ironclaw_authorization`,
//! `ironclaw_outbound`, `ironclaw_run_state`).
//!
//! All paths are alias-relative [`ScopedPath`] strings under the `/secrets`
//! mount alias. Tenant and user isolation are enforced structurally by the
//! caller-supplied [`MountView`](ironclaw_host_api::MountView): composition
//! wires `/secrets` → `/tenants/<tenant_id>/users/<user_id>/secrets`, so
//! storage code never has to encode or remember tenant/user identity. The
//! agent/project sub-scope remains in the path because secrets are partitioned
//! within a user's namespace by integration/project; the AAD (see
//! [`filesystem_secret_aad`](crate::filesystem_secret_aad)) binds ciphertext
//! to the same `(tenant, user, agent, project, handle)` tuple so cross-owner
//! reads fail closed both at the path layer and via decryption.
//!
//! Path layout (alias-relative):
//!
//! - `/secrets/agents/<agent>/projects/<project>/secrets/<handle>.json`
//! - `/secrets/agents/<agent>/projects/<project>/secret-leases/<lease_id>.json`
//! - `/secrets/agents/<agent>/projects/<project>/credential-accounts/<account_id>.json`
//! - `/secrets/agents/<agent>/projects/<project>/credential-sessions/<session_id>.json`
//!
//! `agents/<agent>` and `projects/<project>` segments are omitted when the
//! scope does not carry that field (mirroring the legacy
//! `secret_owner_root` shape so existing AAD bindings remain valid).
//!
//! Encryption-at-rest currently lives **inside this store** rather than as a
//! generic [`EncryptedBackend`] backend decorator. The
//! [`ironclaw_filesystem::CLAUDE.md`](../ironclaw_filesystem/CLAUDE.md) invariant
//! `5` describes the eventual destination: a backend wrapper that encrypts
//! `Entry::body` plus any `IndexValue::Bytes` projection. Until that decorator
//! lands, the same [`SecretsCrypto`] used by the libSQL/Postgres backends is
//! applied here so the on-disk material does not leak through any backend
//! mounted at `/secrets/*`. The crypto seam is intentionally narrow so the
//! decorator port can later strip these calls without touching trait surface.
//! TODO(reborn/fs-secrets): once `EncryptedBackend` ships, replace the inline
//! `encrypt`/`decrypt` calls with plaintext writes wrapped by the decorator.

use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FilesystemError, Filter,
    IndexKey, IndexKind, IndexName, IndexSpec, IndexValue, Page, RecordKind, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{HostApiError, ResourceScope, ScopedPath, SecretHandle, Timestamp};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{
    CredentialAccount, CredentialAccountId, CredentialAccountStatus, CredentialAccountStore,
    CredentialBrokerError, CredentialSession, CredentialSessionId, CredentialSessionStore,
    DEFAULT_SECRET_LEASE_TTL_SECONDS, SecretError, SecretLease, SecretLeaseId, SecretLeaseStatus,
    SecretMaterial, SecretMetadata, SecretStore, SecretStoreError, SecretsCrypto,
    credential_account_aad, credential_session_aad, filesystem_secret_aad,
};

// (Master-key sentinel constants and `KEY_CHECK_PATH` removed alongside
// `verify_can_decrypt_existing_secrets`; see comment in the impl block.)

// -- Record kind constants ---------------------------------------------------
//
// Every persisted entry must carry a `RecordKind` so that record-aware
// backends (Postgres, libSQL) can distinguish schema families and reject
// byte-only `CasExpectation::Absent` blind-write attempts that the
// `DiskFilesystem` byte-only backend would otherwise let through.

const SECRET_RECORD_KIND: &str = "secret_record";
const SECRET_LEASE_KIND: &str = "secret_lease";
const CREDENTIAL_ACCOUNT_KIND: &str = "credential_account";
const CREDENTIAL_SESSION_KIND: &str = "credential_session";

// -- Serialized DTOs --------------------------------------------------------
//
// These types are intentionally private. They carry encrypted payload bytes
// plus enough plaintext metadata to satisfy the trait surface (e.g. scope, id,
// status). The encrypted blob is opaque from the backend's perspective and
// nothing in this file ever writes plaintext secret material to the
// filesystem.

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSecret {
    scope: ResourceScope,
    handle: SecretHandle,
    /// AES-256-GCM ciphertext over the UTF-8 secret material.
    encrypted_value: Vec<u8>,
    /// Per-record HKDF salt.
    key_salt: Vec<u8>,
    /// Optional expiry, mirroring the legacy `Secret::expires_at` field.
    expires_at: Option<Timestamp>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredLease {
    scope: ResourceScope,
    handle: SecretHandle,
    lease_id: SecretLeaseId,
    status: SecretLeaseStatus,
    lease_expires_at: Timestamp,
    secret_expires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccount {
    scope: ResourceScope,
    id: CredentialAccountId,
    /// AES-256-GCM ciphertext over the JSON-encoded `CredentialAccount` body.
    /// We encrypt the entire account record because `secret_handles`,
    /// `allowed_targets`, and the redacted metadata blob all carry
    /// integration-shape information that operators may not want visible to
    /// anyone with raw storage access.
    encrypted_payload: Vec<u8>,
    key_salt: Vec<u8>,
    status: CredentialAccountStatus,
    updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StoredSession {
    /// AES-256-GCM ciphertext over the JSON-encoded
    /// `SerializableCredentialSession` body.
    encrypted_payload: Vec<u8>,
    key_salt: Vec<u8>,
    uses: u64,
}

// CredentialSession is intentionally not `Serialize`/`Deserialize` (its fields
// are private). We snapshot it into a private wire shape only inside this
// module before encrypting.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableCredentialSession {
    scope: ResourceScope,
    invocation_id: ironclaw_host_api::InvocationId,
    capability_id: ironclaw_host_api::CapabilityId,
    extension_id: ironclaw_host_api::ExtensionId,
    account_id: CredentialAccountId,
    secret_handles: Vec<SecretHandle>,
    allowed_targets: Vec<crate::CredentialTargetPolicy>,
    expires_at: Option<Timestamp>,
    max_uses: Option<u64>,
    correlation_id: String,
}

impl SerializableCredentialSession {
    fn from_session(session: &CredentialSession) -> Self {
        Self {
            scope: session.scope().clone(),
            invocation_id: session.invocation_id(),
            capability_id: session.capability_id().clone(),
            extension_id: session.extension_id().clone(),
            account_id: session.account_id().clone(),
            secret_handles: session.secret_handles().to_vec(),
            allowed_targets: session.allowed_targets().to_vec(),
            expires_at: session.expires_at(),
            max_uses: session.max_uses(),
            correlation_id: session.correlation_id().to_private_storage_string(),
        }
    }

    fn into_session(self) -> Result<CredentialSession, CredentialBrokerError> {
        let correlation_id = CredentialSessionId::parse(&self.correlation_id).map_err(|error| {
            CredentialBrokerError::BrokerUnavailable {
                reason: format!("invalid stored session id: {error}"),
            }
        })?;
        Ok(crate::__internal_session_for_filesystem_store(
            self.scope,
            self.invocation_id,
            self.capability_id,
            self.extension_id,
            self.account_id,
            self.secret_handles,
            self.allowed_targets,
            self.expires_at,
            self.max_uses,
            correlation_id,
        ))
    }
}

// -- FilesystemSecretStore --------------------------------------------------

/// Filesystem-backed [`SecretStore`].
///
/// Construct with a [`ScopedFilesystem`] over any [`RootFilesystem`].
/// Encryption is currently embedded (see the module docstring) and uses the
/// same [`SecretsCrypto`] as the libSQL/Postgres backends. Tenant and user
/// isolation are enforced by the caller's
/// [`MountView`](ironclaw_host_api::MountView): the `/secrets` alias resolves
/// to a per-tenant/per-user [`VirtualPath`](ironclaw_host_api::VirtualPath)
/// before any backend dispatch, so two stores wrapping the same backend with
/// different mounts cannot read each other's data.
pub struct FilesystemSecretStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    crypto: Arc<SecretsCrypto>,
    lease_ttl: Duration,
    tenant_index_roots: Mutex<HashSet<String>>,
}

impl<F> FilesystemSecretStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, crypto: Arc<SecretsCrypto>) -> Self {
        Self {
            filesystem,
            crypto,
            lease_ttl: Duration::seconds(DEFAULT_SECRET_LEASE_TTL_SECONDS),
            tenant_index_roots: Mutex::new(HashSet::new()),
        }
    }

    pub fn with_lease_ttl(
        filesystem: Arc<ScopedFilesystem<F>>,
        crypto: Arc<SecretsCrypto>,
        lease_ttl: Duration,
    ) -> Self {
        Self {
            filesystem,
            crypto,
            lease_ttl,
            tenant_index_roots: Mutex::new(HashSet::new()),
        }
    }

    // The FS-stored master-key sentinel and `verify_can_decrypt_existing_secrets`
    // method that used to live here were removed when the per-tenant
    // `ScopedFilesystem` design landed: the sentinel record would have moved to
    // a per-tenant path, so the startup-readiness check could no longer be a
    // single process-wide signal. The master key is sourced from
    // config/env (`secret_master_key: SecretMaterial` in composition); a
    // wrong key surfaces on the first per-tenant decrypt op rather than at
    // startup. See PR #3679 / 2026-05-16 design discussion.

    async fn read_secret(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<StoredSecret>, SecretStoreError> {
        let path = secret_path(scope, handle)?;
        let Some(versioned) = self
            .filesystem
            .get(scope, &path)
            .await
            .map_err(fs_to_secret_store_error)?
        else {
            return Ok(None);
        };
        let stored: StoredSecret = deserialize_secret(&versioned.entry.body)?;
        if !same_scope_owner(&stored.scope, scope) || &stored.handle != handle {
            return Ok(None);
        }
        Ok(Some(stored))
    }

    async fn write_secret(&self, secret: &StoredSecret) -> Result<(), SecretStoreError> {
        let path = secret_path(&secret.scope, &secret.handle)?;
        let body = serialize_secret(secret)?;
        let kind = RecordKind::new(SECRET_RECORD_KIND).map_err(|error| {
            SecretStoreError::StoreUnavailable {
                reason: format!("invalid secret record kind: {error}"),
            }
        })?;
        let mut base_entry = Entry::bytes(body).with_content_type(ContentType::json());
        base_entry.kind = Some(kind);
        let entry = tag_entry_with_tenant(base_entry, &secret.scope);
        self.ensure_tenant_id_index(&secret.scope).await?;
        self.filesystem
            .put(&secret.scope, &path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_secret_store_error)
    }

    async fn write_lease(&self, lease: &StoredLease) -> Result<(), SecretStoreError> {
        let path = lease_path(&lease.scope, lease.lease_id)?;
        let entry = serialize_lease_entry(lease)?;
        self.ensure_tenant_id_index(&lease.scope).await?;
        self.filesystem
            .put(&lease.scope, &path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_secret_store_error)
    }

    async fn ensure_tenant_id_index(&self, scope: &ResourceScope) -> Result<(), SecretStoreError> {
        let root = scoped_path_secret("/secrets")?;
        let resolved_root = self
            .filesystem
            .resolve(scope, &root)
            .map_err(fs_to_secret_store_error)?
            .as_str()
            .to_string();
        {
            let roots = self.tenant_index_roots.lock().await;
            if roots.contains(&resolved_root) {
                return Ok(());
            }
        }
        ensure_tenant_id_index_secret(&self.filesystem, scope).await?;
        let mut roots = self.tenant_index_roots.lock().await;
        roots.insert(resolved_root);
        Ok(())
    }

    fn lease_to_public(stored: &StoredLease) -> SecretLease {
        SecretLease {
            id: stored.lease_id,
            scope: stored.scope.clone(),
            handle: stored.handle.clone(),
            status: stored.status,
        }
    }

    fn effective_status(stored: &StoredLease, now: Timestamp) -> SecretLeaseStatus {
        match stored.status {
            SecretLeaseStatus::Active => {
                let lease_expired = stored.lease_expires_at <= now;
                let secret_expired = stored
                    .secret_expires_at
                    .is_some_and(|expires_at| expires_at <= now);
                if lease_expired || secret_expired {
                    SecretLeaseStatus::Expired
                } else {
                    SecretLeaseStatus::Active
                }
            }
            other => other,
        }
    }
}

#[async_trait]
impl<F> SecretStore for FilesystemSecretStore<F>
where
    F: RootFilesystem,
{
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        let plaintext = material.expose_secret().as_bytes();
        let aad = filesystem_secret_aad(&scope, &handle);
        let (encrypted_value, key_salt) = self
            .crypto
            .encrypt(plaintext, &aad)
            .map_err(secret_error_to_store_error)?;
        let now = Utc::now();
        let stored = StoredSecret {
            scope: scope.clone(),
            handle: handle.clone(),
            encrypted_value,
            key_salt,
            expires_at,
            created_at: now,
            updated_at: now,
        };
        self.write_secret(&stored).await?;
        Ok(SecretMetadata {
            scope,
            handle,
            expires_at,
        })
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok(self
            .read_secret(scope, handle)
            .await?
            .map(|stored| SecretMetadata {
                scope: stored.scope,
                handle: stored.handle,
                expires_at: stored.expires_at,
            }))
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        let root = secret_owner_root(scope)?;
        let mut offset = 0;
        let mut metadata = Vec::new();
        loop {
            let entries = match self
                .filesystem
                .query(
                    scope,
                    &root,
                    &Filter::All,
                    Page::new(offset, Page::MAX_LIMIT),
                )
                .await
            {
                Ok(entries) => entries,
                Err(error) if is_not_found(&error) => return Ok(metadata),
                Err(error) => return Err(fs_to_secret_store_error(error)),
            };
            let entry_count = entries.len();
            for versioned in entries {
                let stored: StoredSecret = deserialize_secret(&versioned.entry.body)?;
                if same_scope_owner(&stored.scope, scope) {
                    metadata.push(SecretMetadata {
                        scope: stored.scope,
                        handle: stored.handle,
                        expires_at: stored.expires_at,
                    });
                }
            }
            if entry_count < Page::MAX_LIMIT as usize {
                return Ok(metadata);
            }
            offset += entry_count as u64;
        }
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        let path = secret_path(scope, handle)?;
        match self.filesystem.delete(scope, &path).await {
            Ok(()) => Ok(true),
            Err(error) if is_not_found(&error) => Ok(false),
            Err(error) => Err(fs_to_secret_store_error(error)),
        }
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        let stored = self.read_secret(scope, handle).await?.ok_or_else(|| {
            SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            }
        })?;
        if let Some(expires_at) = stored.expires_at
            && expires_at <= Utc::now()
        {
            return Err(SecretStoreError::SecretExpired);
        }
        let lease_id = SecretLeaseId::new();
        let lease = StoredLease {
            scope: scope.clone(),
            handle: handle.clone(),
            lease_id,
            status: SecretLeaseStatus::Active,
            lease_expires_at: Utc::now() + self.lease_ttl,
            secret_expires_at: stored.expires_at,
        };
        self.write_lease(&lease).await?;
        Ok(Self::lease_to_public(&lease))
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        let path = lease_path(scope, lease_id)?;
        // `cas_update` calls apply as `FnMut` on each CAS retry; clone scope
        // inside each invocation so the async block can move it without
        // consuming a captured variable that must outlive the retry loop.
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            deserialize_secret::<StoredLease>,
            |lease: &StoredLease| serialize_lease_entry(lease),
            |current: Option<StoredLease>| {
                let scope_iter = scope.clone();
                let self_ref = self;
                async move {
                    let lease = current.ok_or_else(|| unknown_lease(&scope_iter, lease_id))?;
                    if !same_scope_for_lease(&lease.scope, &scope_iter) {
                        return Err(unknown_lease(&scope_iter, lease_id));
                    }
                    match Self::effective_status(&lease, Utc::now()) {
                        SecretLeaseStatus::Consumed => {
                            Err(SecretStoreError::LeaseConsumed { lease_id })
                        }
                        SecretLeaseStatus::Revoked => {
                            Err(SecretStoreError::LeaseRevoked { lease_id })
                        }
                        SecretLeaseStatus::Expired => {
                            let already_marked = lease.status == SecretLeaseStatus::Expired;
                            if already_marked {
                                // Already expired: surface LeaseExpired via the apply error
                                // path. No write is issued because the closure returns Err
                                // (this is not the PartialEq unchanged-snapshot skip path).
                                Err(SecretStoreError::LeaseExpired { lease_id })
                            } else {
                                // Best-effort expiry promotion: write the updated
                                // status, then surface LeaseExpired. On
                                // VersionMismatch retry `already_marked` will be
                                // true so the helper terminates without a second
                                // write. Either way the caller sees LeaseExpired.
                                let mut updated = lease;
                                updated.status = SecretLeaseStatus::Expired;
                                Ok(CasApply::new(
                                    updated,
                                    Err::<SecretMaterial, _>(SecretStoreError::LeaseExpired {
                                        lease_id,
                                    }),
                                ))
                            }
                        }
                        SecretLeaseStatus::Active => {
                            let stored = self_ref
                                .read_secret(&scope_iter, &lease.handle)
                                .await?
                                .ok_or_else(|| SecretStoreError::UnknownSecret {
                                    scope: Box::new(scope_iter.clone()),
                                    handle: lease.handle.clone(),
                                })?;
                            let aad = filesystem_secret_aad(&scope_iter, &lease.handle);
                            let decrypted = self_ref
                                .crypto
                                .decrypt(&stored.encrypted_value, &stored.key_salt, &aad)
                                .map_err(secret_error_to_store_error)?;
                            let material = SecretMaterial::from(decrypted.expose().to_string());
                            let mut updated = lease;
                            updated.status = SecretLeaseStatus::Consumed;
                            Ok(CasApply::new(updated, Ok(material)))
                        }
                    }
                }
            },
        )
        .await
        .map_err(|cas_err| map_cas_error_secret(cas_err, "consume"))
        // The apply closure returns `Result<SecretMaterial, SecretStoreError>`
        // as the outcome so the BestEffortCommit-equivalent expired-promotion
        // path can write AND surface an error. The outer `Ok` here means the
        // CAS loop succeeded (including writing the expiry marker); unwrap the
        // inner error to get the caller-facing `SecretStoreError`.
        .and_then(|result| result)
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        let path = lease_path(scope, lease_id)?;
        let scope_clone = scope.clone();
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            deserialize_secret::<StoredLease>,
            |lease: &StoredLease| serialize_lease_entry(lease),
            |current: Option<StoredLease>| {
                let outcome = (|| {
                    let lease = current.ok_or_else(|| unknown_lease(&scope_clone, lease_id))?;
                    if !same_scope_for_lease(&lease.scope, &scope_clone) {
                        return Err(unknown_lease(&scope_clone, lease_id));
                    }
                    // Idempotent on terminal states: revoking an already-Consumed
                    // or already-Revoked lease succeeds without rewriting the
                    // marker, so a race with `consume` cannot overwrite the
                    // Consumed signal. Return the UNCHANGED lease so the helper
                    // skips the write (PartialEq no-op path).
                    match lease.status {
                        SecretLeaseStatus::Consumed
                        | SecretLeaseStatus::Revoked
                        | SecretLeaseStatus::Expired => {
                            let public = Self::lease_to_public(&lease);
                            Ok(CasApply::new(lease, public))
                        }
                        SecretLeaseStatus::Active => {
                            // Promote a stale Active lease to Expired before
                            // revoking, mirroring the in-memory adapter's
                            // `expire_stale_active_leases` step.
                            let mut updated = lease;
                            updated.status = if Self::effective_status(&updated, Utc::now())
                                == SecretLeaseStatus::Expired
                            {
                                SecretLeaseStatus::Expired
                            } else {
                                SecretLeaseStatus::Revoked
                            };
                            let public = Self::lease_to_public(&updated);
                            Ok(CasApply::new(updated, public))
                        }
                    }
                })();
                async move { outcome }
            },
        )
        .await
        .map_err(|cas_err| map_cas_error_secret(cas_err, "revoke"))
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        // TODO(perf): this is an N+1 scan — list_dir over the per-owner
        // lease root followed by one `get` per lease entry. The cardinality
        // is bounded because `lease_root` already encodes the full owner
        // prefix (tenant/user/[agent]/[project]), so only the missions /
        // threads / invocations under that owner contribute, and active
        // one-shot leases are short-lived (TTL `DEFAULT_SECRET_LEASE_TTL_SECONDS`).
        // If we add an index here it should sit on a composite scope-derived
        // key (mission_id + thread_id + invocation_id) and route through
        // `RootFilesystem::query` with `Filter::Eq` — the secrets store
        // currently declares no indexes (no `ensure_*` calls in `new`), so
        // adding that path is a follow-up. Until then, the list+get fan-out
        // is acceptable because N is bounded by the owner prefix.
        let root = lease_root(scope)?;
        let entries = match self.filesystem.list_dir(scope, &root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_secret_store_error(error)),
        };
        let now = Utc::now();
        let mut leases = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            // `list_dir` returned a `VirtualPath`; reconstruct the equivalent
            // `ScopedPath` under our prefix so the per-op ACL on the follow-up
            // `get` still runs against the caller's MountView.
            let scoped_child = join_scoped_secret(&root, &entry.name)?;
            let Some(versioned) = self
                .filesystem
                .get(scope, &scoped_child)
                .await
                .map_err(fs_to_secret_store_error)?
            else {
                continue;
            };
            let mut stored: StoredLease = deserialize_secret(&versioned.entry.body)?;
            if !same_scope_for_lease(&stored.scope, scope) {
                continue;
            }
            stored.status = Self::effective_status(&stored, now);
            leases.push(Self::lease_to_public(&stored));
        }
        Ok(leases)
    }
}

// -- FilesystemCredentialBroker --------------------------------------------

/// Filesystem-backed implementation of [`CredentialAccountStore`] and
/// [`CredentialSessionStore`].
///
/// One concrete type backs both traits because production callers wire a single
/// broker (matching `InMemoryCredentialBroker`). The shared broker also keeps
/// the account/session foreign-key relationship intact on the filesystem
/// (sessions read accounts on `create_session` analogues outside this crate).
pub struct FilesystemCredentialBroker<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    crypto: Arc<SecretsCrypto>,
    tenant_index_roots: Mutex<HashSet<String>>,
}

impl<F> FilesystemCredentialBroker<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, crypto: Arc<SecretsCrypto>) -> Self {
        Self {
            filesystem,
            crypto,
            tenant_index_roots: Mutex::new(HashSet::new()),
        }
    }

    fn encrypt_payload(
        &self,
        value: &impl Serialize,
        aad: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), CredentialBrokerError> {
        let bytes = serde_json::to_vec(value).map_err(|error| {
            CredentialBrokerError::BrokerUnavailable {
                reason: format!("failed to serialize credential payload: {error}"),
            }
        })?;
        self.crypto
            .encrypt(&bytes, aad)
            .map_err(secret_error_to_broker_error)
    }

    fn decrypt_payload<T>(
        &self,
        payload: &[u8],
        salt: &[u8],
        aad: &[u8],
    ) -> Result<T, CredentialBrokerError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let decrypted = self
            .crypto
            .decrypt(payload, salt, aad)
            .map_err(secret_error_to_broker_error)?;
        serde_json::from_str(decrypted.expose()).map_err(|error| {
            CredentialBrokerError::BrokerUnavailable {
                reason: format!("failed to deserialize credential payload: {error}"),
            }
        })
    }

    async fn ensure_tenant_id_index(
        &self,
        scope: &ResourceScope,
    ) -> Result<(), CredentialBrokerError> {
        let root = scoped_path_broker("/secrets")?;
        let resolved_root = self
            .filesystem
            .resolve(scope, &root)
            .map_err(fs_to_broker_error)?
            .as_str()
            .to_string();
        {
            let roots = self.tenant_index_roots.lock().await;
            if roots.contains(&resolved_root) {
                return Ok(());
            }
        }
        ensure_tenant_id_index_broker(&self.filesystem, scope).await?;
        let mut roots = self.tenant_index_roots.lock().await;
        roots.insert(resolved_root);
        Ok(())
    }
}

#[async_trait]
impl<F> CredentialAccountStore for FilesystemCredentialBroker<F>
where
    F: RootFilesystem,
{
    async fn put_account(
        &self,
        account: CredentialAccount,
    ) -> Result<CredentialAccount, CredentialBrokerError> {
        let aad = credential_account_aad(&account.scope, &account.id);
        let (encrypted_payload, key_salt) = self.encrypt_payload(&account, &aad)?;
        let stored = StoredAccount {
            scope: account.scope.clone(),
            id: account.id.clone(),
            encrypted_payload,
            key_salt,
            status: account.status,
            updated_at: account.updated_at,
        };
        let path = credential_account_path(&account.scope, &account.id)?;
        let body = serialize_credential(&stored)?;
        let kind = RecordKind::new(CREDENTIAL_ACCOUNT_KIND).map_err(|error| {
            CredentialBrokerError::BrokerUnavailable {
                reason: format!("invalid credential account record kind: {error}"),
            }
        })?;
        let mut base_entry = Entry::bytes(body).with_content_type(ContentType::json());
        base_entry.kind = Some(kind);
        let entry = tag_entry_with_tenant(base_entry, &account.scope);
        self.ensure_tenant_id_index(&account.scope).await?;
        self.filesystem
            .put(&account.scope, &path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_broker_error)?;
        Ok(account)
    }

    async fn get_account(
        &self,
        scope: &ResourceScope,
        account_id: &CredentialAccountId,
    ) -> Result<Option<CredentialAccount>, CredentialBrokerError> {
        let path = credential_account_path(scope, account_id)?;
        let Some(versioned) = self
            .filesystem
            .get(scope, &path)
            .await
            .map_err(fs_to_broker_error)?
        else {
            return Ok(None);
        };
        let stored: StoredAccount = deserialize_credential(&versioned.entry.body)?;
        if !same_scope_owner(&stored.scope, scope) || &stored.id != account_id {
            return Ok(None);
        }
        let aad = credential_account_aad(scope, account_id);
        let account = self.decrypt_payload::<CredentialAccount>(
            &stored.encrypted_payload,
            &stored.key_salt,
            &aad,
        )?;
        Ok(Some(account))
    }

    async fn accounts_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<CredentialAccount>, CredentialBrokerError> {
        let root = credential_account_root(scope)?;
        let entries = match self.filesystem.list_dir(scope, &root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_broker_error(error)),
        };
        let mut accounts = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            // `list_dir` returned a `VirtualPath`; reconstruct the equivalent
            // `ScopedPath` under our prefix so the per-op ACL on the follow-up
            // `get` still runs against the caller's MountView.
            let scoped_child = join_scoped_broker(&root, &entry.name)?;
            let Some(versioned) = self
                .filesystem
                .get(scope, &scoped_child)
                .await
                .map_err(fs_to_broker_error)?
            else {
                continue;
            };
            let stored: StoredAccount = deserialize_credential(&versioned.entry.body)?;
            if !same_scope_owner(&stored.scope, scope) {
                continue;
            }
            let aad = credential_account_aad(&stored.scope, &stored.id);
            let account = self.decrypt_payload::<CredentialAccount>(
                &stored.encrypted_payload,
                &stored.key_salt,
                &aad,
            )?;
            accounts.push(account);
        }
        Ok(accounts)
    }
}

#[async_trait]
impl<F> CredentialSessionStore for FilesystemCredentialBroker<F>
where
    F: RootFilesystem,
{
    async fn issue_session(
        &self,
        session: CredentialSession,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let wire = SerializableCredentialSession::from_session(&session);
        let aad = credential_session_aad(session.scope(), session.correlation_id());
        let (encrypted_payload, key_salt) = self.encrypt_payload(&wire, &aad)?;
        let stored = StoredSession {
            encrypted_payload,
            key_salt,
            uses: 0,
        };
        let path = credential_session_path(session.scope(), session.correlation_id())?;
        let entry = serialize_session_entry(&stored, session.scope())?;
        self.ensure_tenant_id_index(session.scope()).await?;
        self.filesystem
            .put(session.scope(), &path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_broker_error)?;
        Ok(session)
    }

    async fn get_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
    ) -> Result<Option<CredentialSession>, CredentialBrokerError> {
        let path = credential_session_path(scope, session_id)?;
        let Some(versioned) = self
            .filesystem
            .get(scope, &path)
            .await
            .map_err(fs_to_broker_error)?
        else {
            return Ok(None);
        };
        let stored: StoredSession = deserialize_credential(&versioned.entry.body)?;
        let aad = credential_session_aad(scope, session_id);
        let wire: SerializableCredentialSession =
            self.decrypt_payload(&stored.encrypted_payload, &stored.key_salt, &aad)?;
        if wire.scope != *scope {
            return Ok(None);
        }
        Ok(Some(wire.into_session()?))
    }

    async fn validate_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let path = credential_session_path(scope, session_id)?;
        let Some(versioned) = self
            .filesystem
            .get(scope, &path)
            .await
            .map_err(fs_to_broker_error)?
        else {
            return Err(CredentialBrokerError::UnknownSession { session_id });
        };
        let stored: StoredSession = deserialize_credential(&versioned.entry.body)?;
        let aad = credential_session_aad(scope, session_id);
        let wire: SerializableCredentialSession =
            self.decrypt_payload(&stored.encrypted_payload, &stored.key_salt, &aad)?;
        if wire.scope != *scope {
            return Err(CredentialBrokerError::UnknownSession { session_id });
        }
        ensure_stored_session_usable(&wire, stored.uses, session_id, now)?;
        wire.into_session()
    }

    async fn consume_session_use(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let path = credential_session_path(scope, session_id)?;
        // Capture a cloned scope so the encode closure can borrow it without
        // conflicting with the apply closure that also needs it. The apply
        // closure clones once more per retry (FnMut requirement).
        let scope_for_encode = scope.clone();
        cas_update(
            self.filesystem.as_ref(),
            scope,
            &path,
            deserialize_credential::<StoredSession>,
            |stored: &StoredSession| serialize_session_entry(stored, &scope_for_encode),
            |current: Option<StoredSession>| {
                let scope_iter = scope.clone();
                let self_ref = self;
                async move {
                    let stored =
                        current.ok_or(CredentialBrokerError::UnknownSession { session_id })?;
                    let aad = credential_session_aad(&scope_iter, session_id);
                    let wire: SerializableCredentialSession = self_ref.decrypt_payload(
                        &stored.encrypted_payload,
                        &stored.key_salt,
                        &aad,
                    )?;
                    if wire.scope != scope_iter {
                        return Err(CredentialBrokerError::UnknownSession { session_id });
                    }
                    ensure_stored_session_usable(&wire, stored.uses, session_id, now)?;
                    let session = wire.into_session()?;
                    let mut updated = stored;
                    updated.uses += 1;
                    Ok(CasApply::new(updated, session))
                }
            },
        )
        .await
        .map_err(|cas_err| map_cas_error_broker(cas_err, "use"))
    }
}

// -- Paths ------------------------------------------------------------------
//
// All paths returned here are alias-relative [`ScopedPath`] strings under the
// `/secrets` mount alias. Tenant and user identity are NOT encoded in the
// path — the caller's [`MountView`] resolves `/secrets` to a
// tenant/user-scoped [`VirtualPath`](ironclaw_host_api::VirtualPath) before
// any backend dispatch, so two stores sharing one backend but constructed
// with different MountViews cannot collide on identical (agent, project,
// handle) tuples.
//
// The agent/project segments remain in the path because secrets are
// partitioned within a user's namespace by integration/project; AAD
// (`filesystem_secret_aad`) binds the same `(tenant, user, agent, project,
// handle)` tuple so cross-owner reads fail closed both at the path layer and
// at decrypt.

fn secret_path(
    scope: &ResourceScope,
    handle: &SecretHandle,
) -> Result<ScopedPath, SecretStoreError> {
    scoped_path_secret(&format!(
        "{}/secrets/{}.json",
        secret_owner_alias(scope),
        handle.as_str()
    ))
}

fn lease_path(
    scope: &ResourceScope,
    lease_id: SecretLeaseId,
) -> Result<ScopedPath, SecretStoreError> {
    scoped_path_secret(&format!("{}/{lease_id}.json", lease_root(scope)?.as_str()))
}

fn lease_root(scope: &ResourceScope) -> Result<ScopedPath, SecretStoreError> {
    scoped_path_secret(&format!("{}/secret-leases", secret_owner_alias(scope)))
}

/// Alias-relative root prefix for the `secrets/` subdirectory under a
/// given owner scope. Used as the `prefix` argument to
/// [`ScopedFilesystem::ensure_index`] so the `tenant_id` projection is
/// declared on the same subtree the corresponding writes land in.
fn secret_owner_root(scope: &ResourceScope) -> Result<ScopedPath, SecretStoreError> {
    scoped_path_secret(&format!("{}/secrets", secret_owner_alias(scope)))
}

fn credential_account_path(
    scope: &ResourceScope,
    account_id: &CredentialAccountId,
) -> Result<ScopedPath, CredentialBrokerError> {
    scoped_path_broker(&format!(
        "{}/{}.json",
        credential_account_root(scope)?.as_str(),
        account_id.as_str()
    ))
}

fn credential_account_root(scope: &ResourceScope) -> Result<ScopedPath, CredentialBrokerError> {
    scoped_path_broker(&format!(
        "{}/credential-accounts",
        secret_owner_alias(scope)
    ))
}

fn credential_session_path(
    scope: &ResourceScope,
    session_id: CredentialSessionId,
) -> Result<ScopedPath, CredentialBrokerError> {
    scoped_path_broker(&format!(
        "{}/credential-sessions/{}.json",
        secret_owner_alias(scope),
        session_id.to_private_storage_string()
    ))
}

/// Build the alias-relative owner prefix for a scope, starting from the
/// `/secrets` mount alias. Tenant and user are intentionally absent — they
/// live in the MountView the caller supplied.
fn secret_owner_alias(scope: &ResourceScope) -> String {
    let mut base = String::from("/secrets");
    if let Some(agent_id) = &scope.agent_id {
        base.push_str("/agents/");
        base.push_str(agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base.push_str("/projects/");
        base.push_str(project_id.as_str());
    }
    base
}

fn scoped_path_secret(raw: &str) -> Result<ScopedPath, SecretStoreError> {
    ScopedPath::new(raw).map_err(host_api_to_secret_store_error)
}

fn scoped_path_broker(raw: &str) -> Result<ScopedPath, CredentialBrokerError> {
    ScopedPath::new(raw).map_err(host_api_to_broker_error)
}

/// Join a leaf segment onto a `ScopedPath` prefix. Mirrors the engine's
/// `join_scoped` helper: `list_dir` returns
/// [`VirtualPath`](ironclaw_host_api::VirtualPath) results (post-resolution),
/// but the follow-up `get` must run through the `ScopedFilesystem` so the
/// per-op ACL is enforced — so callers strip the leaf name and rejoin it
/// onto the original `ScopedPath` prefix.
fn join_scoped_secret(prefix: &ScopedPath, leaf: &str) -> Result<ScopedPath, SecretStoreError> {
    scoped_path_secret(&format!(
        "{}/{}",
        prefix.as_str().trim_end_matches('/'),
        leaf
    ))
}

fn join_scoped_broker(
    prefix: &ScopedPath,
    leaf: &str,
) -> Result<ScopedPath, CredentialBrokerError> {
    scoped_path_broker(&format!(
        "{}/{}",
        prefix.as_str().trim_end_matches('/'),
        leaf
    ))
}

// -- Scope predicates -------------------------------------------------------

fn same_scope_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
}

fn same_scope_for_lease(left: &ResourceScope, right: &ResourceScope) -> bool {
    same_scope_owner(left, right)
        && left.mission_id == right.mission_id
        && left.thread_id == right.thread_id
        && left.invocation_id == right.invocation_id
}

fn ensure_stored_session_usable(
    wire: &SerializableCredentialSession,
    uses: u64,
    session_id: CredentialSessionId,
    now: Timestamp,
) -> Result<(), CredentialBrokerError> {
    if wire.expires_at.is_some_and(|expires_at| expires_at <= now) {
        return Err(CredentialBrokerError::SessionExpired { session_id });
    }
    if wire.max_uses.is_some_and(|max_uses| uses >= max_uses) {
        return Err(CredentialBrokerError::SessionUseLimitExceeded { session_id });
    }
    Ok(())
}

// -- CAS error mapping helpers ----------------------------------------------

fn serialize_lease_entry(lease: &StoredLease) -> Result<Entry, SecretStoreError> {
    let body = serialize_secret(lease)?;
    let kind =
        RecordKind::new(SECRET_LEASE_KIND).map_err(|error| SecretStoreError::StoreUnavailable {
            reason: format!("invalid secret lease record kind: {error}"),
        })?;
    let mut base_entry = Entry::bytes(body).with_content_type(ContentType::json());
    base_entry.kind = Some(kind);
    Ok(tag_entry_with_tenant(base_entry, &lease.scope))
}

fn serialize_session_entry(
    stored: &StoredSession,
    scope: &ResourceScope,
) -> Result<Entry, CredentialBrokerError> {
    let body = serialize_credential(stored)?;
    let kind = RecordKind::new(CREDENTIAL_SESSION_KIND).map_err(|error| {
        CredentialBrokerError::BrokerUnavailable {
            reason: format!("invalid credential session record kind: {error}"),
        }
    })?;
    let mut base_entry = Entry::bytes(body).with_content_type(ContentType::json());
    base_entry.kind = Some(kind);
    Ok(tag_entry_with_tenant(base_entry, scope))
}

fn unknown_lease(scope: &ResourceScope, lease_id: SecretLeaseId) -> SecretStoreError {
    SecretStoreError::UnknownLease {
        scope: Box::new(scope.clone()),
        lease_id,
    }
}

/// Map the shared CAS helper's [`CasUpdateError`] into a [`SecretStoreError`].
///
/// `op` names the operation (e.g. `"consume"`, `"revoke"`) for the
/// retries-exhausted reason string; this matches the stable wire strings
/// locked in by the retry-exhaustion tests.
fn map_cas_error_secret(error: CasUpdateError<SecretStoreError>, op: &str) -> SecretStoreError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout | CasUpdateError::RetriesExhausted => {
            SecretStoreError::StoreUnavailable {
                reason: format!("secret lease {op} retry limit exceeded"),
            }
        }
        CasUpdateError::CasUnsupported => SecretStoreError::StoreUnavailable {
            reason: "backend does not support versioned compare-and-swap".to_string(),
        },
        CasUpdateError::Backend(fs_err) => fs_to_secret_store_error(fs_err),
    }
}

/// Map the shared CAS helper's [`CasUpdateError`] into a [`CredentialBrokerError`].
fn map_cas_error_broker(
    error: CasUpdateError<CredentialBrokerError>,
    op: &str,
) -> CredentialBrokerError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout | CasUpdateError::RetriesExhausted => {
            CredentialBrokerError::BrokerUnavailable {
                reason: format!("credential session {op} retry limit exceeded"),
            }
        }
        CasUpdateError::CasUnsupported => CredentialBrokerError::BrokerUnavailable {
            reason: "backend does not support versioned compare-and-swap".to_string(),
        },
        CasUpdateError::Backend(fs_err) => fs_to_broker_error(fs_err),
    }
}

// -- Error mapping ----------------------------------------------------------

fn fs_to_secret_store_error(error: FilesystemError) -> SecretStoreError {
    // The crate guardrail forbids leaking backend error detail strings through
    // secret-related metadata or audit records, but the existing
    // SecretStoreError::StoreUnavailable variant already carries a string
    // (the libSQL/Postgres backends populate it with `error.to_string()`).
    // Preserve parity: we collapse the FilesystemError into the same
    // sanitized envelope. FilesystemError variants do not include host paths
    // (they hold VirtualPath/ScopedPath only), so this does not leak host
    // paths.
    match error {
        FilesystemError::NotFound { .. } => SecretStoreError::StoreUnavailable {
            reason: "filesystem entry missing".to_string(),
        },
        FilesystemError::PermissionDenied { .. } => SecretStoreError::StoreUnavailable {
            reason: "filesystem permission denied".to_string(),
        },
        FilesystemError::VersionMismatch { .. } => SecretStoreError::StoreUnavailable {
            reason: "filesystem version mismatch".to_string(),
        },
        other => SecretStoreError::StoreUnavailable {
            reason: format!(
                "filesystem error: {}",
                sanitize_error_kind(other.to_string())
            ),
        },
    }
}

fn fs_to_broker_error(error: FilesystemError) -> CredentialBrokerError {
    CredentialBrokerError::BrokerUnavailable {
        reason: format!(
            "filesystem error: {}",
            sanitize_error_kind(error.to_string())
        ),
    }
}

fn host_api_to_secret_store_error(error: HostApiError) -> SecretStoreError {
    SecretStoreError::StoreUnavailable {
        reason: format!("invalid filesystem path: {error}"),
    }
}

fn host_api_to_broker_error(error: HostApiError) -> CredentialBrokerError {
    CredentialBrokerError::BrokerUnavailable {
        reason: format!("invalid filesystem path: {error}"),
    }
}

fn secret_error_to_store_error(error: SecretError) -> SecretStoreError {
    match error {
        SecretError::Expired => SecretStoreError::SecretExpired,
        SecretError::InvalidMasterKey => SecretStoreError::BackendMisconfigured {
            reason: "secrets master key unavailable".to_string(),
        },
        other => SecretStoreError::StoreUnavailable {
            reason: format!(
                "secret cryptography failure: {}",
                sanitize_error_kind(other.to_string())
            ),
        },
    }
}

fn secret_error_to_broker_error(error: SecretError) -> CredentialBrokerError {
    CredentialBrokerError::BrokerUnavailable {
        reason: format!(
            "credential cryptography failure: {}",
            sanitize_error_kind(error.to_string())
        ),
    }
}

fn serialize_secret<T>(value: &T) -> Result<Vec<u8>, SecretStoreError>
where
    T: Serialize,
{
    serde_json::to_vec(value).map_err(|error| SecretStoreError::StoreUnavailable {
        reason: format!("failed to serialize secret entry: {error}"),
    })
}

fn deserialize_secret<T>(bytes: &[u8]) -> Result<T, SecretStoreError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| SecretStoreError::StoreUnavailable {
        reason: format!("failed to deserialize secret entry: {error}"),
    })
}

fn serialize_credential<T>(value: &T) -> Result<Vec<u8>, CredentialBrokerError>
where
    T: Serialize,
{
    serde_json::to_vec(value).map_err(|error| CredentialBrokerError::BrokerUnavailable {
        reason: format!("failed to serialize credential entry: {error}"),
    })
}

fn deserialize_credential<T>(bytes: &[u8]) -> Result<T, CredentialBrokerError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice(bytes).map_err(|error| CredentialBrokerError::BrokerUnavailable {
        reason: format!("failed to deserialize credential entry: {error}"),
    })
}

fn is_not_found(error: &FilesystemError) -> bool {
    matches!(error, FilesystemError::NotFound { .. })
}

// ── Indexed projections (defense in depth) ─────────────────────
//
// Path-prefix scoping via the caller's [`MountView`] is the primary
// tenant-isolation boundary; the indexed `tenant_id` projection here is
// belt-and-suspenders so an admin-tier query can filter explicitly by
// tenant, and a path-rewriting bug surfaces as a query-time mismatch
// rather than silent cross-tenant leakage. See
// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.

/// Index key under which the tenant id is projected on every secret /
/// credential record write. Production secrets/credentials never read by
/// this key directly — path-prefix scoping handles routing — but it is
/// available for admin-tier queries and as a defense-in-depth shield
/// against MountView misconfiguration.
fn index_key_tenant_id() -> IndexKey {
    IndexKey::new("tenant_id").unwrap_or_else(|_| {
        unreachable!("secrets index key `tenant_id` must be a simple identifier")
    })
}

fn index_name_secrets_tenant() -> IndexName {
    IndexName::new("secrets_by_tenant").unwrap_or_else(|_| {
        unreachable!("secrets index name `secrets_by_tenant` must be a simple identifier")
    })
}

/// Decorate `entry` with a `tenant_id` indexed projection scoped to
/// `scope.tenant_id`. Callers compose this with the existing
/// `with_content_type` decoration so encrypted-at-rest bodies still
/// carry the defense-in-depth tenant axis.
fn tag_entry_with_tenant(entry: Entry, scope: &ResourceScope) -> Entry {
    entry.with_indexed(
        index_key_tenant_id(),
        IndexValue::Text(scope.tenant_id.as_str().to_string()),
    )
}

/// Declare the `tenant_id` exact-equality index on the `/secrets` mount,
/// tolerating backends that don't materialize indexes (DiskFilesystem).
/// Idempotent across the mount lifetime and avoids per-owner DDL churn under
/// concurrent secret/lease writes.
async fn ensure_tenant_id_index_secret<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
) -> Result<(), SecretStoreError>
where
    F: RootFilesystem,
{
    let spec = IndexSpec::new(
        index_name_secrets_tenant(),
        vec![index_key_tenant_id()],
        IndexKind::Exact,
    );
    let root = scoped_path_secret("/secrets")?;
    match filesystem.ensure_index(scope, &root, &spec).await {
        Ok(()) => Ok(()),
        Err(FilesystemError::Unsupported { .. }) => Ok(()),
        Err(error) => Err(fs_to_secret_store_error(error)),
    }
}

async fn ensure_tenant_id_index_broker<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
) -> Result<(), CredentialBrokerError>
where
    F: RootFilesystem,
{
    let spec = IndexSpec::new(
        index_name_secrets_tenant(),
        vec![index_key_tenant_id()],
        IndexKind::Exact,
    );
    let root = scoped_path_broker("/secrets")?;
    match filesystem.ensure_index(scope, &root, &spec).await {
        Ok(()) => Ok(()),
        Err(FilesystemError::Unsupported { .. }) => Ok(()),
        Err(error) => Err(fs_to_broker_error(error)),
    }
}

// Light scrubber: drops anything that looks like an absolute host path. The
// secrets crate guardrails forbid leaking host paths through audit/error
// records; the filesystem layer already keeps host paths internal, but this
// belt-and-braces step prevents accidental leakage if a future backend grows
// less disciplined error formatting.
fn sanitize_error_kind(reason: String) -> String {
    if reason.contains("/Users/") || reason.contains("/home/") || reason.contains("/tmp/") {
        "redacted backend failure".to_string()
    } else {
        reason
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::Utc;
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{
        AgentId, CapabilityId, ExtensionId, InvocationId, MissionId, MountAlias, MountGrant,
        MountPermissions, MountView, NetworkMethod, ProjectId, ResourceScope, SecretHandle,
        TenantId, ThreadId, UserId, VirtualPath,
    };
    use secrecy::ExposeSecret;
    use serde_json::json;

    use super::*;
    use crate::{
        CredentialAccountId, CredentialAccountStatus, CredentialPathPolicy, CredentialTargetPolicy,
        InMemoryCredentialBroker, RedactedJson,
    };

    fn test_crypto() -> Arc<SecretsCrypto> {
        Arc::new(
            SecretsCrypto::new(SecretMaterial::from(
                "0123456789abcdef0123456789abcdef".to_string(),
            ))
            .expect("master key length is valid"),
        )
    }

    /// Build a `ScopedFilesystem` over `backend` whose `/secrets` alias
    /// resolves to a tenant/user-scoped [`VirtualPath`] subtree. Tests pass
    /// different `target_root` values to simulate distinct
    /// (tenant, user) tuples sharing one underlying backend — exactly the
    /// shape composition produces in production.
    fn build_scoped_fs<B>(backend: Arc<B>, target_root: &str) -> Arc<ScopedFilesystem<B>>
    where
        B: RootFilesystem,
    {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/secrets").expect("alias"),
            VirtualPath::new(target_root).expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    /// Default test mount: `/secrets` → tenant-scoped target. Mirrors the
    /// production composition shape (`/secrets` →
    /// `/tenants/<tenant>/users/<user>/secrets`).
    fn default_scoped_fs<B>(backend: Arc<B>) -> Arc<ScopedFilesystem<B>>
    where
        B: RootFilesystem,
    {
        build_scoped_fs(backend, "/secrets/tenants/test/users/test/secrets")
    }

    fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("project-a").unwrap()),
            mission_id: Some(MissionId::new("mission-a").unwrap()),
            thread_id: Some(ThreadId::new("thread-a").unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    fn sample_account(
        scope: ResourceScope,
        id: CredentialAccountId,
        secret_handle: SecretHandle,
    ) -> CredentialAccount {
        CredentialAccount {
            scope,
            id,
            provider_or_extension_id: ExtensionId::new("openai").unwrap(),
            label: "Production".to_string(),
            status: CredentialAccountStatus::Active,
            secret_handles: vec![secret_handle],
            allowed_targets: vec![CredentialTargetPolicy {
                scheme: "https".to_string(),
                host: "api.example.com".to_string(),
                port: Some(443),
                path: CredentialPathPolicy::Prefix("/v1/".to_string()),
                methods: vec![NetworkMethod::Get],
            }],
            redacted_metadata: RedactedJson::new(json!({"last_four": "1234"})),
            updated_at: Utc::now(),
        }
    }

    struct DeleteCountingBackend {
        inner: Arc<InMemoryBackend>,
        get_calls: AtomicUsize,
        delete_calls: AtomicUsize,
    }

    impl DeleteCountingBackend {
        fn new(inner: Arc<InMemoryBackend>) -> Self {
            Self {
                inner,
                get_calls: AtomicUsize::new(0),
                delete_calls: AtomicUsize::new(0),
            }
        }

        fn get_calls(&self) -> usize {
            self.get_calls.load(Ordering::SeqCst)
        }

        fn delete_calls(&self) -> usize {
            self.delete_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl RootFilesystem for DeleteCountingBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.inner.capabilities()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.get(path).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.delete_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.delete(path).await
        }

        async fn query(
            &self,
            path: &VirtualPath,
            filter: &Filter,
            page: Page,
        ) -> Result<Vec<VersionedEntry>, FilesystemError> {
            self.inner.query(path, filter, page).await
        }

        async fn ensure_index(
            &self,
            path: &VirtualPath,
            spec: &IndexSpec,
        ) -> Result<(), FilesystemError> {
            self.inner.ensure_index(path, spec).await
        }
    }

    #[tokio::test]
    async fn filesystem_secret_store_round_trips_material() {
        let fs = Arc::new(InMemoryBackend::new());
        let scoped = default_scoped_fs(Arc::clone(&fs));
        let store = FilesystemSecretStore::new(scoped, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();

        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
                None,
            )
            .await
            .unwrap();

        assert!(store.metadata(&scope, &handle).await.unwrap().is_some());

        let lease = store.lease_once(&scope, &handle).await.unwrap();
        let material = store.consume(&scope, lease.id).await.unwrap();
        assert_eq!(material.expose_secret(), "super-secret");

        let second = store.consume(&scope, lease.id).await.unwrap_err();
        assert!(second.is_consumed());
    }

    #[tokio::test]
    async fn filesystem_secret_store_concurrent_consume_has_exactly_one_winner() {
        const CONSUMERS: usize = 32;

        let fs = Arc::new(InMemoryBackend::new());
        let store = Arc::new(FilesystemSecretStore::new(
            default_scoped_fs(fs),
            test_crypto(),
        ));
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("one-shot-secret"),
                None,
            )
            .await
            .unwrap();
        let lease = store.lease_once(&scope, &handle).await.unwrap();
        let lease_id = lease.id;
        let barrier = Arc::new(tokio::sync::Barrier::new(CONSUMERS));

        let mut tasks = Vec::with_capacity(CONSUMERS);
        for _ in 0..CONSUMERS {
            let store = Arc::clone(&store);
            let scope = scope.clone();
            let barrier = Arc::clone(&barrier);
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                store.consume(&scope, lease_id).await
            }));
        }

        let mut successes = 0;
        let mut consumed = 0;
        for task in tasks {
            match task.await.expect("consumer task should not panic") {
                Ok(material) => {
                    successes += 1;
                    assert_eq!(material.expose_secret(), "one-shot-secret");
                }
                Err(error) if error.is_consumed() => consumed += 1,
                Err(error) => panic!("unexpected consume error: {error:?}"),
            }
        }

        assert_eq!(successes, 1, "only one concurrent consume may succeed");
        assert_eq!(
            consumed,
            CONSUMERS - 1,
            "every losing consumer must observe LeaseConsumed"
        );
    }

    #[tokio::test]
    async fn filesystem_secret_store_lists_metadata_for_scope_owner() {
        let fs = Arc::new(InMemoryBackend::new());
        let scoped = default_scoped_fs(Arc::clone(&fs));
        let store = FilesystemSecretStore::new(scoped, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let mut other_project_scope = scope.clone();
        other_project_scope.project_id = Some(ProjectId::new("project-b").unwrap());

        store
            .put(
                scope.clone(),
                SecretHandle::new("api_key").unwrap(),
                SecretMaterial::from("secret-a"),
                None,
            )
            .await
            .unwrap();
        store
            .put(
                scope.clone(),
                SecretHandle::new("model_key").unwrap(),
                SecretMaterial::from("secret-b"),
                None,
            )
            .await
            .unwrap();
        store
            .put(
                other_project_scope,
                SecretHandle::new("other_project_key").unwrap(),
                SecretMaterial::from("secret-c"),
                None,
            )
            .await
            .unwrap();

        let mut handles: Vec<_> = store
            .metadata_for_scope(&scope)
            .await
            .unwrap()
            .into_iter()
            .map(|metadata| metadata.handle.as_str().to_string())
            .collect();
        handles.sort();

        assert_eq!(handles, vec!["api_key", "model_key"]);
    }

    /// Operator-wide secrets are stored under [`ResourceScope::system`], whose
    /// reserved tenant/user id carries control bytes that normal `TenantId`
    /// validation rejects. The persisted `StoredSecret` tags the entry with
    /// that scope, so a read-back deserializes it — this must round-trip.
    /// Regression for the WebUI NEAR AI save returning `service_unavailable`:
    /// the key wrote but every read-back (metadata/lease) errored on the
    /// system tenant id.
    #[tokio::test]
    async fn filesystem_secret_store_round_trips_system_scope() {
        let fs = Arc::new(InMemoryBackend::new());
        let scoped = default_scoped_fs(Arc::clone(&fs));
        let store = FilesystemSecretStore::new(scoped, test_crypto());
        let scope = ResourceScope::system();
        let handle = SecretHandle::new("llm_provider_nearai_api_key").unwrap();

        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("sk-operator-wide"),
                None,
            )
            .await
            .unwrap();

        assert!(store.metadata(&scope, &handle).await.unwrap().is_some());
        let lease = store.lease_once(&scope, &handle).await.unwrap();
        let material = store.consume(&scope, lease.id).await.unwrap();
        assert_eq!(material.expose_secret(), "sk-operator-wide");
    }

    #[tokio::test]
    async fn filesystem_secret_store_delete_skips_pre_read() {
        let backend = Arc::new(DeleteCountingBackend::new(Arc::new(InMemoryBackend::new())));
        let store =
            FilesystemSecretStore::new(default_scoped_fs(Arc::clone(&backend)), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();

        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
                None,
            )
            .await
            .unwrap();
        let get_calls_before_delete = backend.get_calls();

        let removed = store.delete(&scope, &handle).await.unwrap();

        assert!(removed);
        assert_eq!(
            backend.get_calls(),
            get_calls_before_delete,
            "delete should not add a pre-read before the backend delete path"
        );
        assert_eq!(backend.delete_calls(), 1);
    }

    #[tokio::test]
    async fn filesystem_secret_store_encrypts_at_rest() {
        let fs = Arc::new(InMemoryBackend::new());
        let scoped = default_scoped_fs(Arc::clone(&fs));
        let store = FilesystemSecretStore::new(Arc::clone(&scoped), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();

        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("plaintext-sentinel-7e3d"),
                None,
            )
            .await
            .unwrap();

        // Resolve the alias-relative ScopedPath to its backing VirtualPath
        // through the same MountView the store uses, so the at-rest check
        // reads exactly the bytes the backend stored.
        let scoped_path = secret_path(&scope, &handle).unwrap();
        let virtual_path = scoped.resolve(&scope, &scoped_path).unwrap();
        let versioned = fs
            .get(&virtual_path)
            .await
            .unwrap()
            .expect("entry persisted");
        let raw = String::from_utf8_lossy(&versioned.entry.body);
        assert!(
            !raw.contains("plaintext-sentinel-7e3d"),
            "secret material must be encrypted at rest"
        );
    }

    /// Within a single store (one MountView, one tenant/user), distinct
    /// project scopes still produce disjoint paths — `secret_owner_alias`
    /// encodes the project segment under `/secrets/agents/.../projects/<id>`
    /// — so a lease issued under project-A cannot be consumed under
    /// project-B even though tenant/user agree.
    #[tokio::test]
    async fn filesystem_secret_store_isolates_projects_within_same_mount() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(default_scoped_fs(fs), test_crypto());
        let mut scope_project_a = sample_scope("tenant-a", "user-a");
        scope_project_a.project_id = Some(ProjectId::new("project-a").unwrap());
        let mut scope_project_b = scope_project_a.clone();
        scope_project_b.project_id = Some(ProjectId::new("project-b").unwrap());
        let handle = SecretHandle::new("shared_name").unwrap();

        store
            .put(
                scope_project_a.clone(),
                handle.clone(),
                SecretMaterial::from("aaa"),
                None,
            )
            .await
            .unwrap();
        store
            .put(
                scope_project_b.clone(),
                handle.clone(),
                SecretMaterial::from("bbb"),
                None,
            )
            .await
            .unwrap();

        let lease_a = store.lease_once(&scope_project_a, &handle).await.unwrap();
        let cross = store
            .consume(&scope_project_b, lease_a.id)
            .await
            .unwrap_err();
        assert!(cross.is_unknown_lease());
    }

    #[tokio::test]
    async fn filesystem_secret_store_revoke_blocks_consume() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(default_scoped_fs(fs), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
                None,
            )
            .await
            .unwrap();

        let lease = store.lease_once(&scope, &handle).await.unwrap();
        store.revoke(&scope, lease.id).await.unwrap();
        let error = store.consume(&scope, lease.id).await.unwrap_err();
        assert!(error.is_revoked());
    }

    /// F2 regression: when `consume` wins the race against `revoke`, the
    /// consumed marker must survive — the late-arriving `revoke` must observe
    /// the Consumed state and become a no-op rather than overwriting it with
    /// Revoked. Before the fix, `revoke` read with no version, computed the
    /// new status from the stale `Active` snapshot it observed, and wrote
    /// back with `CasExpectation::Any`, silently clobbering the consume.
    #[tokio::test]
    async fn filesystem_secret_store_revoke_after_consume_is_idempotent() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(default_scoped_fs(fs), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
                None,
            )
            .await
            .unwrap();

        let lease = store.lease_once(&scope, &handle).await.unwrap();
        // Consume first, then revoke. The revoke must not overwrite the
        // Consumed status, and the returned lease must still report
        // Consumed so external observers see the terminal state.
        let _material = store.consume(&scope, lease.id).await.unwrap();
        let revoked = store.revoke(&scope, lease.id).await.unwrap();
        assert_eq!(
            revoked.status,
            SecretLeaseStatus::Consumed,
            "F2: revoke after consume must remain Consumed, not promote to Revoked"
        );
        // Calling revoke again is still idempotent.
        let revoked_again = store.revoke(&scope, lease.id).await.unwrap();
        assert_eq!(revoked_again.status, SecretLeaseStatus::Consumed);
    }

    /// F2 regression: revoking an already-Revoked lease is idempotent and
    /// does not rewrite the record.
    #[tokio::test]
    async fn filesystem_secret_store_revoke_is_idempotent_on_revoked() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(default_scoped_fs(fs), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
                None,
            )
            .await
            .unwrap();

        let lease = store.lease_once(&scope, &handle).await.unwrap();
        let first = store.revoke(&scope, lease.id).await.unwrap();
        let second = store.revoke(&scope, lease.id).await.unwrap();
        assert_eq!(first.status, SecretLeaseStatus::Revoked);
        assert_eq!(second.status, SecretLeaseStatus::Revoked);
    }

    #[tokio::test]
    async fn filesystem_secret_store_missing_secret_does_not_create_lease() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(default_scoped_fs(fs), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("missing").unwrap();

        let error = store.lease_once(&scope, &handle).await.unwrap_err();
        assert!(error.is_unknown_secret());
        assert!(store.leases_for_scope(&scope).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn filesystem_credential_broker_round_trips_account_and_session() {
        let fs = Arc::new(InMemoryBackend::new());
        let broker = FilesystemCredentialBroker::new(default_scoped_fs(fs), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("openai_prod").unwrap();
        let account = sample_account(
            scope.clone(),
            account_id.clone(),
            SecretHandle::new("openai_key").unwrap(),
        );

        broker.put_account(account.clone()).await.unwrap();
        let fetched = broker
            .get_account(&scope, &account_id)
            .await
            .unwrap()
            .expect("account persisted");
        assert_eq!(fetched, account);

        let in_memory = InMemoryCredentialBroker::new();
        in_memory.put_account(account.clone()).unwrap();
        let session = in_memory
            .create_session(crate::CredentialSessionRequest {
                scope: scope.clone(),
                invocation_id: scope.invocation_id,
                capability_id: CapabilityId::new("openai.chat").unwrap(),
                extension_id: ExtensionId::new("openai").unwrap(),
                account_id: account_id.clone(),
                method: NetworkMethod::Get,
                url: "https://api.example.com/v1/models".to_string(),
                expires_at: Some(Utc::now() + chrono::Duration::seconds(60)),
                max_uses: Some(2),
            })
            .unwrap();
        let issued = broker.issue_session(session.clone()).await.unwrap();
        let correlation = issued.correlation_id();

        let fetched_session = broker
            .get_session(&scope, correlation)
            .await
            .unwrap()
            .expect("session persisted");
        assert_eq!(fetched_session.account_id(), &account_id);
        broker
            .validate_session(&scope, correlation, Utc::now())
            .await
            .unwrap();
        broker
            .consume_session_use(&scope, correlation, Utc::now())
            .await
            .unwrap();
        broker
            .consume_session_use(&scope, correlation, Utc::now())
            .await
            .unwrap();
        let limit_error = broker
            .consume_session_use(&scope, correlation, Utc::now())
            .await
            .unwrap_err();
        assert!(limit_error.is_use_limit_exceeded());
    }

    #[tokio::test]
    async fn filesystem_credential_broker_account_at_rest_is_encrypted() {
        let fs = Arc::new(InMemoryBackend::new());
        let scoped = default_scoped_fs(Arc::clone(&fs));
        let broker = FilesystemCredentialBroker::new(Arc::clone(&scoped), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("github_prod").unwrap();
        let mut account = sample_account(
            scope.clone(),
            account_id.clone(),
            SecretHandle::new("github_key").unwrap(),
        );
        account.label = "leak-sentinel-92ab".to_string();
        broker.put_account(account).await.unwrap();

        let scoped_path = credential_account_path(&scope, &account_id).unwrap();
        let virtual_path = scoped.resolve(&scope, &scoped_path).unwrap();
        let versioned = fs
            .get(&virtual_path)
            .await
            .unwrap()
            .expect("entry persisted");
        let raw = String::from_utf8_lossy(&versioned.entry.body);
        assert!(
            !raw.contains("leak-sentinel-92ab"),
            "credential account label must be encrypted at rest"
        );
    }

    // ─── CAS retry regression tests (PR #3679 review fix) ───────────────
    //
    // The next two tests simulate a concurrent multi-process writer landing
    // between the read and the CAS write inside `consume` /
    // `consume_session_use`. They wrap `InMemoryBackend` with a
    // `VersionRacingBackend` that, on the first `put` against the watched
    // path with a `CasExpectation::Version(_)`, bumps the stored version
    // out-of-band before delegating. The delegated put then fails with
    // `FilesystemError::VersionMismatch`, the retry loop re-reads, and the
    // second attempt succeeds.

    use std::sync::Arc as StdArc;
    use std::sync::atomic::AtomicBool;

    use ironclaw_filesystem::{
        BackendCapabilities, DirEntry, FileStat, Filter, IndexSpec, Page, RecordVersion,
        RootFilesystem, VersionedEntry,
    };

    struct VersionRacingBackend {
        inner: StdArc<InMemoryBackend>,
        watched: String,
        raced: AtomicBool,
    }

    impl VersionRacingBackend {
        fn new(inner: StdArc<InMemoryBackend>, watched: VirtualPath) -> Self {
            Self {
                inner,
                watched: watched.as_str().to_string(),
                raced: AtomicBool::new(false),
            }
        }

        fn raced(&self) -> bool {
            self.raced.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl RootFilesystem for VersionRacingBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.inner.capabilities()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            // Only race the first `put` with a versioned CAS against the
            // watched path. Everything else (initial `Any` writes, retries,
            // unrelated paths) goes through untouched.
            let should_race = path.as_str() == self.watched
                && matches!(cas, CasExpectation::Version(_))
                && !self.raced.swap(true, Ordering::SeqCst);
            if should_race && let Some(current) = self.inner.get(path).await? {
                // Out-of-band write under `Any` to bump the path's stored
                // version. This is what a competing process's `put` would
                // look like to our backend — the entry body is a copy of
                // the current entry so the lease state itself doesn't
                // change, only the version moves forward. After the OOB
                // bump the caller's CAS version is stale, so the delegated
                // put below will return VersionMismatch — exactly the
                // contention shape the retry loop must absorb.
                let _ = self
                    .inner
                    .put(path, current.entry, CasExpectation::Any)
                    .await;
            }
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.inner.get(path).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }

        async fn query(
            &self,
            path: &VirtualPath,
            filter: &Filter,
            page: Page,
        ) -> Result<Vec<VersionedEntry>, FilesystemError> {
            self.inner.query(path, filter, page).await
        }

        async fn ensure_index(
            &self,
            path: &VirtualPath,
            spec: &IndexSpec,
        ) -> Result<(), FilesystemError> {
            self.inner.ensure_index(path, spec).await
        }
    }

    /// Sibling of [`VersionRacingBackend`] for CAS-exhaustion tests: races
    /// *every* versioned `put` against the watched path rather than only the
    /// first. Used to drive [`cas_update`] past its [`FILESYSTEM_CAS_RETRIES`]
    /// budget so the exhaustion branch and its caller-visible reason string are
    /// exercised end-to-end.
    struct AlwaysRacingBackend {
        inner: StdArc<InMemoryBackend>,
        watched: String,
        races: AtomicUsize,
    }

    impl AlwaysRacingBackend {
        fn new(inner: StdArc<InMemoryBackend>, watched: VirtualPath) -> Self {
            Self {
                inner,
                watched: watched.as_str().to_string(),
                races: AtomicUsize::new(0),
            }
        }

        fn races(&self) -> usize {
            self.races.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl RootFilesystem for AlwaysRacingBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.inner.capabilities()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            let should_race =
                path.as_str() == self.watched && matches!(cas, CasExpectation::Version(_));
            if should_race && let Some(current) = self.inner.get(path).await? {
                self.races.fetch_add(1, Ordering::SeqCst);
                let _ = self
                    .inner
                    .put(path, current.entry, CasExpectation::Any)
                    .await;
            }
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.inner.get(path).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }

        async fn query(
            &self,
            path: &VirtualPath,
            filter: &Filter,
            page: Page,
        ) -> Result<Vec<VersionedEntry>, FilesystemError> {
            self.inner.query(path, filter, page).await
        }

        async fn ensure_index(
            &self,
            path: &VirtualPath,
            spec: &IndexSpec,
        ) -> Result<(), FilesystemError> {
            self.inner.ensure_index(path, spec).await
        }
    }

    /// Sibling of [`VersionRacingBackend`]/[`AlwaysRacingBackend`] for the
    /// fail-closed CAS-unsupported test (#5234 review follow-up): advertises
    /// a *known* capability shape without `TxnCapability::Cas` so
    /// `cas_update`'s pre-flight gate refuses up front with
    /// `CasUpdateError::CasUnsupported`, while every actual op still
    /// delegates to `inner` unchanged. `BackendCapabilities::bytes_only()`
    /// is the right shape here — it sets enough flags to be "known"
    /// (`capabilities_known` compares against the all-zero default) while
    /// leaving `txn` at its `TxnCapability::None` default, so the op-time
    /// CAS-support check fails.
    struct CasUnsupportedBackend {
        inner: StdArc<InMemoryBackend>,
    }

    impl CasUnsupportedBackend {
        fn new(inner: StdArc<InMemoryBackend>) -> Self {
            Self { inner }
        }
    }

    #[async_trait]
    impl RootFilesystem for CasUnsupportedBackend {
        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::bytes_only()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.inner.get(path).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }

        async fn query(
            &self,
            path: &VirtualPath,
            filter: &Filter,
            page: Page,
        ) -> Result<Vec<VersionedEntry>, FilesystemError> {
            self.inner.query(path, filter, page).await
        }

        async fn ensure_index(
            &self,
            path: &VirtualPath,
            spec: &IndexSpec,
        ) -> Result<(), FilesystemError> {
            self.inner.ensure_index(path, spec).await
        }
    }

    #[tokio::test]
    async fn filesystem_secret_store_consume_retries_on_version_mismatch() {
        // First write the lease through a plain backend so the lease and
        // secret material exist, then swap in the racing backend so the
        // very next `put` (the consume's CAS write) hits a forced
        // VersionMismatch.
        let inner = StdArc::new(InMemoryBackend::new());
        let bootstrap_scoped = default_scoped_fs(StdArc::clone(&inner));
        let bootstrap_store = FilesystemSecretStore::new(bootstrap_scoped, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        bootstrap_store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret-cas"),
                None,
            )
            .await
            .unwrap();
        let lease = bootstrap_store.lease_once(&scope, &handle).await.unwrap();

        // The racing backend watches the post-resolution VirtualPath, so
        // resolve the alias-relative ScopedPath through the same MountView
        // shape composition uses in production.
        let scoped_lease = lease_path(&scope, lease.id).unwrap();
        let watched = bootstrap_store
            .filesystem
            .resolve(&scope, &scoped_lease)
            .unwrap();
        let racing = StdArc::new(VersionRacingBackend::new(StdArc::clone(&inner), watched));
        let racing_scoped = default_scoped_fs(StdArc::clone(&racing));
        let racing_store = FilesystemSecretStore::new(racing_scoped, test_crypto());

        let material = racing_store.consume(&scope, lease.id).await.unwrap();
        assert_eq!(material.expose_secret(), "super-secret-cas");
        assert!(
            racing.raced(),
            "racing backend must have observed the first put and bumped the version"
        );
        // Lease is now consumed; a second consume must return LeaseConsumed,
        // proving the retried CAS write actually persisted the new state.
        let second = racing_store.consume(&scope, lease.id).await.unwrap_err();
        assert!(second.is_consumed());
    }

    #[tokio::test]
    async fn filesystem_broker_consume_session_use_retries_on_version_mismatch() {
        let inner = StdArc::new(InMemoryBackend::new());
        let bootstrap_scoped = default_scoped_fs(StdArc::clone(&inner));
        let bootstrap_broker = FilesystemCredentialBroker::new(bootstrap_scoped, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("openai_cas").unwrap();
        let account = sample_account(
            scope.clone(),
            account_id.clone(),
            SecretHandle::new("openai_cas_key").unwrap(),
        );
        bootstrap_broker.put_account(account.clone()).await.unwrap();

        let in_memory = InMemoryCredentialBroker::new();
        in_memory.put_account(account).unwrap();
        let session = in_memory
            .create_session(crate::CredentialSessionRequest {
                scope: scope.clone(),
                invocation_id: scope.invocation_id,
                capability_id: CapabilityId::new("openai.chat").unwrap(),
                extension_id: ExtensionId::new("openai").unwrap(),
                account_id: account_id.clone(),
                method: NetworkMethod::Get,
                url: "https://api.example.com/v1/models".to_string(),
                expires_at: Some(Utc::now() + chrono::Duration::seconds(60)),
                max_uses: Some(3),
            })
            .unwrap();
        bootstrap_broker
            .issue_session(session.clone())
            .await
            .unwrap();
        let correlation = session.correlation_id();
        let scoped_session_path = credential_session_path(&scope, correlation).unwrap();
        let watched = bootstrap_broker
            .filesystem
            .resolve(&scope, &scoped_session_path)
            .unwrap();

        let racing = StdArc::new(VersionRacingBackend::new(StdArc::clone(&inner), watched));
        let racing_scoped = default_scoped_fs(StdArc::clone(&racing));
        let racing_broker = FilesystemCredentialBroker::new(racing_scoped, test_crypto());

        racing_broker
            .consume_session_use(&scope, correlation, Utc::now())
            .await
            .unwrap();
        assert!(
            racing.raced(),
            "racing backend must have observed the first put and bumped the version"
        );

        // After the retried increment lands, two further legitimate
        // consumes should bring `uses` to 3 (the max) and the next call
        // must fail with the use-limit error — proving the retry actually
        // wrote the incremented counter rather than dropping it.
        racing_broker
            .consume_session_use(&scope, correlation, Utc::now())
            .await
            .unwrap();
        racing_broker
            .consume_session_use(&scope, correlation, Utc::now())
            .await
            .unwrap();
        let exceeded = racing_broker
            .consume_session_use(&scope, correlation, Utc::now())
            .await
            .unwrap_err();
        assert!(exceeded.is_use_limit_exceeded());
    }

    /// `revoke` shares the same CAS retry loop as `consume`. A single
    /// out-of-band version bump between read and write must be absorbed by
    /// the helper, and the second attempt must land the `Revoked` marker.
    /// Mirrors `filesystem_secret_store_consume_retries_on_version_mismatch`.
    #[tokio::test]
    async fn filesystem_secret_store_revoke_retries_on_version_mismatch() {
        let inner = StdArc::new(InMemoryBackend::new());
        let bootstrap_scoped = default_scoped_fs(StdArc::clone(&inner));
        let bootstrap_store = FilesystemSecretStore::new(bootstrap_scoped, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        bootstrap_store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret-revoke-cas"),
                None,
            )
            .await
            .unwrap();
        let lease = bootstrap_store.lease_once(&scope, &handle).await.unwrap();

        let scoped_lease = lease_path(&scope, lease.id).unwrap();
        let watched = bootstrap_store
            .filesystem
            .resolve(&scope, &scoped_lease)
            .unwrap();
        let racing = StdArc::new(VersionRacingBackend::new(StdArc::clone(&inner), watched));
        let racing_scoped = default_scoped_fs(StdArc::clone(&racing));
        let racing_store = FilesystemSecretStore::new(racing_scoped, test_crypto());

        let revoked = racing_store.revoke(&scope, lease.id).await.unwrap();
        assert_eq!(revoked.status, SecretLeaseStatus::Revoked);
        assert!(
            racing.raced(),
            "racing backend must have observed the first put and bumped the version"
        );

        // The retried CAS write actually persisted the Revoked status —
        // a follow-up consume must see Revoked, not Active.
        let blocked = racing_store.consume(&scope, lease.id).await.unwrap_err();
        assert!(blocked.is_revoked());
    }

    /// A backend that races *every* versioned put exhausts the CAS retry
    /// budget. `revoke` must surface this as
    /// `StoreUnavailable { reason: "secret lease revoke retry limit exceeded" }`.
    /// The wire string is part of the caller-visible contract — locking it
    /// in here catches regressions like #3880's first review pass, where the
    /// session-use exhaustion reason silently changed during refactor.
    #[tokio::test]
    async fn filesystem_secret_store_revoke_exhausts_cas_retry_budget() {
        let inner = StdArc::new(InMemoryBackend::new());
        let bootstrap_scoped = default_scoped_fs(StdArc::clone(&inner));
        let bootstrap_store = FilesystemSecretStore::new(bootstrap_scoped, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        bootstrap_store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret-revoke-exhaust"),
                None,
            )
            .await
            .unwrap();
        let lease = bootstrap_store.lease_once(&scope, &handle).await.unwrap();

        let scoped_lease = lease_path(&scope, lease.id).unwrap();
        let watched = bootstrap_store
            .filesystem
            .resolve(&scope, &scoped_lease)
            .unwrap();
        let racing = StdArc::new(AlwaysRacingBackend::new(StdArc::clone(&inner), watched));
        let racing_scoped = default_scoped_fs(StdArc::clone(&racing));
        let racing_store = FilesystemSecretStore::new(racing_scoped, test_crypto());

        let error = racing_store.revoke(&scope, lease.id).await.unwrap_err();
        match error {
            SecretStoreError::StoreUnavailable { reason } => assert_eq!(
                reason, "secret lease revoke retry limit exceeded",
                "wire string for revoke retry exhaustion must be stable"
            ),
            other => panic!("expected StoreUnavailable, got {other:?}"),
        }
        assert_eq!(
            racing.races(),
            ironclaw_filesystem::FILESYSTEM_CAS_RETRIES,
            "each retry attempt must have been raced"
        );
    }

    /// `consume_session_use` exhaustion path: locks in
    /// `BrokerUnavailable { reason: "credential session use retry limit exceeded" }`
    /// against regressions like the one caught in #3880 review where the
    /// helper-op string drifted from `"use"` to `"consume use"`.
    #[tokio::test]
    async fn filesystem_broker_consume_session_use_exhausts_cas_retry_budget() {
        let inner = StdArc::new(InMemoryBackend::new());
        let bootstrap_scoped = default_scoped_fs(StdArc::clone(&inner));
        let bootstrap_broker = FilesystemCredentialBroker::new(bootstrap_scoped, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("openai_exhaust").unwrap();
        let account = sample_account(
            scope.clone(),
            account_id.clone(),
            SecretHandle::new("openai_exhaust_key").unwrap(),
        );
        bootstrap_broker.put_account(account.clone()).await.unwrap();

        let in_memory = InMemoryCredentialBroker::new();
        in_memory.put_account(account).unwrap();
        let session = in_memory
            .create_session(crate::CredentialSessionRequest {
                scope: scope.clone(),
                invocation_id: scope.invocation_id,
                capability_id: CapabilityId::new("openai.chat").unwrap(),
                extension_id: ExtensionId::new("openai").unwrap(),
                account_id: account_id.clone(),
                method: NetworkMethod::Get,
                url: "https://api.example.com/v1/models".to_string(),
                expires_at: Some(Utc::now() + chrono::Duration::seconds(60)),
                max_uses: Some(5),
            })
            .unwrap();
        bootstrap_broker
            .issue_session(session.clone())
            .await
            .unwrap();
        let correlation = session.correlation_id();
        let scoped_session_path = credential_session_path(&scope, correlation).unwrap();
        let watched = bootstrap_broker
            .filesystem
            .resolve(&scope, &scoped_session_path)
            .unwrap();

        let racing = StdArc::new(AlwaysRacingBackend::new(StdArc::clone(&inner), watched));
        let racing_scoped = default_scoped_fs(StdArc::clone(&racing));
        let racing_broker = FilesystemCredentialBroker::new(racing_scoped, test_crypto());

        let error = racing_broker
            .consume_session_use(&scope, correlation, Utc::now())
            .await
            .unwrap_err();
        match error {
            CredentialBrokerError::BrokerUnavailable { reason } => assert_eq!(
                reason, "credential session use retry limit exceeded",
                "wire string for consume_session_use retry exhaustion must be stable"
            ),
            other => panic!("expected BrokerUnavailable, got {other:?}"),
        }
        assert_eq!(
            racing.races(),
            ironclaw_filesystem::FILESYSTEM_CAS_RETRIES,
            "each retry attempt must have been raced"
        );
    }

    /// Regression for the ScopedFilesystem migration: two stores share one
    /// [`InMemoryBackend`] but each is constructed with a [`MountView`]
    /// whose `/secrets` alias resolves to a different tenant-scoped
    /// [`VirtualPath`] subtree. Writing the same `(user_id, project_id,
    /// handle)` tuple on tenant A's store must NOT make the secret visible
    /// from tenant B's store. Before this migration, `FilesystemSecretStore`
    /// held a raw `Arc<F: RootFilesystem>` and encoded tenant identity in
    /// the path itself — any composition layer that forgot to prefix the
    /// path with tenant would leak across tenants, with the type system
    /// saying nothing. The structural fix routes every op through
    /// `ScopedFilesystem`, so two MountViews over the same backend cannot
    /// see each other's data.
    #[tokio::test]
    async fn filesystem_secret_store_isolates_two_tenants_with_same_user_project_ids() {
        let backend = Arc::new(InMemoryBackend::new());
        let store_a = FilesystemSecretStore::new(
            build_scoped_fs(
                Arc::clone(&backend),
                "/secrets/tenants/a/users/alice/secrets",
            ),
            test_crypto(),
        );
        let store_b = FilesystemSecretStore::new(
            build_scoped_fs(
                Arc::clone(&backend),
                "/secrets/tenants/b/users/alice/secrets",
            ),
            test_crypto(),
        );

        // Identical `(user_id, project_id)` for both — the only thing
        // keeping the two stores apart is the mount-time tenant prefix.
        let scope_a = ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("alice").unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("project-1").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let scope_b = ResourceScope {
            tenant_id: TenantId::new("tenant-b").unwrap(),
            user_id: UserId::new("alice").unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("project-1").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let handle = SecretHandle::new("api_key").unwrap();

        store_a
            .put(
                scope_a.clone(),
                handle.clone(),
                SecretMaterial::from("tenant-a-secret"),
                None,
            )
            .await
            .unwrap();

        // Tenant A sees its own secret.
        assert!(
            store_a.metadata(&scope_a, &handle).await.unwrap().is_some(),
            "tenant A must see the secret it just wrote"
        );

        // Tenant B does NOT see tenant A's secret, despite identical
        // (user_id, project_id, handle). Both metadata() and lease_once()
        // must fail closed.
        assert!(
            store_b.metadata(&scope_b, &handle).await.unwrap().is_none(),
            "tenant B must NOT see tenant A's secret (cross-tenant path leak)"
        );
        let cross = store_b.lease_once(&scope_b, &handle).await.unwrap_err();
        assert!(
            cross.is_unknown_secret(),
            "tenant B lease_once must fail closed with UnknownSecret; got {cross:?}"
        );

        // Tenant B's own leases_for_scope is empty under (user, project)
        // shared with tenant A.
        let b_leases = store_b.leases_for_scope(&scope_b).await.unwrap();
        assert!(
            b_leases.is_empty(),
            "tenant B leases_for_scope must be empty under shared (user, project); got {} leases",
            b_leases.len()
        );
    }

    /// Regression for the AAD-alignment bug fixed in `199137b57`: AAD binds
    /// ciphertext to the *owner* scope (`tenant/user/agent/project`) only,
    /// not the full invocation scope. Two reads issued under different
    /// invocation/mission/thread ids but identical owner scope must
    /// successfully decrypt the secret the first invocation wrote. Before
    /// the AAD-alignment fix this failed with `DecryptionFailed` because
    /// AAD bound mission/thread/invocation but the storage path bound only
    /// the owner scope. The ScopedFilesystem migration preserves the
    /// invariant: tenant/user move into the MountView, but agent/project
    /// remain in both the path and the AAD, so cross-invocation reads
    /// within one owner still round-trip cleanly.
    #[tokio::test]
    async fn filesystem_secret_store_aad_validates_cross_invocation_within_same_owner() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(default_scoped_fs(fs), test_crypto());

        // Same tenant/user/agent/project across both invocations; only the
        // invocation/mission/thread fields differ.
        let writer_scope = ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("user-a").unwrap(),
            agent_id: Some(AgentId::new("agent-1").unwrap()),
            project_id: Some(ProjectId::new("project-1").unwrap()),
            mission_id: Some(MissionId::new("mission-write").unwrap()),
            thread_id: Some(ThreadId::new("thread-write").unwrap()),
            invocation_id: InvocationId::new(),
        };
        let mut reader_scope = writer_scope.clone();
        reader_scope.mission_id = Some(MissionId::new("mission-read").unwrap());
        reader_scope.thread_id = Some(ThreadId::new("thread-read").unwrap());
        reader_scope.invocation_id = InvocationId::new();
        assert_ne!(
            writer_scope.invocation_id, reader_scope.invocation_id,
            "test setup error: writer and reader invocations must differ"
        );

        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                writer_scope.clone(),
                handle.clone(),
                SecretMaterial::from("cross-invocation-secret"),
                None,
            )
            .await
            .unwrap();

        // The reader invocation issues its own lease under the same owner
        // and must successfully decrypt — AAD binds owner scope only.
        let lease = store.lease_once(&reader_scope, &handle).await.unwrap();
        let material = store.consume(&reader_scope, lease.id).await.unwrap();
        assert_eq!(
            material.expose_secret(),
            "cross-invocation-secret",
            "AAD must bind owner scope only — cross-invocation reads under \
             the same (tenant, user, agent, project) must succeed",
        );
    }

    /// Defense-in-depth regression for the tenant-isolation indexed
    /// projection (see
    /// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`):
    /// every secret/lease/account/session write decorates its `Entry`
    /// with a `tenant_id` projection so an admin-tier query can filter
    /// explicitly by tenant and a path-rewriting bug surfaces as a
    /// query-time mismatch.
    ///
    /// Writes a secret under tenant A's scope, then issues a raw
    /// `RootFilesystem::query` against the secrets prefix with
    /// `Filter::Eq { key: "tenant_id", value: <tenant-a> }` and asserts
    /// the record is returned. Querying for a different tenant must
    /// return zero rows.
    #[tokio::test]
    async fn filesystem_secret_store_writes_tenant_id_indexed_projection() {
        use ironclaw_filesystem::{Filter, IndexKey, IndexValue, Page};

        let backend = Arc::new(InMemoryBackend::new());
        let scoped = default_scoped_fs(Arc::clone(&backend));
        let store = FilesystemSecretStore::new(Arc::clone(&scoped), test_crypto());
        let scope = sample_scope("tenant-a", "alice");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("indexed-projection-secret"),
                None,
            )
            .await
            .unwrap();

        // Resolve the alias-relative secrets prefix to the backing
        // VirtualPath via the same MountView the store uses so the raw
        // query targets exactly the bytes the backend stored.
        let prefix = secret_owner_root(&scope).unwrap();
        let virtual_prefix = scoped.resolve(&scope, &prefix).unwrap();
        let tenant_key = IndexKey::new("tenant_id").unwrap();

        let hit = backend
            .query(
                &virtual_prefix,
                &Filter::Eq {
                    key: tenant_key.clone(),
                    value: IndexValue::Text(scope.tenant_id.as_str().to_string()),
                },
                Page::new(0, Page::MAX_LIMIT),
            )
            .await
            .unwrap();
        assert_eq!(
            hit.len(),
            1,
            "tenant_id projection must surface the secret via Filter::Eq",
        );

        let miss = backend
            .query(
                &virtual_prefix,
                &Filter::Eq {
                    key: tenant_key,
                    value: IndexValue::Text("tenant-b".to_string()),
                },
                Page::new(0, Page::MAX_LIMIT),
            )
            .await
            .unwrap();
        assert!(
            miss.is_empty(),
            "tenant_id projection must NOT surface tenant-a's secret under tenant-b query; got {} rows",
            miss.len(),
        );
    }

    // `wrong_crypto` helper deleted alongside master-key sentinel tests.

    // Master-key sentinel tests were deleted alongside
    // `verify_can_decrypt_existing_secrets` (PR #3679). Master-key
    // correctness is now verified by the first per-tenant decrypt op rather
    // than a process-wide startup sentinel; see the comment in the impl
    // block above.

    /// Regression for the `already_marked` branch inside `consume`.
    ///
    /// When the stored lease status is already `Expired` (not merely
    /// effectively-expired due to an elapsed TTL), `consume` must return
    /// `LeaseExpired` without issuing a write — the CAS closure returns `Err`
    /// so `cas_update` skips the write entirely.
    ///
    /// How we force `stored.status == Expired`:
    ///   1. Create a store with a negative lease TTL so every freshly issued
    ///      lease has `lease_expires_at` in the past.
    ///   2. First `consume`: `effective_status` sees `Active` + elapsed TTL
    ///      → returns `Expired`; `already_marked = (Active == Expired)` = false
    ///      → the promotion branch writes `status = Expired` and surfaces
    ///      `LeaseExpired`.
    ///   3. Second `consume`: reads the now-stored `Expired` status.
    ///      `already_marked = (Expired == Expired)` = true → no write,
    ///      returns `LeaseExpired` via the apply-error path.
    ///
    /// The no-write assertion is the load-bearing check: the backend
    /// `RecordVersion` must not advance after the second consume.
    #[tokio::test]
    async fn consume_already_expired_lease_returns_expired_without_rewrite() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = default_scoped_fs(Arc::clone(&backend));
        // Negative TTL: every issued lease is born past its expiry.
        let store = FilesystemSecretStore::with_lease_ttl(
            Arc::clone(&scoped),
            test_crypto(),
            chrono::Duration::seconds(-1),
        );
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();

        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("already-expired-secret"),
                None,
            )
            .await
            .unwrap();

        // lease_expires_at = Utc::now() + (-1s) → already in the past.
        let lease = store.lease_once(&scope, &handle).await.unwrap();

        // First consume: effective_status(Active, now) sees elapsed TTL →
        // Expired. already_marked = (Active == Expired) = false → promotion
        // branch writes status=Expired and surfaces LeaseExpired.
        let first_err = store.consume(&scope, lease.id).await.unwrap_err();
        assert!(
            first_err.is_expired(),
            "first consume must return LeaseExpired (promotion path); got {first_err:?}"
        );

        // Capture the backend version after the promotion write. The promotion
        // branch issued exactly one CAS write, so the version advanced from the
        // initial write_lease value to this snapshot.
        let scoped_lease_path = lease_path(&scope, lease.id).unwrap();
        let virtual_path = scoped.resolve(&scope, &scoped_lease_path).unwrap();
        let version_after_promotion = backend
            .get(&virtual_path)
            .await
            .unwrap()
            .expect("lease record must exist after promotion write")
            .version;

        // Second consume: reads stored status=Expired. effective_status passes
        // Expired through. already_marked = (Expired == Expired) = true →
        // closure returns Err immediately; cas_update skips the write.
        let second_err = store.consume(&scope, lease.id).await.unwrap_err();
        assert!(
            second_err.is_expired(),
            "second consume must return LeaseExpired (already_marked path); got {second_err:?}"
        );

        // Load-bearing assertion: no write was issued on the already_marked
        // path. The backend version must NOT advance.
        let version_after_second = backend
            .get(&virtual_path)
            .await
            .unwrap()
            .expect("lease record must still exist after second consume")
            .version;
        assert_eq!(
            version_after_promotion.get(),
            version_after_second.get(),
            "already_marked path must issue no write — backend version must not advance \
             (after promotion: v{}, after second consume: v{})",
            version_after_promotion.get(),
            version_after_second.get(),
        );
    }

    /// `consume` must fail closed — not blind-overwrite — when the mounted
    /// backend cannot honor compare-and-swap. Bootstraps a real lease through
    /// a CAS-capable `InMemoryBackend`, then drives `consume` through
    /// `CasUnsupportedBackend` (same inner store, capabilities overridden to
    /// a known non-CAS shape) so `cas_update`'s pre-flight gate fires
    /// `CasUpdateError::CasUnsupported` deterministically before any read or
    /// write. #5234 review follow-up: `map_cas_error_secret`'s
    /// `CasUnsupported -> StoreUnavailable` arm was untested.
    #[tokio::test]
    async fn filesystem_secret_store_consume_fails_closed_on_cas_unsupported_backend() {
        let inner = StdArc::new(InMemoryBackend::new());
        let bootstrap_scoped = default_scoped_fs(StdArc::clone(&inner));
        let bootstrap_store = FilesystemSecretStore::new(bootstrap_scoped, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        bootstrap_store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret-cas-unsupported"),
                None,
            )
            .await
            .unwrap();
        let lease = bootstrap_store.lease_once(&scope, &handle).await.unwrap();

        let unsupported = StdArc::new(CasUnsupportedBackend::new(StdArc::clone(&inner)));
        let unsupported_scoped = default_scoped_fs(StdArc::clone(&unsupported));
        let unsupported_store = FilesystemSecretStore::new(unsupported_scoped, test_crypto());

        let error = unsupported_store
            .consume(&scope, lease.id)
            .await
            .unwrap_err();
        match error {
            SecretStoreError::StoreUnavailable { reason } => {
                assert!(
                    reason.contains("compare-and-swap"),
                    "expected CAS-unsupported reason, got: {reason}"
                );
            }
            other => panic!("expected StoreUnavailable, got {other:?}"),
        }
    }

    /// `consume_session_use` must fail closed for the same reason as
    /// `filesystem_secret_store_consume_fails_closed_on_cas_unsupported_backend`,
    /// via the broker's `map_cas_error_broker` mapping. #5234 review
    /// follow-up: the `CasUnsupported -> BrokerUnavailable` arm was untested.
    #[tokio::test]
    async fn filesystem_broker_consume_session_use_fails_closed_on_cas_unsupported_backend() {
        let inner = StdArc::new(InMemoryBackend::new());
        let bootstrap_scoped = default_scoped_fs(StdArc::clone(&inner));
        let bootstrap_broker = FilesystemCredentialBroker::new(bootstrap_scoped, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("openai_cas_unsupported").unwrap();
        let account = sample_account(
            scope.clone(),
            account_id.clone(),
            SecretHandle::new("openai_cas_unsupported_key").unwrap(),
        );
        bootstrap_broker.put_account(account.clone()).await.unwrap();

        let in_memory = InMemoryCredentialBroker::new();
        in_memory.put_account(account).unwrap();
        let session = in_memory
            .create_session(crate::CredentialSessionRequest {
                scope: scope.clone(),
                invocation_id: scope.invocation_id,
                capability_id: CapabilityId::new("openai.chat").unwrap(),
                extension_id: ExtensionId::new("openai").unwrap(),
                account_id: account_id.clone(),
                method: NetworkMethod::Get,
                url: "https://api.example.com/v1/models".to_string(),
                expires_at: Some(Utc::now() + chrono::Duration::seconds(60)),
                max_uses: Some(3),
            })
            .unwrap();
        bootstrap_broker
            .issue_session(session.clone())
            .await
            .unwrap();
        let correlation = session.correlation_id();

        let unsupported = StdArc::new(CasUnsupportedBackend::new(StdArc::clone(&inner)));
        let unsupported_scoped = default_scoped_fs(StdArc::clone(&unsupported));
        let unsupported_broker = FilesystemCredentialBroker::new(unsupported_scoped, test_crypto());

        let error = unsupported_broker
            .consume_session_use(&scope, correlation, Utc::now())
            .await
            .unwrap_err();
        match error {
            CredentialBrokerError::BrokerUnavailable { reason } => {
                assert!(
                    reason.contains("compare-and-swap"),
                    "expected CAS-unsupported reason, got: {reason}"
                );
            }
            other => panic!("expected BrokerUnavailable, got {other:?}"),
        }
    }
}
