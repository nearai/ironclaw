//! Where document vectors come from: the in-memory cache, the owner's durable
//! store, or a bounded background embedding job.
//!
//! # Why jobs
//!
//! Embedding a large catalog can take longer than a turn is willing to wait
//! (the hybrid ranker gives a fit five seconds). The work therefore never runs
//! inside the fit's own future: missing documents are handed to a job, a task
//! owned by the provider, and the fit only *waits* for it. Dropping the fit
//! (a caller's timeout) drops the wait, never the work: the job keeps
//! embedding, one batch at a time, and each batch is cached and persisted as
//! it lands, so the next fit finds it.
//!
//! # Bounds
//!
//! - A document is in at most one job at a time: a fit needing a document
//!   another job is already embedding joins that job instead of starting one.
//! - At most [`MAX_RUNNING_JOBS`] jobs embed at once; the rest wait for a
//!   permit.
//! - At most [`MAX_PENDING_JOBS`] jobs exist at all. Past that, a fit fails as
//!   `Unavailable` (the hybrid ranker then falls back to BM25F, as for any
//!   dense failure) and background indexing requests are dropped; both are
//!   asked again at the next fit or catalog change.
//! - Every job embeds at most one corpus ([`crate::MAX_CORPUS_DEFINITIONS`]).
//!
//! # Scope
//!
//! The memory cache is content-addressed and process-wide, as before: a
//! vector is a pure function of the document text and the embedding space,
//! so a cache hit can never make one user's ranking depend on another's
//! tools. The durable store is partitioned per [`ToolCorpusOwner`]: a fit for
//! one owner reads only that owner's stored vectors, and a vector reaches an
//! owner's store only because that owner's own corpus contains its document.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use ironclaw_llm::embeddings::EmbeddingProvider;
use ironclaw_loop_contracts::{ToolCorpusOwner, ToolRetrievalError};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;

use crate::cache::VectorCache;
use crate::document::{DocumentDigest, ToolDocument};
use crate::provider::{LOG_TARGET, check_vector, invalid_output, map_embedding_error, summary};
use crate::store::{EmbeddingSpace, VectorStore};

/// Documents per embedding request a job sends. Each batch is cached and
/// persisted before the next is sent, so a job cut short keeps what it did.
pub(crate) const INDEX_BATCH_SIZE: usize = 32;
/// Jobs embedding at the same time.
pub(crate) const MAX_RUNNING_JOBS: usize = 2;
/// Jobs that may exist at once, running or waiting for a permit.
pub(crate) const MAX_PENDING_JOBS: usize = 16;
/// Owners whose persisted-vector ledger is kept in memory; past it the
/// ledger is cleared, which only costs one extra store read per owner.
const MAX_LEDGER_OWNERS: usize = 256;

/// One slot per corpus document: its vector, or `None` until embedded.
pub(crate) type VectorSlots = Vec<Option<Arc<[f32]>>>;

/// A late-bindable handle to the durable store, so the provider can be built
/// before the storage it persists to exists (composition builds storage with
/// the runtime). Unbound, the provider keeps vectors in memory only.
#[derive(Clone, Default)]
pub struct ToolVectorStoreSlot {
    store: Arc<OnceLock<Arc<dyn VectorStore>>>,
}

impl std::fmt::Debug for ToolVectorStoreSlot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolVectorStoreSlot")
            .field("bound", &self.store.get().is_some())
            .finish()
    }
}

impl ToolVectorStoreSlot {
    /// An unbound slot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the store. Only the first binding takes effect; returns whether
    /// this one did.
    pub fn bind<F>(&self, store: crate::store::FilesystemToolVectorStore<F>) -> bool
    where
        F: ironclaw_filesystem::RootFilesystem + ?Sized + 'static,
    {
        self.store.set(Arc::new(store)).is_ok()
    }

    pub fn is_bound(&self) -> bool {
        self.store.get().is_some()
    }

    pub(crate) fn get(&self) -> Option<&Arc<dyn VectorStore>> {
        self.store.get()
    }
}

/// What a job ends with; waiters read their vectors from it, not from the
/// cache, so cache eviction can never make a finished job look unfinished.
struct Job {
    results: Mutex<HashMap<DocumentDigest, Arc<[f32]>>>,
    done: watch::Sender<Option<Result<(), ToolRetrievalError>>>,
}

