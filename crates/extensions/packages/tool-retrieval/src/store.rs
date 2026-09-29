//! Durable tool-vector store: a typed wrapper over [`ScopedFilesystem`].
//!
//! # Layout
//!
//! Everything lives under the per-user `/tool-vectors` mount alias, which
//! production composition resolves to `/tenants/<tenant>/users/<user>/tool-vectors`:
//!
//! - `/tool-vectors/<space>/<digest>.f32` — one vector: the little-endian
//!   `f32` components, nothing else. `<space>` names the embedding space
//!   (provider id, model, dimension; see [`EmbeddingSpace`]) and `<digest>`
//!   is the SHA-256 of the tool document, both in hex.
//! - `/tool-vectors/manifest.json` — the owner's bounded index of stored
//!   vectors across every space, with the day each was last used. It exists
//!   to bound the store: it is never consulted to decide whether a vector is
//!   present (the vector file is), so a manifest entry whose file is gone is
//!   a miss, never an error.
//!
//! # Size bound
//!
//! An owner keeps at most `capacity` vectors across all spaces. A save that
//! pushes the manifest over it evicts the least recently used entries (oldest
//! day first, then space and digest, so eviction is deterministic), never one
//! the save itself wrote. Vectors from a retired embedding space are
//! evicted by the same rule once newer vectors need their room.
//!
//! # Crash safety
//!
//! Every write keeps "manifest entry without a file" as the only possible
//! inconsistency, which is harmless (a miss) and self-healing (the entry is
//! oldest, so it is evicted first). Saves add manifest entries *before*
//! writing files; evictions delete files *before* removing manifest entries.
//! So a file is never left outside the manifest, and the size bound holds
//! across crashes.
//!
//! # Concurrency
//!
//! Manifest read-modify-writes go through [`cas_update`], so processes
//! sharing a backend never lose each other's entries. Vector files are
//! immutable for their path (the content of a path is a pure function of
//! the space and the document), so writing one is a plain put.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FilesystemError, RecordKind,
    RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::ids::InvocationId;
use ironclaw_host_api::path::ScopedPath;
use ironclaw_host_api::resource::ResourceScope;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use ironclaw_loop_contracts::ToolCorpusOwner;

use crate::document::DocumentDigest;

/// Mount alias every path of this store lives under. Composition grants it
/// per user.
pub const TOOL_VECTOR_MOUNT_ALIAS: &str = "/tool-vectors";

/// Default most vectors one owner keeps: twice the corpus limit, so a whole
/// catalog under two embedding spaces (the old one and the new one, across
/// a model change) fits.
pub const DEFAULT_STORED_VECTORS_PER_OWNER: usize = 2 * crate::provider::MAX_CORPUS_DEFINITIONS;

const MANIFEST_PATH: &str = "/tool-vectors/manifest.json";
const MANIFEST_SCHEMA_VERSION: u8 = 1;
const VECTOR_RECORD_KIND: &str = "tool_vector";
/// Parallel reads per load: enough to hide backend latency, small enough
/// not to monopolise a connection pool.
const LOAD_CONCURRENCY: usize = 16;
/// Largest vector the store accepts or reads back, in components.
const MAX_VECTOR_COMPONENTS: usize = 16_384;

