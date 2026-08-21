use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasApply, CasExpectation, Entry, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    artifact::{
        AccountedArtifactPersister, ArtifactAccessError, ArtifactAccessPort, ArtifactByteRange,
        ArtifactDigest, ArtifactId, ArtifactLineRange, ArtifactNamespaceId, ArtifactOwnerScope,
        ArtifactPersistencePort, ArtifactReadChunk, ArtifactReadRequest, ArtifactRef,
        ArtifactSelector, ArtifactWriteError, ArtifactWriteHandle, ArtifactWriteMetadata,
        ArtifactWriteState, CompletedArtifact,
    },
    ids::{InvocationId, ThreadId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, ScopedPath, VirtualPath},
    resource::{ReservationStatus, ResourceReceipt, ResourceScope, resource_scope_path_segment},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const ARTIFACT_ALIAS: &str = "/artifacts";
pub const TOOL_ARTIFACT_CHUNK_BYTES: usize = 64 * 1024;

/// Name the exact leg that failed, then collapse to the wire-compatible
/// `ArtifactWriteError::Storage`.
///
/// `ArtifactWriteError` is a unit-variant contract type shared with the host
/// API, so a cause cannot ride the error value. Without this the whole
/// projection reports one indistinguishable "artifact storage failed" and the
/// operator cannot tell an unbound service from a missing record from a chunk
/// invariant — exactly the ambiguity that made the incident expensive to
/// diagnose. The model-visible summary stays generic; the DEBUG log names the leg.
fn legacy_projection_failure(
    leg: &'static str,
    cause: impl std::fmt::Display,
) -> ArtifactWriteError {
    tracing::debug!(
        operation = "project_legacy_result",
        leg,
        cause = %cause,
        "legacy tool result projection failed"
    );
    ArtifactWriteError::Storage
}

