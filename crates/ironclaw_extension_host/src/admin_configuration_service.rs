//! Manifest-driven service for tenant administrator configuration.
//!
//! Descriptors remain declarative data owned by `ironclaw_extensions`. This
//! service folds that catalog, validates operator input against it, stages
//! secret material in the tenant-shared managed scope, and publishes only
//! redacted value references through the durable configuration store.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;

use ironclaw_extensions::{AdminConfigurationGroupId, ExtensionAdminConfigurationDescriptor};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{ExtensionId, ResourceScope, SecretHandle};
use ironclaw_secrets::{SecretMaterial, SecretStore};
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};

use crate::{
    AdminConfigurationCommit, AdminConfigurationIdempotencyKey, AdminConfigurationRecord,
    AdminConfigurationRequestDigest, AdminConfigurationReserveOutcome,
    AdminConfigurationStoreError, AdminConfigurationValueRef, FilesystemAdminConfigurationStore,
};

const MAX_VALUE_BYTES: usize = 16 * 1024;
const MAX_TOTAL_VALUE_BYTES: usize = 256 * 1024;

/// Reconcile every extension that consumes one administrator configuration
/// group, reporting one aggregate failure only after all consumers had a
/// chance to refresh.
///
/// This is owner-side policy rather than composition policy: callers provide
/// the manifest-derived consumer set and a statically dispatched runtime
/// operation, while this service owns the all-consumers/partial-failure rule.
pub async fn reconcile_admin_configuration_consumers<R, Fut, E>(
    group_id: &AdminConfigurationGroupId,
    extension_ids: &BTreeSet<ExtensionId>,
    reconcile: R,
) -> Result<(), AdminConfigurationServiceError>
where
    R: Fn(ExtensionId) -> Fut,
    Fut: Future<Output = Result<(), E>>,
    E: std::fmt::Display,
{
    let mut failed_reconciliations = 0usize;
    for extension_id in extension_ids {
        if let Err(error) = reconcile(extension_id.clone()).await {
            failed_reconciliations += 1;
            tracing::warn!(
                %extension_id,
                %error,
                "extension refresh after administrator configuration failed"
            );
        }
    }
    if failed_reconciliations == 0 {
        return Ok(());
    }
    tracing::warn!(
        %group_id,
        failed_reconciliations,
        affected_extensions = extension_ids.len(),
        "administrator configuration runtime reconciliation was incomplete"
    );
    Err(AdminConfigurationServiceError::RuntimeReconciliationFailed)
}

/// One value submitted by an authenticated administrator.
///
/// `SecretMaterial` keeps even non-secret form input redacted in debug output;
/// descriptor metadata decides which values are returned by the query view.
pub struct AdminConfigurationSubmittedValue {
    pub handle: SecretHandle,
    pub value: SecretMaterial,
}

/// Redacted query state for one manifest-declared field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminConfigurationFieldState {
    pub handle: SecretHandle,
    pub label: String,
    pub secret: bool,
    pub required: bool,
    pub provided: bool,
    pub value: Option<String>,
}

/// Redacted query state for one manifest-declared configuration group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminConfigurationGroupState {
    pub group_id: AdminConfigurationGroupId,
    pub display_name: String,
    pub description: String,
    pub revision: u64,
    pub complete: bool,
    pub fields: Vec<AdminConfigurationFieldState>,
}

/// Stable, value-free service failures suitable for an API adapter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdminConfigurationServiceError {
    #[error("admin-configuration descriptor is invalid")]
    InvalidDescriptor,
    #[error("admin-configuration group descriptors conflict")]
    DescriptorConflict,
    #[error("admin-configuration group is unknown")]
    UnknownGroup,
    #[error("admin-configuration field is unknown")]
    UnknownField,
    #[error("admin-configuration field was submitted more than once")]
    DuplicateField,
    #[error("required admin-configuration field is missing")]
    MissingRequiredField,
    #[error("admin-configuration value exceeds its size limit")]
    ValueTooLarge,
    #[error("admin-configuration idempotency key conflicts with an earlier request")]
    IdempotencyConflict,
    #[error("admin-configuration revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("admin-configuration runtime reconciliation failed")]
    RuntimeReconciliationFailed,
    #[error("admin-configuration runtime rollback failed")]
    RuntimeRollbackFailed,
    #[error("admin-configuration service is unavailable")]
    Unavailable,
}