/// The filesystem scope an owner's records resolve under. Stored vectors
/// are partitioned by owner: a vector is only ever read back for the owner
/// that stored it, so a document from one user's private tools can neither
/// be served from, nor probed in, another user's store.
fn owner_scope(owner: &ToolCorpusOwner) -> ResourceScope {
    ResourceScope {
        tenant_id: owner.tenant_id.clone(),
        user_id: owner.user_id.clone(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

/// The embedding space a vector lives in: which provider, which model, and
/// which configured dimension produced it.
///
/// Vectors from different spaces are never comparable, so the space is part
/// of every stored vector's key: a model or dimension change reads nothing
/// the old configuration wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingSpace {
    id: String,
}

impl EmbeddingSpace {
    /// `dimension` is the configured dimension, or `None` for the model's
    /// native one.
    pub fn new(provider_id: &str, model: &str, dimension: Option<usize>) -> Self {
        let mut hasher = Sha256::new();
        // Length-prefixed fields, so no two (provider, model) pairs collide
        // by moving a separator.
        for field in [provider_id.as_bytes(), model.as_bytes()] {
            hasher.update((field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        match dimension {
            Some(dimension) => {
                hasher.update([1]);
                hasher.update((dimension as u64).to_le_bytes());
            }
            None => hasher.update([0]),
        }
        let digest: [u8; 32] = hasher.finalize().into();
        // 128 bits name a space; the full document digest names a vector.
        Self {
            id: hex(&digest[..16]),
        }
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }
}

/// Why a store operation failed. Never carries document text: paths hold
/// only hex digests.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VectorStoreError {
    #[error("tool vector store filesystem error: {reason}")]
    Filesystem { reason: String },
    #[error("tool vector store manifest is malformed: {reason}")]
    MalformedManifest { reason: String },
    #[error("tool vector store could not update its manifest: {reason}")]
    ManifestUpdate { reason: String },
}

impl VectorStoreError {
    /// Stable label for debug logs.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Filesystem { .. } => "filesystem",
            Self::MalformedManifest { .. } => "malformed_manifest",
            Self::ManifestUpdate { .. } => "manifest_update",
        }
    }
}

/// The durable operations the dense ranker needs, over any backend.
///
/// One implementation, [`FilesystemToolVectorStore`]; the trait only erases
/// its filesystem type parameter so the provider is not generic over it.
#[async_trait::async_trait]
pub(crate) trait VectorStore: Send + Sync {
    async fn load(
        &self,
        owner: &ToolCorpusOwner,
        space: &EmbeddingSpace,
        digests: &[DocumentDigest],
    ) -> Result<Vec<Option<Arc<[f32]>>>, VectorStoreError>;

    async fn save(
        &self,
        owner: &ToolCorpusOwner,
        space: &EmbeddingSpace,
        vectors: &[(DocumentDigest, Arc<[f32]>)],
        today: u32,
    ) -> Result<(), VectorStoreError>;

    async fn touch(
        &self,
        owner: &ToolCorpusOwner,
        space: &EmbeddingSpace,
        digests: &[DocumentDigest],
        today: u32,
    ) -> Result<(), VectorStoreError>;
}

/// Tool vectors persisted through a [`ScopedFilesystem`] under the
/// `/tool-vectors` alias. See the module docs for layout and bounds.
pub struct FilesystemToolVectorStore<F: RootFilesystem + ?Sized> {
    filesystem: Arc<ScopedFilesystem<F>>,
    capacity: usize,
}

impl<F: RootFilesystem + ?Sized> std::fmt::Debug for FilesystemToolVectorStore<F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilesystemToolVectorStore")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

impl<F: RootFilesystem + ?Sized> FilesystemToolVectorStore<F> {
    /// A store keeping at most `capacity` vectors per owner (at least one).
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>, capacity: usize) -> Self {
        Self {
            filesystem,
            capacity: capacity.max(1),
        }
    }

    /// The stored vectors for `digests` under `space`, in order; `None` where
    /// a vector is absent or unreadable.
    pub async fn load_vectors(
        &self,
        owner: &ToolCorpusOwner,
        space: &EmbeddingSpace,
        digests: &[DocumentDigest],
    ) -> Result<Vec<Option<Arc<[f32]>>>, VectorStoreError> {
        let scope = owner_scope(owner);
        let mut slots: Vec<Option<Arc<[f32]>>> = Vec::with_capacity(digests.len());
        for chunk in digests.chunks(LOAD_CONCURRENCY) {
            let mut reads = Vec::with_capacity(chunk.len());
            for digest in chunk {
                let path = vector_path(space, digest)?;
                let filesystem = Arc::clone(&self.filesystem);
                let scope = scope.clone();
                reads.push(async move { filesystem.get(&scope, &path).await });
            }
            for read in futures::future::join_all(reads).await {
                let entry = read.map_err(filesystem_error)?;
                slots.push(entry.and_then(|entry| decode_vector(&entry.entry.body)));
            }
        }
        Ok(slots)
    }

    /// Store `vectors` under `space`, marking them used `today` (days since
    /// the Unix epoch), then evict the owner's least recently used vectors
    /// beyond capacity. Vectors this call stores are never its victims.
    pub async fn save_vectors(
        &self,
        owner: &ToolCorpusOwner,
        space: &EmbeddingSpace,
        vectors: &[(DocumentDigest, Arc<[f32]>)],
        today: u32,
    ) -> Result<(), VectorStoreError> {
        if vectors.is_empty() {
            return Ok(());
        }
        let scope = owner_scope(owner);
        let saved: BTreeSet<ManifestKey> = vectors
            .iter()
            .map(|(digest, _)| ManifestKey::new(space, digest))
            .collect();

        // 1. Manifest first: a crash after this leaves entries without
        //    files (misses), never files outside the manifest.
        let victims = self
            .update_manifest(&scope, |manifest| {
                for key in &saved {
                    manifest.entries.insert(key.clone(), today);
                }
                let victims = manifest.victims(self.capacity, &saved);
                Ok((true, victims))
            })
            .await?;

        // 2. The vectors themselves.
        for (digest, vector) in vectors {
            if vector.is_empty() || vector.len() > MAX_VECTOR_COMPONENTS {
                continue;
            }
            let path = vector_path(space, digest)?;
            self.filesystem
                .put(&scope, &path, encode_vector(vector)?, CasExpectation::Any)
                .await
                .map_err(filesystem_error)?;
        }

        // 3. Evict: files first, then their manifest entries, and only
        //    entries nobody used since the victims were chosen.
        if victims.is_empty() {
            return Ok(());
        }
        for (key, _) in &victims {
            match self.filesystem.delete(&scope, &key.path()?).await {
                Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
                Err(error) => return Err(filesystem_error(error)),
            }
        }
        self.update_manifest(&scope, |manifest| {
            let mut changed = false;
            for (key, day) in &victims {
                if manifest.entries.get(key) == Some(day) {
                    manifest.entries.remove(key);
                    changed = true;
                }
            }
            Ok((changed, ()))
        })
        .await
    }

    /// Mark the stored `digests` under `space` as used `today`. Entries not
    /// in the manifest are left alone: touching never adds.
    pub async fn touch_vectors(
        &self,
        owner: &ToolCorpusOwner,
        space: &EmbeddingSpace,
        digests: &[DocumentDigest],
        today: u32,
    ) -> Result<(), VectorStoreError> {
        if digests.is_empty() {
            return Ok(());
        }
        let scope = owner_scope(owner);
        let keys: Vec<ManifestKey> = digests
            .iter()
            .map(|digest| ManifestKey::new(space, digest))
            .collect();
        self.update_manifest(&scope, |manifest| {
            let mut changed = false;
            for key in &keys {
                if let Some(day) = manifest.entries.get_mut(key)
                    && *day < today
                {
                    *day = today;
                    changed = true;
                }
            }
            Ok((changed, ()))
        })
        .await
    }

    /// Every manifest entry of `owner`, as `(space id, digest hex, day)`.
    /// For tests and diagnostics.
    pub async fn manifest_entries(
        &self,
        owner: &ToolCorpusOwner,
    ) -> Result<Vec<(String, String, u32)>, VectorStoreError> {
        let scope = owner_scope(owner);
        let path = manifest_path()?;
        let Some(entry) = self
            .filesystem
            .get(&scope, &path)
            .await
            .map_err(filesystem_error)?
        else {
            return Ok(Vec::new());
        };
        let manifest = Manifest::decode(&entry.entry.body)?;
        Ok(manifest
            .entries
            .into_iter()
            .map(|(key, day)| (key.space, key.digest, day))
            .collect())
    }

    /// Read-modify-write the owner's manifest through [`cas_update`].
    /// `mutate` returns whether it changed the manifest and its outcome; it
    /// is re-run on every CAS retry against a fresh read.
    async fn update_manifest<T, M>(
        &self,
        scope: &ResourceScope,
        mutate: M,
    ) -> Result<T, VectorStoreError>
    where
        T: Clone,
        M: Fn(&mut Manifest) -> Result<(bool, T), VectorStoreError>,
    {
        let path = manifest_path()?;
        cas_update(
            &self.filesystem,
            scope,
            &path,
            Manifest::decode,
            Manifest::encode,
            |current: Option<Manifest>| {
                let result = (|| {
                    let mut manifest = current.clone().unwrap_or_default();
                    let (changed, outcome) = mutate(&mut manifest)?;
                    Ok(if changed {
                        CasApply::new(manifest, outcome)
                    } else {
                        CasApply::no_op(manifest, outcome)
                    })
                })();
                async move { result }
            },
        )
        .await
        .map_err(|error| match error {
            CasUpdateError::Apply(error) => error,
            other => VectorStoreError::ManifestUpdate {
                reason: other.to_string(),
            },
        })
    }
}

#[async_trait::async_trait]
impl<F: RootFilesystem + ?Sized + 'static> VectorStore for FilesystemToolVectorStore<F> {
    async fn load(
        &self,
        owner: &ToolCorpusOwner,
        space: &EmbeddingSpace,
        digests: &[DocumentDigest],
    ) -> Result<Vec<Option<Arc<[f32]>>>, VectorStoreError> {
        self.load_vectors(owner, space, digests).await
    }

    async fn save(
        &self,
        owner: &ToolCorpusOwner,
        space: &EmbeddingSpace,
        vectors: &[(DocumentDigest, Arc<[f32]>)],
        today: u32,
    ) -> Result<(), VectorStoreError> {
        self.save_vectors(owner, space, vectors, today).await
    }

    async fn touch(
        &self,
        owner: &ToolCorpusOwner,
        space: &EmbeddingSpace,
        digests: &[DocumentDigest],
        today: u32,
    ) -> Result<(), VectorStoreError> {
        self.touch_vectors(owner, space, digests, today).await
    }
}