/// The same leg naming for a failure with no bound source error — an absent
/// record, an unbound service, or a violated invariant.
fn legacy_projection_missing(leg: &'static str) -> ArtifactWriteError {
    legacy_projection_failure(leg, "absent or invariant violated")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredArtifactChunk {
    index: u64,
    byte_len: u64,
    #[serde(default)]
    byte_start: Option<u64>,
    #[serde(default)]
    byte_end: Option<u64>,
    #[serde(default)]
    line_start: Option<u64>,
    #[serde(default)]
    line_end: Option<u64>,
    #[serde(default)]
    line_after: Option<u64>,
    #[serde(default)]
    digest: Option<ArtifactDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum ToolArtifactBacking {
    #[default]
    StoredChunks,
    LegacyResultRecord {
        thread_scope: crate::ThreadScope,
        thread_id: ThreadId,
        result_ref: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredArtifactMetadata {
    artifact_id: ArtifactId,
    namespace: ArtifactNamespaceId,
    owner_scope: ArtifactOwnerScope,
    producer_capability_id: ironclaw_host_api::ids::CapabilityId,
    content_type: String,
    expected_bytes: Option<u64>,
    #[serde(default)]
    write_key: Option<InvocationId>,
    total_bytes: u64,
    chunks: Vec<StoredArtifactChunk>,
    created_at: DateTime<Utc>,
    finalized_at: Option<DateTime<Utc>>,
    #[serde(default)]
    backing: ToolArtifactBacking,
    digest: Option<ArtifactDigest>,
    total_lines: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct StoredArtifactReceipt {
    artifact_id: ArtifactId,
}

#[derive(Debug, Clone)]
pub struct LegacyResultArtifactRequest {
    pub owner_scope: ArtifactOwnerScope,
    pub namespace: ArtifactNamespaceId,
    pub producer_capability_id: ironclaw_host_api::ids::CapabilityId,
    pub thread_scope: crate::ThreadScope,
    pub thread_id: ThreadId,
    pub result_ref: String,
}

/// Backend-neutral immutable artifact store over the canonical filesystem.
#[derive(Clone)]
pub struct DurableToolArtifactStore {
    filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    legacy_result_service: Arc<OnceLock<Arc<dyn crate::SessionThreadService>>>,
}

impl std::fmt::Debug for DurableToolArtifactStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableToolArtifactStore")
            .field(
                "legacy_result_service",
                &self.legacy_result_service.get().is_some(),
            )
            .field("filesystem", &"<RootFilesystem>")
            .finish()
    }
}

impl DurableToolArtifactStore {
    pub fn new<F>(filesystem: Arc<F>) -> Result<Self, ArtifactWriteError>
    where
        F: RootFilesystem + 'static,
    {
        let root: Arc<dyn RootFilesystem> = filesystem;
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new(ARTIFACT_ALIAS).map_err(|_| ArtifactWriteError::Storage)?,
            VirtualPath::new(ARTIFACT_ALIAS).map_err(|_| ArtifactWriteError::Storage)?,
            MountPermissions::read_write(),
        )])
        .map_err(|_| ArtifactWriteError::Storage)?;
        Ok(Self {
            filesystem: Arc::new(ScopedFilesystem::with_fixed_view(root, view)),
            legacy_result_service: Arc::new(OnceLock::new()),
        })
    }

    fn namespace_root(
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
    ) -> Result<String, ArtifactWriteError> {
        let agent = owner.agent_id.as_ref().map_or_else(
            || "absent".to_string(),
            |value| format!("value-{}", resource_scope_path_segment(value.as_str())),
        );
        let project = owner.project_id.as_ref().map_or_else(
            || "absent".to_string(),
            |value| format!("value-{}", resource_scope_path_segment(value.as_str())),
        );
        Ok(format!(
            "{ARTIFACT_ALIAS}/tenants/{}/users/{}/agents/{agent}/projects/{project}/namespaces/{}",
            resource_scope_path_segment(owner.tenant_id.as_str()),
            resource_scope_path_segment(owner.user_id.as_str()),
            namespace.as_run_id().as_uuid(),
        ))
    }

    fn scoped_path(value: String) -> Result<ScopedPath, ArtifactWriteError> {
        ScopedPath::new(value).map_err(|_| ArtifactWriteError::Storage)
    }

    fn counter_path(
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
    ) -> Result<ScopedPath, ArtifactWriteError> {
        Self::scoped_path(format!(
            "{}/next-id.json",
            Self::namespace_root(owner, namespace)?
        ))
    }

    fn metadata_path(
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
        artifact_id: ArtifactId,
    ) -> Result<ScopedPath, ArtifactWriteError> {
        Self::scoped_path(format!(
            "{}/artifacts/{}/metadata.json",
            Self::namespace_root(owner, namespace)?,
            artifact_id.get()
        ))
    }

    fn chunk_path(
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
        artifact_id: ArtifactId,
        index: u64,
    ) -> Result<ScopedPath, ArtifactWriteError> {
        Self::scoped_path(format!(
            "{}/artifacts/{}/chunks/{index}.bin",
            Self::namespace_root(owner, namespace)?,
            artifact_id.get()
        ))
    }

    fn receipt_path(
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
        receipt: ironclaw_host_api::ids::ResourceReservationId,
    ) -> Result<ScopedPath, ArtifactWriteError> {
        Self::scoped_path(format!(
            "{}/receipts/{receipt}.json",
            Self::namespace_root(owner, namespace)?
        ))
    }

    fn write_binding_path(
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
        write_key: InvocationId,
    ) -> Result<ScopedPath, ArtifactWriteError> {
        Self::scoped_path(format!(
            "{}/write-bindings/{write_key}.json",
            Self::namespace_root(owner, namespace)?
        ))
    }

    async fn write_binding_artifact_id(
        &self,
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
        write_key: InvocationId,
    ) -> Result<Option<ArtifactId>, ArtifactWriteError> {
        let path = Self::write_binding_path(owner, namespace, write_key)?;
        self.filesystem
            .get(&ResourceScope::system(), &path)
            .await
            .map_err(|_| ArtifactWriteError::Storage)?
            .map(|stored| {
                serde_json::from_slice::<StoredArtifactReceipt>(&stored.entry.body)
                    .map(|binding| binding.artifact_id)
                    .map_err(|_| ArtifactWriteError::Storage)
            })
            .transpose()
    }

    fn legacy_mapping_path(
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
        thread_id: &ThreadId,
        result_ref: &str,
    ) -> Result<ScopedPath, ArtifactWriteError> {
        let mut hasher = Sha256::new();
        hasher.update(thread_id.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(result_ref.as_bytes());
        let digest = hasher.finalize();
        let mut key = [0_u8; 16];
        key.copy_from_slice(&digest[..16]);
        Self::scoped_path(format!(
            "{}/legacy-results/{}.json",
            Self::namespace_root(owner, namespace)?,
            uuid::Uuid::from_bytes(key),
        ))
    }

    fn encode<T: Serialize>(value: &T) -> Result<Entry, ArtifactWriteError> {
        serde_json::to_vec(value)
            .map(Entry::bytes)
            .map_err(|_| ArtifactWriteError::Storage)
    }

    async fn metadata(
        &self,
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
        artifact_id: ArtifactId,
    ) -> Result<
        Option<(StoredArtifactMetadata, ironclaw_filesystem::RecordVersion)>,
        ArtifactWriteError,
    > {
        let path = Self::metadata_path(owner, namespace, artifact_id)?;
        let stored = self
            .filesystem
            .get(&ResourceScope::system(), &path)
            .await
            .map_err(|_| ArtifactWriteError::Storage)?;
        stored
            .map(|versioned| {
                serde_json::from_slice(&versioned.entry.body)
                    .map(|metadata| (metadata, versioned.version))
                    .map_err(|_| ArtifactWriteError::Storage)
            })
            .transpose()
    }

    async fn receipt_artifact_id(
        &self,
        owner: &ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
        receipt: ironclaw_host_api::ids::ResourceReservationId,
    ) -> Result<Option<ArtifactId>, ArtifactWriteError> {
        let path = Self::receipt_path(owner, namespace, receipt)?;
        self.filesystem
            .get(&ResourceScope::system(), &path)
            .await
            .map_err(|_| ArtifactWriteError::Storage)?
            .map(|stored| {
                serde_json::from_slice::<StoredArtifactReceipt>(&stored.entry.body)
                    .map(|binding| binding.artifact_id)
                    .map_err(|_| ArtifactWriteError::Storage)
            })
            .transpose()
    }

    async fn resume_accounted_persist(
        &self,
        requested: &ArtifactWriteMetadata,
        artifact_id: ArtifactId,
        bytes: &[u8],
    ) -> Result<CompletedArtifact, ArtifactWriteError> {
        let (stored, _) = self
            .metadata(&requested.owner_scope, requested.namespace, artifact_id)
            .await?
            .ok_or(ArtifactWriteError::Storage)?;
        if stored.owner_scope != requested.owner_scope
            || stored.namespace != requested.namespace
            || stored.producer_capability_id != requested.producer_capability_id
            || stored.content_type != requested.content_type
            || stored.expected_bytes != requested.expected_bytes
        {
            return Err(ArtifactWriteError::Storage);
        }

        let expected_digest = ArtifactDigest::from_bytes(bytes);
        if stored.finalized_at.is_some() {
            if stored.total_bytes != bytes.len() as u64
                || stored.digest != Some(expected_digest)
                || stored.total_lines.is_none()
            {
                return Err(ArtifactWriteError::DigestMismatch);
            }
            return Ok(CompletedArtifact {
                artifact_ref: ArtifactRef::new(artifact_id),
                byte_len: stored.total_bytes,
                total_lines: stored.total_lines,
                content_type: stored.content_type,
                digest: expected_digest,
            });
        }

        let mut offset = 0_usize;
        for chunk in &stored.chunks {
            let persisted = self.read_chunk(&stored, chunk).await?;
            let end = offset
                .checked_add(persisted.len())
                .ok_or(ArtifactWriteError::Storage)?;
            if bytes.get(offset..end) != Some(persisted.as_slice()) {
                return Err(ArtifactWriteError::DigestMismatch);
            }
            offset = end;
        }
        let handle = ArtifactWriteHandle::new(
            artifact_id,
            requested.owner_scope.clone(),
            requested.namespace,
        );
        if let Some(remaining) = bytes.get(offset..) {
            self.append(&handle, remaining).await?;
        } else {
            return Err(ArtifactWriteError::DigestMismatch);
        }
        self.finalize(handle).await
    }

    async fn read_chunk(
        &self,
        metadata: &StoredArtifactMetadata,
        chunk: &StoredArtifactChunk,
    ) -> Result<Vec<u8>, ArtifactWriteError> {
        let path = Self::chunk_path(
            &metadata.owner_scope,
            metadata.namespace,
            metadata.artifact_id,
            chunk.index,
        )?;
        let entry = self
            .filesystem
            .get(&ResourceScope::system(), &path)
            .await
            .map_err(|_| ArtifactWriteError::Storage)?
            .ok_or(ArtifactWriteError::Storage)?;
        if chunk
            .byte_start
            .zip(chunk.byte_end)
            .is_some_and(|(start, end)| end.checked_sub(start) != Some(chunk.byte_len))
            || entry.entry.body.len() as u64 != chunk.byte_len
            || chunk
                .digest
                .is_some_and(|digest| ArtifactDigest::from_bytes(&entry.entry.body) != digest)
        {
            return Err(ArtifactWriteError::Storage);
        }
        Ok(entry.entry.body)
    }

    pub fn bind_legacy_result_service(
        &self,
        service: Arc<dyn crate::SessionThreadService>,
    ) -> Result<(), ArtifactWriteError> {
        self.legacy_result_service
            .set(service)
            .map_err(|_| ArtifactWriteError::Storage)
    }

    async fn legacy_mapping_artifact_id(
        &self,
        request: &LegacyResultArtifactRequest,
    ) -> Result<Option<ArtifactId>, ArtifactWriteError> {
        let path = Self::legacy_mapping_path(
            &request.owner_scope,
            request.namespace,
            &request.thread_id,
            &request.result_ref,
        )?;
        self.filesystem
            .get(&ResourceScope::system(), &path)
            .await
            .map_err(|error| legacy_projection_failure("legacy_mapping_get", error))?
            .map(|stored| {
                serde_json::from_slice::<StoredArtifactReceipt>(&stored.entry.body)
                    .map(|binding| binding.artifact_id)
                    .map_err(|error| legacy_projection_failure("legacy_mapping_decode", error))
            })
            .transpose()
    }

    fn completed_legacy_artifact(
        metadata: StoredArtifactMetadata,
    ) -> Result<CompletedArtifact, ArtifactWriteError> {
        let digest = metadata
            .digest
            .ok_or_else(|| legacy_projection_missing("finalized_digest"))?;
        if metadata.finalized_at.is_none() {
            return Err(legacy_projection_missing("finalized_at"));
        }
        if !matches!(
            metadata.backing,
            ToolArtifactBacking::LegacyResultRecord { .. }
        ) {
            return Err(legacy_projection_missing("legacy_result_backing"));
        }
        Ok(CompletedArtifact {
            artifact_ref: ArtifactRef::new(metadata.artifact_id),
            byte_len: metadata.total_bytes,
            total_lines: metadata.total_lines,
            content_type: metadata.content_type,
            digest,
        })
    }

    pub async fn project_legacy_result(
        &self,
        request: LegacyResultArtifactRequest,
    ) -> Result<CompletedArtifact, ArtifactWriteError> {
        if let Some(artifact_id) = self.legacy_mapping_artifact_id(&request).await? {
            let (metadata, _) = self
                .metadata(&request.owner_scope, request.namespace, artifact_id)
                .await?
                .ok_or_else(|| legacy_projection_missing("mapped_artifact_metadata"))?;
            if metadata.finalized_at.is_some() {
                return Self::completed_legacy_artifact(metadata);
            }
        }

        let service = self
            .legacy_result_service
            .get()
            .ok_or_else(|| legacy_projection_missing("legacy_result_service_binding"))?;
        let mut offset = 0_u64;
        let mut total_bytes = None;
        let mut hasher = Sha256::new();
        let mut newline_count = 0_u64;
        let mut last_byte = None;
        loop {
            let chunk = service
                .read_tool_result_record(crate::ReadToolResultRecordRequest {
                    scope: request.thread_scope.clone(),
                    thread_id: request.thread_id.clone(),
                    result_ref: request.result_ref.clone(),
                    offset,
                    max_bytes: crate::TOOL_RESULT_RECORD_READ_MAX_BYTES,
                })
                .await
                .map_err(|error| legacy_projection_failure("read_tool_result_record", error))?
                // The most common leg in production: live capability results
                // persist artifacts, not tool result records, so only a
                // prepared-context seeded transcript has a record to project.
                .ok_or_else(|| legacy_projection_missing("tool_result_record"))?;
            if total_bytes
                .replace(chunk.total_bytes)
                .is_some_and(|total| total != chunk.total_bytes)
                || chunk.content.is_empty() && chunk.next_offset.is_some()
            {
                return Err(legacy_projection_missing("stable_record_chunk_invariant"));
            }
            hasher.update(&chunk.content);
            newline_count = newline_count
                .checked_add(chunk.content.iter().filter(|byte| **byte == b'\n').count() as u64)
                .ok_or_else(|| legacy_projection_missing("newline_count_overflow"))?;
            last_byte = chunk.content.last().copied().or(last_byte);
            offset = offset
                .checked_add(chunk.content.len() as u64)
                .ok_or_else(|| legacy_projection_missing("read_offset_overflow"))?;
            match chunk.next_offset {
                Some(next) if next == offset => {}
                Some(_) => return Err(legacy_projection_missing("record_chunk_offset_continuity")),
                None => break,
            }
        }
        let total_bytes =
            total_bytes.ok_or_else(|| legacy_projection_missing("record_total_bytes"))?;
        if offset != total_bytes {
            return Err(legacy_projection_missing("record_total_bytes_agreement"));
        }
        let total_lines = if total_bytes == 0 {
            0
        } else if last_byte == Some(b'\n') {
            newline_count
        } else {
            newline_count
                .checked_add(1)
                .ok_or_else(|| legacy_projection_missing("line_count_overflow"))?
        };
        let digest = ArtifactDigest::from_sha256_bytes(hasher.finalize().into());
        let handle = self
            .allocate(ArtifactWriteMetadata {
                write_key: None,
                namespace: request.namespace,
                owner_scope: request.owner_scope.clone(),
                producer_capability_id: request.producer_capability_id.clone(),
                content_type: "application/json".to_string(),
                expected_bytes: Some(total_bytes),
            })
            .await?;
        let mapping_path = Self::legacy_mapping_path(
            &request.owner_scope,
            request.namespace,
            &request.thread_id,
            &request.result_ref,
        )?;
        let candidate = StoredArtifactReceipt {
            artifact_id: handle.artifact_id(),
        };
        let artifact_id = match self
            .filesystem
            .put(
                &ResourceScope::system(),
                &mapping_path,
                Self::encode(&candidate)?,
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) => handle.artifact_id(),
            // Lost the mapping CAS to a concurrent projection: adopt its
            // artifact id instead of minting a second one.
            Err(error) => {
                tracing::debug!(
                    operation = "project_legacy_result",
                    leg = "legacy_mapping_cas_lost",
                    cause = %error,
                    "adopting the concurrent projection's artifact id"
                );
                self.legacy_mapping_artifact_id(&request)
                    .await?
                    .ok_or_else(|| legacy_projection_missing("legacy_mapping_cas_winner"))?
            }
        };
        let (mut metadata, version) = self
            .metadata(&request.owner_scope, request.namespace, artifact_id)
            .await?
            .ok_or_else(|| legacy_projection_missing("allocated_artifact_metadata"))?;
        if metadata.finalized_at.is_some() {
            return Self::completed_legacy_artifact(metadata);
        }
        metadata.backing = ToolArtifactBacking::LegacyResultRecord {
            thread_scope: request.thread_scope,
            thread_id: request.thread_id,
            result_ref: request.result_ref,
        };
        metadata.total_bytes = total_bytes;
        metadata.total_lines = Some(total_lines);
        metadata.digest = Some(digest);
        metadata.finalized_at = Some(Utc::now());
        let metadata_path =
            Self::metadata_path(&request.owner_scope, request.namespace, artifact_id)?;
        match self
            .filesystem
            .put(
                &ResourceScope::system(),
                &metadata_path,
                Self::encode(&metadata)?,
                CasExpectation::Version(version),
            )
            .await
        {
            Ok(_) => Self::completed_legacy_artifact(metadata),
            // Lost the metadata CAS to a concurrent projection of the same
            // record: its finalized metadata is equally authoritative.
            Err(error) => {
                tracing::debug!(
                    operation = "project_legacy_result",
                    leg = "legacy_metadata_cas_lost",
                    cause = %error,
                    "replaying the concurrent projection's finalized metadata"
                );
                let (winner, _) = self
                    .metadata(&request.owner_scope, request.namespace, artifact_id)
                    .await?
                    .ok_or_else(|| legacy_projection_missing("legacy_metadata_cas_winner"))?;
                Self::completed_legacy_artifact(winner)
            }
        }
    }

    async fn read_byte_range(
        &self,
        metadata: &StoredArtifactMetadata,
        range: ArtifactByteRange,
        max_output_bytes: u64,
    ) -> Result<ArtifactReadChunk, ArtifactAccessError> {
        if range.end < range.start || range.start >= metadata.total_bytes {
            return Err(ArtifactAccessError::InvalidSelector);
        }
        let exclusive_end = range.end.saturating_add(1).min(metadata.total_bytes);
        let requested_len = exclusive_end.saturating_sub(range.start);
        if requested_len > max_output_bytes {
            return Err(ArtifactAccessError::OversizedUnsliced);
        }
        let expected_len =
            usize::try_from(requested_len).map_err(|_| ArtifactAccessError::InvalidSelector)?;

        let content = if let ToolArtifactBacking::LegacyResultRecord {
            thread_scope,
            thread_id,
            result_ref,
        } = &metadata.backing
        {
            let service = self
                .legacy_result_service
                .get()
                .ok_or(ArtifactAccessError::Storage)?;
            let chunk = service
                .read_tool_result_record(crate::ReadToolResultRecordRequest {
                    scope: thread_scope.clone(),
                    thread_id: thread_id.clone(),
                    result_ref: result_ref.clone(),
                    offset: range.start,
                    max_bytes: expected_len,
                })
                .await
                .map_err(|_| ArtifactAccessError::Storage)?
                .ok_or(ArtifactAccessError::Storage)?;
            if chunk.total_bytes != metadata.total_bytes || chunk.content.len() != expected_len {
                return Err(ArtifactAccessError::Storage);
            }
            chunk.content
        } else {
            let mut content = Vec::with_capacity(expected_len);
            let mut legacy_offset = 0_u64;
            for chunk in &metadata.chunks {
                let chunk_start = chunk.byte_start.unwrap_or(legacy_offset);
                let chunk_end = chunk
                    .byte_end
                    .unwrap_or_else(|| chunk_start.saturating_add(chunk.byte_len));
                legacy_offset = chunk_end;
                if chunk_end <= range.start || chunk_start >= exclusive_end {
                    continue;
                }
                let bytes = self
                    .read_chunk(metadata, chunk)
                    .await
                    .map_err(|_| ArtifactAccessError::Storage)?;
                let local_start = usize::try_from(range.start.saturating_sub(chunk_start))
                    .map_err(|_| ArtifactAccessError::Storage)?
                    .min(bytes.len());
                let local_end = usize::try_from(exclusive_end.saturating_sub(chunk_start))
                    .map_err(|_| ArtifactAccessError::Storage)?
                    .min(bytes.len());
                content.extend_from_slice(&bytes[local_start..local_end]);
            }
            if content.len() != expected_len {
                return Err(ArtifactAccessError::Storage);
            }
            content
        };

        Ok(ArtifactReadChunk {
            content,
            content_type: metadata.content_type.clone(),
            total_bytes: metadata.total_bytes,
            total_lines: metadata.total_lines,
            complete: range.start == 0 && exclusive_end == metadata.total_bytes,
        })
    }
    async fn bound_write_handle(
        &self,
        requested: &ArtifactWriteMetadata,
        artifact_id: ArtifactId,
    ) -> Result<ArtifactWriteHandle, ArtifactWriteError> {
        let (stored, _) = self
            .metadata(&requested.owner_scope, requested.namespace, artifact_id)
            .await?
            .ok_or(ArtifactWriteError::InvalidHandle)?;
        if stored.owner_scope != requested.owner_scope
            || stored.namespace != requested.namespace
            || stored.producer_capability_id != requested.producer_capability_id
            || stored.content_type != requested.content_type
            || stored.expected_bytes != requested.expected_bytes
            || stored.write_key != requested.write_key
        {
            return Err(ArtifactWriteError::DigestMismatch);
        }
        Ok(ArtifactWriteHandle::new(
            artifact_id,
            requested.owner_scope.clone(),
            requested.namespace,
        ))
    }
}