impl Job {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            results: Mutex::new(HashMap::new()),
            done: watch::Sender::new(None),
        })
    }

    async fn wait(&self) -> Result<(), ToolRetrievalError> {
        let mut receiver = self.done.subscribe();
        let outcome = receiver
            .wait_for(Option::is_some)
            .await
            .map_err(|_closed| unavailable("a tool indexing job ended without a result"))?;
        match outcome.as_ref() {
            Some(result) => result.clone(),
            None => Err(unavailable("a tool indexing job ended without a result")),
        }
    }

    fn result(&self, digest: &DocumentDigest) -> Option<Arc<[f32]>> {
        self.results.lock().ok()?.get(digest).cloned()
    }
}

/// Per owner: which document vectors are known to be in its durable store,
/// and the last day its vectors were marked used.
#[derive(Default)]
struct OwnerLedger {
    owners: HashMap<ToolCorpusOwner, OwnerRecord>,
}

#[derive(Default)]
struct OwnerRecord {
    persisted: HashSet<DocumentDigest>,
    touched_day: Option<u32>,
}

impl OwnerLedger {
    fn record(&mut self, owner: &ToolCorpusOwner) -> &mut OwnerRecord {
        if !self.owners.contains_key(owner) && self.owners.len() >= MAX_LEDGER_OWNERS {
            self.owners.clear();
        }
        self.owners.entry(owner.clone()).or_default()
    }
}

/// Vectors gathered for a corpus before any embedding.
pub(crate) struct Gathered {
    pub(crate) vectors: VectorSlots,
    /// Documents with a vector already (memory or store).
    pub(crate) stored: usize,
    /// Of those, read from the durable store just now.
    pub(crate) loaded: usize,
    /// Vectors the owner's store lacks although the cache had them.
    persist: Vec<(DocumentDigest, Arc<[f32]>)>,
    /// Stored vectors to mark used today, when not yet done today.
    touch: Vec<DocumentDigest>,
}

/// Work handed to a job: documents to embed, and vectors (already in hand)
/// to persist and mark used for the owner.
struct JobWork {
    owner: Option<ToolCorpusOwner>,
    embed: Vec<ToolDocument>,
    persist: Vec<(DocumentDigest, Arc<[f32]>)>,
    touch: Vec<DocumentDigest>,
}

impl JobWork {
    fn is_empty(&self) -> bool {
        self.embed.is_empty() && self.persist.is_empty() && self.touch.is_empty()
    }
}

/// The provider's vector sources and job bookkeeping.
pub(crate) struct Indexing {
    pub(crate) embedder: Arc<dyn EmbeddingProvider>,
    pub(crate) cache: Mutex<VectorCache>,
    store: ToolVectorStoreSlot,
    space: EmbeddingSpace,
    in_flight: Mutex<HashMap<DocumentDigest, Arc<Job>>>,
    ledger: Mutex<OwnerLedger>,
    permits: Semaphore,
}

impl Indexing {
    pub(crate) fn new(
        embedder: Arc<dyn EmbeddingProvider>,
        cache_capacity: usize,
        store: ToolVectorStoreSlot,
    ) -> Self {
        let space = EmbeddingSpace::new(
            embedder.provider_id(),
            embedder.model_name(),
            embedder.dimension(),
        );
        Self {
            embedder,
            cache: Mutex::new(VectorCache::new(cache_capacity)),
            store,
            space,
            in_flight: Mutex::new(HashMap::new()),
            ledger: Mutex::new(OwnerLedger::default()),
            permits: Semaphore::new(MAX_RUNNING_JOBS),
        }
    }

