//! Durable storage adapters for Reborn OpenAI-compatible refs.
//!
//! This crate keeps persistence behind the
//! [`OpenAiCompatRefStore`](ironclaw_reborn_openai_compat::OpenAiCompatRefStore)
//! port. The OpenAI-compatible contract crate stays side-effect free; Reborn
//! composition can choose this filesystem-backed adapter when wiring concrete
//! route behavior.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, Entry, FilesystemError, RecordKind, RecordVersion, RootFilesystem,
};
use ironclaw_host_api::VirtualPath;
use ironclaw_reborn_openai_compat::{
    OpenAiCompatActorScope, OpenAiCompatBindInternalRefs, OpenAiCompatIdempotencyKey,
    OpenAiCompatPublicId, OpenAiCompatRecordAcceptedAck, OpenAiCompatRefError,
    OpenAiCompatRefLookup, OpenAiCompatRefReservation, OpenAiCompatRefReservationOutcome,
    OpenAiCompatRefStore, OpenAiCompatResourceBinding, OpenAiCompatResourceMapping,
    OpenAiCompatRouteSurface,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;

const DEFAULT_REF_ROOT: &str = "/engine/openai_compat/refs";
const MAPPING_RECORD_KIND: &str = "openai_compat_ref_mapping";
const IDEMPOTENCY_INDEX_RECORD_KIND: &str = "openai_compat_idempotency_index";
const FILESYSTEM_CAS_RETRIES: usize = 5;

#[derive(Clone)]
pub struct FilesystemOpenAiCompatRefStore {
    filesystem: Arc<dyn RootFilesystem>,
    root: VirtualPath,
    cas_retries: usize,
}

impl FilesystemOpenAiCompatRefStore {
    pub fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self::with_root(filesystem, default_ref_root())
    }

    pub fn with_root(filesystem: Arc<dyn RootFilesystem>, root: VirtualPath) -> Self {
        Self {
            filesystem,
            root,
            cas_retries: FILESYSTEM_CAS_RETRIES,
        }
    }

    pub fn with_cas_retries(mut self, cas_retries: usize) -> Self {
        self.cas_retries = cas_retries;
        self
    }

    fn mapping_path(
        &self,
        public_id: &OpenAiCompatPublicId,
    ) -> Result<VirtualPath, OpenAiCompatRefError> {
        mapping_path(&self.root, public_id)
    }

    fn idempotency_index_path(
        &self,
        owner: &OpenAiCompatActorScope,
        surface: OpenAiCompatRouteSurface,
        key: &OpenAiCompatIdempotencyKey,
    ) -> Result<VirtualPath, OpenAiCompatRefError> {
        idempotency_index_path(&self.root, owner, surface, key)
    }

    async fn load_mapping_entry(
        &self,
        public_id: &OpenAiCompatPublicId,
    ) -> Result<Option<(OpenAiCompatResourceMapping, RecordVersion)>, OpenAiCompatRefError> {
        let path = self.mapping_path(public_id)?;
        let Some(entry) = self
            .filesystem
            .get(&path)
            .await
            .map_err(filesystem_error("load OpenAI-compatible ref mapping"))?
        else {
            return Ok(None);
        };
        ensure_entry_kind(&entry.entry, MAPPING_RECORD_KIND)?;
        let mapping: OpenAiCompatResourceMapping = entry
            .entry
            .parse_json()
            .map_err(corrupt_mapping("deserialize OpenAI-compatible ref mapping"))?;
        mapping.validate()?;
        if mapping.public_id != *public_id {
            return Err(OpenAiCompatRefError::CorruptMapping);
        }
        Ok(Some((mapping, entry.version)))
    }

    async fn load_required_mapping(
        &self,
        public_id: &OpenAiCompatPublicId,
    ) -> Result<OpenAiCompatResourceMapping, OpenAiCompatRefError> {
        self.load_mapping_entry(public_id)
            .await?
            .map(|(mapping, _)| mapping)
            .ok_or(OpenAiCompatRefError::CorruptMapping)
    }

    async fn put_mapping(
        &self,
        mapping: &OpenAiCompatResourceMapping,
        cas: CasExpectation,
    ) -> Result<(), SaveRecordError> {
        let path = self.mapping_path(&mapping.public_id)?;
        match self
            .filesystem
            .put(&path, entry_for_mapping(mapping)?, cas)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(SaveRecordError::CasConflict),
            Err(error) => Err(SaveRecordError::Ref(filesystem_error(
                "save OpenAI-compatible ref mapping",
            )(error))),
        }
    }

    async fn load_idempotency_index(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<StoredOpenAiCompatIdempotencyIndex>, OpenAiCompatRefError> {
        let Some(entry) = self
            .filesystem
            .get(path)
            .await
            .map_err(filesystem_error("load OpenAI-compatible idempotency index"))?
        else {
            return Ok(None);
        };
        ensure_entry_kind(&entry.entry, IDEMPOTENCY_INDEX_RECORD_KIND)?;
        let index: StoredOpenAiCompatIdempotencyIndex = entry.entry.parse_json().map_err(
            corrupt_mapping("deserialize OpenAI-compatible idempotency index"),
        )?;
        index.validate()?;
        Ok(Some(index))
    }

    async fn put_idempotency_index(
        &self,
        path: &VirtualPath,
        index: &StoredOpenAiCompatIdempotencyIndex,
    ) -> Result<(), SaveRecordError> {
        match self
            .filesystem
            .put(
                path,
                entry_for_idempotency_index(index)?,
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => Err(SaveRecordError::CasConflict),
            Err(error) => Err(SaveRecordError::Ref(filesystem_error(
                "save OpenAI-compatible idempotency index",
            )(error))),
        }
    }

    async fn delete_mapping_best_effort(&self, public_id: &OpenAiCompatPublicId) {
        if let Ok(path) = self.mapping_path(public_id) {
            let _ = self.filesystem.delete(&path).await;
        }
    }

    async fn reserve_with_cas(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError> {
        for _ in 0..=self.cas_retries {
            if let Some(key) = request.idempotency_key.as_ref() {
                let index_path =
                    self.idempotency_index_path(&request.owner, request.surface, key)?;
                if let Some(index) = self.load_idempotency_index(&index_path).await? {
                    if !index.matches_request(&request.owner, request.surface, key) {
                        return Err(OpenAiCompatRefError::CorruptMapping);
                    }
                    let mapping = self.load_required_mapping(&index.public_id).await?;
                    if !index.matches_mapping(&mapping) {
                        return Err(OpenAiCompatRefError::CorruptMapping);
                    }
                    if mapping.request_fingerprint == request.request_fingerprint {
                        return Ok(OpenAiCompatRefReservationOutcome::Replayed(mapping));
                    }
                    return Ok(OpenAiCompatRefReservationOutcome::Conflict(
                        ironclaw_reborn_openai_compat::OpenAiCompatIdempotencyConflict {
                            surface: request.surface,
                        },
                    ));
                }

                let mapping = new_pending_mapping(&request);
                match self.put_mapping(&mapping, CasExpectation::Absent).await {
                    Ok(()) => {}
                    Err(SaveRecordError::CasConflict) => continue,
                    Err(SaveRecordError::Ref(error)) => return Err(error),
                }
                let index = StoredOpenAiCompatIdempotencyIndex {
                    owner: request.owner.clone(),
                    surface: request.surface,
                    key: key.clone(),
                    public_id: mapping.public_id.clone(),
                };
                match self.put_idempotency_index(&index_path, &index).await {
                    Ok(()) => return Ok(OpenAiCompatRefReservationOutcome::Created(mapping)),
                    Err(SaveRecordError::CasConflict) => {
                        self.delete_mapping_best_effort(&mapping.public_id).await;
                        continue;
                    }
                    Err(SaveRecordError::Ref(error)) => return Err(error),
                }
            }

            let mapping = new_pending_mapping(&request);
            match self.put_mapping(&mapping, CasExpectation::Absent).await {
                Ok(()) => return Ok(OpenAiCompatRefReservationOutcome::Created(mapping)),
                Err(SaveRecordError::CasConflict) => continue,
                Err(SaveRecordError::Ref(error)) => return Err(error),
            }
        }
        Err(OpenAiCompatRefError::StoreUnavailable)
    }

    async fn bind_with_cas(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        for _ in 0..=self.cas_retries {
            let Some((mut mapping, version)) = self.load_mapping_entry(&request.public_id).await?
            else {
                return Ok(None);
            };
            if !mapping.is_authorized_for(&request.owner) {
                return Ok(None);
            }
            mapping.binding = OpenAiCompatResourceBinding::Bound {
                internal_refs: request.internal_refs.clone(),
            };
            match self
                .put_mapping(&mapping, CasExpectation::Version(version))
                .await
            {
                Ok(()) => return Ok(Some(mapping)),
                Err(SaveRecordError::CasConflict) => continue,
                Err(SaveRecordError::Ref(error)) => return Err(error),
            }
        }
        Err(OpenAiCompatRefError::StoreUnavailable)
    }

    async fn record_accepted_ack_with_cas(
        &self,
        request: OpenAiCompatRecordAcceptedAck,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        for _ in 0..=self.cas_retries {
            let Some((mut mapping, version)) = self.load_mapping_entry(&request.public_id).await?
            else {
                return Ok(None);
            };
            if !mapping.is_authorized_for(&request.owner) {
                return Ok(None);
            }
            mapping.accepted_ack = Some(request.accepted_ack.clone());
            match self
                .put_mapping(&mapping, CasExpectation::Version(version))
                .await
            {
                Ok(()) => return Ok(Some(mapping)),
                Err(SaveRecordError::CasConflict) => continue,
                Err(SaveRecordError::Ref(error)) => return Err(error),
            }
        }
        Err(OpenAiCompatRefError::StoreUnavailable)
    }
}

#[cfg(feature = "libsql")]
pub struct RebornLibSqlOpenAiCompatRefStore {
    inner: FilesystemOpenAiCompatRefStore,
}

#[cfg(feature = "libsql")]
impl RebornLibSqlOpenAiCompatRefStore {
    pub fn new(filesystem: Arc<LibSqlRootFilesystem>) -> Self {
        Self {
            inner: FilesystemOpenAiCompatRefStore::new(filesystem),
        }
    }

    pub fn with_root(filesystem: Arc<LibSqlRootFilesystem>, root: VirtualPath) -> Self {
        Self {
            inner: FilesystemOpenAiCompatRefStore::with_root(filesystem, root),
        }
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl OpenAiCompatRefStore for RebornLibSqlOpenAiCompatRefStore {
    async fn reserve(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError> {
        self.inner.reserve(request).await
    }

    async fn bind_internal_refs(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.bind_internal_refs(request).await
    }

    async fn record_accepted_ack(
        &self,
        request: OpenAiCompatRecordAcceptedAck,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.record_accepted_ack(request).await
    }
    async fn lookup_authorized(
        &self,
        request: OpenAiCompatRefLookup,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.lookup_authorized(request).await
    }
}

#[cfg(feature = "postgres")]
pub struct RebornPostgresOpenAiCompatRefStore {
    inner: FilesystemOpenAiCompatRefStore,
}

#[cfg(feature = "postgres")]
impl RebornPostgresOpenAiCompatRefStore {
    pub fn new(filesystem: Arc<PostgresRootFilesystem>) -> Self {
        Self {
            inner: FilesystemOpenAiCompatRefStore::new(filesystem),
        }
    }

    pub fn with_root(filesystem: Arc<PostgresRootFilesystem>, root: VirtualPath) -> Self {
        Self {
            inner: FilesystemOpenAiCompatRefStore::with_root(filesystem, root),
        }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl OpenAiCompatRefStore for RebornPostgresOpenAiCompatRefStore {
    async fn reserve(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError> {
        self.inner.reserve(request).await
    }

    async fn bind_internal_refs(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.bind_internal_refs(request).await
    }

    async fn record_accepted_ack(
        &self,
        request: OpenAiCompatRecordAcceptedAck,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.record_accepted_ack(request).await
    }

    async fn lookup_authorized(
        &self,
        request: OpenAiCompatRefLookup,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.inner.lookup_authorized(request).await
    }
}

#[async_trait]
impl OpenAiCompatRefStore for FilesystemOpenAiCompatRefStore {
    async fn reserve(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError> {
        self.reserve_with_cas(request).await
    }

    async fn bind_internal_refs(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.bind_with_cas(request).await
    }

    async fn record_accepted_ack(
        &self,
        request: OpenAiCompatRecordAcceptedAck,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        self.record_accepted_ack_with_cas(request).await
    }

    async fn lookup_authorized(
        &self,
        request: OpenAiCompatRefLookup,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        let Some((mapping, _)) = self.load_mapping_entry(&request.public_id).await? else {
            return Ok(None);
        };
        if !mapping.is_authorized_for(&request.requester) {
            return Ok(None);
        }
        Ok(Some(mapping))
    }
}

enum SaveRecordError {
    CasConflict,
    Ref(OpenAiCompatRefError),
}

impl From<OpenAiCompatRefError> for SaveRecordError {
    fn from(error: OpenAiCompatRefError) -> Self {
        Self::Ref(error)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredOpenAiCompatIdempotencyIndex {
    owner: OpenAiCompatActorScope,
    surface: OpenAiCompatRouteSurface,
    key: OpenAiCompatIdempotencyKey,
    public_id: OpenAiCompatPublicId,
}

impl StoredOpenAiCompatIdempotencyIndex {
    fn validate(&self) -> Result<(), OpenAiCompatRefError> {
        if self.public_id.resource_kind() != self.surface.resource_kind() {
            return Err(OpenAiCompatRefError::CorruptMapping);
        }
        Ok(())
    }

    fn matches_request(
        &self,
        owner: &OpenAiCompatActorScope,
        surface: OpenAiCompatRouteSurface,
        key: &OpenAiCompatIdempotencyKey,
    ) -> bool {
        &self.owner == owner && self.surface == surface && &self.key == key
    }

    fn matches_mapping(&self, mapping: &OpenAiCompatResourceMapping) -> bool {
        mapping.public_id == self.public_id
            && mapping.owner == self.owner
            && mapping.surface == self.surface
            && mapping.idempotency_key.as_ref() == Some(&self.key)
    }
}

fn new_pending_mapping(request: &OpenAiCompatRefReservation) -> OpenAiCompatResourceMapping {
    let mapping = OpenAiCompatResourceMapping {
        public_id: OpenAiCompatPublicId::generate_for(request.surface),
        owner: request.owner.clone(),
        surface: request.surface,
        request_fingerprint: request.request_fingerprint.clone(),
        created_at: ironclaw_reborn_openai_compat::unix_timestamp_now(),
        idempotency_key: request.idempotency_key.clone(),
        accepted_ack: None,
        binding: OpenAiCompatResourceBinding::Pending,
    };
    debug_assert!(mapping.validate().is_ok());
    mapping
}

fn entry_for_mapping(mapping: &OpenAiCompatResourceMapping) -> Result<Entry, OpenAiCompatRefError> {
    mapping.validate()?;
    let payload = serde_json::to_value(mapping).map_err(corrupt_mapping(
        "serialize OpenAI-compatible ref mapping payload",
    ))?;
    let kind =
        RecordKind::new(MAPPING_RECORD_KIND).map_err(|_| OpenAiCompatRefError::StoreUnavailable)?;
    Entry::record(kind, &payload).map_err(corrupt_mapping(
        "serialize OpenAI-compatible ref mapping entry",
    ))
}

fn entry_for_idempotency_index(
    index: &StoredOpenAiCompatIdempotencyIndex,
) -> Result<Entry, OpenAiCompatRefError> {
    index.validate()?;
    let payload = serde_json::to_value(index).map_err(corrupt_mapping(
        "serialize OpenAI-compatible idempotency index payload",
    ))?;
    let kind = RecordKind::new(IDEMPOTENCY_INDEX_RECORD_KIND)
        .map_err(|_| OpenAiCompatRefError::StoreUnavailable)?;
    Entry::record(kind, &payload).map_err(corrupt_mapping(
        "serialize OpenAI-compatible idempotency index entry",
    ))
}

fn mapping_path(
    root: &VirtualPath,
    public_id: &OpenAiCompatPublicId,
) -> Result<VirtualPath, OpenAiCompatRefError> {
    let (kind_dir, id) = public_id_path_parts(public_id);
    child_path(root, &format!("by_public_id/{kind_dir}/{id}.json"))
}

fn public_id_path_parts(public_id: &OpenAiCompatPublicId) -> (&'static str, &str) {
    match public_id {
        OpenAiCompatPublicId::ChatCompletion(id) => ("chat_completions", id.as_str()),
        OpenAiCompatPublicId::Response(id) => ("responses", id.as_str()),
    }
}

fn idempotency_index_path(
    root: &VirtualPath,
    owner: &OpenAiCompatActorScope,
    surface: OpenAiCompatRouteSurface,
    key: &OpenAiCompatIdempotencyKey,
) -> Result<VirtualPath, OpenAiCompatRefError> {
    let digest = idempotency_index_digest(owner, surface, key)?;
    child_path(
        root,
        &format!("by_idempotency/{}/{digest}.json", surface_dir(surface)),
    )
}

fn idempotency_index_digest(
    owner: &OpenAiCompatActorScope,
    surface: OpenAiCompatRouteSurface,
    key: &OpenAiCompatIdempotencyKey,
) -> Result<String, OpenAiCompatRefError> {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        owner: &'a OpenAiCompatActorScope,
        surface: OpenAiCompatRouteSurface,
        key: &'a OpenAiCompatIdempotencyKey,
    }

    let payload = DigestInput {
        owner,
        surface,
        key,
    };
    let bytes = serde_json::to_vec(&payload).map_err(corrupt_mapping(
        "serialize OpenAI-compatible idempotency index key",
    ))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn surface_dir(surface: OpenAiCompatRouteSurface) -> &'static str {
    match surface {
        OpenAiCompatRouteSurface::ChatCompletions => "chat_completions",
        OpenAiCompatRouteSurface::ResponsesApi => "responses_api",
        OpenAiCompatRouteSurface::ResponsesV1 => "responses_v1",
    }
}

fn child_path(root: &VirtualPath, child: &str) -> Result<VirtualPath, OpenAiCompatRefError> {
    VirtualPath::new(format!("{}/{}", root.as_str().trim_end_matches('/'), child))
        .map_err(|_| OpenAiCompatRefError::StoreUnavailable)
}

fn ensure_entry_kind(entry: &Entry, expected: &str) -> Result<(), OpenAiCompatRefError> {
    if entry
        .kind
        .as_ref()
        .is_some_and(|kind| kind.as_str() == expected)
    {
        return Ok(());
    }
    Err(OpenAiCompatRefError::CorruptMapping)
}

fn default_ref_root() -> VirtualPath {
    VirtualPath::new(DEFAULT_REF_ROOT).expect("DEFAULT_REF_ROOT is valid") // safety: hard-coded /engine virtual path literal.
}

fn filesystem_error(
    operation: &'static str,
) -> impl FnOnce(FilesystemError) -> OpenAiCompatRefError {
    move |error| {
        tracing::error!(
            operation,
            error_type = std::any::type_name_of_val(&error),
            "OpenAI-compatible ref store filesystem operation failed"
        );
        OpenAiCompatRefError::StoreUnavailable
    }
}

fn corrupt_mapping<E>(operation: &'static str) -> impl FnOnce(E) -> OpenAiCompatRefError
where
    E: std::fmt::Display,
{
    move |error| {
        tracing::error!(
            operation,
            error_type = std::any::type_name_of_val(&error),
            "OpenAI-compatible ref store mapping payload is invalid"
        );
        OpenAiCompatRefError::CorruptMapping
    }
}