#[async_trait]
impl ArtifactPersistencePort for DurableToolArtifactStore {
    async fn allocate(
        &self,
        metadata: ArtifactWriteMetadata,
    ) -> Result<ArtifactWriteHandle, ArtifactWriteError> {
        let requested = metadata.clone();
        if let Some(write_key) = metadata.write_key
            && let Some(artifact_id) = self
                .write_binding_artifact_id(&metadata.owner_scope, metadata.namespace, write_key)
                .await?
        {
            return self.bound_write_handle(&metadata, artifact_id).await;
        }

        let counter_path = Self::counter_path(&metadata.owner_scope, metadata.namespace)?;
        let artifact_id = cas_update(
            self.filesystem.as_ref(),
            &ResourceScope::system(),
            &counter_path,
            |bytes| serde_json::from_slice::<u64>(bytes).map_err(|_| ArtifactWriteError::Storage),
            Self::encode,
            |current| async move {
                let next_id = current.unwrap_or(0);
                let following = next_id.checked_add(1).ok_or(ArtifactWriteError::Storage)?;
                Ok(CasApply::new(following, ArtifactId::new(next_id)))
            },
        )
        .await
        .map_err(|_| ArtifactWriteError::Storage)?;

        let stored = StoredArtifactMetadata {
            artifact_id,
            namespace: metadata.namespace,
            owner_scope: metadata.owner_scope.clone(),
            producer_capability_id: metadata.producer_capability_id,
            content_type: metadata.content_type,
            expected_bytes: metadata.expected_bytes,
            write_key: metadata.write_key,
            total_bytes: 0,
            chunks: Vec::new(),
            created_at: Utc::now(),
            finalized_at: None,
            backing: ToolArtifactBacking::StoredChunks,
            digest: None,
            total_lines: None,
        };
        let path = Self::metadata_path(&metadata.owner_scope, metadata.namespace, artifact_id)?;
        self.filesystem
            .put(
                &ResourceScope::system(),
                &path,
                Self::encode(&stored)?,
                CasExpectation::Absent,
            )
            .await
            .map_err(|_| ArtifactWriteError::Storage)?;

        if let Some(write_key) = requested.write_key {
            let binding_path =
                Self::write_binding_path(&requested.owner_scope, requested.namespace, write_key)?;
            if self
                .filesystem
                .put(
                    &ResourceScope::system(),
                    &binding_path,
                    Self::encode(&StoredArtifactReceipt { artifact_id })?,
                    CasExpectation::Absent,
                )
                .await
                .is_err()
            {
                let adopted = self
                    .write_binding_artifact_id(
                        &requested.owner_scope,
                        requested.namespace,
                        write_key,
                    )
                    .await?
                    .ok_or(ArtifactWriteError::Storage)?;
                return self.bound_write_handle(&requested, adopted).await;
            }
        }
        self.bound_write_handle(&requested, artifact_id).await
    }