    /// Start a fit and gather every vector that already exists: the memory
    /// cache first, then (for an owner, and only documents not already known
    /// to be there) the owner's durable store.
    pub(crate) async fn gather(
        &self,
        owner: Option<&ToolCorpusOwner>,
        documents: &[ToolDocument],
    ) -> Result<(u64, Gathered), ToolRetrievalError> {
        let store = owner.and(self.store.get());
        let known: HashSet<DocumentDigest> = match (owner, store) {
            (Some(owner), Some(_)) => {
                let mut ledger = self.lock_ledger()?;
                ledger.record(owner).persisted.clone()
            }
            _ => HashSet::new(),
        };

        let mut loaded = 0;
        let mut from_store: HashMap<DocumentDigest, Arc<[f32]>> = HashMap::new();
        let mut store_misses: HashSet<DocumentDigest> = HashSet::new();
        if let (Some(owner), Some(store)) = (owner, store) {
            let unknown: Vec<DocumentDigest> = documents
                .iter()
                .map(|document| document.digest)
                .filter(|digest| !known.contains(digest))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            if !unknown.is_empty() {
                match store.load(owner, &self.space, &unknown).await {
                    Ok(slots) => {
                        let expected = self.embedder.dimension();
                        for (digest, vector) in unknown.into_iter().zip(slots) {
                            match vector {
                                Some(vector)
                                    if expected
                                        .is_none_or(|dimension| dimension == vector.len()) =>
                                {
                                    from_store.insert(digest, vector);
                                }
                                _ => {
                                    store_misses.insert(digest);
                                }
                            }
                        }
                    }
                    Err(error) => {
                        tracing::debug!(
                            target: LOG_TARGET,
                            error_kind = error.kind_label(),
                            "reading stored tool vectors failed; embedding instead"
                        );
                        store_misses.extend(unknown);
                    }
                }
            }
        }

        let mut cache = self.lock_cache()?;
        let generation = cache.begin_fit();
        for (digest, vector) in &from_store {
            cache.insert(*digest, Arc::clone(vector), generation);
        }
        cache.evict_to_capacity(generation);
        let mut stored = 0;
        let mut persist = Vec::new();
        let mut vectors = Vec::with_capacity(documents.len());
        for document in documents {
            let vector = cache.get(&document.digest, generation);
            if let Some(vector) = &vector {
                stored += 1;
                if from_store.contains_key(&document.digest) {
                    loaded += 1;
                } else if store_misses.contains(&document.digest) {
                    persist.push((document.digest, Arc::clone(vector)));
                }
            }
            vectors.push(vector);
        }
        drop(cache);

        let mut touch = Vec::new();
        if let (Some(owner), Some(_)) = (owner, store) {
            let today = today();
            let mut ledger = self.lock_ledger()?;
            let record = ledger.record(owner);
            record.persisted.extend(from_store.keys().copied());
            if record.touched_day != Some(today) {
                record.touched_day = Some(today);
                touch = documents
                    .iter()
                    .map(|document| document.digest)
                    .filter(|digest| record.persisted.contains(digest))
                    .collect();
            }
        }
        persist.sort_by_key(|(digest, _)| *digest);
        persist.dedup_by_key(|(digest, _)| *digest);
        Ok((
            generation,
            Gathered {
                vectors,
                stored,
                loaded,
                persist,
                touch,
            },
        ))
    }

    fn lock_cache(&self) -> Result<std::sync::MutexGuard<'_, VectorCache>, ToolRetrievalError> {
        self.cache.lock().map_err(|_poisoned| {
            unavailable("the dense tool index cache is unusable after a panic")
        })
    }

    fn lock_ledger(&self) -> Result<std::sync::MutexGuard<'_, OwnerLedger>, ToolRetrievalError> {
        self.ledger.lock().map_err(|_poisoned| {
            unavailable("the dense tool index ledger is unusable after a panic")
        })
    }

    fn lock_in_flight(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<DocumentDigest, Arc<Job>>>, ToolRetrievalError>
    {
        self.in_flight.lock().map_err(|_poisoned| {
            unavailable("the dense tool indexing queue is unusable after a panic")
        })
    }

    fn mark_persisted(
        &self,
        owner: &ToolCorpusOwner,
        digests: impl IntoIterator<Item = DocumentDigest>,
    ) {
        if let Ok(mut ledger) = self.ledger.lock() {
            ledger.record(owner).persisted.extend(digests);
        }
    }
}

/// The provider's background tasks. Dropping it (with the provider) aborts
/// every job still running: jobs hold the shared [`Indexing`] state, never
/// the task set, so there is no ownership cycle keeping them alive.
#[derive(Default)]
pub(crate) struct Jobs {
    tasks: Mutex<JoinSet<()>>,
}

/// Jobs a fit is waiting for, and the documents each will supply.
pub(crate) struct Pending {
    jobs: Vec<Arc<Job>>,
}

