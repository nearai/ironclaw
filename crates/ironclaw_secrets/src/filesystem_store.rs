//! Filesystem-backed implementations of the scoped secret and credential stores.
//!
//! Routes persistence through the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) surface so secrets,
//! credential accounts, and credential sessions all share the same dispatch
//! fabric as the rest of Reborn (`ironclaw_processes`, `ironclaw_authorization`,
//! `ironclaw_outbound`, `ironclaw_run_state`).
//!
//! Path layout, all under tenant/user/agent/project/mission/thread-scoped
//! prefixes:
//!
//! - `/secrets/<owner-prefix>/secrets/<handle>.json` — encrypted secret material
//! - `/secrets/<owner-prefix>/secret-leases/<lease_id>.json` — active/consumed/
//!   revoked/expired lease metadata
//! - `/secrets/<owner-prefix>/credential-accounts/<account_id>.json` — credential
//!   account records (encrypted target/extension metadata)
//! - `/secrets/<owner-prefix>/credential-sessions/<session_id>.json` — credential
//!   session records (encrypted session payload + use counter)
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_filesystem::{CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem};
use ironclaw_host_api::{
    AgentId, HostApiError, MissionId, ProjectId, ResourceScope, SecretHandle, ThreadId, Timestamp,
    VirtualPath,
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::{
    CredentialAccount, CredentialAccountId, CredentialAccountStatus, CredentialAccountStore,
    CredentialBrokerError, CredentialSession, CredentialSessionId, CredentialSessionStore,
    DEFAULT_SECRET_LEASE_TTL_SECONDS, SecretError, SecretLease, SecretLeaseId, SecretLeaseStatus,
    SecretMaterial, SecretMetadata, SecretStore, SecretStoreError, SecretsCrypto,
    credential_account_aad, credential_session_aad, filesystem_secret_aad,
};

/// Maximum number of CAS retries before a multi-process write loop gives up
/// and surfaces a transient backend error. Mirrors the bound used in
/// `ironclaw_engine::store::filesystem` and `ironclaw_authorization` — three
/// attempts is enough to absorb realistic contention without papering over
/// pathological hot-spots that should be surfaced to the caller.
const CAS_RETRY_ATTEMPTS: usize = 3;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
/// Construct with any [`RootFilesystem`]. Encryption is currently embedded
/// (see the module docstring) and uses the same [`SecretsCrypto`] as the
/// libSQL/Postgres backends.
pub struct FilesystemSecretStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
    crypto: Arc<SecretsCrypto>,
    lease_ttl: Duration,
}