    async fn state(
        &self,
        handle: &ArtifactWriteHandle,
    ) -> Result<ArtifactWriteState, ArtifactWriteError> {
        let (metadata, _) = self
            .metadata(
                handle.owner_scope(),
                handle.namespace(),
                handle.artifact_id(),
            )
            .await?
            .ok_or(ArtifactWriteError::InvalidHandle)?;
        if metadata.owner_scope != *handle.owner_scope() || metadata.namespace != handle.namespace()
        {
            return Err(ArtifactWriteError::InvalidHandle);
        }
        let mut hasher = Sha256::new();
        let mut persisted_bytes = 0_u64;
        for chunk in &metadata.chunks {
            let bytes = self.read_chunk(&metadata, chunk).await?;
            persisted_bytes = persisted_bytes
                .checked_add(bytes.len() as u64)
                .ok_or(ArtifactWriteError::Storage)?;
            hasher.update(bytes);
        }
        if persisted_bytes != metadata.total_bytes {
            return Err(ArtifactWriteError::Storage);
        }
        let persisted_digest = ArtifactDigest::from_sha256_bytes(hasher.finalize().into());
        let completed = if metadata.finalized_at.is_some() {
            let digest = metadata.digest.ok_or(ArtifactWriteError::Storage)?;
            if digest != persisted_digest {
                return Err(ArtifactWriteError::DigestMismatch);
            }
            Some(CompletedArtifact {
                artifact_ref: ArtifactRef::new(metadata.artifact_id),
                byte_len: metadata.total_bytes,
                total_lines: metadata.total_lines,
                content_type: metadata.content_type,
                digest,
            })
        } else {
            None
        };
        Ok(ArtifactWriteState {
            persisted_bytes,
            persisted_digest,
            completed,
        })
    }
    async fn append(
        &self,
        handle: &ArtifactWriteHandle,
        chunk: &[u8],
    ) -> Result<(), ArtifactWriteError> {
        if chunk.is_empty() {
            return Ok(());
        }
        let (mut metadata, mut version) = self
            .metadata(
                handle.owner_scope(),
                handle.namespace(),
                handle.artifact_id(),
            )
            .await?
            .ok_or(ArtifactWriteError::InvalidHandle)?;
        if metadata.finalized_at.is_some() {
            return Err(ArtifactWriteError::InvalidHandle);
        }
        let metadata_path = Self::metadata_path(
            handle.owner_scope(),
            handle.namespace(),
            handle.artifact_id(),
        )?;

        for bytes in chunk.chunks(TOOL_ARTIFACT_CHUNK_BYTES) {
            let index = metadata.chunks.len() as u64;
            let path = Self::chunk_path(
                handle.owner_scope(),
                handle.namespace(),
                handle.artifact_id(),
                index,
            )?;
            if self
                .filesystem
                .put(
                    &ResourceScope::system(),
                    &path,
                    Entry::bytes(bytes.to_vec()),
                    CasExpectation::Absent,
                )
                .await
                .is_err()
            {
                let existing = self
                    .filesystem
                    .get(&ResourceScope::system(), &path)
                    .await
                    .map_err(|_| ArtifactWriteError::Storage)?
                    .ok_or(ArtifactWriteError::Storage)?;
                if existing.entry.body != bytes {
                    return Err(ArtifactWriteError::DigestMismatch);
                }
            }
            let byte_start = metadata.total_bytes;
            let byte_len = bytes.len() as u64;
            let byte_end = byte_start
                .checked_add(byte_len)
                .ok_or(ArtifactWriteError::Storage)?;
            let line_start = metadata
                .chunks
                .last()
                .and_then(|chunk| chunk.line_after)
                .unwrap_or(1);
            let newline_count = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
            let line_after = line_start
                .checked_add(newline_count)
                .ok_or(ArtifactWriteError::Storage)?;
            let line_end = if bytes.last() == Some(&b'\n') {
                line_after.saturating_sub(1)
            } else {
                line_after
            };
            metadata.total_bytes = byte_end;
            metadata.chunks.push(StoredArtifactChunk {
                index,
                byte_len,
                byte_start: Some(byte_start),
                byte_end: Some(byte_end),
                line_start: Some(line_start),
                line_end: Some(line_end),
                line_after: Some(line_after),
                digest: Some(ArtifactDigest::from_bytes(bytes)),
            });

            match self
                .filesystem
                .put(
                    &ResourceScope::system(),
                    &metadata_path,
                    Self::encode(&metadata)?,
                    CasExpectation::Version(version),
                )
                .await
            {
                Ok(next_version) => version = next_version,
                Err(_) => {
                    let (winner, winner_version) = self
                        .metadata(
                            handle.owner_scope(),
                            handle.namespace(),
                            handle.artifact_id(),
                        )
                        .await?
                        .ok_or(ArtifactWriteError::InvalidHandle)?;
                    if winner != metadata {
                        return Err(ArtifactWriteError::Storage);
                    }
                    metadata = winner;
                    version = winner_version;
                }
            }
        }
        Ok(())
    }