/// Concrete host service over the two genuine storage substrates.
///
/// There is deliberately no service trait or vendor branch. Runtime
/// polymorphism already exists at the filesystem and secret-store boundaries;
/// configuration semantics are shared by every manifest descriptor.
pub struct AdminConfigurationService<F, S>
where
    F: RootFilesystem + ?Sized,
    S: SecretStore + ?Sized,
{
    store: FilesystemAdminConfigurationStore<F>,
    secrets: Arc<S>,
    descriptors: BTreeMap<AdminConfigurationGroupId, ExtensionAdminConfigurationDescriptor>,
}

impl<F, S> AdminConfigurationService<F, S>
where
    F: RootFilesystem + ?Sized,
    S: SecretStore + ?Sized,
{
    pub fn new(
        store: FilesystemAdminConfigurationStore<F>,
        secrets: Arc<S>,
        descriptors: impl IntoIterator<Item = ExtensionAdminConfigurationDescriptor>,
    ) -> Result<Self, AdminConfigurationServiceError> {
        let mut folded = BTreeMap::new();
        for descriptor in descriptors {
            descriptor
                .validate()
                .map_err(|_| AdminConfigurationServiceError::InvalidDescriptor)?;
            match folded.get(&descriptor.group_id) {
                Some(existing) if existing != &descriptor => {
                    return Err(AdminConfigurationServiceError::DescriptorConflict);
                }
                Some(_) => {}
                None => {
                    folded.insert(descriptor.group_id.clone(), descriptor);
                }
            }
        }
        Ok(Self {
            store,
            secrets,
            descriptors: folded,
        })
    }

    /// Return every manifest-declared group, including groups used only by
    /// extensions that have not been installed yet.
    pub async fn list(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<AdminConfigurationGroupState>, AdminConfigurationServiceError> {
        let mut groups = Vec::with_capacity(self.descriptors.len());
        for (group_id, descriptor) in &self.descriptors {
            let record = self
                .store
                .get(scope, group_id)
                .await
                .map_err(map_store_error)?;
            let commit = record.as_ref().map(commit_from_record);
            groups.push(render_group(descriptor, commit.as_ref()));
        }
        Ok(groups)
    }

    /// Return one manifest-declared group as a redacted revisioned view.
    pub async fn get(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
    ) -> Result<AdminConfigurationGroupState, AdminConfigurationServiceError> {
        let descriptor = self
            .descriptors
            .get(group_id)
            .ok_or(AdminConfigurationServiceError::UnknownGroup)?;
        let record = self
            .store
            .get(scope, group_id)
            .await
            .map_err(map_store_error)?;
        let commit = record.as_ref().map(commit_from_record);
        Ok(render_group(descriptor, commit.as_ref()))
    }

    /// Resolve one non-secret value for a runtime consumer. The manifest
    /// descriptor remains authoritative for both the group and field kind;
    /// callers cannot use this as a generic record reader.
    pub async fn non_secret_value(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        handle: &SecretHandle,
    ) -> Result<Option<String>, AdminConfigurationServiceError> {
        let descriptor = self
            .descriptors
            .get(group_id)
            .ok_or(AdminConfigurationServiceError::UnknownGroup)?;
        let field = descriptor
            .fields
            .iter()
            .find(|field| &field.handle == handle)
            .ok_or(AdminConfigurationServiceError::UnknownField)?;
        if field.secret {
            return Err(AdminConfigurationServiceError::UnknownField);
        }
        let record = self
            .store
            .get(scope, group_id)
            .await
            .map_err(map_store_error)?;
        Ok(record
            .and_then(|record| record.values.get(handle).cloned())
            .and_then(|value| match value {
                AdminConfigurationValueRef::Inline(value) => Some(value),
                AdminConfigurationValueRef::Secret(_) => None,
            }))
    }

    /// Consume one secret value for a trusted runtime consumer. Secret bytes
    /// stay behind the existing one-shot lease boundary and are never returned
    /// by the operator query projection.
    pub async fn secret_material(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, AdminConfigurationServiceError> {
        let descriptor = self
            .descriptors
            .get(group_id)
            .ok_or(AdminConfigurationServiceError::UnknownGroup)?;
        let field = descriptor
            .fields
            .iter()
            .find(|field| &field.handle == handle)
            .ok_or(AdminConfigurationServiceError::UnknownField)?;
        if !field.secret {
            return Err(AdminConfigurationServiceError::UnknownField);
        }
        let record = self
            .store
            .get(scope, group_id)
            .await
            .map_err(map_store_error)?;
        let Some(AdminConfigurationValueRef::Secret(stored_handle)) =
            record.and_then(|record| record.values.get(handle).cloned())
        else {
            return Ok(None);
        };
        let shared_scope = scope.tenant_shared_managed_scope();
        let lease = self
            .secrets
            .lease_once(&shared_scope, &stored_handle)
            .await
            .map_err(|error| {
                tracing::warn!(error = ?error, "admin-configuration secret lease failed");
                AdminConfigurationServiceError::Unavailable
            })?;
        self.secrets
            .consume(&shared_scope, lease.id)
            .await
            .map(Some)
            .map_err(|error| {
                tracing::warn!(error = ?error, "admin-configuration secret consume failed");
                AdminConfigurationServiceError::Unavailable
            })
    }

    /// Replace one group using client-owned concurrency and retry identities.
    pub async fn replace(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        idempotency_key: &AdminConfigurationIdempotencyKey,
        expected_revision: u64,
        submitted: Vec<AdminConfigurationSubmittedValue>,
    ) -> Result<AdminConfigurationGroupState, AdminConfigurationServiceError> {
        self.replace_with_reconcile(
            scope,
            group_id,
            idempotency_key,
            expected_revision,
            submitted,
            || std::future::ready(Ok(())),
        )
        .await
    }

    /// Replace one group and reconcile every runtime consumer as one bounded
    /// saga owned by the configuration service.
    ///
    /// The candidate revision is published before `reconcile` so existing
    /// runtime resolvers observe one authoritative configuration. Reconcile
    /// failure rolls back that exact revision with CAS, preserving a later
    /// concurrent writer. Old secret material is retained until reconcile
    /// succeeds; on rollback the candidate's newly staged secrets are
    /// removed and `reconcile` runs again against the restored record. An
    /// exact idempotent replay still calls `reconcile`, healing interruption
    /// after durable publication but before runtime refresh.
    pub async fn replace_with_reconcile<R, Fut>(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        idempotency_key: &AdminConfigurationIdempotencyKey,
        expected_revision: u64,
        submitted: Vec<AdminConfigurationSubmittedValue>,
        reconcile: R,
    ) -> Result<AdminConfigurationGroupState, AdminConfigurationServiceError>
    where
        R: Fn() -> Fut,
        Fut: Future<Output = Result<(), AdminConfigurationServiceError>>,
    {
        let descriptor = self
            .descriptors
            .get(group_id)
            .ok_or(AdminConfigurationServiceError::UnknownGroup)?;
        let previous = self
            .store
            .get(scope, group_id)
            .await
            .map_err(map_store_error)?;
        let validated = validate_submitted(descriptor, previous.as_ref(), submitted)?;
        let request_digest = request_digest(descriptor, expected_revision, &validated);
        let reservation = self
            .store
            .reserve(
                scope,
                group_id,
                idempotency_key,
                request_digest,
                expected_revision,
            )
            .await
            .map_err(map_store_error)?;
        let reservation = match reservation {
            AdminConfigurationReserveOutcome::Replay(commit) => {
                reconcile().await?;
                return Ok(render_group(descriptor, Some(&commit)));
            }
            AdminConfigurationReserveOutcome::Reserved(reservation) => reservation,
        };

        let shared_scope = scope.tenant_shared_managed_scope();
        let mut effective = BTreeMap::new();
        let mut staged_handles = Vec::new();
        for field in &descriptor.fields {
            let submitted_value = validated.get(&field.handle);
            if field.secret {
                let exposed = submitted_value.map(ExposeSecret::expose_secret);
                if let Some(value) = exposed.filter(|value| !value.is_empty()) {
                    let handle =
                        staged_secret_handle(group_id, &field.handle, reservation.revision)?;
                    // `put` may report an ambiguous backend failure after the
                    // write landed. Register the deterministic handle first so
                    // every error path attempts cleanup.
                    staged_handles.push(handle.clone());
                    if let Err(error) = self
                        .secrets
                        .put(
                            shared_scope.clone(),
                            handle.clone(),
                            SecretMaterial::from(value.to_string()),
                            None,
                        )
                        .await
                    {
                        tracing::warn!(error = ?error, "admin-configuration secret staging failed");
                        self.cleanup_staged(&shared_scope, &staged_handles).await;
                        return Err(AdminConfigurationServiceError::Unavailable);
                    }
                    effective.insert(
                        field.handle.clone(),
                        AdminConfigurationValueRef::Secret(handle),
                    );
                } else if let Some(AdminConfigurationValueRef::Secret(handle)) = previous
                    .as_ref()
                    .and_then(|record| record.values.get(&field.handle))
                {
                    effective.insert(
                        field.handle.clone(),
                        AdminConfigurationValueRef::Secret(handle.clone()),
                    );
                }
            } else if let Some(value) = submitted_value {
                effective.insert(
                    field.handle.clone(),
                    AdminConfigurationValueRef::Inline(value.expose_secret().to_string()),
                );
            }
        }

        let committed = match self.store.commit(scope, &reservation, effective).await {
            Ok(commit) => commit,
            Err(error) => {
                if !self
                    .reservation_may_be_published(scope, group_id, reservation.revision)
                    .await
                {
                    self.cleanup_staged(&shared_scope, &staged_handles).await;
                }
                return Err(map_store_error(error));
            }
        };
        if let Err(reconcile_error) = reconcile().await {
            if let Err(rollback_error) = self
                .store
                .rollback_commit(scope, &reservation, previous.as_ref())
                .await
            {
                tracing::warn!(
                    error = ?rollback_error,
                    failed_revision = reservation.revision,
                    "admin-configuration runtime reconciliation failed and the durable revision could not be rolled back"
                );
                return Err(AdminConfigurationServiceError::RuntimeRollbackFailed);
            }
            self.cleanup_staged(&shared_scope, &staged_handles).await;
            if let Err(restore_error) = reconcile().await {
                tracing::warn!(
                    error = ?restore_error,
                    restored_revision = reservation.expected_revision,
                    "admin-configuration durable rollback succeeded but runtime restoration failed"
                );
                return Err(AdminConfigurationServiceError::RuntimeRollbackFailed);
            }
            return Err(reconcile_error);
        }
        self.cleanup_replaced_secrets(&shared_scope, previous.as_ref(), &committed)
            .await;
        Ok(render_group(descriptor, Some(&committed)))
    }

    async fn reservation_may_be_published(
        &self,
        scope: &ResourceScope,
        group_id: &AdminConfigurationGroupId,
        revision: u64,
    ) -> bool {
        match self.store.get(scope, group_id).await {
            Ok(Some(record)) => record.revision == revision,
            Ok(None) => false,
            Err(error) => {
                tracing::warn!(error = ?error, "admin-configuration publication check failed");
                true
            }
        }
    }

    async fn cleanup_staged(&self, scope: &ResourceScope, handles: &[SecretHandle]) {
        for handle in handles {
            if let Err(error) = self.secrets.delete(scope, handle).await {
                tracing::warn!(error = ?error, "admin-configuration staged secret cleanup failed");
            }
        }
    }

    async fn cleanup_replaced_secrets(
        &self,
        scope: &ResourceScope,
        previous: Option<&AdminConfigurationRecord>,
        committed: &AdminConfigurationCommit,
    ) {
        let retained = committed
            .values
            .values()
            .filter_map(|value| match value {
                AdminConfigurationValueRef::Secret(handle) => Some(handle),
                AdminConfigurationValueRef::Inline(_) => None,
            })
            .collect::<BTreeSet<_>>();
        let Some(previous) = previous else {
            return;
        };
        for value in previous.values.values() {
            let AdminConfigurationValueRef::Secret(handle) = value else {
                continue;
            };
            if retained.contains(handle) {
                continue;
            }
            if let Err(error) = self.secrets.delete(scope, handle).await {
                tracing::warn!(error = ?error, "admin-configuration replaced secret cleanup failed");
            }
        }
    }
}

fn validate_submitted(
    descriptor: &ExtensionAdminConfigurationDescriptor,
    previous: Option<&AdminConfigurationRecord>,
    submitted: Vec<AdminConfigurationSubmittedValue>,
) -> Result<BTreeMap<SecretHandle, SecretMaterial>, AdminConfigurationServiceError> {
    let declared = descriptor
        .fields
        .iter()
        .map(|field| &field.handle)
        .collect::<BTreeSet<_>>();
    let mut validated = BTreeMap::new();
    let mut total_bytes = 0usize;
    for value in submitted {
        if !declared.contains(&value.handle) {
            return Err(AdminConfigurationServiceError::UnknownField);
        }
        let value_bytes = value.value.expose_secret().len();
        if value_bytes > MAX_VALUE_BYTES {
            return Err(AdminConfigurationServiceError::ValueTooLarge);
        }
        total_bytes = total_bytes
            .checked_add(value_bytes)
            .ok_or(AdminConfigurationServiceError::ValueTooLarge)?;
        if total_bytes > MAX_TOTAL_VALUE_BYTES {
            return Err(AdminConfigurationServiceError::ValueTooLarge);
        }
        if validated.insert(value.handle, value.value).is_some() {
            return Err(AdminConfigurationServiceError::DuplicateField);
        }
    }
    for field in &descriptor.fields {
        if !field.required {
            continue;
        }
        let present = match validated.get(&field.handle) {
            Some(value) if field.secret && !value.expose_secret().is_empty() => true,
            Some(value) if !field.secret && !value.expose_secret().trim().is_empty() => true,
            _ if field.secret => matches!(
                previous.and_then(|record| record.values.get(&field.handle)),
                Some(AdminConfigurationValueRef::Secret(_))
            ),
            _ => false,
        };
        if !present {
            return Err(AdminConfigurationServiceError::MissingRequiredField);
        }
    }
    Ok(validated)
}

fn request_digest(
    descriptor: &ExtensionAdminConfigurationDescriptor,
    expected_revision: u64,
    submitted: &BTreeMap<SecretHandle, SecretMaterial>,
) -> AdminConfigurationRequestDigest {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, b"ironclaw-admin-configuration-v1");
    hash_part(&mut hasher, descriptor.group_id.as_str().as_bytes());
    hash_part(&mut hasher, &expected_revision.to_be_bytes());
    for field in &descriptor.fields {
        hash_part(&mut hasher, field.handle.as_str().as_bytes());
        let value = submitted.get(&field.handle);
        if field.secret {
            // Idempotency is intentionally key-dominant for secret material:
            // the durable digest records only submission state, never a plain
            // verifier of secret bytes. Reusing a completed key with another
            // nonblank secret replays the original action; a new click must
            // mint a new client idempotency key.
            let marker = match value.map(ExposeSecret::expose_secret) {
                None => b"secret:missing".as_slice(),
                Some("") => b"secret:blank".as_slice(),
                Some(_) => b"secret:nonblank".as_slice(),
            };
            hash_part(&mut hasher, marker);
        } else {
            match value {
                Some(value) => hash_part(&mut hasher, value.expose_secret().as_bytes()),
                None => hash_part(&mut hasher, b"nonsecret:missing"),
            }
        }
    }
    AdminConfigurationRequestDigest::from_bytes(hasher.finalize().into())
}

fn staged_secret_handle(
    group_id: &AdminConfigurationGroupId,
    field: &SecretHandle,
    revision: u64,
) -> Result<SecretHandle, AdminConfigurationServiceError> {
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, group_id.as_str().as_bytes());
    hash_part(&mut hasher, field.as_str().as_bytes());
    let digest = hasher.finalize();
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    SecretHandle::new(format!("admincfg-r{revision}-{suffix}"))
        .map_err(|_| AdminConfigurationServiceError::InvalidDescriptor)
}

