//! Generic durable channel-identity binding store (extension-runtime §5.5,
//! migration H.4).
//!
//! One tenant-scoped filesystem store backs the post-OAuth channel identity
//! bindings for every channel extension: primary records keyed by
//! `(provider, provider_user_id)` under
//! `/tenant-shared/channel-identities/identities`, plus a best-effort
//! per-`(provider, user)` inverse index under
//! `/tenant-shared/channel-identities/identities-by-user` so connection
//! checks can resolve a bound caller by listing only that caller's own
//! bindings. The index is advisory: a missing marker only falls back to the
//! full scan, and readers verify the primary record before trusting a
//! marker, so a stale marker can never be a false positive.
//!
//! # Not the principal identity store
//!
//! There are two durable external-identity stores in the Reborn stack and this
//! is the *binding* one. It answers "which already-authenticated Reborn user is
//! this channel actor?" and it **never mints a user**. Minting, the user
//! profile, and the verified-email index belong to `ironclaw_identity`,
//! which keys on `(tenant, surface_kind, provider_kind, provider_instance,
//! subject)` and owns `resolve_or_create`. Neither store subsumes the other and
//! neither is a migration target for the other; see
//! `crates/domains/ironclaw_identity/CONTRACT.md`, "Two external-identity
//! stores", for the full split.
//!
//! Two consequences worth knowing before changing this file:
//!
//! * **The ports this implements stay in `ironclaw_host_api::user_identity`.**
//!   Relocating them into `ironclaw_identity` was proposed and refuted
//!   (2026-08-04): that crate implements none of them, and because it depends on
//!   `ironclaw_host_api` rather than the reverse, the move would force *this*
//!   crate to take a new dependency purely to name a port it implements.
//! * **This store is fixed to one tenant at construction**, where the principal
//!   store takes the tenant per call. That is a deliberate difference, not an
//!   oversight — but it is the shape to revisit if multi-tenant channel binding
//!   is ever required.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FilesystemError,
    FilesystemOperation, RecordVersion, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    error::HostApiError,
    ids::{InvocationId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, ScopedPath, VirtualPath},
    resource::{ResourceScope, resource_scope_path_segment},
    user_identity::{
        RebornUserIdentityBinding, RebornUserIdentityBindingDeleteStore,
        RebornUserIdentityBindingError, RebornUserIdentityBindingStore, RebornUserIdentityLookup,
        RebornUserIdentityLookupError,
    },
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const CHANNEL_IDENTITY_ALIAS: &str = "/tenant-shared/channel-identities";
const IDENTITY_ROOT: &str = "/tenant-shared/channel-identities/identities";
const IDENTITY_BY_USER_ROOT: &str = "/tenant-shared/channel-identities/identities-by-user";

/// The per-scope mount view for the channel-identity subtree: one alias onto
/// the tenant's shared `channel-identities` root.
pub fn channel_identity_mount_view(scope: &ResourceScope) -> Result<MountView, HostApiError> {
    let tenant = resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![MountGrant::new(
        MountAlias::new(CHANNEL_IDENTITY_ALIAS)?,
        VirtualPath::new(format!("/tenants/{tenant}/shared/channel-identities"))?,
        MountPermissions::read_write_list_delete(),
    )])
}

/// The generic filesystem-backed channel-identity binding store.
pub struct FilesystemChannelIdentityStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    scope: ResourceScope,
}

/// Result of an identity bind that may need compensation by its caller.
#[derive(Debug)]
pub enum IdentityBindingTransaction {
    /// This call created the binding and therefore owns the exact rollback
    /// receipt for that durable incarnation.
    Created(IdentityBindingRollbackReceipt),
    /// The same IronClaw user already owned this exact provider identity. The
    /// durable record was adopted by this call so any older receipt is stale.
    Existing,
}