    async fn finalize(
        &self,
        handle: ArtifactWriteHandle,
    ) -> Result<CompletedArtifact, ArtifactWriteError> {
        let (mut metadata, version) = self
            .metadata(
                handle.owner_scope(),
                handle.namespace(),
                handle.artifact_id(),
            )
            .await?
            .ok_or(ArtifactWriteError::InvalidHandle)?;
        if metadata.finalized_at.is_some() {
            return Err(ArtifactWriteError::InvalidHandle);
        }
        if metadata
            .expected_bytes
            .is_some_and(|expected| expected != metadata.total_bytes)
        {
            return Err(ArtifactWriteError::DigestMismatch);
        }

        let mut hasher = Sha256::new();
        let mut newline_count = 0_u64;
        let mut last_byte = None;
        for chunk in &metadata.chunks {
            let bytes = self.read_chunk(&metadata, chunk).await?;
            hasher.update(&bytes);
            newline_count = newline_count
                .checked_add(bytes.iter().filter(|byte| **byte == b'\n').count() as u64)
                .ok_or(ArtifactWriteError::Storage)?;
            last_byte = bytes.last().copied().or(last_byte);
        }
        let total_lines = if metadata.total_bytes == 0 {
            0
        } else if last_byte == Some(b'\n') {
            newline_count
        } else {
            newline_count
                .checked_add(1)
                .ok_or(ArtifactWriteError::Storage)?
        };
        let digest = ArtifactDigest::from_sha256_bytes(hasher.finalize().into());
        metadata.digest = Some(digest);
        metadata.total_lines = Some(total_lines);
        metadata.finalized_at = Some(Utc::now());

        let path = Self::metadata_path(
            handle.owner_scope(),
            handle.namespace(),
            handle.artifact_id(),
        )?;
        self.filesystem
            .put(
                &ResourceScope::system(),
                &path,
                Self::encode(&metadata)?,
                CasExpectation::Version(version),
            )
            .await
            .map_err(|_| ArtifactWriteError::Storage)?;

        Ok(CompletedArtifact {
            artifact_ref: ArtifactRef::new(handle.artifact_id()),
            byte_len: metadata.total_bytes,
            total_lines: metadata.total_lines,
            content_type: metadata.content_type,
            digest,
        })
    }
}

#[async_trait]
impl AccountedArtifactPersister for DurableToolArtifactStore {
    async fn persist(
        &self,
        metadata: ArtifactWriteMetadata,
        bytes: &[u8],
        receipt: &ResourceReceipt,
    ) -> Result<CompletedArtifact, ArtifactWriteError> {
        let accounted_output_bytes = receipt
            .actual
            .as_ref()
            .map(|usage| usage.output_bytes)
            .unwrap_or_default();
        let byte_len = u64::try_from(bytes.len()).map_err(|_| ArtifactWriteError::Budget)?;
        if receipt.status != ReservationStatus::Reconciled
            || ArtifactOwnerScope::from_resource_scope(&receipt.scope) != metadata.owner_scope
            || accounted_output_bytes < byte_len
            || metadata
                .expected_bytes
                .is_some_and(|expected| expected != byte_len)
        {
            return Err(ArtifactWriteError::Budget);
        }
        if let Some(artifact_id) = self
            .receipt_artifact_id(&metadata.owner_scope, metadata.namespace, receipt.id)
            .await?
        {
            return self
                .resume_accounted_persist(&metadata, artifact_id, bytes)
                .await;
        }

        let handle = self.allocate(metadata.clone()).await?;
        let artifact_id = handle.artifact_id();
        let receipt_path =
            Self::receipt_path(&metadata.owner_scope, metadata.namespace, receipt.id)?;
        if self
            .filesystem
            .put(
                &ResourceScope::system(),
                &receipt_path,
                Self::encode(&StoredArtifactReceipt { artifact_id })?,
                CasExpectation::Absent,
            )
            .await
            .is_err()
        {
            let adopted = self
                .receipt_artifact_id(&metadata.owner_scope, metadata.namespace, receipt.id)
                .await?
                .ok_or(ArtifactWriteError::Storage)?;
            return self
                .resume_accounted_persist(&metadata, adopted, bytes)
                .await;
        }
        for chunk in bytes.chunks(TOOL_ARTIFACT_CHUNK_BYTES) {
            self.append(&handle, chunk).await?;
        }
        self.finalize(handle).await
    }
}