impl<F> FilesystemSecretStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>, crypto: Arc<SecretsCrypto>) -> Self {
        Self {
            filesystem,
            crypto,
            lease_ttl: Duration::seconds(DEFAULT_SECRET_LEASE_TTL_SECONDS),
        }
    }

    pub fn with_lease_ttl(
        filesystem: Arc<F>,
        crypto: Arc<SecretsCrypto>,
        lease_ttl: Duration,
    ) -> Self {
        Self {
            filesystem,
            crypto,
            lease_ttl,
        }
    }

    async fn read_secret(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<StoredSecret>, SecretStoreError> {
        let path = secret_path(scope, handle)?;
        let Some(versioned) = self
            .filesystem
            .get(&path)
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
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_secret_store_error)
    }

    async fn write_lease(&self, lease: &StoredLease) -> Result<(), SecretStoreError> {
        let path = lease_path(&lease.scope, lease.lease_id)?;
        let body = serialize_secret(lease)?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map(|_| ())
            .map_err(fs_to_secret_store_error)
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
            expires_at: None,
            created_at: now,
            updated_at: now,
        };
        self.write_secret(&stored).await?;
        Ok(SecretMetadata { scope, handle })
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
            }))
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
        let lock = filesystem_secret_lock_for_lease(scope, lease_id);
        let _guard = lock.lock().await;
        // The process-local mutex above only serializes writers in this
        // process; multi-process callers sharing the same backend root could
        // otherwise both observe an `Active` lease, both decrypt, and both
        // overwrite the consumed marker. Re-read and retry on
        // `FilesystemError::VersionMismatch` so a concurrent consume from
        // another process loses the race deterministically. Pattern mirrors
        // `ironclaw_engine::store::filesystem::update_thread_state`.
        let path = lease_path(scope, lease_id)?;
        for _ in 0..CAS_RETRY_ATTEMPTS {
            let Some(versioned) = self
                .filesystem
                .get(&path)
                .await
                .map_err(fs_to_secret_store_error)?
            else {
                return Err(SecretStoreError::UnknownLease {
                    scope: Box::new(scope.clone()),
                    lease_id,
                });
            };
            let mut lease: StoredLease = deserialize_secret(&versioned.entry.body)?;
            if !same_scope_for_lease(&lease.scope, scope) {
                return Err(SecretStoreError::UnknownLease {
                    scope: Box::new(scope.clone()),
                    lease_id,
                });
            }
            let now = Utc::now();
            let effective = Self::effective_status(&lease, now);
            match effective {
                SecretLeaseStatus::Active => {}
                SecretLeaseStatus::Consumed => {
                    return Err(SecretStoreError::LeaseConsumed { lease_id });
                }
                SecretLeaseStatus::Revoked => {
                    return Err(SecretStoreError::LeaseRevoked { lease_id });
                }
                SecretLeaseStatus::Expired => {
                    if lease.status != SecretLeaseStatus::Expired {
                        lease.status = SecretLeaseStatus::Expired;
                        // Best-effort expiry promotion. If another writer
                        // raced us we'll observe Expired on the next read
                        // and return the same error to the caller.
                        let body = serialize_secret(&lease)?;
                        let entry = Entry::bytes(body).with_content_type(ContentType::json());
                        match self
                            .filesystem
                            .put(&path, entry, CasExpectation::Version(versioned.version))
                            .await
                        {
                            Ok(_) | Err(FilesystemError::VersionMismatch { .. }) => {}
                            Err(error) => return Err(fs_to_secret_store_error(error)),
                        }
                    }
                    return Err(SecretStoreError::LeaseExpired { lease_id });
                }
            }

            let stored = self
                .read_secret(scope, &lease.handle)
                .await?
                .ok_or_else(|| SecretStoreError::UnknownSecret {
                    scope: Box::new(scope.clone()),
                    handle: lease.handle.clone(),
                })?;
            let aad = filesystem_secret_aad(scope, &lease.handle);
            let decrypted = self
                .crypto
                .decrypt(&stored.encrypted_value, &stored.key_salt, &aad)
                .map_err(secret_error_to_store_error)?;
            let material = SecretMaterial::from(decrypted.expose().to_string());

            lease.status = SecretLeaseStatus::Consumed;
            let body = serialize_secret(&lease)?;
            let entry = Entry::bytes(body).with_content_type(ContentType::json());
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Version(versioned.version))
                .await
            {
                Ok(_) => return Ok(material),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_to_secret_store_error(error)),
            }
        }
        Err(SecretStoreError::StoreUnavailable {
            reason: "secret lease consume retry limit exceeded".to_string(),
        })
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        let lock = filesystem_secret_lock_for_lease(scope, lease_id);
        let _guard = lock.lock().await;
        // The process-local mutex above only serializes writers in this
        // process; multi-process callers sharing the same backend root could
        // otherwise both observe an `Active` lease and the slower `revoke`
        // could clobber a `Consumed` marker written by a concurrent
        // `consume`. Re-read with the row version and write with
        // `CasExpectation::Version`, retrying on `VersionMismatch`. Pattern
        // mirrors `consume` and `consume_session_use` above. See F2 (Medium)
        // in the 2026-05 audit.
        let path = lease_path(scope, lease_id)?;
        for _ in 0..CAS_RETRY_ATTEMPTS {
            let Some(versioned) = self
                .filesystem
                .get(&path)
                .await
                .map_err(fs_to_secret_store_error)?
            else {
                return Err(SecretStoreError::UnknownLease {
                    scope: Box::new(scope.clone()),
                    lease_id,
                });
            };
            let mut lease: StoredLease = deserialize_secret(&versioned.entry.body)?;
            if !same_scope_for_lease(&lease.scope, scope) {
                return Err(SecretStoreError::UnknownLease {
                    scope: Box::new(scope.clone()),
                    lease_id,
                });
            }
            // Idempotent on terminal states: revoking an already-Consumed or
            // already-Revoked lease succeeds without rewriting the marker, so
            // a race with `consume` cannot overwrite the Consumed signal.
            // Expired is similarly terminal — `effective_status` decides
            // whether an Active lease has aged into Expired.
            match lease.status {
                SecretLeaseStatus::Consumed
                | SecretLeaseStatus::Revoked
                | SecretLeaseStatus::Expired => {
                    return Ok(Self::lease_to_public(&lease));
                }
                SecretLeaseStatus::Active => {}
            }
            let now = Utc::now();
            // Promote a stale Active lease to Expired before revoking, mirroring
            // the in-memory adapter's `expire_stale_active_leases` step.
            if Self::effective_status(&lease, now) == SecretLeaseStatus::Expired {
                lease.status = SecretLeaseStatus::Expired;
            } else {
                lease.status = SecretLeaseStatus::Revoked;
            }
            let body = serialize_secret(&lease)?;
            let entry = Entry::bytes(body).with_content_type(ContentType::json());
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Version(versioned.version))
                .await
            {
                Ok(_) => return Ok(Self::lease_to_public(&lease)),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_to_secret_store_error(error)),
            }
        }
        Err(SecretStoreError::StoreUnavailable {
            reason: "secret lease revoke retry limit exceeded".to_string(),
        })
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
        let entries = match self.filesystem.list_dir(&root).await {
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
            let Some(versioned) = self
                .filesystem
                .get(&entry.path)
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
    filesystem: Arc<F>,
    crypto: Arc<SecretsCrypto>,
}