/// Opaque proof that one call created one exact durable binding incarnation.
#[derive(Debug)]
pub struct IdentityBindingRollbackReceipt {
    provider: String,
    provider_user_id: String,
    user_id: UserId,
    binding_nonce: String,
}

impl std::fmt::Debug for FilesystemChannelIdentityStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemChannelIdentityStore")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl FilesystemChannelIdentityStore {
    pub fn new(filesystem: Arc<dyn RootFilesystem>, tenant_id: TenantId, user_id: UserId) -> Self {
        let scoped = Arc::new(ScopedFilesystem::new(
            filesystem,
            channel_identity_mount_view,
        ));
        Self {
            filesystem: scoped,
            scope: ResourceScope {
                tenant_id,
                user_id,
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
        }
    }

    /// Bind one provider identity and return compensation authority only when
    /// this call created it. A same-owner adoption rewrites the record with a
    /// fresh nonce, invalidating an older concurrent attempt's receipt.
    pub async fn bind_user_identity_transactionally(
        &self,
        binding: RebornUserIdentityBinding,
    ) -> Result<IdentityBindingTransaction, RebornUserIdentityBindingError> {
        let path =
            Self::identity_path(binding.provider.as_str(), binding.provider_user_id.as_str())
                .map_err(map_binding_fs_error)?;
        let binding_nonce = InvocationId::new().to_string();
        let transaction_created = cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            decode_identity_record,
            encode_identity_record,
            |current: Option<StoredChannelUserIdentity>| {
                let binding = binding.clone();
                let binding_nonce = binding_nonce.clone();
                async move {
                    let (created_at, created) = match current {
                        Some(existing) if existing.deleted_at.is_none() => {
                            if existing.user_id != binding.user_id.as_str() {
                                return Err(
                                    RebornUserIdentityBindingError::ProviderIdentityAlreadyBound,
                                );
                            }
                            (existing.created_at, false)
                        }
                        _ => (Utc::now(), true),
                    };
                    Ok(CasApply::new(
                        StoredChannelUserIdentity::from_binding(
                            &binding,
                            created_at,
                            binding_nonce,
                        ),
                        created,
                    ))
                }
            },
        )
        .await
        .map_err(map_identity_cas_error)?;
        self.write_user_binding_index_marker(&binding).await;
        if transaction_created {
            Ok(IdentityBindingTransaction::Created(
                IdentityBindingRollbackReceipt {
                    provider: binding.provider.as_str().to_string(),
                    provider_user_id: binding.provider_user_id.as_str().to_string(),
                    user_id: binding.user_id,
                    binding_nonce,
                },
            ))
        } else {
            Ok(IdentityBindingTransaction::Existing)
        }
    }

    /// Logically delete exactly the binding incarnation named by `receipt`.
    ///
    /// Returns `false` when a later attempt adopted or replaced the binding;
    /// that newer state is never removed by this rollback. Keeping a durable
    /// tombstone also prevents a physical delete/recreate ABA cycle from
    /// resetting a backend's path version beneath an older receipt.
    pub async fn rollback_identity_binding(
        &self,
        receipt: IdentityBindingRollbackReceipt,
    ) -> Result<bool, RebornUserIdentityBindingError> {
        let path = Self::identity_path(&receipt.provider, &receipt.provider_user_id)
            .map_err(map_binding_fs_error)?;
        let fallback = StoredChannelUserIdentity::tombstone_for(
            &receipt.provider,
            &receipt.provider_user_id,
            receipt.user_id.as_str(),
            &receipt.binding_nonce,
        );
        let rolled_back = cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            decode_identity_record,
            encode_identity_record,
            |current: Option<StoredChannelUserIdentity>| {
                let receipt_provider = receipt.provider.clone();
                let receipt_provider_user_id = receipt.provider_user_id.clone();
                let receipt_user_id = receipt.user_id.clone();
                let receipt_nonce = receipt.binding_nonce.clone();
                let fallback = fallback.clone();
                async move {
                    let Some(current) = current else {
                        return Ok(CasApply::no_op(fallback, false));
                    };
                    let matches = current.deleted_at.is_none()
                        && current.provider == receipt_provider
                        && current.provider_user_id == receipt_provider_user_id
                        && current.user_id == receipt_user_id.as_str()
                        && current.binding_nonce == receipt_nonce;
                    if !matches {
                        return Ok(CasApply::no_op(current, false));
                    }
                    Ok(CasApply::new(current.into_tombstone(), true))
                }
            },
        )
        .await
        .map_err(map_identity_cas_error)?;
        if rolled_back {
            self.delete_user_binding_index_marker(
                &receipt.provider,
                receipt.user_id.as_str(),
                &receipt.provider_user_id,
            )
            .await;
        }
        Ok(rolled_back)
    }

    /// Whether this user owns another identity under the same installation
    /// prefix. Used by single-account channel connection strategies to refuse
    /// an implicit identity swap.
    pub async fn user_has_other_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: &str,
        expected_provider_user_id: &str,
    ) -> Result<bool, RebornUserIdentityBindingError> {
        let provider_dir = scoped_path(&format!("{IDENTITY_ROOT}/{}", path_segment(provider)))
            .map_err(map_binding_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &provider_dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(false),
            Err(error) => return Err(map_binding_fs_error(error)),
        };
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let path = scoped_path(&format!(
                "{IDENTITY_ROOT}/{}/{}",
                path_segment(provider),
                entry.name
            ))
            .map_err(map_binding_fs_error)?;
            let Some((candidate, _)) = self
                .read_record::<StoredChannelUserIdentity>(&path)
                .await
                .map_err(map_binding_fs_error)?
            else {
                continue;
            };
            if candidate.deleted_at.is_none()
                && candidate.provider == provider
                && candidate.user_id == user_id.as_str()
                && candidate
                    .provider_user_id
                    .starts_with(provider_user_id_prefix)
                && candidate.provider_user_id != expected_provider_user_id
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The (tenant, user) identity scope this store reads and writes under —
    /// captured by the channel-connection test bundle so its restart-survival
    /// reopen probe reconstructs the store with the same scoping production
    /// composed (`build_runtime`' channel egress scope). Tests only.
    #[cfg(feature = "test-support")]
    pub fn identity_scope_tenant_and_user(&self) -> (&TenantId, &UserId) {
        (&self.scope.tenant_id, &self.scope.user_id)
    }

    async fn read_record<T>(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<(T, RecordVersion)>, FilesystemError>
    where
        T: DeserializeOwned,
    {
        let Some(versioned) = self.filesystem.get(&self.scope, path).await? else {
            return Ok(None);
        };
        let value = serde_json::from_slice(&versioned.entry.body).map_err(|_| {
            FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::ReadFile,
                reason: "channel-identity record is invalid JSON".into(),
            }
        })?;
        Ok(Some((value, versioned.version)))
    }

    async fn write_record<T>(
        &self,
        path: &ScopedPath,
        value: &T,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError>
    where
        T: Serialize,
    {
        let body =
            serde_json::to_vec(value).map_err(|_| FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::WriteFile,
                reason: "channel-identity record could not be serialized".into(),
            })?;
        self.filesystem
            .put(
                &self.scope,
                path,
                Entry::bytes(body).with_content_type(ContentType::json()),
                cas,
            )
            .await
    }

    fn identity_path(
        provider: &str,
        provider_user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{IDENTITY_ROOT}/{}/{}.json",
            path_segment(provider),
            path_segment(provider_user_id)
        ))
    }

    fn identity_user_index_dir(
        provider: &str,
        user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        scoped_path(&format!(
            "{IDENTITY_BY_USER_ROOT}/{}/{}",
            path_segment(provider),
            path_segment(user_id)
        ))
    }

    fn identity_user_index_path(
        provider: &str,
        user_id: &str,
        provider_user_id: &str,
    ) -> Result<ScopedPath, FilesystemError> {
        // The marker file name reuses `path_segment(provider_user_id)`,
        // exactly like the primary record, so the primary path can be
        // rebuilt from a marker entry name without decoding.
        scoped_path(&format!(
            "{IDENTITY_BY_USER_ROOT}/{}/{}/{}.json",
            path_segment(provider),
            path_segment(user_id),
            path_segment(provider_user_id)
        ))
    }

    /// Best-effort write of the per-user index marker for a binding.
    async fn write_user_binding_index_marker(&self, binding: &RebornUserIdentityBinding) {
        let path = match Self::identity_user_index_path(
            binding.provider.as_str(),
            binding.user_id.as_str(),
            binding.provider_user_id.as_str(),
        ) {
            Ok(path) => path,
            Err(error) => {
                tracing::debug!(%error, "could not build channel user-binding index path");
                return;
            }
        };
        let marker = StoredUserBindingIndexMarker {
            provider_user_id: binding.provider_user_id.as_str().to_string(),
        };
        if let Err(error) = self.write_record(&path, &marker, CasExpectation::Any).await {
            tracing::debug!(
                %error,
                "failed to write channel user-binding index marker; connection check will fall back to a scan"
            );
        }
    }

    /// Best-effort delete of a per-user index marker.
    async fn delete_user_binding_index_marker(
        &self,
        provider: &str,
        user_id: &str,
        provider_user_id: &str,
    ) {
        let path = match Self::identity_user_index_path(provider, user_id, provider_user_id) {
            Ok(path) => path,
            Err(_) => return,
        };
        match self.filesystem.delete(&self.scope, &path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
            Err(error) => {
                tracing::debug!(%error, "failed to delete channel user-binding index marker");
            }
        }
    }

    /// Fast-path connection check via the per-user index; `true` only after
    /// verifying the primary record still matches.
    async fn user_binding_via_index_marker(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        let dir = Self::identity_user_index_dir(provider, user_id.as_str())
            .map_err(map_lookup_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(false),
            Err(error) => return Err(map_lookup_fs_error(error)),
        };
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let primary = scoped_path(&format!(
                "{IDENTITY_ROOT}/{}/{}",
                path_segment(provider),
                entry.name
            ))
            .map_err(map_lookup_fs_error)?;
            let Some((record, _)) = self
                .read_record::<StoredChannelUserIdentity>(&primary)
                .await
                .map_err(map_lookup_fs_error)?
            else {
                continue;
            };
            if identity_record_matches_user_binding(
                &record,
                provider,
                user_id,
                provider_user_id_prefix,
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[async_trait::async_trait]
impl RebornUserIdentityLookup for FilesystemChannelIdentityStore {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        let path = Self::identity_path(provider, provider_user_id).map_err(map_lookup_fs_error)?;
        let Some((record, _)) = self
            .read_record::<StoredChannelUserIdentity>(&path)
            .await
            .map_err(map_lookup_fs_error)?
        else {
            return Ok(None);
        };
        if record.deleted_at.is_some() {
            return Ok(None);
        }
        let user_id = UserId::new(record.user_id)
            .map_err(|error| RebornUserIdentityLookupError::InvalidUserId(error.to_string()))?;
        Ok(Some(user_id))
    }

    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        self.user_has_provider_binding_with_provider_user_id_prefix(provider, user_id, None)
            .await
    }

    async fn user_has_provider_binding_with_provider_user_id_prefix(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        if self
            .user_binding_via_index_marker(provider, user_id, provider_user_id_prefix)
            .await?
        {
            return Ok(true);
        }
        // Bindings written before the index existed have no marker: fall
        // back to the full provider scan.
        let provider_dir = scoped_path(&format!("{IDENTITY_ROOT}/{}", path_segment(provider)))
            .map_err(map_lookup_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &provider_dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(false),
            Err(error) => return Err(map_lookup_fs_error(error)),
        };
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let path = scoped_path(&format!(
                "{IDENTITY_ROOT}/{}/{}",
                path_segment(provider),
                entry.name
            ))
            .map_err(map_lookup_fs_error)?;
            let Some((record, _)) = self
                .read_record::<StoredChannelUserIdentity>(&path)
                .await
                .map_err(map_lookup_fs_error)?
            else {
                continue;
            };
            if identity_record_matches_user_binding(
                &record,
                provider,
                user_id,
                provider_user_id_prefix,
            ) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[async_trait::async_trait]
impl RebornUserIdentityBindingStore for FilesystemChannelIdentityStore {
    async fn bind_user_identity(
        &self,
        binding: RebornUserIdentityBinding,
    ) -> Result<(), RebornUserIdentityBindingError> {
        let path =
            Self::identity_path(binding.provider.as_str(), binding.provider_user_id.as_str())
                .map_err(map_binding_fs_error)?;
        let binding_nonce = InvocationId::new().to_string();
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            decode_identity_record,
            encode_identity_record,
            |current: Option<StoredChannelUserIdentity>| {
                let binding = binding.clone();
                let binding_nonce = binding_nonce.clone();
                async move {
                    let created_at = match current {
                        Some(existing) if existing.deleted_at.is_none() => {
                            if existing.user_id != binding.user_id.as_str() {
                                return Err(
                                    RebornUserIdentityBindingError::ProviderIdentityAlreadyBound,
                                );
                            }
                            existing.created_at
                        }
                        _ => Utc::now(),
                    };
                    Ok(CasApply::new(
                        StoredChannelUserIdentity::from_binding(
                            &binding,
                            created_at,
                            binding_nonce,
                        ),
                        (),
                    ))
                }
            },
        )
        .await
        .map_err(map_identity_cas_error)?;
        self.write_user_binding_index_marker(&binding).await;
        Ok(())
    }
}

#[async_trait::async_trait]
impl RebornUserIdentityBindingDeleteStore for FilesystemChannelIdentityStore {
    async fn delete_user_identity_bindings_for_user(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<usize, RebornUserIdentityBindingError> {
        let provider_dir = scoped_path(&format!("{IDENTITY_ROOT}/{}", path_segment(provider)))
            .map_err(map_binding_fs_error)?;
        let entries = match self.filesystem.list_dir(&self.scope, &provider_dir).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => return Ok(0),
            Err(error) => return Err(map_binding_fs_error(error)),
        };
        let mut deleted = 0;
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let path = scoped_path(&format!(
                "{IDENTITY_ROOT}/{}/{}",
                path_segment(provider),
                entry.name
            ))
            .map_err(map_binding_fs_error)?;
            let Some((candidate, _)) = self
                .read_record::<StoredChannelUserIdentity>(&path)
                .await
                .map_err(map_binding_fs_error)?
            else {
                continue;
            };
            if !identity_record_matches_user_binding(
                &candidate,
                provider,
                user_id,
                provider_user_id_prefix,
            ) {
                continue;
            }
            let fallback = candidate.clone();
            let provider = provider.to_string();
            let user_id = user_id.clone();
            let provider_user_id_prefix = provider_user_id_prefix.map(str::to_string);
            let removed_provider_user_id = cas_update(
                self.filesystem.as_ref(),
                &self.scope,
                &path,
                decode_identity_record,
                encode_identity_record,
                |current: Option<StoredChannelUserIdentity>| {
                    let fallback = fallback.clone();
                    let provider = provider.clone();
                    let user_id = user_id.clone();
                    let provider_user_id_prefix = provider_user_id_prefix.clone();
                    async move {
                        let Some(current) = current else {
                            return Ok(CasApply::no_op(fallback, None));
                        };
                        if !identity_record_matches_user_binding(
                            &current,
                            &provider,
                            &user_id,
                            provider_user_id_prefix.as_deref(),
                        ) {
                            return Ok(CasApply::no_op(current, None));
                        }
                        let provider_user_id = current.provider_user_id.clone();
                        Ok(CasApply::new(
                            current.into_tombstone(),
                            Some(provider_user_id),
                        ))
                    }
                },
            )
            .await
            .map_err(map_identity_cas_error)?;
            if let Some(provider_user_id) = removed_provider_user_id {
                deleted += 1;
                self.delete_user_binding_index_marker(
                    &provider,
                    user_id.as_str(),
                    &provider_user_id,
                )
                .await;
            }
        }
        Ok(deleted)
    }
}