#[async_trait]
impl ArtifactAccessPort for DurableToolArtifactStore {
    async fn read(
        &self,
        request: ArtifactReadRequest,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError> {
        let metadata = self
            .metadata(
                &request.owner_scope,
                request.namespace,
                request.target.artifact_id,
            )
            .await
            .map_err(|_| ArtifactAccessError::Storage)?
            .map(|(metadata, _)| metadata);
        let Some(metadata) = metadata else {
            return Ok(None);
        };
        if metadata.owner_scope != request.owner_scope || metadata.namespace != request.namespace {
            return Ok(None);
        }
        if metadata.finalized_at.is_none() {
            return Ok(None);
        }
        if let ArtifactSelector::Bytes(range) = request.target.selector {
            return self
                .read_byte_range(&metadata, range, request.target.max_output_bytes)
                .await
                .map(Some);
        }
        let finalized_digest = metadata.digest.ok_or(ArtifactAccessError::Storage)?;
        if matches!(request.target.selector, ArtifactSelector::Full)
            && metadata.total_bytes > request.target.max_output_bytes
        {
            return Err(ArtifactAccessError::OversizedUnsliced);
        }

        let ranges: Vec<ArtifactLineRange> = match &request.target.selector {
            ArtifactSelector::Full => Vec::new(),
            ArtifactSelector::Lines(range) | ArtifactSelector::RawLines(range) => vec![*range],
            ArtifactSelector::MultiLines(ranges) => ranges.clone(),
            _ => return Err(ArtifactAccessError::InvalidSelector),
        };
        if ranges
            .iter()
            .any(|range| range.start == 0 || range.end < range.start)
        {
            return Err(ArtifactAccessError::InvalidSelector);
        }

        if let ToolArtifactBacking::LegacyResultRecord {
            thread_scope,
            thread_id,
            result_ref,
        } = &metadata.backing
        {
            let service = self
                .legacy_result_service
                .get()
                .ok_or(ArtifactAccessError::Storage)?;
            let mut offset = 0_u64;
            let mut line = 1_u64;
            let mut output = Vec::new();
            let mut hasher = Sha256::new();
            loop {
                let chunk = service
                    .read_tool_result_record(crate::ReadToolResultRecordRequest {
                        scope: thread_scope.clone(),
                        thread_id: thread_id.clone(),
                        result_ref: result_ref.clone(),
                        offset,
                        max_bytes: crate::TOOL_RESULT_RECORD_READ_MAX_BYTES,
                    })
                    .await
                    .map_err(|_| ArtifactAccessError::Storage)?
                    .ok_or(ArtifactAccessError::Storage)?;
                if chunk.total_bytes != metadata.total_bytes
                    || chunk.content.is_empty() && chunk.next_offset.is_some()
                {
                    return Err(ArtifactAccessError::Storage);
                }
                hasher.update(&chunk.content);
                let chunk_len = chunk.content.len() as u64;
                for byte in chunk.content {
                    let selected = ranges.is_empty()
                        || ranges
                            .iter()
                            .any(|range| (range.start..=range.end).contains(&line));
                    if selected {
                        if output.len() as u64 >= request.target.max_output_bytes {
                            return Err(ArtifactAccessError::OversizedUnsliced);
                        }
                        output.push(byte);
                    }
                    if byte == b'\n' {
                        line = line.checked_add(1).ok_or(ArtifactAccessError::Storage)?;
                    }
                }
                offset = offset
                    .checked_add(chunk_len)
                    .ok_or(ArtifactAccessError::Storage)?;
                match chunk.next_offset {
                    Some(next) if next == offset => {}
                    Some(_) => return Err(ArtifactAccessError::Storage),
                    None => break,
                }
            }
            if offset != metadata.total_bytes
                || ArtifactDigest::from_sha256_bytes(hasher.finalize().into()) != finalized_digest
            {
                return Err(ArtifactAccessError::Storage);
            }
            return Ok(Some(ArtifactReadChunk {
                content: output,
                content_type: metadata.content_type,
                total_bytes: metadata.total_bytes,
                total_lines: metadata.total_lines,
                complete: ranges.is_empty(),
            }));
        }
        let indexed_chunks = metadata.chunks.iter().all(|chunk| {
            chunk.line_start.is_some()
                && chunk.line_end.is_some()
                && chunk.byte_start.is_some()
                && chunk.byte_end.is_some()
        });
        let mut output = Vec::new();
        let mut full_hasher = ranges.is_empty().then(Sha256::new);
        let mut legacy_line = 1;
        for chunk in metadata.chunks.iter().filter(|chunk| {
            ranges.is_empty()
                || !indexed_chunks
                || ranges.iter().any(|range| {
                    range.start <= chunk.line_end.unwrap_or(u64::MAX)
                        && range.end >= chunk.line_start.unwrap_or(1)
                })
        }) {
            let bytes = self
                .read_chunk(&metadata, chunk)
                .await
                .map_err(|_| ArtifactAccessError::Storage)?;
            if let Some(hasher) = full_hasher.as_mut() {
                hasher.update(&bytes);
            }
            let mut line = chunk.line_start.unwrap_or(legacy_line);
            for byte in bytes {
                let selected = ranges.is_empty()
                    || ranges
                        .iter()
                        .any(|range| (range.start..=range.end).contains(&line));
                if selected {
                    if output.len() as u64 >= request.target.max_output_bytes {
                        return Err(ArtifactAccessError::OversizedUnsliced);
                    }
                    output.push(byte);
                }
                if byte == b'\n' {
                    line = line.checked_add(1).ok_or(ArtifactAccessError::Storage)?;
                }
            }
            legacy_line = line;
        }
        if let Some(hasher) = full_hasher
            && ArtifactDigest::from_sha256_bytes(hasher.finalize().into()) != finalized_digest
        {
            return Err(ArtifactAccessError::Storage);
        }

        Ok(Some(ArtifactReadChunk {
            content: output,
            content_type: metadata.content_type,
            total_bytes: metadata.total_bytes,
            total_lines: metadata.total_lines,
            complete: ranges.is_empty(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionThreadService;
    use ironclaw_filesystem::{Fault, FaultInjecting, FilesystemOperation, InMemoryBackend};
    use ironclaw_host_api::{
        ids::{AgentId, CapabilityId, InvocationId, ProjectId, RunId, TenantId, ThreadId, UserId},
        resource::ResourceScope,
    };

    #[tokio::test]
    async fn absent_and_named_optional_owner_axes_never_share_artifact_authority() {
        let store = DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new())).expect("store");
        let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
        let base = ArtifactOwnerScope {
            tenant_id: TenantId::new("scope-isolation-tenant").expect("tenant"),
            user_id: UserId::new("scope-isolation-user").expect("user"),
            agent_id: None,
            project_id: None,
        };
        let write = |owner_scope: ArtifactOwnerScope, bytes: &'static [u8]| {
            let store = store.clone();
            async move {
                let handle = store
                    .allocate(ArtifactWriteMetadata {
                        write_key: None,
                        owner_scope,
                        namespace,
                        producer_capability_id: CapabilityId::new("builtin.test")
                            .expect("capability"),
                        content_type: "text/plain".to_string(),
                        expected_bytes: Some(bytes.len() as u64),
                    })
                    .await
                    .expect("allocate");
                store.append(&handle, bytes).await.expect("append");
                store.finalize(handle).await.expect("finalize")
            }
        };
        let absent = write(base.clone(), b"absent").await;
        let colliding_agent = ArtifactOwnerScope {
            agent_id: Some(AgentId::new("_none").expect("agent")),
            ..base.clone()
        };
        assert!(
            store
                .read(ArtifactReadRequest {
                    owner_scope: colliding_agent.clone(),
                    namespace,
                    target: ironclaw_host_api::artifact::ArtifactReadTarget {
                        artifact_id: absent.artifact_ref.id(),
                        selector: ArtifactSelector::Full,
                        max_output_bytes: 64,
                    },
                })
                .await
                .expect("cross-scope lookup")
                .is_none(),
            "a named agent must not read the absent-agent scope"
        );
        let named_agent = write(colliding_agent, b"named-agent").await;
        assert_eq!(named_agent.artifact_ref.id(), ArtifactId::new(0));

        let colliding_project = ArtifactOwnerScope {
            project_id: Some(ProjectId::new("_none").expect("project")),
            ..base
        };
        let named_project = write(colliding_project, b"named-project").await;
        assert_eq!(named_project.artifact_ref.id(), ArtifactId::new(0));
    }

    #[tokio::test]
    async fn byte_range_reads_a_bounded_slice_of_one_long_line() {
        let store = DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new())).expect("store");
        let scope = ResourceScope::local_default(
            UserId::new("byte-range-owner").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");
        let owner_scope = ArtifactOwnerScope::from_resource_scope(&scope);
        let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
        let handle = store
            .allocate(ArtifactWriteMetadata {
                write_key: None,
                owner_scope: owner_scope.clone(),
                namespace,
                producer_capability_id: CapabilityId::new("builtin.test").expect("capability"),
                content_type: "application/json".to_string(),
                expected_bytes: Some(100_000),
            })
            .await
            .expect("allocate");
        store
            .append(&handle, &vec![b'x'; 100_000])
            .await
            .expect("append");
        let completed = store.finalize(handle).await.expect("finalize");
        let chunk = store
            .read(ArtifactReadRequest {
                owner_scope,
                namespace,
                target: ironclaw_host_api::artifact::ArtifactReadTarget {
                    artifact_id: completed.artifact_ref.id(),
                    selector: ArtifactSelector::Bytes(ArtifactByteRange {
                        start: 60_000,
                        end: 60_127,
                    }),
                    max_output_bytes: 128,
                },
            })
            .await
            .expect("byte-range read")
            .expect("artifact exists");
        assert_eq!(chunk.content, vec![b'x'; 128]);
        assert!(!chunk.complete);
        assert_eq!(chunk.total_bytes, 100_000);
    }

    #[tokio::test]
    async fn append_retry_adopts_an_identical_chunk_after_metadata_write_failure() {
        let backend = Arc::new(
            FaultInjecting::new(InMemoryBackend::new()).with_fault(
                Fault::on(FilesystemOperation::WriteFile)
                    .path("metadata.json")
                    .nth(2)
                    .backend("injected metadata write failure"),
            ),
        );
        let store = DurableToolArtifactStore::new(backend).expect("store");
        let scope = ResourceScope::local_default(
            UserId::new("append-retry-owner").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");
        let owner_scope = ArtifactOwnerScope::from_resource_scope(&scope);
        let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
        let handle = store
            .allocate(ArtifactWriteMetadata {
                write_key: None,
                owner_scope: owner_scope.clone(),
                namespace,
                producer_capability_id: CapabilityId::new("builtin.test").expect("capability"),
                content_type: "text/plain".to_string(),
                expected_bytes: Some(7),
            })
            .await
            .expect("allocate");
        assert_eq!(
            store.append(&handle, b"durable").await,
            Err(ArtifactWriteError::Storage),
        );
        store
            .append(&handle, b"durable")
            .await
            .expect("retry adopts identical chunk");
        let completed = store.finalize(handle).await.expect("finalize");
        assert_eq!(completed.byte_len, 7);

        let read = store
            .read(ArtifactReadRequest {
                owner_scope,
                namespace,
                target: ironclaw_host_api::artifact::ArtifactReadTarget {
                    artifact_id: completed.artifact_ref.id(),
                    selector: ArtifactSelector::Full,
                    max_output_bytes: 7,
                },
            })
            .await
            .expect("read")
            .expect("artifact");
        assert_eq!(read.content, b"durable");
    }

    #[tokio::test]
    async fn invocation_write_binding_resumes_and_recovers_completed_artifact() {
        let store = DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new())).expect("store");
        let scope = ResourceScope::local_default(
            UserId::new("write-binding-owner").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");
        let owner_scope = ArtifactOwnerScope::from_resource_scope(&scope);
        let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
        let write_key = InvocationId::new();
        let metadata = ArtifactWriteMetadata {
            write_key: Some(write_key),
            owner_scope,
            namespace,
            producer_capability_id: CapabilityId::new("builtin.test").expect("capability"),
            content_type: "text/plain".to_string(),
            expected_bytes: Some((TOOL_ARTIFACT_CHUNK_BYTES + 7) as u64),
        };
        let first = store.allocate(metadata.clone()).await.expect("allocate");
        store
            .append(&first, &vec![b'x'; TOOL_ARTIFACT_CHUNK_BYTES])
            .await
            .expect("append first chunk");

        let resumed = store
            .allocate(metadata.clone())
            .await
            .expect("resume allocation");
        assert_eq!(resumed.artifact_id(), first.artifact_id());
        let state = store.state(&resumed).await.expect("write state");
        assert_eq!(state.persisted_bytes, TOOL_ARTIFACT_CHUNK_BYTES as u64);
        assert!(state.completed.is_none());
        store
            .append(&resumed, b"tail-7!")
            .await
            .expect("append remaining bytes");
        let completed = store.finalize(resumed).await.expect("finalize");

        let replayed = store.allocate(metadata).await.expect("replay allocation");
        let state = store.state(&replayed).await.expect("completed write state");
        assert_eq!(state.completed, Some(completed));
    }

    #[tokio::test]
    async fn line_range_skips_unrelated_corrupt_chunk_and_full_read_detects_it() {
        let store = DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new())).expect("store");
        let scope = ResourceScope::local_default(
            UserId::new("range-corruption-owner").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");
        let owner_scope = ArtifactOwnerScope::from_resource_scope(&scope);
        let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
        let handle = store
            .allocate(ArtifactWriteMetadata {
                write_key: None,
                owner_scope: owner_scope.clone(),
                namespace,
                producer_capability_id: CapabilityId::new("builtin.test").expect("capability"),
                content_type: "text/plain".to_string(),
                expected_bytes: None,
            })
            .await
            .expect("allocate");
        let mut content = b"a\n".repeat(TOOL_ARTIFACT_CHUNK_BYTES / 2);
        content.extend_from_slice(b"b\n");
        store.append(&handle, &content).await.expect("append");
        let completed = store.finalize(handle).await.expect("finalize");

        let second_chunk_path = DurableToolArtifactStore::chunk_path(
            &owner_scope,
            namespace,
            completed.artifact_ref.id(),
            1,
        )
        .expect("chunk path");
        let stored = store
            .filesystem
            .get(&ResourceScope::system(), &second_chunk_path)
            .await
            .expect("read chunk")
            .expect("second chunk exists");
        store
            .filesystem
            .put(
                &ResourceScope::system(),
                &second_chunk_path,
                Entry::bytes(b"c\n".to_vec()),
                CasExpectation::Version(stored.version),
            )
            .await
            .expect("corrupt second chunk");

        let first_line = store
            .read(ArtifactReadRequest {
                owner_scope: owner_scope.clone(),
                namespace,
                target: ironclaw_host_api::artifact::ArtifactReadTarget {
                    artifact_id: completed.artifact_ref.id(),
                    selector: ArtifactSelector::Lines(ArtifactLineRange { start: 1, end: 1 }),
                    max_output_bytes: 24 * 1024,
                },
            })
            .await
            .expect("unrelated corrupt chunk must not be read")
            .expect("artifact");
        assert_eq!(first_line.content, b"a\n");

        let full_error = store
            .read(ArtifactReadRequest {
                owner_scope,
                namespace,
                target: ironclaw_host_api::artifact::ArtifactReadTarget {
                    artifact_id: completed.artifact_ref.id(),
                    selector: ArtifactSelector::Full,
                    max_output_bytes: content.len() as u64,
                },
            })
            .await
            .expect_err("full read must detect corrupt chunk");
        assert_eq!(full_error, ArtifactAccessError::Storage);
    }

    #[tokio::test]
    async fn retained_pre_index_chunk_metadata_remains_readable() {
        let store = DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new())).expect("store");
        let scope = ResourceScope::local_default(
            UserId::new("legacy-artifact-owner").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");
        let owner_scope = ArtifactOwnerScope::from_resource_scope(&scope);
        let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
        let handle = store
            .allocate(ArtifactWriteMetadata {
                write_key: None,
                owner_scope: owner_scope.clone(),
                namespace,
                producer_capability_id: CapabilityId::new("builtin.test").expect("capability"),
                content_type: "text/plain".to_string(),
                expected_bytes: None,
            })
            .await
            .expect("allocate");
        let artifact_id = handle.artifact_id();
        store
            .append(&handle, b"one\ntwo\nthree\n")
            .await
            .expect("append");
        store.finalize(handle).await.expect("finalize");

        let metadata_path =
            DurableToolArtifactStore::metadata_path(&owner_scope, namespace, artifact_id)
                .expect("metadata path");
        let stored = store
            .filesystem
            .get(&ResourceScope::system(), &metadata_path)
            .await
            .expect("read metadata")
            .expect("metadata exists");
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&stored.entry.body).expect("decode metadata");
        for chunk in legacy["chunks"].as_array_mut().expect("chunk array") {
            let chunk = chunk.as_object_mut().expect("chunk object");
            for field in [
                "byte_start",
                "byte_end",
                "line_start",
                "line_end",
                "line_after",
                "digest",
            ] {
                chunk.remove(field);
            }
        }
        store
            .filesystem
            .put(
                &ResourceScope::system(),
                &metadata_path,
                Entry::bytes(serde_json::to_vec(&legacy).expect("encode legacy metadata")),
                CasExpectation::Version(stored.version),
            )
            .await
            .expect("write legacy metadata");

