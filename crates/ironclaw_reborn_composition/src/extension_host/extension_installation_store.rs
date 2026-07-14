use async_trait::async_trait;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionHealthSnapshot, ExtensionInstallation,
    ExtensionInstallationError, ExtensionInstallationId, ExtensionInstallationPersistedParts,
    ExtensionInstallationStore, ExtensionManifestRecord, ExtensionManifestRef,
    ExtensionRemovalCleanupRequirement, InMemoryExtensionInstallationStore,
    MANIFEST_SCHEMA_VERSION, MAX_MANIFEST_BYTES, ManifestHash, ManifestSource, ManifestV2Error,
    canonicalize_installation_rows,
};
use ironclaw_filesystem::{
    CasApply, CasUpdateError, Entry, FilesystemError, RootFilesystem, cas_update_root,
};
use ironclaw_host_api::{ExtensionId, VirtualPath, sha256_digest_token};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc, oneshot};

use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;

const DEFAULT_INSTALLATION_STATE_PATH: &str = "/system/extensions/.installations/state.json";
const INSTALLATION_STATE_IO_ERROR: &str = "failed to load extension installation state";
const MUTATION_QUEUE_CAPACITY: usize = 32;
const PRE_TRAIN_A_SLACK_MANIFEST_HASH: &str =
    "sha256:851f9f08a3d11bd2f1dfbc0318c6e6f642442e4720c4edbe8b553f7343e63d0f";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NonCasLoadPolicy {
    /// Hosted and durable backends must provide versioned compare-and-swap.
    RequireCas,
    /// Local-development-only compatibility for the byte filesystem. Mutations
    /// are serialized by one process-local worker and persisted with atomic
    /// file replacement; this is not a multi-process coordination contract.
    AllowNonCasLocalDev,
}

pub(crate) struct FilesystemExtensionInstallationStore {
    published: std::sync::Arc<RwLock<PublishedState>>,
    mutation_tx: mpsc::Sender<MutationRequest>,
}

struct PublishedState {
    generation: u64,
    inner: InMemoryExtensionInstallationStore,
}

struct PublishedStateUpdate {
    generation: u64,
    state: WireState,
}

struct MutationRequest {
    mutation: WireStateMutation,
    result_tx: oneshot::Sender<Result<(), ExtensionInstallationError>>,
}

#[derive(Clone, Copy)]
enum MutationMode {
    Cas,
    LocalNonCas,
}

#[derive(Clone)]
enum WireStateMutation {
    UpsertManifest(Box<ExtensionManifestRecord>),
    UpsertManifestAndInstallation(Box<(ExtensionManifestRecord, ExtensionInstallation)>),
    UpsertInstallation(Box<ExtensionInstallation>),
    SetActivationState(ExtensionInstallationId, ExtensionActivationState),
    DeleteInstallation(ExtensionInstallationId),
    DeleteManifest(ExtensionId),
    UpdateHealth(ExtensionInstallationId, ExtensionHealthSnapshot),
}

impl FilesystemExtensionInstallationStore {
    pub(crate) async fn load_at(
        filesystem: std::sync::Arc<dyn RootFilesystem>,
        state_path: VirtualPath,
    ) -> Result<Self, ExtensionInstallationError> {
        Self::load_at_with_policy(filesystem, state_path, NonCasLoadPolicy::RequireCas).await
    }

    pub(crate) async fn load_at_with_policy(
        filesystem: std::sync::Arc<dyn RootFilesystem>,
        state_path: VirtualPath,
        non_cas_policy: NonCasLoadPolicy,
    ) -> Result<Self, ExtensionInstallationError> {
        let inner = InMemoryExtensionInstallationStore::default();
        let (state, mutation_mode) =
            match load_normalized_snapshot(filesystem.as_ref(), &state_path).await {
                Ok(state) => (state, MutationMode::Cas),
                Err(CasUpdateError::Apply(error)) => return Err(error),
                Err(error @ CasUpdateError::CasUnsupported) => {
                    if non_cas_policy == NonCasLoadPolicy::AllowNonCasLocalDev {
                        (
                            load_normalized_snapshot_without_cas(filesystem.as_ref(), &state_path)
                                .await?,
                            MutationMode::LocalNonCas,
                        )
                    } else {
                        return Err(map_load_backend_error(&state_path, &error));
                    }
                }
                Err(error) => {
                    return Err(map_load_backend_error(&state_path, &error));
                }
            };
        if let Some(state) = state {
            // The CAS outcome comes from the winning snapshot. Loading only
            // after the bounded update finishes prevents a losing attempt from
            // exposing stale or partially normalized state in memory.
            state.load_into(&inner).await?;
        }
        let published = std::sync::Arc::new(RwLock::new(PublishedState {
            generation: 0,
            inner,
        }));
        let mutation_tx = spawn_mutation_worker(
            filesystem,
            state_path,
            std::sync::Arc::clone(&published),
            mutation_mode,
            non_cas_policy == NonCasLoadPolicy::AllowNonCasLocalDev,
        );
        Ok(Self {
            published,
            mutation_tx,
        })
    }