/// One stored vector's manifest identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ManifestKey {
    space: String,
    digest: String,
}

impl ManifestKey {
    fn new(space: &EmbeddingSpace, digest: &DocumentDigest) -> Self {
        Self {
            space: space.id().to_string(),
            digest: hex(digest),
        }
    }

    fn path(&self) -> Result<ScopedPath, VectorStoreError> {
        // Both halves were validated as lowercase hex when decoded, so the
        // path cannot escape the alias.
        scoped_path(format!(
            "{TOOL_VECTOR_MOUNT_ALIAS}/{}/{}.f32",
            self.space, self.digest
        ))
    }
}

/// The owner's bounded index of stored vectors: key to the day (since the
/// Unix epoch) it was last stored or used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Manifest {
    entries: BTreeMap<ManifestKey, u32>,
}

#[derive(Serialize, Deserialize)]
struct ManifestWire {
    version: u8,
    entries: Vec<ManifestEntryWire>,
}

#[derive(Serialize, Deserialize)]
struct ManifestEntryWire {
    space: String,
    digest: String,
    day: u32,
}

impl Manifest {
    fn decode(bytes: &[u8]) -> Result<Self, VectorStoreError> {
        let malformed = |reason: String| VectorStoreError::MalformedManifest { reason };
        let wire: ManifestWire =
            serde_json::from_slice(bytes).map_err(|error| malformed(error.to_string()))?;
        if wire.version != MANIFEST_SCHEMA_VERSION {
            return Err(malformed(format!(
                "unsupported manifest version {}",
                wire.version
            )));
        }
        let mut entries = BTreeMap::new();
        for entry in wire.entries {
            if !is_hex(&entry.space, 32) || !is_hex(&entry.digest, 64) {
                return Err(malformed("an entry key is not lowercase hex".to_string()));
            }
            entries.insert(
                ManifestKey {
                    space: entry.space,
                    digest: entry.digest,
                },
                entry.day,
            );
        }
        Ok(Self { entries })
    }