impl<F> FilesystemCredentialBroker<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>, crypto: Arc<SecretsCrypto>) -> Self {
        Self { filesystem, crypto }
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
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
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
            .get(&path)
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
        let entries = match self.filesystem.list_dir(&root).await {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => return Ok(Vec::new()),
            Err(error) => return Err(fs_to_broker_error(error)),
        };
        let mut accounts = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let Some(versioned) = self
                .filesystem
                .get(&entry.path)
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
        let body = serialize_credential(&stored)?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
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
            .get(&path)
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
        let lock = filesystem_session_lock(&path);
        let _guard = lock.lock().await;
        let Some(versioned) = self
            .filesystem
            .get(&path)
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
        let lock = filesystem_session_lock(&path);
        let _guard = lock.lock().await;
        // Multi-process callers sharing the same backend root must not both
        // pass the max-uses check and overwrite each other's increment. We
        // load the current version, evaluate the use-limit condition, and
        // write the incremented counter with a CAS expectation. On
        // `VersionMismatch` we re-read, re-evaluate the condition, and retry.
        // Pattern mirrors `ironclaw_engine::store::filesystem::update_thread_state`.
        for _ in 0..CAS_RETRY_ATTEMPTS {
            let Some(versioned) = self
                .filesystem
                .get(&path)
                .await
                .map_err(fs_to_broker_error)?
            else {
                return Err(CredentialBrokerError::UnknownSession { session_id });
            };
            let mut stored: StoredSession = deserialize_credential(&versioned.entry.body)?;
            let aad = credential_session_aad(scope, session_id);
            let wire: SerializableCredentialSession =
                self.decrypt_payload(&stored.encrypted_payload, &stored.key_salt, &aad)?;
            if wire.scope != *scope {
                return Err(CredentialBrokerError::UnknownSession { session_id });
            }
            ensure_stored_session_usable(&wire, stored.uses, session_id, now)?;
            stored.uses += 1;
            let body = serialize_credential(&stored)?;
            let entry = Entry::bytes(body).with_content_type(ContentType::json());
            match self
                .filesystem
                .put(&path, entry, CasExpectation::Version(versioned.version))
                .await
            {
                Ok(_) => return wire.into_session(),
                Err(FilesystemError::VersionMismatch { .. }) => continue,
                Err(error) => return Err(fs_to_broker_error(error)),
            }
        }
        Err(CredentialBrokerError::BrokerUnavailable {
            reason: "credential session use retry limit exceeded".to_string(),
        })
    }
}

// -- Paths ------------------------------------------------------------------

fn secret_path(
    scope: &ResourceScope,
    handle: &SecretHandle,
) -> Result<VirtualPath, SecretStoreError> {
    VirtualPath::new(format!(
        "{}/secrets/{}.json",
        secret_owner_root(scope),
        handle.as_str()
    ))
    .map_err(host_api_to_secret_store_error)
}

