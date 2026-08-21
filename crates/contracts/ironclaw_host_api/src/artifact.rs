//! Neutral host contracts for durable tool-output artifacts.
//!
//! Artifact IDs are deliberately guessable. Authority comes from the host-sealed
//! owner scope and spawn-tree namespace carried beside every storage request.

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    ids::{AgentId, CapabilityId, InvocationId, ProjectId, RunId, TenantId, UserId},
    resource::{ResourceReceipt, ResourceScope},
};

/// Maximum canonical result content carried inline to the model.
///
/// Sized against measured behavior rather than a round number. The two places
/// that enforce it — the canonical bound in `ironclaw_capabilities::dispatch`
/// and the inline-result validator in `ironclaw_loop_contracts` — both arrived
/// with the artifact surface, so before it existed a tool result reached the
/// model unbounded: a PinchBench baseline shows 78 `read` payloads above 24 KiB,
/// median 46.5 KiB and maximum 75.4 KiB, at a higher pass rate and a third of
/// the cost.
///
/// A smaller ceiling is not the cheaper choice. Context accumulates within a
/// turn, so consuming a file of `F` bytes in windows of `C` costs roughly
/// `F^2 / 2C` tokens: halving the payload doubles the tokens and the round
/// trips. Measured across two runs of the same suite, 5.9x smaller payloads
/// cost 2.53x the input tokens.
///
/// 64 KiB stays under the largest observed unbounded payload while bounding the
/// two costs a larger ceiling really does carry: over-fetch noise that persists
/// for the rest of the turn, and untrusted content (HTTP bodies, MCP and wasm
/// results) entering model context in bigger slices.
pub const ARTIFACT_INLINE_PREVIEW_MAX_BYTES: usize = 64 * 1024;

/// Numeric artifact identity within one spawn-tree namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactId(u64);

impl ArtifactId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Durable artifact namespace shared by a root run and its descendants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactNamespaceId(RunId);

impl ArtifactNamespaceId {
    pub const fn from_root_run(run_id: RunId) -> Self {
        Self(run_id)
    }

    pub const fn as_run_id(self) -> RunId {
        self.0
    }
}

/// Stable owner axes used for artifact authorization and storage paths.
///
/// Mission, thread, and invocation identities are intentionally absent: child
/// runs in one spawn tree must share artifacts while remaining within the same
/// tenant, owner, agent, and project boundary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactOwnerScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
}

impl ArtifactOwnerScope {
    pub fn from_resource_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
        }
    }
}

/// Canonical `artifact://<numeric-id>` reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactRef(ArtifactId);

impl ArtifactRef {
    pub const fn new(id: ArtifactId) -> Self {
        Self(id)
    }

    pub const fn id(self) -> ArtifactId {
        self.0
    }
}

impl fmt::Display for ArtifactRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "artifact://{}", self.0.get())
    }
}