    pub(crate) fn default_state_path() -> Result<VirtualPath, ExtensionInstallationError> {
        default_installation_state_path()
    }

    async fn current_inner(&self) -> InMemoryExtensionInstallationStore {
        self.published.read().await.inner.clone()
    }

    async fn apply_mutation(
        &self,
        mutation: WireStateMutation,
    ) -> Result<(), ExtensionInstallationError> {
        let (result_tx, result_rx) = oneshot::channel();
        self.mutation_tx
            .send(MutationRequest {
                mutation,
                result_tx,
            })
            .await
            .map_err(|_| invalid_installation_error(INSTALLATION_STATE_IO_ERROR))?;
        result_rx
            .await
            .map_err(|_| invalid_installation_error(INSTALLATION_STATE_IO_ERROR))?
    }
}

fn spawn_mutation_worker(
    filesystem: std::sync::Arc<dyn RootFilesystem>,
    state_path: VirtualPath,
    published: std::sync::Arc<RwLock<PublishedState>>,
    initial_mode: MutationMode,
    allow_non_cas_local_dev: bool,
) -> mpsc::Sender<MutationRequest> {
    let (mutation_tx, mut mutation_rx) = mpsc::channel::<MutationRequest>(MUTATION_QUEUE_CAPACITY);
    tokio::spawn(async move {
        let mut generation = 0_u64;
        let mut mode = initial_mode;
        while let Some(request) = mutation_rx.recv().await {
            let result = match generation.checked_add(1) {
                Some(next_generation) => {
                    generation = next_generation;
                    let update = match mode {
                        MutationMode::Cas => match apply_cas_mutation_request(
                            filesystem.as_ref(),
                            &state_path,
                            request.mutation.clone(),
                            generation,
                        )
                        .await
                        {
                            Ok(update) => Ok(update),
                            Err(CasUpdateError::CasUnsupported) if allow_non_cas_local_dev => {
                                // Composite/local routers cannot always advertise the
                                // capabilities of the selected mount. A missing or already
                                // normalized snapshot can therefore make the load-time CAS
                                // probe a no-op. Discover the limitation on the first real
                                // mutation, switch this store-owned worker once, and keep all
                                // later local writes serialized through it.
                                mode = MutationMode::LocalNonCas;
                                apply_local_mutation_request(
                                    filesystem.as_ref(),
                                    &state_path,
                                    request.mutation,
                                    generation,
                                )
                                .await
                            }
                            Err(error) => Err(map_store_cas_error(&state_path, error)),
                        },
                        MutationMode::LocalNonCas => {
                            apply_local_mutation_request(
                                filesystem.as_ref(),
                                &state_path,
                                request.mutation,
                                generation,
                            )
                            .await
                        }
                    };
                    match update {
                        Ok(update) => publish_update(&published, update).await,
                        Err(error) => Err(error),
                    }
                }
                None => Err(invalid_installation_error(
                    "extension mutation generation overflowed",
                )),
            };
            let _send_result = request.result_tx.send(result);
        }
    });
    mutation_tx
}

async fn apply_cas_mutation_request(
    filesystem: &dyn RootFilesystem,
    state_path: &VirtualPath,
    mutation: WireStateMutation,
    generation: u64,
) -> Result<PublishedStateUpdate, CasUpdateError<ExtensionInstallationError>> {
    cas_update_root(
        filesystem,
        state_path,
        decode_wire_state,
        encode_wire_state,
        |current| {
            let mutation = mutation.clone();
            async move {
                let current = normalize_wire_state(current.unwrap_or_default())?;
                let candidate = InMemoryExtensionInstallationStore::default();
                current.load_into(&candidate).await?;
                mutation.apply_to(&candidate).await?;
                let state = WireState::from_store(&candidate).await?;
                Ok(CasApply::new(
                    state.clone(),
                    PublishedStateUpdate { generation, state },
                ))
            }
        },
    )
    .await
}

async fn publish_update(
    published: &RwLock<PublishedState>,
    update: PublishedStateUpdate,
) -> Result<(), ExtensionInstallationError> {
    // Build the replacement projection before taking the publication lock.
    // Backend I/O and validation therefore never park readers behind the
    // store-owned mutation worker.
    let replacement = InMemoryExtensionInstallationStore::default();
    update.state.load_into(&replacement).await?;
    let mut current = published.write().await;
    if update.generation > current.generation {
        *current = PublishedState {
            generation: update.generation,
            inner: replacement,
        };
    }
    Ok(())
}