fn lease_path(
    scope: &ResourceScope,
    lease_id: SecretLeaseId,
) -> Result<VirtualPath, SecretStoreError> {
    VirtualPath::new(format!("{}/{lease_id}.json", lease_root(scope)?.as_str()))
        .map_err(host_api_to_secret_store_error)
}

fn lease_root(scope: &ResourceScope) -> Result<VirtualPath, SecretStoreError> {
    VirtualPath::new(format!("{}/secret-leases", secret_owner_root(scope)))
        .map_err(host_api_to_secret_store_error)
}

fn credential_account_path(
    scope: &ResourceScope,
    account_id: &CredentialAccountId,
) -> Result<VirtualPath, CredentialBrokerError> {
    VirtualPath::new(format!(
        "{}/{}.json",
        credential_account_root(scope)?.as_str(),
        account_id.as_str()
    ))
    .map_err(host_api_to_broker_error)
}

fn credential_account_root(scope: &ResourceScope) -> Result<VirtualPath, CredentialBrokerError> {
    VirtualPath::new(format!("{}/credential-accounts", secret_owner_root(scope)))
        .map_err(host_api_to_broker_error)
}

fn credential_session_path(
    scope: &ResourceScope,
    session_id: CredentialSessionId,
) -> Result<VirtualPath, CredentialBrokerError> {
    VirtualPath::new(format!(
        "{}/credential-sessions/{}.json",
        secret_owner_root(scope),
        session_id.to_private_storage_string()
    ))
    .map_err(host_api_to_broker_error)
}