impl Jobs {
    /// Hand `missing` documents (plus any vectors to persist or mark used)
    /// to jobs: documents already being embedded join that job; the rest,
    /// with the persistence work, form one new job. Returns the jobs to
    /// wait for.
    pub(crate) fn start(
        &self,
        shared: &Arc<Indexing>,
        owner: Option<&ToolCorpusOwner>,
        missing: &[&ToolDocument],
        gathered: &mut Gathered,
    ) -> Result<Pending, ToolRetrievalError> {
        let mut tasks = self.tasks.lock().map_err(|_poisoned| {
            unavailable("the dense tool indexing queue is unusable after a panic")
        })?;
        while tasks.try_join_next().is_some() {}
        let mut in_flight = shared.lock_in_flight()?;
        let mut jobs: Vec<Arc<Job>> = Vec::new();
        let mut embed = Vec::new();
        let mut seen = HashSet::new();
        for document in missing {
            if !seen.insert(document.digest) {
                continue;
            }
            match in_flight.get(&document.digest) {
                Some(job) => {
                    if !jobs.iter().any(|known| Arc::ptr_eq(known, job)) {
                        jobs.push(Arc::clone(job));
                    }
                }
                None => embed.push((*document).clone()),
            }
        }
        let work = JobWork {
            owner: owner.cloned(),
            embed,
            persist: std::mem::take(&mut gathered.persist),
            touch: std::mem::take(&mut gathered.touch),
        };
        if work.is_empty() {
            return Ok(Pending { jobs });
        }
        if tasks.len() >= MAX_PENDING_JOBS {
            if !work.embed.is_empty() {
                return Err(unavailable(
                    "the dense tool indexing queue is full; try again after it drains",
                ));
            }
            // Only bookkeeping was asked for: skip it, the next fit asks again.
            return Ok(Pending { jobs });
        }
        let job = Job::new();
        for document in &work.embed {
            in_flight.insert(document.digest, Arc::clone(&job));
        }
        if !work.embed.is_empty() {
            jobs.push(Arc::clone(&job));
        }
        drop(in_flight);
        tasks.spawn(run_job(Arc::clone(shared), work, job));
        Ok(Pending { jobs })
    }

    /// Gather and index `documents` for `owner` without waiting: the
    /// catalog-change path. Returns whether the request was accepted.
    pub(crate) fn start_background(
        &self,
        shared: &Arc<Indexing>,
        owner: ToolCorpusOwner,
        documents: Vec<ToolDocument>,
    ) -> bool {
        // Called from synchronous code: without a runtime to run the job on,
        // drop the request (the next fit embeds what is missing) rather than
        // panic in `JoinSet::spawn`.
        if tokio::runtime::Handle::try_current().is_err() {
            return false;
        }
        let Ok(mut tasks) = self.tasks.lock() else {
            return false;
        };
        while tasks.try_join_next().is_some() {}
        if tasks.len() >= MAX_PENDING_JOBS {
            return false;
        }
        let shared = Arc::clone(shared);
        tasks.spawn(async move {
            let (_generation, mut gathered) = match shared.gather(Some(&owner), &documents).await {
                Ok(gathered) => gathered,
                Err(error) => {
                    tracing::debug!(
                        target: LOG_TARGET,
                        error_kind = error.kind_label(),
                        "background tool indexing could not read existing vectors"
                    );
                    return;
                }
            };
            let missing: Vec<&ToolDocument> = documents
                .iter()
                .zip(&gathered.vectors)
                .filter(|(_, vector)| vector.is_none())
                .map(|(document, _)| document)
                .collect();
            // Register this task's own job inline, so the documents are
            // marked in flight before the embedding starts.
            let (work, job) = {
                let Ok(mut in_flight) = shared.lock_in_flight() else {
                    return;
                };
                let job = Job::new();
                let mut embed = Vec::new();
                for document in missing {
                    if let std::collections::hash_map::Entry::Vacant(slot) =
                        in_flight.entry(document.digest)
                    {
                        slot.insert(Arc::clone(&job));
                        embed.push(document.clone());
                    }
                }
                let work = JobWork {
                    owner: Some(owner.clone()),
                    embed,
                    persist: std::mem::take(&mut gathered.persist),
                    touch: std::mem::take(&mut gathered.touch),
                };
                (work, job)
            };
            tracing::debug!(
                target: LOG_TARGET,
                documents = documents.len(),
                stored = gathered.stored,
                loaded = gathered.loaded,
                to_embed = work.embed.len(),
                "background tool indexing pass"
            );
            if !work.is_empty() {
                run_job(shared, work, job).await;
            }
        });
        true
    }
}

impl Pending {
    pub(crate) fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Wait for every job, then fill the missing slots from their results.
    /// Returns how many slots it filled.
    pub(crate) async fn fill(
        &self,
        documents: &[ToolDocument],
        vectors: &mut VectorSlots,
    ) -> Result<usize, ToolRetrievalError> {
        for job in &self.jobs {
            job.wait().await?;
        }
        let mut filled = 0;
        for (document, slot) in documents.iter().zip(vectors.iter_mut()) {
            if slot.is_some() {
                continue;
            }
            if let Some(vector) = self
                .jobs
                .iter()
                .find_map(|job| job.result(&document.digest))
            {
                *slot = Some(vector);
                filled += 1;
            }
        }
        Ok(filled)
    }
}