        let read = store
            .read(ArtifactReadRequest {
                owner_scope,
                namespace,
                target: ironclaw_host_api::artifact::ArtifactReadTarget {
                    artifact_id,
                    selector: ArtifactSelector::Lines(ArtifactLineRange { start: 2, end: 3 }),
                    max_output_bytes: 24 * 1024,
                },
            })
            .await
            .expect("legacy metadata reads")
            .expect("artifact");
        assert_eq!(read.content, b"two\nthree\n");
    }

    #[tokio::test]
    async fn legacy_result_projection_is_durable_idempotent_and_selector_readable() {
        let backend = Arc::new(InMemoryBackend::new());
        let service = Arc::new(crate::InMemorySessionThreadService::default());
        let store = DurableToolArtifactStore::new(Arc::clone(&backend)).expect("store");
        let service_port: Arc<dyn crate::SessionThreadService> = service.clone();
        store
            .bind_legacy_result_service(service_port)
            .expect("bind legacy result reader");
        let owner_user_id = UserId::new("projection-owner").expect("user");
        let thread_scope = crate::ThreadScope {
            tenant_id: TenantId::new("projection-tenant").expect("tenant"),
            agent_id: AgentId::new("projection-agent").expect("agent"),
            project_id: None,
            owner_user_id: Some(owner_user_id.clone()),
            mission_id: None,
        };
        let thread_id = ThreadId::new("projection-thread").expect("thread");
        service
            .ensure_thread(crate::EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "projection-test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure thread");
        let result_ref = "result:historical-output".to_string();
        let content = b"one\ntwo\nthree\n".to_vec();
        service
            .put_tool_result_record(crate::PutToolResultRecordRequest {
                scope: thread_scope.clone(),
                thread_id: thread_id.clone(),
                result_ref: result_ref.clone(),
                content: content.clone(),
            })
            .await
            .expect("store historical result");
        let resource_scope = thread_scope.to_resource_scope();
        let owner_scope = ArtifactOwnerScope::from_resource_scope(&resource_scope);
        let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
        let request = LegacyResultArtifactRequest {
            owner_scope: owner_scope.clone(),
            namespace,
            producer_capability_id: CapabilityId::new("builtin.test").expect("capability"),
            thread_scope,
            thread_id,
            result_ref,
        };

        let (projected, concurrent) = tokio::join!(
            store.project_legacy_result(request.clone()),
            store.project_legacy_result(request.clone()),
        );
        let projected = projected.expect("project result");
        let concurrent = concurrent.expect("concurrent projection");
        assert_eq!(concurrent, projected);
        let replay = store
            .project_legacy_result(request)
            .await
            .expect("replay projection");
        assert_eq!(replay, projected);
        let mut finalized_count = 0;
        for id in [ArtifactId::new(0), ArtifactId::new(1)] {
            finalized_count += store
                .metadata(&owner_scope, namespace, id)
                .await
                .expect("metadata scan")
                .is_some_and(|(metadata, _)| metadata.finalized_at.is_some())
                as usize;
        }
        assert_eq!(finalized_count, 1, "a racing projection must finalize once");
        let (metadata, _) = store
            .metadata(&owner_scope, namespace, projected.artifact_ref.id())
            .await
            .expect("metadata lookup")
            .expect("metadata");
        assert!(
            metadata.chunks.is_empty(),
            "projection must not copy payload chunks"
        );
        assert!(matches!(
            metadata.backing,
            ToolArtifactBacking::LegacyResultRecord { .. }
        ));

        let selected = store
            .read(ArtifactReadRequest {
                owner_scope,
                namespace,
                target: ironclaw_host_api::artifact::ArtifactReadTarget {
                    artifact_id: projected.artifact_ref.id(),
                    selector: ArtifactSelector::Lines(ArtifactLineRange { start: 2, end: 2 }),
                    max_output_bytes: 24 * 1024,
                },
            })
            .await
            .expect("selector read")
            .expect("projected artifact");
        assert_eq!(selected.content, b"two\n");
        assert_eq!(selected.total_bytes, content.len() as u64);
        assert_eq!(selected.total_lines, Some(3));
    }

    /// `ArtifactWriteError` is a unit-variant contract type, so a projection
    /// failure reaches the operator as one indistinguishable "artifact storage
    /// failed". Two very different legs — an unbound legacy result service and
    /// a result ref with no durable record behind it — must be told apart in
    /// the DEBUG log, or diagnosing an incident means reading the source.
    #[tokio::test]
    #[tracing_test::traced_test]
    async fn legacy_projection_failure_names_the_failing_leg_in_the_debug_log() {
        let unbound = DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new()))
            .expect("store without a legacy result service");
        let thread_scope = crate::ThreadScope {
            tenant_id: TenantId::new("leg-tenant").expect("tenant"),
            agent_id: AgentId::new("leg-agent").expect("agent"),
            project_id: None,
            owner_user_id: Some(UserId::new("leg-owner").expect("user")),
            mission_id: None,
        };
        let request = LegacyResultArtifactRequest {
            owner_scope: ArtifactOwnerScope::from_resource_scope(
                &ResourceScope::local_default(
                    UserId::new("leg-owner").expect("user"),
                    InvocationId::new(),
                )
                .expect("scope"),
            ),
            namespace: ArtifactNamespaceId::from_root_run(RunId::new()),
            producer_capability_id: CapabilityId::new("builtin.legacy_result").expect("capability"),
            thread_scope: thread_scope.clone(),
            thread_id: ThreadId::new("leg-thread").expect("thread"),
            result_ref: "result:no-durable-record".to_string(),
        };

        assert_eq!(
            unbound.project_legacy_result(request.clone()).await,
            Err(ArtifactWriteError::Storage)
        );
        assert!(logs_contain("leg=\"legacy_result_service_binding\""));

        let bound = DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new()))
            .expect("store with a legacy result service");
        let backing = Arc::new(crate::InMemorySessionThreadService::default());
        // The thread exists and the result ref is well formed — only the
        // durable tool result record is absent, which is what a LIVE capability
        // result looks like: it persists an artifact, never a record.
        backing
            .ensure_thread(crate::EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(request.thread_id.clone()),
                created_by_actor_id: "leg-test".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("ensure thread");
        let service: Arc<dyn SessionThreadService> = backing;
        bound
            .bind_legacy_result_service(service)
            .expect("bind legacy result reader");

        assert_eq!(
            bound.project_legacy_result(request).await,
            Err(ArtifactWriteError::Storage)
        );
        assert!(
            logs_contain("leg=\"tool_result_record\""),
            "a live capability result has no durable record to project; the log must say so"
        );
    }
}