fn secret_owner_root(scope: &ResourceScope) -> String {
    let mut base = format!(
        "/secrets/tenants/{}/users/{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    );
    if let Some(agent_id) = &scope.agent_id {
        base = format!("{base}/agents/{}", agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        base = format!("{base}/projects/{}", project_id.as_str());
    }
    base
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

// -- Per-record locks -------------------------------------------------------
//
// Mirror the `FILESYSTEM_RECORD_LOCKS` shape used in `ironclaw_run_state` and
// `ironclaw_authorization`: process-local locks keyed by virtual path. The
// filesystem-backed store is therefore safe for concurrent operations within a
// single instance; multi-process callers must use the libSQL/Postgres
// backends with transactional writes (mirrored in the crate's CLAUDE.md
// `ironclaw_run_state` guardrails).

type FilesystemRecordLock = Arc<tokio::sync::Mutex<()>>;

static FILESYSTEM_RECORD_LOCKS: OnceLock<Mutex<HashMap<String, FilesystemRecordLock>>> =
    OnceLock::new();

fn filesystem_secret_lock(key: String) -> FilesystemRecordLock {
    let locks = FILESYSTEM_RECORD_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = lock_or_recover(locks);
    Arc::clone(
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

fn filesystem_secret_lock_for_lease(
    scope: &ResourceScope,
    lease_id: SecretLeaseId,
) -> FilesystemRecordLock {
    // We can't reuse `lease_path` here because it returns Result; we just need
    // a stable key for the in-process mutex. The path string is stable for the
    // same scope/lease and not a host path.
    let key = format!("lease|{}|{}", owner_lock_prefix(scope), lease_id);
    filesystem_secret_lock(key)
}

fn filesystem_session_lock(path: &VirtualPath) -> FilesystemRecordLock {
    filesystem_secret_lock(format!("session|{}", path.as_str()))
}

fn owner_lock_prefix(scope: &ResourceScope) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str(),
        scope.agent_id.as_ref().map(AgentId::as_str).unwrap_or(""),
        scope
            .project_id
            .as_ref()
            .map(ProjectId::as_str)
            .unwrap_or(""),
        scope
            .mission_id
            .as_ref()
            .map(MissionId::as_str)
            .unwrap_or(""),
        scope.thread_id.as_ref().map(ThreadId::as_str).unwrap_or(""),
        scope.invocation_id,
    )
}

fn lock_or_recover<T>(mutex: &Mutex<HashMap<String, T>>) -> MutexGuard<'_, HashMap<String, T>> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    use chrono::Utc;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        CapabilityId, ExtensionId, InvocationId, MissionId, NetworkMethod, ProjectId,
        ResourceScope, SecretHandle, TenantId, ThreadId, UserId,
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

    #[tokio::test]
    async fn filesystem_secret_store_round_trips_material() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(Arc::clone(&fs), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();

        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
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
    async fn filesystem_secret_store_encrypts_at_rest() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(Arc::clone(&fs), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();

        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("plaintext-sentinel-7e3d"),
            )
            .await
            .unwrap();

        let path = secret_path(&scope, &handle).unwrap();
        let versioned = fs.get(&path).await.unwrap().expect("entry persisted");
        let raw = String::from_utf8_lossy(&versioned.entry.body);
        assert!(
            !raw.contains("plaintext-sentinel-7e3d"),
            "secret material must be encrypted at rest"
        );
    }

    #[tokio::test]
    async fn filesystem_secret_store_isolates_scopes() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(fs, test_crypto());
        let tenant_a = sample_scope("tenant-a", "user-a");
        let tenant_b = sample_scope("tenant-b", "user-a");
        let handle = SecretHandle::new("shared_name").unwrap();

        store
            .put(
                tenant_a.clone(),
                handle.clone(),
                SecretMaterial::from("aaa"),
            )
            .await
            .unwrap();
        store
            .put(
                tenant_b.clone(),
                handle.clone(),
                SecretMaterial::from("bbb"),
            )
            .await
            .unwrap();

        let lease_a = store.lease_once(&tenant_a, &handle).await.unwrap();
        let cross = store.consume(&tenant_b, lease_a.id).await.unwrap_err();
        assert!(cross.is_unknown_lease());
    }

    #[tokio::test]
    async fn filesystem_secret_store_revoke_blocks_consume() {
        let fs = Arc::new(InMemoryBackend::new());
        let store = FilesystemSecretStore::new(fs, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
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
        let store = FilesystemSecretStore::new(fs, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
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
        let store = FilesystemSecretStore::new(fs, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret"),
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
        let store = FilesystemSecretStore::new(fs, test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("missing").unwrap();

        let error = store.lease_once(&scope, &handle).await.unwrap_err();
        assert!(error.is_unknown_secret());
        assert!(store.leases_for_scope(&scope).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn filesystem_credential_broker_round_trips_account_and_session() {
        let fs = Arc::new(InMemoryBackend::new());
        let broker = FilesystemCredentialBroker::new(Arc::clone(&fs), test_crypto());
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
        let broker = FilesystemCredentialBroker::new(Arc::clone(&fs), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let account_id = CredentialAccountId::new("github_prod").unwrap();
        let mut account = sample_account(
            scope.clone(),
            account_id.clone(),
            SecretHandle::new("github_key").unwrap(),
        );
        account.label = "leak-sentinel-92ab".to_string();
        broker.put_account(account).await.unwrap();

        let path = credential_account_path(&scope, &account_id).unwrap();
        let versioned = fs.get(&path).await.unwrap().expect("entry persisted");
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
    use std::sync::atomic::{AtomicBool, Ordering};

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

    #[tokio::test]
    async fn filesystem_secret_store_consume_retries_on_version_mismatch() {
        // First write the lease through a plain backend so the lease and
        // secret material exist, then swap in the racing backend so the
        // very next `put` (the consume's CAS write) hits a forced
        // VersionMismatch.
        let inner = StdArc::new(InMemoryBackend::new());
        let bootstrap_store = FilesystemSecretStore::new(StdArc::clone(&inner), test_crypto());
        let scope = sample_scope("tenant-a", "user-a");
        let handle = SecretHandle::new("api_key").unwrap();
        bootstrap_store
            .put(
                scope.clone(),
                handle.clone(),
                SecretMaterial::from("super-secret-cas"),
            )
            .await
            .unwrap();
        let lease = bootstrap_store.lease_once(&scope, &handle).await.unwrap();

        let lease_path_for_test = lease_path(&scope, lease.id).unwrap();
        let racing = StdArc::new(VersionRacingBackend::new(
            StdArc::clone(&inner),
            lease_path_for_test,
        ));
        let racing_store = FilesystemSecretStore::new(StdArc::clone(&racing), test_crypto());

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
        let bootstrap_broker =
            FilesystemCredentialBroker::new(StdArc::clone(&inner), test_crypto());
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
        let session_path_for_test = credential_session_path(&scope, correlation).unwrap();

        let racing = StdArc::new(VersionRacingBackend::new(
            StdArc::clone(&inner),
            session_path_for_test,
        ));
        let racing_broker = FilesystemCredentialBroker::new(StdArc::clone(&racing), test_crypto());

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
}