fn hash_part(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn commit_from_record(record: &AdminConfigurationRecord) -> AdminConfigurationCommit {
    AdminConfigurationCommit {
        revision: record.revision,
        values: record.values.clone(),
    }
}

fn render_group(
    descriptor: &ExtensionAdminConfigurationDescriptor,
    commit: Option<&AdminConfigurationCommit>,
) -> AdminConfigurationGroupState {
    let fields = descriptor
        .fields
        .iter()
        .map(|field| {
            let stored = commit.and_then(|commit| commit.values.get(&field.handle));
            let (provided, value) = match stored {
                Some(AdminConfigurationValueRef::Inline(value)) => {
                    (!value.trim().is_empty(), Some(value.clone()))
                }
                Some(AdminConfigurationValueRef::Secret(_)) => (true, None),
                None => (false, None),
            };
            AdminConfigurationFieldState {
                handle: field.handle.clone(),
                label: field.label.clone(),
                secret: field.secret,
                required: field.required,
                provided,
                value,
            }
        })
        .collect::<Vec<_>>();
    let complete = fields.iter().all(|field| !field.required || field.provided);
    AdminConfigurationGroupState {
        group_id: descriptor.group_id.clone(),
        display_name: descriptor.display_name.clone(),
        description: descriptor.description.clone(),
        revision: commit.map_or(0, |commit| commit.revision),
        complete,
        fields,
    }
}

fn map_store_error(error: AdminConfigurationStoreError) -> AdminConfigurationServiceError {
    match error {
        AdminConfigurationStoreError::IdempotencyConflict => {
            AdminConfigurationServiceError::IdempotencyConflict
        }
        AdminConfigurationStoreError::RevisionConflict { expected, actual } => {
            AdminConfigurationServiceError::RevisionConflict { expected, actual }
        }
        AdminConfigurationStoreError::InvalidIdempotencyKey
        | AdminConfigurationStoreError::IdempotencyCapacityExhausted
        | AdminConfigurationStoreError::UnknownReservation
        | AdminConfigurationStoreError::StaleReservation
        | AdminConfigurationStoreError::InvalidRecord
        | AdminConfigurationStoreError::CasUnsupported
        | AdminConfigurationStoreError::Contended
        | AdminConfigurationStoreError::Unavailable => AdminConfigurationServiceError::Unavailable,
    }
}