/// One job: persist and mark used what is already in hand, then embed the
/// documents batch by batch, caching and persisting each batch as it lands.
async fn run_job(shared: Arc<Indexing>, work: JobWork, job: Arc<Job>) {
    let JobWork {
        owner,
        embed,
        persist,
        touch,
    } = work;
    let store = owner
        .as_ref()
        .and_then(|owner| shared.store.get().map(|store| (owner, store)));
    let today = today();
    if let Some((owner, store)) = store {
        if !persist.is_empty() {
            match store.save(owner, &shared.space, &persist, today).await {
                Ok(()) => shared.mark_persisted(owner, persist.iter().map(|(digest, _)| *digest)),
                Err(error) => tracing::debug!(
                    target: LOG_TARGET,
                    error_kind = error.kind_label(),
                    "persisting cached tool vectors failed"
                ),
            }
        }
        if !touch.is_empty()
            && let Err(error) = store.touch(owner, &shared.space, &touch, today).await
        {
            tracing::debug!(
                target: LOG_TARGET,
                error_kind = error.kind_label(),
                "marking stored tool vectors used failed"
            );
        }
    }

    let result = if embed.is_empty() {
        Ok(())
    } else {
        embed_batches(&shared, store, &embed, &job, today).await
    };
    if let Ok(mut in_flight) = shared.in_flight.lock() {
        for document in &embed {
            if in_flight
                .get(&document.digest)
                .is_some_and(|registered| Arc::ptr_eq(registered, &job))
            {
                in_flight.remove(&document.digest);
            }
        }
    }
    job.done.send_replace(Some(result));
}

async fn embed_batches(
    shared: &Indexing,
    store: Option<(&ToolCorpusOwner, &Arc<dyn VectorStore>)>,
    embed: &[ToolDocument],
    job: &Job,
    today: u32,
) -> Result<(), ToolRetrievalError> {
    let _permit = shared
        .permits
        .acquire()
        .await
        .map_err(|_closed| unavailable("the dense tool indexing queue is closed"))?;
    let mut dimension = shared.embedder.dimension();
    let mut embedded = 0;
    for batch in embed.chunks(INDEX_BATCH_SIZE) {
        let texts: Vec<String> = batch.iter().map(|document| document.text.clone()).collect();
        let vectors = shared
            .embedder
            .embed(&texts)
            .await
            .map_err(map_embedding_error)?;
        if vectors.len() != texts.len() {
            return Err(invalid_output(
                "the embedding endpoint returned a different number of vectors than documents",
            ));
        }
        // Validate the whole batch before caching any of it, so a bad
        // response cannot poison later fits.
        for vector in &vectors {
            check_vector(vector, &mut dimension)?;
        }
        let batch: Vec<(DocumentDigest, Arc<[f32]>)> = batch
            .iter()
            .zip(vectors)
            .map(|(document, vector)| (document.digest, Arc::from(vector)))
            .collect();
        {
            let mut cache = shared.lock_cache()?;
            let generation = cache.current_generation();
            for (digest, vector) in &batch {
                cache.insert(*digest, Arc::clone(vector), generation);
            }
            cache.evict_to_capacity(generation);
        }
        if let Ok(mut results) = job.results.lock() {
            results.extend(
                batch
                    .iter()
                    .map(|(digest, vector)| (*digest, Arc::clone(vector))),
            );
        }
        if let Some((owner, store)) = store {
            match store.save(owner, &shared.space, &batch, today).await {
                Ok(()) => shared.mark_persisted(owner, batch.iter().map(|(digest, _)| *digest)),
                Err(error) => tracing::debug!(
                    target: LOG_TARGET,
                    error_kind = error.kind_label(),
                    "persisting embedded tool vectors failed; they stay cached in memory"
                ),
            }
        }
        embedded += batch.len();
        tracing::debug!(
            target: LOG_TARGET,
            embedded,
            remaining = embed.len() - embedded,
            "tool indexing batch stored"
        );
    }
    Ok(())
}

/// Days since the Unix epoch: the manifest's last-used stamp.
fn today() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u32::try_from(elapsed.as_secs() / 86_400).unwrap_or(u32::MAX))
        .unwrap_or(0)
}

fn unavailable(reason: &str) -> ToolRetrievalError {
    ToolRetrievalError::Unavailable {
        reason: summary(reason),
    }
}