/// The durable identity record. Field-compatible with the pre-generic
/// channel-lane records so migration H.4 copies them forward verbatim
/// (modulo the installation-prefix rewrite).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredChannelUserIdentity {
    provider: String,
    provider_user_id: String,
    user_id: String,
    #[serde(default)]
    binding_nonce: String,
    #[serde(default)]
    deleted_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl StoredChannelUserIdentity {
    fn from_binding(
        binding: &RebornUserIdentityBinding,
        created_at: DateTime<Utc>,
        binding_nonce: String,
    ) -> Self {
        Self {
            provider: binding.provider.as_str().to_string(),
            provider_user_id: binding.provider_user_id.as_str().to_string(),
            user_id: binding.user_id.as_str().to_string(),
            binding_nonce,
            deleted_at: None,
            created_at,
            updated_at: Utc::now(),
        }
    }

    fn into_tombstone(mut self) -> Self {
        let now = Utc::now();
        self.deleted_at = Some(now);
        self.updated_at = now;
        self
    }

    fn tombstone_for(
        provider: &str,
        provider_user_id: &str,
        user_id: &str,
        binding_nonce: &str,
    ) -> Self {
        let now = Utc::now();
        Self {
            provider: provider.to_string(),
            provider_user_id: provider_user_id.to_string(),
            user_id: user_id.to_string(),
            binding_nonce: binding_nonce.to_string(),
            deleted_at: Some(now),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Per-user index marker; the file name encodes the `provider_user_id`.
#[derive(Debug, Serialize, Deserialize)]
struct StoredUserBindingIndexMarker {
    provider_user_id: String,
}

fn identity_record_matches_user_binding(
    record: &StoredChannelUserIdentity,
    provider: &str,
    user_id: &UserId,
    provider_user_id_prefix: Option<&str>,
) -> bool {
    record.deleted_at.is_none()
        && record.provider == provider
        && record.user_id == user_id.as_str()
        && provider_user_id_prefix
            .map(|prefix| record.provider_user_id.starts_with(prefix))
            .unwrap_or(true)
}

pub fn path_segment(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn scoped_path(raw: &str) -> Result<ScopedPath, FilesystemError> {
    ScopedPath::new(raw).map_err(|error| FilesystemError::BackendInfrastructure {
        operation: FilesystemOperation::WriteFile,
        reason: format!("invalid channel-identity path under {CHANNEL_IDENTITY_ALIAS}: {error}"),
    })
}

fn map_lookup_fs_error(error: FilesystemError) -> RebornUserIdentityLookupError {
    RebornUserIdentityLookupError::Backend(error.to_string())
}

fn map_binding_fs_error(error: FilesystemError) -> RebornUserIdentityBindingError {
    RebornUserIdentityBindingError::Backend(error.to_string())
}

fn decode_identity_record(
    bytes: &[u8],
) -> Result<StoredChannelUserIdentity, RebornUserIdentityBindingError> {
    serde_json::from_slice(bytes).map_err(|_| {
        RebornUserIdentityBindingError::Backend(
            "channel-identity record is invalid JSON".to_string(),
        )
    })
}

fn encode_identity_record(
    record: &StoredChannelUserIdentity,
) -> Result<Entry, RebornUserIdentityBindingError> {
    let body = serde_json::to_vec(record).map_err(|_| {
        RebornUserIdentityBindingError::Backend(
            "channel-identity record could not be serialized".to_string(),
        )
    })?;
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn map_identity_cas_error(
    error: CasUpdateError<RebornUserIdentityBindingError>,
) -> RebornUserIdentityBindingError {
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout | CasUpdateError::RetriesExhausted => {
            RebornUserIdentityBindingError::Backend(
                "channel-identity CAS retries exhausted".to_string(),
            )
        }
        CasUpdateError::CasUnsupported => RebornUserIdentityBindingError::Backend(
            "channel-identity backend does not support versioned compare-and-swap".to_string(),
        ),
        CasUpdateError::Backend(error) => map_binding_fs_error(error),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::InMemoryBackend;

    use ironclaw_host_api::user_identity::{
        RebornIdentityProviderId, RebornIdentityProviderUserId,
    };

    use super::*;

    fn store() -> FilesystemChannelIdentityStore {
        FilesystemChannelIdentityStore::new(
            Arc::new(InMemoryBackend::new()),
            TenantId::new("tenant-alpha").expect("tenant"),
            UserId::new("operator").expect("user"),
        )
    }

    fn binding(provider_user_id: &str, user: &str) -> RebornUserIdentityBinding {
        RebornUserIdentityBinding {
            provider: RebornIdentityProviderId::new("vendorx").expect("provider"),
            provider_user_id: RebornIdentityProviderUserId::new(provider_user_id)
                .expect("provider user id"),
            user_id: UserId::new(user).expect("user"),
        }
    }

    #[tokio::test]
    async fn bind_resolve_prefix_check_and_delete_round_trip() {
        let store = store();
        store
            .bind_user_identity(binding("install-1:U123", "user-alice"))
            .await
            .expect("bind");

        assert_eq!(
            store
                .resolve_user_identity("vendorx", "install-1:U123")
                .await
                .expect("resolve"),
            Some(UserId::new("user-alice").expect("user"))
        );
        assert!(
            store
                .user_has_provider_binding("vendorx", &UserId::new("user-alice").expect("user"))
                .await
                .expect("check")
        );
        assert!(
            store
                .user_has_provider_binding_with_provider_user_id_prefix(
                    "vendorx",
                    &UserId::new("user-alice").expect("user"),
                    Some("install-1:"),
                )
                .await
                .expect("prefix check")
        );
        assert!(
            !store
                .user_has_provider_binding_with_provider_user_id_prefix(
                    "vendorx",
                    &UserId::new("user-alice").expect("user"),
                    Some("install-2:"),
                )
                .await
                .expect("foreign prefix check")
        );

        let deleted = store
            .delete_user_identity_bindings_for_user(
                "vendorx",
                &UserId::new("user-alice").expect("user"),
                Some("install-1:U123"),
            )
            .await
            .expect("delete");
        assert_eq!(deleted, 1);
        assert_eq!(
            store
                .resolve_user_identity("vendorx", "install-1:U123")
                .await
                .expect("resolve after delete"),
            None
        );
        assert!(
            !store
                .user_has_provider_binding("vendorx", &UserId::new("user-alice").expect("user"))
                .await
                .expect("check after delete")
        );
    }

    #[tokio::test]
    async fn rebinding_to_a_different_user_is_rejected() {
        let store = store();
        store
            .bind_user_identity(binding("install-1:U123", "user-alice"))
            .await
            .expect("bind");

        let error = store
            .bind_user_identity(binding("install-1:U123", "user-bob"))
            .await
            .expect_err("identity already bound to another user");
        assert!(matches!(
            error,
            RebornUserIdentityBindingError::ProviderIdentityAlreadyBound
        ));

        // Same user re-binding is an idempotent refresh.
        store
            .bind_user_identity(binding("install-1:U123", "user-alice"))
            .await
            .expect("same-user rebind");
    }

    #[tokio::test]
    async fn transactional_binding_rolls_back_only_the_created_incarnation() {
        let store = store();
        let binding = binding("install-1:U123", "user-alice");

        let outcome = store
            .bind_user_identity_transactionally(binding.clone())
            .await
            .expect("transactional bind");
        let IdentityBindingTransaction::Created(receipt) = outcome else {
            panic!("first binding must carry a rollback receipt");
        };
        assert!(
            store
                .rollback_identity_binding(receipt)
                .await
                .expect("rollback")
        );
        assert_eq!(
            store
                .resolve_user_identity("vendorx", "install-1:U123")
                .await
                .expect("resolve after rollback"),
            None
        );
    }

    #[tokio::test]
    async fn later_same_identity_adoption_makes_an_older_rollback_receipt_stale() {
        let store = store();
        let binding = binding("install-1:U123", "user-alice");

        let first = store
            .bind_user_identity_transactionally(binding.clone())
            .await
            .expect("first bind");
        let IdentityBindingTransaction::Created(old_receipt) = first else {
            panic!("first binding must be created");
        };
        assert!(matches!(
            store
                .bind_user_identity_transactionally(binding)
                .await
                .expect("later adoption"),
            IdentityBindingTransaction::Existing
        ));

        assert!(
            !store
                .rollback_identity_binding(old_receipt)
                .await
                .expect("stale rollback")
        );
        assert_eq!(
            store
                .resolve_user_identity("vendorx", "install-1:U123")
                .await
                .expect("resolve adopted binding"),
            Some(UserId::new("user-alice").expect("user"))
        );
    }

    #[tokio::test]
    async fn delete_and_recreate_makes_an_older_rollback_receipt_stale() {
        let store = store();
        let first = store
            .bind_user_identity_transactionally(binding("install-1:U123", "user-alice"))
            .await
            .expect("first bind");
        let IdentityBindingTransaction::Created(old_receipt) = first else {
            panic!("first binding must be created");
        };

        assert_eq!(
            store
                .delete_user_identity_bindings_for_user(
                    "vendorx",
                    &UserId::new("user-alice").expect("user"),
                    Some("install-1:U123"),
                )
                .await
                .expect("delete first binding"),
            1
        );
        store
            .bind_user_identity(binding("install-1:U123", "user-bob"))
            .await
            .expect("replacement bind");

        assert!(
            !store
                .rollback_identity_binding(old_receipt)
                .await
                .expect("stale rollback")
        );
        assert_eq!(
            store
                .resolve_user_identity("vendorx", "install-1:U123")
                .await
                .expect("resolve replacement binding"),
            Some(UserId::new("user-bob").expect("user"))
        );
    }

    #[tokio::test]
    async fn concurrent_different_owner_binds_commit_exactly_one_identity() {
        let store = Arc::new(store());
        let alice_store = Arc::clone(&store);
        let bob_store = Arc::clone(&store);
        let (alice, bob) = tokio::join!(
            async move {
                alice_store
                    .bind_user_identity_transactionally(binding("install-1:U123", "user-alice"))
                    .await
            },
            async move {
                bob_store
                    .bind_user_identity_transactionally(binding("install-1:U123", "user-bob"))
                    .await
            }
        );

        assert_eq!(
            usize::from(alice.is_ok()) + usize::from(bob.is_ok()),
            1,
            "CAS must admit exactly one owner: alice={alice:?}, bob={bob:?}"
        );
        assert!(
            [alice.as_ref().err(), bob.as_ref().err()]
                .into_iter()
                .flatten()
                .all(|error| matches!(
                    error,
                    RebornUserIdentityBindingError::ProviderIdentityAlreadyBound
                )),
            "the losing writer must see the ownership conflict"
        );
        assert!(
            store
                .resolve_user_identity("vendorx", "install-1:U123")
                .await
                .expect("resolve winner")
                .is_some(),
            "the winning owner remains durably resolvable"
        );
    }
}