impl FromStr for ArtifactRef {
    type Err = ArtifactRefParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(raw_id) = value.strip_prefix("artifact://") else {
            return Err(ArtifactRefParseError::MissingNumericId);
        };
        if raw_id.is_empty() {
            return Err(ArtifactRefParseError::MissingNumericId);
        }
        if !raw_id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ArtifactRefParseError::NonNumericId {
                value: raw_id.to_string(),
            });
        }
        let id = raw_id
            .parse::<u64>()
            .map_err(|_| ArtifactRefParseError::OutOfRange)?;
        Ok(Self::new(ArtifactId::new(id)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArtifactRefParseError {
    #[error("artifact:// URL requires a numeric ID: artifact://0")]
    MissingNumericId,
    #[error("artifact:// ID must be numeric, got: {value}")]
    NonNumericId { value: String },
    #[error("artifact:// ID is outside the supported numeric range")]
    OutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactLineRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ArtifactSelector {
    Full,
    Lines(ArtifactLineRange),
    MultiLines(Vec<ArtifactLineRange>),
    RawLines(ArtifactLineRange),
    Bytes(ArtifactByteRange),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReadTarget {
    pub artifact_id: ArtifactId,
    pub selector: ArtifactSelector,
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReadRequest {
    pub owner_scope: ArtifactOwnerScope,
    pub namespace: ArtifactNamespaceId,
    pub target: ArtifactReadTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactReadChunk {
    pub content: Vec<u8>,
    pub content_type: String,
    pub total_bytes: u64,
    pub total_lines: Option<u64>,
    pub complete: bool,
}

/// SHA-256 digest of the immutable raw artifact bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactDigest([u8; 32]);

impl ArtifactDigest {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Construct from the finalized output of an incremental SHA-256 hasher.
    pub const fn from_sha256_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for ArtifactDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ArtifactDigest(<sha256>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactWriteMetadata {
    pub owner_scope: ArtifactOwnerScope,
    pub namespace: ArtifactNamespaceId,
    pub producer_capability_id: CapabilityId,
    pub content_type: String,
    pub expected_bytes: Option<u64>,
    /// Stable host-issued invocation key used to resume interrupted writes.
    pub write_key: Option<InvocationId>,
}

/// Opaque authority-bearing handle for one incomplete artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactWriteHandle {
    artifact_id: ArtifactId,
    owner_scope: ArtifactOwnerScope,
    namespace: ArtifactNamespaceId,
}

impl ArtifactWriteHandle {
    pub fn new(
        artifact_id: ArtifactId,
        owner_scope: ArtifactOwnerScope,
        namespace: ArtifactNamespaceId,
    ) -> Self {
        Self {
            artifact_id,
            owner_scope,
            namespace,
        }
    }

    pub const fn artifact_id(&self) -> ArtifactId {
        self.artifact_id
    }

    pub fn owner_scope(&self) -> &ArtifactOwnerScope {
        &self.owner_scope
    }

    pub const fn namespace(&self) -> ArtifactNamespaceId {
        self.namespace
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedArtifact {
    pub artifact_ref: ArtifactRef,
    pub byte_len: u64,
    pub total_lines: Option<u64>,
    pub content_type: String,
    pub digest: ArtifactDigest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactWriteState {
    pub persisted_bytes: u64,
    pub persisted_digest: ArtifactDigest,
    pub completed: Option<CompletedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArtifactAccessError {
    #[error("artifact is unavailable")]
    Unavailable,
    #[error("artifact selector is invalid")]
    InvalidSelector,
    #[error("artifact is too large for an unsliced read")]
    OversizedUnsliced,
    #[error("artifact storage failed")]
    Storage,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArtifactWriteError {
    #[error("artifact budget is unavailable")]
    Budget,
    #[error("artifact write handle is invalid")]
    InvalidHandle,
    #[error("artifact digest did not match persisted bytes")]
    DigestMismatch,
    #[error("artifact storage failed")]
    Storage,
}

#[async_trait]
pub trait ArtifactAccessPort: Send + Sync {
    async fn read(
        &self,
        request: ArtifactReadRequest,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError>;
}

#[async_trait]
pub trait ArtifactPersistencePort: Send + Sync {
    async fn allocate(
        &self,
        metadata: ArtifactWriteMetadata,
    ) -> Result<ArtifactWriteHandle, ArtifactWriteError>;

    async fn state(
        &self,
        _handle: &ArtifactWriteHandle,
    ) -> Result<ArtifactWriteState, ArtifactWriteError> {
        Ok(ArtifactWriteState {
            persisted_bytes: 0,
            persisted_digest: ArtifactDigest::from_bytes(&[]),
            completed: None,
        })
    }

    async fn append(
        &self,
        handle: &ArtifactWriteHandle,
        chunk: &[u8],
    ) -> Result<(), ArtifactWriteError>;

    async fn finalize(
        &self,
        handle: ArtifactWriteHandle,
    ) -> Result<CompletedArtifact, ArtifactWriteError>;
}

#[async_trait]
pub trait ScopedArtifactReader: Send + Sync {
    async fn read(
        &self,
        target: ArtifactReadTarget,
    ) -> Result<Option<ArtifactReadChunk>, ArtifactAccessError>;
}

#[async_trait]
pub trait ArtifactWriter: Send + Sync {
    async fn allocate(
        &self,
        metadata: ArtifactWriteMetadata,
    ) -> Result<ArtifactWriteHandle, ArtifactWriteError>;

    async fn append(
        &self,
        handle: &ArtifactWriteHandle,
        chunk: &[u8],
    ) -> Result<(), ArtifactWriteError>;

    async fn finalize(
        &self,
        handle: ArtifactWriteHandle,
    ) -> Result<CompletedArtifact, ArtifactWriteError>;
}

#[async_trait]
pub trait AccountedArtifactPersister: Send + Sync {
    async fn persist(
        &self,
        metadata: ArtifactWriteMetadata,
        bytes: &[u8],
        receipt: &ResourceReceipt,
    ) -> Result<CompletedArtifact, ArtifactWriteError>;
}