    fn encode(&self) -> Result<Entry, VectorStoreError> {
        let wire = ManifestWire {
            version: MANIFEST_SCHEMA_VERSION,
            entries: self
                .entries
                .iter()
                .map(|(key, day)| ManifestEntryWire {
                    space: key.space.clone(),
                    digest: key.digest.clone(),
                    day: *day,
                })
                .collect(),
        };
        let body =
            serde_json::to_vec(&wire).map_err(|error| VectorStoreError::MalformedManifest {
                reason: error.to_string(),
            })?;
        Ok(Entry::bytes(body).with_content_type(ContentType::json()))
    }

    /// The entries to evict to bring the manifest within `capacity`: least
    /// recently used first, never one in `protected`.
    fn victims(
        &self,
        capacity: usize,
        protected: &BTreeSet<ManifestKey>,
    ) -> Vec<(ManifestKey, u32)> {
        let excess = self.entries.len().saturating_sub(capacity);
        if excess == 0 {
            return Vec::new();
        }
        let mut candidates: Vec<(u32, &ManifestKey)> = self
            .entries
            .iter()
            .filter(|(key, _)| !protected.contains(*key))
            .map(|(key, day)| (*day, key))
            .collect();
        candidates.sort();
        candidates
            .into_iter()
            .take(excess)
            .map(|(day, key)| (key.clone(), day))
            .collect()
    }
}

fn manifest_path() -> Result<ScopedPath, VectorStoreError> {
    scoped_path(MANIFEST_PATH.to_string())
}