async fn apply_local_mutation_request(
    filesystem: &dyn RootFilesystem,
    state_path: &VirtualPath,
    mutation: WireStateMutation,
    generation: u64,
) -> Result<PublishedStateUpdate, ExtensionInstallationError> {
    let current = match filesystem
        .get(state_path)
        .await
        .map_err(|error| map_filesystem_load_error(state_path, &error))?
    {
        Some(versioned) => decode_wire_state(&versioned.entry.body)?,
        None => WireState::default(),
    };
    let current = normalize_wire_state(current)?;
    let candidate = InMemoryExtensionInstallationStore::default();
    current.load_into(&candidate).await?;
    mutation.apply_to(&candidate).await?;
    let state = WireState::from_store(&candidate).await?;
    write_snapshot_without_cas(filesystem, state_path, &state).await?;
    Ok(PublishedStateUpdate { generation, state })
}

impl WireStateMutation {
    async fn apply_to(
        &self,
        store: &InMemoryExtensionInstallationStore,
    ) -> Result<(), ExtensionInstallationError> {
        self.reject_retired_slack_write()?;
        match self {
            Self::UpsertManifest(manifest) => {
                store.upsert_manifest(manifest.as_ref().clone()).await
            }
            Self::UpsertManifestAndInstallation(plan) => {
                store
                    .upsert_manifest_and_installation(plan.0.clone(), plan.1.clone())
                    .await
            }
            Self::UpsertInstallation(installation) => {
                store
                    .upsert_installation(installation.as_ref().clone())
                    .await
            }
            Self::SetActivationState(installation_id, state) => {
                store.set_activation_state(installation_id, *state).await
            }
            Self::DeleteInstallation(installation_id) => {
                store.delete_installation(installation_id).await
            }
            Self::DeleteManifest(extension_id) => store.delete_manifest(extension_id).await,
            Self::UpdateHealth(installation_id, health) => {
                store.update_health(installation_id, health.clone()).await
            }
        }
    }

    fn reject_retired_slack_write(&self) -> Result<(), ExtensionInstallationError> {
        let is_retired = |extension_id: &str| matches!(extension_id, "slack_bot" | "slack_user"); // taxonomy-allow: retired-normal-write-rejection
        let writes_retired_id = match self {
            Self::UpsertManifest(manifest) => is_retired(manifest.manifest().id.as_str()),
            Self::UpsertManifestAndInstallation(pair) => {
                is_retired(pair.0.manifest().id.as_str())
                    || is_retired(pair.1.extension_id().as_str())
            }
            Self::UpsertInstallation(installation) => {
                is_retired(installation.extension_id().as_str())
            }
            Self::SetActivationState(installation_id, _)
            | Self::UpdateHealth(installation_id, _) => is_retired(installation_id.as_str()),
            // Destructive operations stay available for recovery and cleanup;
            // unlike the mutations above, they cannot reintroduce retired state.
            Self::DeleteInstallation(_) | Self::DeleteManifest(_) => false,
        };
        if writes_retired_id {
            return Err(invalid_installation_error(
                "retired Slack extension ids cannot be written after Train A migration",
            ));
        }
        Ok(())
    }
}

async fn load_normalized_snapshot(
    filesystem: &dyn RootFilesystem,
    state_path: &VirtualPath,
) -> Result<Option<WireState>, CasUpdateError<ExtensionInstallationError>> {
    cas_update_root(
        filesystem,
        state_path,
        decode_wire_state,
        encode_wire_state,
        |current| async move {
            let Some(current) = current else {
                return Ok(CasApply::no_op(WireState::default(), None));
            };
            let normalized = normalize_wire_state(current)?;
            validate_wire_state(&normalized).await?;
            Ok(CasApply::new(normalized.clone(), Some(normalized)))
        },
    )
    .await
}

async fn load_normalized_snapshot_without_cas(
    filesystem: &dyn RootFilesystem,
    state_path: &VirtualPath,
) -> Result<Option<WireState>, ExtensionInstallationError> {
    let Some(versioned) = filesystem
        .get(state_path)
        .await
        .map_err(|error| map_filesystem_load_error(state_path, &error))?
    else {
        return Ok(None);
    };
    let current = decode_wire_state(&versioned.entry.body)?;
    let normalized = normalize_wire_state(current.clone())?;
    validate_wire_state(&normalized).await?;
    if normalized != current {
        tracing::warn!(
            state_path = %state_path.as_str(),
            "extension installation state was normalized in memory but not persisted because the filesystem backend does not support CAS"
        );
    }
    Ok(Some(normalized))
}

fn decode_wire_state(bytes: &[u8]) -> Result<WireState, ExtensionInstallationError> {
    serde_json::from_slice(bytes).map_err(invalid_installation_error)
}

fn encode_wire_state(state: &WireState) -> Result<Entry, ExtensionInstallationError> {
    serde_json::to_vec_pretty(state)
        .map(Entry::bytes)
        .map_err(invalid_installation_error)
}