#[cfg(test)]
mod accounted_persistence_tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        ids::{CapabilityId, InvocationId, ResourceReservationId, RunId, UserId},
        resource::{ResourceEstimate, ResourceUsage},
    };

    #[tokio::test]
    async fn repeated_receipt_adopts_the_same_completed_artifact() {
        let store = DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new())).expect("store");
        let scope = ResourceScope::local_default(
            UserId::new("receipt-owner").expect("user"),
            InvocationId::new(),
        )
        .expect("scope");
        let bytes = b"durable once";
        let byte_len = bytes.len() as u64;
        let metadata = ArtifactWriteMetadata {
            write_key: None,
            owner_scope: ArtifactOwnerScope::from_resource_scope(&scope),
            namespace: ArtifactNamespaceId::from_root_run(RunId::new()),
            producer_capability_id: CapabilityId::new("builtin.test").expect("capability"),
            content_type: "text/plain".to_string(),
            expected_bytes: Some(byte_len),
        };
        let receipt = ResourceReceipt {
            id: ResourceReservationId::new(),
            scope,
            status: ReservationStatus::Reconciled,
            estimate: ResourceEstimate::default().set_output_bytes(byte_len),
            actual: Some(ResourceUsage::default().set_output_bytes(byte_len)),
        };

        let first = store
            .persist(metadata.clone(), bytes, &receipt)
            .await
            .expect("first persist");
        let replay = store
            .persist(metadata, bytes, &receipt)
            .await
            .expect("replayed persist");

        assert_eq!(replay, first);
    }
}