fn vector_path(
    space: &EmbeddingSpace,
    digest: &DocumentDigest,
) -> Result<ScopedPath, VectorStoreError> {
    ManifestKey::new(space, digest).path()
}

fn scoped_path(raw: String) -> Result<ScopedPath, VectorStoreError> {
    ScopedPath::new(raw).map_err(|error| VectorStoreError::Filesystem {
        reason: error.to_string(),
    })
}

fn encode_vector(vector: &[f32]) -> Result<Entry, VectorStoreError> {
    let kind =
        RecordKind::new(VECTOR_RECORD_KIND).map_err(|error| VectorStoreError::Filesystem {
            reason: error.to_string(),
        })?;
    let body: Vec<u8> = vector
        .iter()
        .flat_map(|component| component.to_le_bytes())
        .collect();
    let mut entry = Entry::bytes(body);
    entry.kind = Some(kind);
    Ok(entry)
}

/// A stored vector, or `None` when the bytes are not a non-empty, bounded
/// run of finite little-endian `f32`s. An unreadable vector is a miss: the
/// document is simply embedded again.
fn decode_vector(bytes: &[u8]) -> Option<Arc<[f32]>> {
    let (chunks, rest) = bytes.as_chunks::<4>();
    if chunks.is_empty() || !rest.is_empty() || chunks.len() > MAX_VECTOR_COMPONENTS {
        return None;
    }
    let vector: Vec<f32> = chunks
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect();
    vector
        .iter()
        .all(|component| component.is_finite())
        .then(|| Arc::from(vector))
}

fn filesystem_error(error: FilesystemError) -> VectorStoreError {
    VectorStoreError::Filesystem {
        reason: error.to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

fn is_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_differ_by_provider_model_and_dimension() {
        let base = EmbeddingSpace::new("openai_compatible", "nomic-embed-text", None);
        assert_eq!(
            base,
            EmbeddingSpace::new("openai_compatible", "nomic-embed-text", None)
        );
        for other in [
            EmbeddingSpace::new("openai", "nomic-embed-text", None),
            EmbeddingSpace::new("openai_compatible", "bge-m3", None),
            EmbeddingSpace::new("openai_compatible", "nomic-embed-text", Some(768)),
            // Moving the separator between fields changes the space.
            EmbeddingSpace::new("openai_compatiblen", "omic-embed-text", None),
        ] {
            assert_ne!(base, other);
        }
        assert!(is_hex(base.id(), 32));
    }

    #[test]
    fn vectors_round_trip_and_bad_bytes_are_misses() {
        let vector = [1.5_f32, -0.25, 0.0];
        let entry = encode_vector(&vector).expect("encode");
        assert_eq!(decode_vector(&entry.body).as_deref(), Some(&vector[..]));
        assert_eq!(decode_vector(&[]), None);
        assert_eq!(decode_vector(&[0, 0, 0]), None);
        assert_eq!(decode_vector(&f32::NAN.to_le_bytes()), None);
    }

    #[test]
    fn victims_are_least_recently_used_and_never_protected() {
        let space = EmbeddingSpace::new("p", "m", None);
        let key = |byte: u8| ManifestKey::new(&space, &[byte; 32]);
        let mut manifest = Manifest::default();
        manifest.entries.insert(key(1), 10);
        manifest.entries.insert(key(2), 5);
        manifest.entries.insert(key(3), 7);
        manifest.entries.insert(key(4), 1);
        let protected = BTreeSet::from([key(4)]);
        let victims = manifest.victims(2, &protected);
        assert_eq!(victims, vec![(key(2), 5), (key(3), 7)]);
        assert!(manifest.victims(4, &protected).is_empty());
    }

    #[test]
    fn manifest_round_trips_and_rejects_non_hex_keys() {
        let space = EmbeddingSpace::new("p", "m", None);
        let mut manifest = Manifest::default();
        manifest
            .entries
            .insert(ManifestKey::new(&space, &[7; 32]), 3);
        let entry = manifest.encode().expect("encode");
        assert_eq!(Manifest::decode(&entry.body).expect("decode"), manifest);

        let hostile = br#"{"version":1,"entries":[{"space":"../../etc","digest":"x","day":1}]}"#;
        assert!(Manifest::decode(hostile).is_err());
        let future = br#"{"version":9,"entries":[]}"#;
        assert!(Manifest::decode(future).is_err());
    }
}