async fn validate_wire_state(state: &WireState) -> Result<(), ExtensionInstallationError> {
    let candidate = InMemoryExtensionInstallationStore::default();
    state.load_into(&candidate).await
}

fn map_load_backend_error(
    state_path: &VirtualPath,
    error: &CasUpdateError<ExtensionInstallationError>,
) -> ExtensionInstallationError {
    tracing::debug!(
        ?error,
        state_path = %state_path.as_str(),
        "extension installation state CAS load failed"
    );
    invalid_installation_error(INSTALLATION_STATE_IO_ERROR)
}

fn map_filesystem_load_error(
    state_path: &VirtualPath,
    error: &FilesystemError,
) -> ExtensionInstallationError {
    tracing::debug!(
        ?error,
        state_path = %state_path.as_str(),
        "extension installation state compatibility load failed"
    );
    invalid_installation_error(INSTALLATION_STATE_IO_ERROR)
}

/// Compatibility writer for the explicitly opted-in local-development
/// backend, which has atomic file replacement but no version CAS. A dedicated
/// per-store worker serializes these transitions without holding a mutex over
/// backend I/O. Hosted and CAS-capable backends never enter this path.
async fn write_snapshot_without_cas(
    filesystem: &dyn RootFilesystem,
    state_path: &VirtualPath,
    state: &WireState,
) -> Result<(), ExtensionInstallationError> {
    let bytes = serde_json::to_vec_pretty(state).map_err(invalid_installation_error)?;
    filesystem
        .write_file(state_path, &bytes)
        .await
        .map_err(|error| {
            tracing::debug!(
                ?error,
                state_path = %state_path.as_str(),
                "local extension installation state write failed"
            );
            invalid_installation_error(INSTALLATION_STATE_IO_ERROR)
        })
}

fn map_store_cas_error(
    state_path: &VirtualPath,
    error: CasUpdateError<ExtensionInstallationError>,
) -> ExtensionInstallationError {
    match error {
        CasUpdateError::Apply(error) => error,
        error => map_load_backend_error(state_path, &error),
    }
}

fn default_installation_state_path() -> Result<VirtualPath, ExtensionInstallationError> {
    VirtualPath::new(DEFAULT_INSTALLATION_STATE_PATH).map_err(|error| {
        ExtensionInstallationError::InvalidInstallation {
            reason: error.to_string(),
        }
    })
}

#[async_trait]
impl ExtensionInstallationStore for FilesystemExtensionInstallationStore {
    async fn list_manifests(
        &self,
    ) -> Result<Vec<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.current_inner().await.list_manifests().await
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionInstallationError> {
        self.current_inner().await.get_manifest(extension_id).await
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionInstallationError> {
        self.apply_mutation(WireStateMutation::UpsertManifest(Box::new(manifest)))
            .await
    }

    async fn upsert_manifest_and_installation(
        &self,
        manifest: ExtensionManifestRecord,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        self.apply_mutation(WireStateMutation::UpsertManifestAndInstallation(Box::new(
            (manifest, installation),
        )))
        .await
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        self.current_inner().await.list_installations().await
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionInstallationError> {
        self.current_inner()
            .await
            .list_enabled_installations()
            .await
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionInstallationError> {
        self.current_inner()
            .await
            .get_installation(installation_id)
            .await
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionInstallationError> {
        self.apply_mutation(WireStateMutation::UpsertInstallation(Box::new(
            installation,
        )))
        .await
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionInstallationError> {
        self.apply_mutation(WireStateMutation::SetActivationState(
            installation_id.clone(),
            state,
        ))
        .await
    }

    async fn delete_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<(), ExtensionInstallationError> {
        self.apply_mutation(WireStateMutation::DeleteInstallation(
            installation_id.clone(),
        ))
        .await
    }

    async fn delete_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<(), ExtensionInstallationError> {
        self.apply_mutation(WireStateMutation::DeleteManifest(extension_id.clone()))
            .await
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionInstallationError> {
        self.apply_mutation(WireStateMutation::UpdateHealth(
            installation_id.clone(),
            health,
        ))
        .await
    }
}

/// Pure, rerunnable persisted-state normalization. The transition is private
/// to this store: public manifest ingestion remains strict and continues to
/// reject the retired top-level capability shape.
fn normalize_wire_state(mut state: WireState) -> Result<WireState, ExtensionInstallationError> {
    // Capture proof from the original bytes before the generic legacy-shape
    // converter rewrites the predecessor tools-only Slack manifest. This
    // digest is the exact host-bundled record shipped immediately before
    // Train A; no source label or caller-supplied manifest hash is trusted as
    // proof on its own.
    let has_exact_pre_train_a_slack = state
        .manifests
        .iter()
        .any(is_exact_pre_train_a_slack_record);
    for manifest in &mut state.manifests {
        normalize_persisted_legacy_manifest(manifest)?;
    }
    remove_retired_slack_user_state(&mut state)?;
    normalize_retired_slack_identity(&mut state, has_exact_pre_train_a_slack)?;
    state.installations = canonicalize_installation_rows(state.installations)?;
    Ok(state)
}

fn is_exact_pre_train_a_slack_record(record: &WireManifestRecord) -> bool {
    matches!(record.source, WireManifestSource::HostBundled)
        && record
            .manifest_hash
            .as_ref()
            .is_some_and(|hash| hash.as_str() == PRE_TRAIN_A_SLACK_MANIFEST_HASH)
        && is_recognized_pre_train_a_slack_cleanup(&record.removal_cleanup_requirements)
        && sha256_digest_token(record.raw_toml.as_bytes()) == PRE_TRAIN_A_SLACK_MANIFEST_HASH
}

fn is_recognized_pre_train_a_slack_cleanup(
    requirements: &[ExtensionRemovalCleanupRequirement],
) -> bool {
    // Older predecessor installations predate persisted cleanup metadata.
    if requirements.is_empty() {
        return true;
    }

    #[cfg(feature = "slack-v2-host-beta")]
    {
        use crate::extension_host::extension_removal_cleanup::{
            ExtensionRemovalChannelId, ExtensionRemovalCleanupAdapterId,
            SLACK_EXTENSION_REMOVAL_CHANNEL_ID, SLACK_PERSONAL_CONNECTION_CLEANUP_ADAPTER_ID,
        };

        let (Ok(adapter), Ok(channel)) = (
            ExtensionRemovalCleanupAdapterId::new(SLACK_PERSONAL_CONNECTION_CLEANUP_ADAPTER_ID),
            ExtensionRemovalChannelId::new(SLACK_EXTENSION_REMOVAL_CHANNEL_ID),
        ) else {
            return false;
        };
        requirements
            == [ExtensionRemovalCleanupRequirement::channel_connection(
                adapter, channel,
            )]
    }

    #[cfg(not(feature = "slack-v2-host-beta"))]
    false
}

/// Remove the retired internal-only Slack user-tools package in the same
/// normalized snapshot as every other persisted-state migration, but only in
/// a Slack-enabled build and after strict manifest/ref authority validation.
/// The former restore-time cleanup issued separate installation and manifest
/// deletes, which could leave a torn state after interruption.
fn remove_retired_slack_user_state(
    state: &mut WireState,
) -> Result<(), ExtensionInstallationError> {
    const RETIRED_SLACK_USER_ID: &str = "slack_user"; // taxonomy-allow: retired-forward-migration

    let manifest_ids = state
        .manifests
        .iter()
        .map(|record| persisted_manifest_id(&record.raw_toml))
        .collect::<Result<Vec<_>, _>>()?;
    let retired_manifest_indices = manifest_ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| (id == RETIRED_SLACK_USER_ID).then_some(index))
        .collect::<Vec<_>>();
    if retired_manifest_indices.iter().any(|index| {
        !matches!(
            state.manifests[*index].source,
            WireManifestSource::HostBundled
        )
    }) {
        return Err(invalid_installation_error(
            "retired internal Slack user-tools manifests must be host-bundled records",
        ));
    }
    let retired_installations = state
        .installations
        .iter()
        .filter(|installation| installation.extension_id().as_str() == RETIRED_SLACK_USER_ID)
        .collect::<Vec<_>>();
    if retired_manifest_indices.is_empty() && retired_installations.is_empty() {
        return Ok(());
    }
    let retired_manifest = validated_retired_manifest_authority(
        state,
        &manifest_ids,
        RETIRED_SLACK_USER_ID,
        "retired internal Slack user-tools",
    )?;
    validate_retired_installation_authority(
        &state.installations,
        RETIRED_SLACK_USER_ID,
        &retired_manifest,
    )?;

    #[cfg(not(feature = "slack-v2-host-beta"))]
    return Err(invalid_installation_error(
        "retired Slack user-tools migration is unavailable in this build",
    ));

    #[cfg(feature = "slack-v2-host-beta")]
    {
        let mut retained_manifests = Vec::with_capacity(state.manifests.len());
        for (record, id) in state.manifests.drain(..).zip(manifest_ids) {
            if id != RETIRED_SLACK_USER_ID {
                retained_manifests.push(record);
            }
        }
        state.manifests = retained_manifests;
        state
            .installations
            .retain(|installation| installation.extension_id().as_str() != RETIRED_SLACK_USER_ID);
        Ok(())
    }
}

/// Resolve the exact manifest that authorizes a destructive/retyping retired
/// state transition. The caller must do this before removing or replacing any
/// evidence: final validation cannot detect provenance that was already
/// discarded from the candidate snapshot.
fn validated_retired_manifest_authority(
    state: &WireState,
    manifest_ids: &[String],
    retired_id: &str,
    description: &str,
) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
    let retired_indices = manifest_ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| (id == retired_id).then_some(index))
        .collect::<Vec<_>>();
    let index = match retired_indices.as_slice() {
        [index] => *index,
        [] => {
            return Err(invalid_installation_error(format!(
                "{description} installations require a matching host-bundled manifest"
            )));
        }
        _ => {
            return Err(invalid_installation_error(format!(
                "persisted extension state contains multiple {description} manifests"
            )));
        }
    };
    let record = &state.manifests[index];
    if !matches!(record.source, WireManifestSource::HostBundled) {
        return Err(invalid_installation_error(format!(
            "{description} manifests must be host-bundled records"
        )));
    }
    record.clone().into_manifest_record()
}

fn validate_retired_installation_authority(
    installations: &[ExtensionInstallation],
    retired_id: &str,
    retired_manifest: &ExtensionManifestRecord,
) -> Result<(), ExtensionInstallationError> {
    for installation in installations
        .iter()
        .filter(|installation| installation.extension_id().as_str() == retired_id)
    {
        if retired_manifest.extension_id() != installation.manifest_ref().extension_id() {
            return Err(ExtensionInstallationError::ManifestExtensionMismatch {
                extension_id: installation.extension_id().clone(),
                manifest_extension_id: installation.manifest_ref().extension_id().clone(),
            });
        }
        match (
            retired_manifest.manifest_hash(),
            installation.manifest_ref().manifest_hash(),
        ) {
            (Some(registered), Some(referenced)) if registered == referenced => {}
            (None, None) => {}
            _ => {
                return Err(ExtensionInstallationError::ManifestHashMismatch {
                    extension_id: installation.extension_id().clone(),
                });
            }
        }
    }
    Ok(())
}

fn normalize_persisted_legacy_manifest(
    record: &mut WireManifestRecord,
) -> Result<(), ExtensionInstallationError> {
    if !matches!(record.source, WireManifestSource::HostBundled) {
        return Ok(());
    }
    let mut document = parse_persisted_manifest_toml(&record.raw_toml)?;
    let Some(root) = document.as_table_mut() else {
        return Err(invalid_installation_error(
            "persisted extension manifest root must be a TOML table",
        ));
    };
    let Some(capabilities) = root.get("capabilities") else {
        return Ok(());
    };
    let exact_legacy_shape = root
        .get("schema_version")
        .and_then(toml::Value::as_str)
        .is_some_and(|version| version == MANIFEST_SCHEMA_VERSION)
        && capabilities
            .as_array()
            .is_some_and(|entries| !entries.is_empty())
        && !root.contains_key("host_api")
        && !root.contains_key("capability_provider");
    if !exact_legacy_shape {
        return Ok(());
    }

    let capabilities = root.remove("capabilities").ok_or_else(|| {
        invalid_installation_error("persisted legacy manifest capabilities disappeared")
    })?;
    let mut contract = toml::value::Table::new();
    contract.insert(
        "id".to_string(),
        toml::Value::String("ironclaw.capability_provider/v1".to_string()),
    );
    contract.insert(
        "section".to_string(),
        toml::Value::String("capability_provider.tools".to_string()),
    );
    root.insert(
        "host_api".to_string(),
        toml::Value::Array(vec![toml::Value::Table(contract)]),
    );
    let mut tools = toml::value::Table::new();
    tools.insert("capabilities".to_string(), capabilities);
    let mut capability_provider = toml::value::Table::new();
    capability_provider.insert("tools".to_string(), toml::Value::Table(tools));
    root.insert(
        "capability_provider".to_string(),
        toml::Value::Table(capability_provider),
    );
    record.raw_toml = toml::to_string_pretty(&document).map_err(invalid_installation_error)?;
    // Validate the converted record before any later identity fold can remove
    // it. This keeps malformed persisted input fail-closed even when the
    // record belongs to a retired extension identity.
    record.clone().into_manifest_record()?;
    Ok(())
}

fn normalize_retired_slack_identity(
    state: &mut WireState,
    has_exact_pre_train_a_slack: bool,
) -> Result<(), ExtensionInstallationError> {
    const RETIRED_SLACK_ID: &str = "slack_bot"; // taxonomy-allow: retired-forward-migration
    const UNIFIED_SLACK_ID: &str = "slack";

    let manifest_ids = state
        .manifests
        .iter()
        .map(|record| persisted_manifest_id(&record.raw_toml))
        .collect::<Result<Vec<_>, _>>()?;
    let has_retired_state = manifest_ids.iter().any(|id| id == RETIRED_SLACK_ID)
        || state
            .installations
            .iter()
            .any(|installation| installation.extension_id().as_str() == RETIRED_SLACK_ID);
    if !has_retired_state {
        return Ok(());
    }
    let retired_manifest = validated_retired_manifest_authority(
        state,
        &manifest_ids,
        RETIRED_SLACK_ID,
        "retired Slack",
    )?;
    validate_retired_installation_authority(
        &state.installations,
        RETIRED_SLACK_ID,
        &retired_manifest,
    )?;

    let bundled_unified_record = bundled_slack_wire_manifest()?;
    bundled_unified_record.clone().into_manifest_record()?;
    let unified_indices = manifest_ids
        .iter()
        .enumerate()
        .filter_map(|(index, id)| (id == UNIFIED_SLACK_ID).then_some(index))
        .collect::<Vec<_>>();
    match unified_indices.as_slice() {
        [] => {
            if state
                .installations
                .iter()
                .any(|installation| installation.extension_id().as_str() == UNIFIED_SLACK_ID)
            {
                return Err(invalid_installation_error(
                    "unified Slack installations require the exact host-bundled manifest",
                ));
            }
            state.manifests.push(bundled_unified_record.clone());
        }
        [index] => {
            let persisted = &state.manifests[*index];
            // `source` is persisted data, not proof by itself. Requiring the
            // complete current binary-bundled record prevents a local or
            // registry package that claimed the reserved `slack` id from
            // receiving retired host-owned credentials during the fold.
            let exact_current = persisted == &bundled_unified_record;
            let exact_predecessor = has_exact_pre_train_a_slack
                && matches!(persisted.source, WireManifestSource::HostBundled)
                && persisted
                    .manifest_hash
                    .as_ref()
                    .is_some_and(|hash| hash.as_str() == PRE_TRAIN_A_SLACK_MANIFEST_HASH)
                && is_recognized_pre_train_a_slack_cleanup(&persisted.removal_cleanup_requirements);
            if !exact_current && !exact_predecessor {
                return Err(invalid_installation_error(
                    "unified Slack migration target must be an exact recognized host-bundled manifest",
                ));
            }
            let persisted_manifest = persisted.clone().into_manifest_record()?;
            validate_retired_installation_authority(
                &state.installations,
                UNIFIED_SLACK_ID,
                &persisted_manifest,
            )?;
            // A recognized predecessor is authenticated before its raw TOML is
            // normalized, then retargeted to the current binary bundle. This
            // is a one-way, enumerated upgrade path rather than a generic
            // HostBundled-source trust bypass.
            state.manifests[*index] = bundled_unified_record.clone();
        }
        _ => {
            return Err(invalid_installation_error(
                "persisted extension state contains multiple unified Slack manifests",
            ));
        }
    }
    let unified_id = ExtensionId::new(UNIFIED_SLACK_ID).map_err(invalid_installation_error)?;
    let unified_ref = ExtensionManifestRef::new(
        unified_id.clone(),
        bundled_unified_record.manifest_hash.clone(),
    );
    let mut retained_manifests = Vec::with_capacity(state.manifests.len());
    for record in state.manifests.drain(..) {
        if persisted_manifest_id(&record.raw_toml)? != RETIRED_SLACK_ID {
            retained_manifests.push(record);
        }
    }
    state.manifests = retained_manifests;

    let mut installations = Vec::with_capacity(state.installations.len());
    for installation in state.installations.drain(..) {
        if matches!(
            installation.extension_id().as_str(),
            RETIRED_SLACK_ID | UNIFIED_SLACK_ID
        ) {
            installations.push(rebuild_installation(
                &installation,
                unified_id.clone(),
                unified_ref.clone(),
                installation.activation_state(),
            )?);
        } else {
            installations.push(installation);
        }
    }
    let enabled_wins = installations.iter().any(|installation| {
        installation.extension_id() == &unified_id
            && installation.activation_state() == ExtensionActivationState::Enabled
    });
    if enabled_wins {
        for installation in &mut installations {
            if installation.extension_id() == &unified_id
                && installation.activation_state() != ExtensionActivationState::Enabled
            {
                *installation = rebuild_installation(
                    installation,
                    unified_id.clone(),
                    installation.manifest_ref().clone(),
                    ExtensionActivationState::Enabled,
                )?;
            }
        }
    }
    state.installations = installations;
    Ok(())
}

fn rebuild_installation(
    installation: &ExtensionInstallation,
    extension_id: ExtensionId,
    manifest_ref: ExtensionManifestRef,
    activation_state: ExtensionActivationState,
) -> Result<ExtensionInstallation, ExtensionInstallationError> {
    ExtensionInstallation::from_persisted_parts(ExtensionInstallationPersistedParts {
        installation_id: installation.installation_id().clone(),
        extension_id,
        activation_state,
        manifest_ref,
        credential_bindings: installation.credential_bindings().to_vec(),
        health: installation.health().clone(),
        updated_at: installation.updated_at(),
        owner: installation.owner().clone(),
    })
}

fn persisted_manifest_id(raw_toml: &str) -> Result<String, ExtensionInstallationError> {
    let document = parse_persisted_manifest_toml(raw_toml)?;
    document
        .as_table()
        .and_then(|root| root.get("id"))
        .and_then(toml::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_installation_error("persisted extension manifest is missing id"))
}

fn parse_persisted_manifest_toml(
    raw_toml: &str,
) -> Result<toml::Value, ExtensionInstallationError> {
    if raw_toml.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestV2Error::ManifestTooLarge {
            bytes: raw_toml.len(),
            max: MAX_MANIFEST_BYTES,
        }
        .into());
    }
    toml::from_str(raw_toml).map_err(|error| {
        ManifestV2Error::Parse {
            reason: error.to_string(),
        }
        .into()
    })
}

#[cfg(feature = "slack-v2-host-beta")]
fn bundled_slack_wire_manifest() -> Result<WireManifestRecord, ExtensionInstallationError> {
    use super::available_extensions::{
        AvailableExtensionCatalog, SLACK_EXTENSION_ID, slack_manifest_digest,
    };

    let catalog = AvailableExtensionCatalog::from_first_party_assets_with_nearai_mcp_config(None)
        .map_err(invalid_installation_error)?;
    let package = catalog
        .search(SLACK_EXTENSION_ID)
        .find(|package| package.package_ref.id.as_str() == SLACK_EXTENSION_ID)
        .ok_or_else(|| invalid_installation_error("unified Slack manifest is unavailable"))?;
    Ok(WireManifestRecord {
        raw_toml: package.manifest_toml.clone(),
        source: WireManifestSource::from_manifest_source(package.package.manifest.source),
        manifest_hash: Some(
            ManifestHash::new(slack_manifest_digest()).map_err(invalid_installation_error)?,
        ),
        removal_cleanup_requirements: package.cleanup_requirements.clone(),
    })
}

#[cfg(not(feature = "slack-v2-host-beta"))]
fn bundled_slack_wire_manifest() -> Result<WireManifestRecord, ExtensionInstallationError> {
    Err(invalid_installation_error(
        "unified Slack manifest is unavailable in this build",
    ))
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WireState {
    manifests: Vec<WireManifestRecord>,
    installations: Vec<ExtensionInstallation>,
}

impl WireState {
    async fn from_store(
        store: &InMemoryExtensionInstallationStore,
    ) -> Result<Self, ExtensionInstallationError> {
        let manifests = store
            .list_manifests()
            .await?
            .into_iter()
            .map(WireManifestRecord::from)
            .collect();
        let installations = store.list_installations().await?;
        Ok(Self {
            manifests,
            installations,
        })
    }

    async fn load_into(
        &self,
        store: &InMemoryExtensionInstallationStore,
    ) -> Result<(), ExtensionInstallationError> {
        for manifest in &self.manifests {
            store
                .upsert_manifest(manifest.clone().into_manifest_record()?)
                .await?;
        }
        for installation in &self.installations {
            store.upsert_installation(installation.clone()).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WireManifestRecord {
    raw_toml: String,
    source: WireManifestSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest_hash: Option<ManifestHash>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    removal_cleanup_requirements: Vec<ExtensionRemovalCleanupRequirement>,
}

impl WireManifestRecord {
    fn into_manifest_record(self) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
        let host_ports = ironclaw_host_runtime::default_host_port_catalog()
            .map_err(invalid_installation_error)?;
        let contracts =
            product_extension_host_api_contract_registry().map_err(invalid_installation_error)?;
        ExtensionManifestRecord::from_toml(
            self.raw_toml,
            self.source.into_manifest_source(),
            &host_ports,
            self.manifest_hash,
            &contracts,
        )
        .map(|record| record.with_removal_cleanup_requirements(self.removal_cleanup_requirements))
    }
}

impl From<ExtensionManifestRecord> for WireManifestRecord {
    fn from(record: ExtensionManifestRecord) -> Self {
        Self {
            raw_toml: record.raw_toml().to_string(),
            source: WireManifestSource::from_manifest_source(record.manifest().source),
            manifest_hash: record.manifest_hash().cloned(),
            removal_cleanup_requirements: record.removal_cleanup_requirements().to_vec(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireManifestSource {
    HostBundled,
    InstalledLocal,
    RegistryInstalled,
}

impl WireManifestSource {
    fn from_manifest_source(source: ManifestSource) -> Self {
        match source {
            ManifestSource::HostBundled => Self::HostBundled,
            ManifestSource::InstalledLocal => Self::InstalledLocal,
            ManifestSource::RegistryInstalled => Self::RegistryInstalled,
        }
    }

    fn into_manifest_source(self) -> ManifestSource {
        match self {
            Self::HostBundled => ManifestSource::HostBundled,
            Self::InstalledLocal => ManifestSource::InstalledLocal,
            Self::RegistryInstalled => ManifestSource::RegistryInstalled,
        }
    }
}

fn invalid_installation_error(error: impl std::fmt::Display) -> ExtensionInstallationError {
    ExtensionInstallationError::InvalidInstallation {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests;
