//! Durable tool vectors: the store's contract on every filesystem backend,
//! and the provider behaviors built on it (restart, model change, timeout,
//! owner isolation, background indexing, bounded jobs).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_filesystem::{
    InMemoryBackend, LibSqlRootFilesystem, PostgresRootFilesystem, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::capability::CapabilityDescriptionTrust;
use ironclaw_host_api::ids::{CapabilityId, TenantId, UserId};
use ironclaw_host_api::mount::{MountGrant, MountPermissions, MountView};
use ironclaw_host_api::path::{MountAlias, VirtualPath};
use ironclaw_llm::embeddings::{EmbeddingError, EmbeddingProvider};
use ironclaw_loop_contracts::{ProviderToolDefinition, ToolCorpusOwner, ToolRetrievalProvider};
use ironclaw_tool_retrieval::{
    DenseToolRetrievalProvider, EmbeddingSpace, FilesystemToolVectorStore, TOOL_VECTOR_MOUNT_ALIAS,
    ToolVectorStoreSlot,
};
use serde_json::json;
use tokio::sync::Semaphore;

const VOCABULARY: &[&str] = &[
    "email", "send", "message", "code", "search", "calendar", "event", "weather", "secret",
];

/// Bag-of-words embedder that records every call, can be held at a gate
/// (each call takes one permit), and tracks how many calls overlap.
struct CountingEmbedder {
    provider_id: String,
    model: String,
    calls: Mutex<Vec<Vec<String>>>,
    gate: Option<Arc<Semaphore>>,
    running: AtomicUsize,
    max_running: AtomicUsize,
}

impl CountingEmbedder {
    fn new(model: &str) -> Arc<Self> {
        Self::build(model, None)
    }

    fn gated(model: &str, gate: Arc<Semaphore>) -> Arc<Self> {
        Self::build(model, Some(gate))
    }

    fn build(model: &str, gate: Option<Arc<Semaphore>>) -> Arc<Self> {
        Arc::new(Self {
            provider_id: "stub_provider".to_string(),
            model: model.to_string(),
            calls: Mutex::new(Vec::new()),
            gate,
            running: AtomicUsize::new(0),
            max_running: AtomicUsize::new(0),
        })
    }

    /// Tool documents embedded so far (queries are not tool documents).
    fn document_embeds(&self) -> usize {
        self.calls
            .lock()
            .expect("lock")
            .iter()
            .flatten()
            .filter(|text| text.starts_with("tool: "))
            .count()
    }

    fn query_embeds(&self) -> usize {
        self.calls
            .lock()
            .expect("lock")
            .iter()
            .flatten()
            .filter(|text| !text.starts_with("tool: "))
            .count()
    }
}

#[async_trait]
impl EmbeddingProvider for CountingEmbedder {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn dimension(&self) -> Option<usize> {
        Some(VOCABULARY.len() + 1)
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_running.fetch_max(now, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.acquire().await.expect("gate open").forget();
        }
        self.calls.lock().expect("lock").push(texts.to_vec());
        self.running.fetch_sub(1, Ordering::SeqCst);
        Ok(texts
            .iter()
            .map(|text| {
                let lower = text.to_lowercase();
                let words: Vec<&str> = lower
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .collect();
                VOCABULARY
                    .iter()
                    .map(|term| words.iter().filter(|word| *word == term).count() as f32)
                    .chain(std::iter::once(0.01))
                    .collect()
            })
            .collect())
    }
}

fn tool(capability_id: &str, description: &str) -> ProviderToolDefinition {
    let mut definition = ProviderToolDefinition::from_parts(
        CapabilityId::new(capability_id).expect("capability id"),
        capability_id.replace('.', "__"),
        description,
        json!({"type": "object", "properties": {}}),
    )
    .expect("definition");
    definition.description_trust = CapabilityDescriptionTrust::VerifiedCatalog;
    definition
}

fn catalog() -> Vec<ProviderToolDefinition> {
    vec![
        tool("mail.send_message", "Send an email message"),
        tool("repo.search_code", "Search code"),
        tool("calendar.create_event", "Create a calendar event"),
        tool("weather.forecast", "Weather forecast"),
    ]
}

fn wide_catalog(size: usize, prefix: &str) -> Vec<ProviderToolDefinition> {
    (0..size)
        .map(|index| tool(&format!("{prefix}.tool_{index:03}"), "Send a message"))
        .collect()
}

fn owner(user: &str) -> ToolCorpusOwner {
    ToolCorpusOwner::new(
        TenantId::new("tenant-a").expect("tenant"),
        UserId::new(user).expect("user"),
    )
}

/// The production alias shape: `/tool-vectors` resolves per caller to
/// `/tenants/<tenant>/users/<user>/tool-vectors`.
fn scoped<F: RootFilesystem + 'static>(root: Arc<F>) -> Arc<ScopedFilesystem<F>> {
    Arc::new(ScopedFilesystem::new(root, |scope| {
        MountView::new(vec![MountGrant::new(
            MountAlias::new(TOOL_VECTOR_MOUNT_ALIAS)?,
            VirtualPath::new(format!(
                "/tenants/{}/users/{}/tool-vectors",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            ))?,
            MountPermissions::read_write_list_delete(),
        )])
    }))
}

fn store_over<F: RootFilesystem + 'static>(
    root: &Arc<F>,
    capacity: usize,
) -> FilesystemToolVectorStore<F> {
    FilesystemToolVectorStore::new(scoped(Arc::clone(root)), capacity)
}

fn provider_over<F: RootFilesystem + 'static>(
    embedder: &Arc<CountingEmbedder>,
    root: &Arc<F>,
) -> DenseToolRetrievalProvider {
    let slot = ToolVectorStoreSlot::new();
    assert!(slot.bind(store_over(root, 4_096)));
    DenseToolRetrievalProvider::with_vector_store(
        Arc::clone(embedder) as Arc<dyn EmbeddingProvider>,
        slot,
    )
}

async fn manifest_len<F: RootFilesystem + 'static>(
    root: &Arc<F>,
    owner: &ToolCorpusOwner,
) -> usize {
    store_over(root, 4_096)
        .manifest_entries(owner)
        .await
        .expect("manifest reads")
        .len()
}

/// Poll `condition` until it holds, for up to ten seconds.
async fn eventually<F, Fut>(what: &str, mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..1_000 {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for: {what}");
}

// ---------------------------------------------------------------------------
// Store contract, run on every backend.
// ---------------------------------------------------------------------------

fn vector(values: &[f32]) -> Arc<[f32]> {
    Arc::from(values.to_vec())
}

async fn store_contract<F: RootFilesystem + 'static>(root: Arc<F>) {
    let alice = owner("alice");
    let bob = owner("bob");
    let space = EmbeddingSpace::new("provider", "model-a", None);
    let other_space = EmbeddingSpace::new("provider", "model-b", None);
    let store = store_over(&root, 3);

    // Round trip, in order, with `None` for what was never stored.
    store
        .save_vectors(
            &alice,
            &space,
            &[
                ([1; 32], vector(&[1.0, 0.0])),
                ([2; 32], vector(&[0.0, 1.0])),
            ],
            10,
        )
        .await
        .expect("save");
    let loaded = store
        .load_vectors(&alice, &space, &[[2; 32], [9; 32], [1; 32]])
        .await
        .expect("load");
    assert_eq!(loaded[0].as_deref(), Some(&[0.0_f32, 1.0][..]));
    assert!(loaded[1].is_none());
    assert_eq!(loaded[2].as_deref(), Some(&[1.0_f32, 0.0][..]));

    // Another owner, or another embedding space, reads nothing.
    let for_bob = store
        .load_vectors(&bob, &space, &[[1; 32], [2; 32]])
        .await
        .expect("load for bob");
    assert!(for_bob.iter().all(Option::is_none), "owners are isolated");
    let other_model = store
        .load_vectors(&alice, &other_space, &[[1; 32], [2; 32]])
        .await
        .expect("load in another space");
    assert!(
        other_model.iter().all(Option::is_none),
        "a model change never reads the old vectors"
    );

    // Marking [1] used later makes [2] the least recently used.
    store
        .touch_vectors(&alice, &space, &[[1; 32]], 11)
        .await
        .expect("touch");
    // Two more vectors push alice to four, over the capacity of three.
    store
        .save_vectors(
            &alice,
            &space,
            &[
                ([3; 32], vector(&[1.0, 1.0])),
                ([4; 32], vector(&[2.0, 1.0])),
            ],
            12,
        )
        .await
        .expect("save over capacity");
    let after = store
        .load_vectors(&alice, &space, &[[1; 32], [2; 32], [3; 32], [4; 32]])
        .await
        .expect("load after eviction");
    assert!(after[0].is_some(), "the recently used vector survives");
    assert!(
        after[1].is_none(),
        "the least recently used vector is evicted"
    );
    assert!(
        after[2].is_some() && after[3].is_some(),
        "the new vectors are kept"
    );
    let entries = store.manifest_entries(&alice).await.expect("manifest");
    assert_eq!(
        entries.len(),
        3,
        "the store stays within capacity: {entries:?}"
    );

    // Bob's own writes are bounded separately and do not evict alice's.
    store
        .save_vectors(&bob, &space, &[([5; 32], vector(&[3.0, 1.0]))], 12)
        .await
        .expect("save for bob");
    assert_eq!(
        store
            .manifest_entries(&alice)
            .await
            .expect("manifest")
            .len(),
        3
    );
    assert_eq!(
        store.manifest_entries(&bob).await.expect("manifest").len(),
        1
    );
}

#[tokio::test]
async fn store_contract_in_memory() {
    store_contract(Arc::new(InMemoryBackend::new())).await;
}

#[tokio::test]
async fn store_contract_libsql_and_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("tool-vectors.db");
    let open = |path: std::path::PathBuf| async move {
        let db = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("libsql"),
        );
        let filesystem = LibSqlRootFilesystem::new(db).expect("filesystem");
        filesystem.run_migrations().await.expect("migrations");
        Arc::new(filesystem)
    };
    let root = open(path.clone()).await;
    store_contract(Arc::clone(&root)).await;
    drop(root);

    // A fresh connection (a restart) reads what the first one wrote.
    let reopened = open(path).await;
    let space = EmbeddingSpace::new("provider", "model-a", None);
    let loaded = store_over(&reopened, 3)
        .load_vectors(&owner("alice"), &space, &[[1; 32], [4; 32]])
        .await
        .expect("load after reopen");
    assert!(
        loaded.iter().all(Option::is_some),
        "vectors survive a reopen"
    );
}

/// The PostgreSQL leg: `IRONCLAW_TOOL_VECTOR_STORE_POSTGRES_URL` when set,
/// else a throwaway container; skipped (with a notice) when neither is
/// available, like the filesystem crate's own PostgreSQL contract.
#[tokio::test]
async fn store_contract_postgres() {
    use testcontainers_modules::testcontainers::{ImageExt, runners::AsyncRunner};
    let mut _container = None;
    let url = match std::env::var("IRONCLAW_TOOL_VECTOR_STORE_POSTGRES_URL") {
        Ok(url) => url,
        Err(_) => {
            let image = testcontainers_modules::postgres::Postgres::default()
                .with_db_name("ironclaw_test")
                .with_user("postgres")
                .with_password("postgres")
                .with_tag("16-alpine");
            let container = match image.start().await {
                Ok(container) => container,
                Err(error) => {
                    eprintln!(
                        "skipping the PostgreSQL tool vector store contract: \
                         docker/testcontainers unavailable ({error})"
                    );
                    return;
                }
            };
            let host = container.get_host().await.expect("container host");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("container port");
            _container = Some(container);
            format!("postgres://postgres:postgres@{host}:{port}/ironclaw_test")
        }
    };
    let config = url.parse::<tokio_postgres::Config>().expect("postgres url");
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("postgres pool");
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool));
    filesystem.run_migrations().await.expect("migrations");
    store_contract(filesystem).await;
}

// ---------------------------------------------------------------------------
// Provider behavior over the store.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn after_a_restart_the_first_fit_embeds_only_the_query() {
    let root = Arc::new(InMemoryBackend::new());
    let alice = owner("alice");

    let first = CountingEmbedder::new("model-a");
    let provider = provider_over(&first, &root);
    provider
        .fit_for_owner(&alice, &catalog())
        .await
        .expect("first fit");
    assert_eq!(
        first.document_embeds(),
        4,
        "the first process embeds the catalog"
    );
    drop(provider);

    // A new process: empty memory cache, same durable store.
    let restarted = CountingEmbedder::new("model-a");
    let provider = provider_over(&restarted, &root);
    let index = provider
        .fit_for_owner(&alice, &catalog())
        .await
        .expect("fit after restart");
    let report = index.fit_report().expect("the dense index reports its fit");
    assert_eq!(
        (
            report.stored,
            report.loaded,
            report.embedded,
            report.missing
        ),
        (4, 4, 0, 0)
    );
    let outcome = index.search("send an email", 3).await.expect("search");
    assert_eq!(outcome.names().first(), Some(&"mail__send_message"));
    assert_eq!(
        restarted.document_embeds(),
        0,
        "no document is embedded again"
    );
    assert_eq!(restarted.query_embeds(), 1, "only the query is embedded");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_model_change_re_embeds_instead_of_reusing_old_vectors() {
    let root = Arc::new(InMemoryBackend::new());
    let alice = owner("alice");
    let old_model = CountingEmbedder::new("model-a");
    provider_over(&old_model, &root)
        .fit_for_owner(&alice, &catalog())
        .await
        .expect("fit under the old model");

    let new_model = CountingEmbedder::new("model-b");
    let index = provider_over(&new_model, &root)
        .fit_for_owner(&alice, &catalog())
        .await
        .expect("fit under the new model");
    assert_eq!(
        new_model.document_embeds(),
        4,
        "every document is re-embedded"
    );
    assert_eq!(index.fit_report().expect("report").loaded, 0);
    // Both spaces are now stored; each keeps its own vectors.
    assert_eq!(manifest_len(&root, &alice).await, 8);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_timed_out_fit_keeps_its_finished_batches_for_the_next_fit() {
    let root = Arc::new(InMemoryBackend::new());
    let alice = owner("alice");
    let corpus = wide_catalog(40, "wide");
    // One permit: the first batch (32 documents) goes through, the second
    // (8) waits at the gate, like a slow CPU embedding server.
    let gate = Arc::new(Semaphore::new(1));
    let slow = CountingEmbedder::gated("model-a", Arc::clone(&gate));
    let provider = provider_over(&slow, &root);

    let timed_out = tokio::time::timeout(
        Duration::from_millis(200),
        provider.fit_for_owner(&alice, &corpus),
    )
    .await;
    assert!(timed_out.is_err(), "the caller gives up on the slow fit");
    eventually("the finished batch to be stored", || async {
        manifest_len(&root, &alice).await == 32
    })
    .await;

    // Another process over the same store embeds only what is left.
    let other = CountingEmbedder::new("model-a");
    provider_over(&other, &root)
        .fit_for_owner(&alice, &corpus)
        .await
        .expect("fit in another process");
    assert_eq!(
        other.document_embeds(),
        8,
        "the stored batch is not re-embedded"
    );

    // The abandoned job was never cancelled: let it finish, and the same
    // provider's next fit embeds nothing at all.
    gate.add_permits(1);
    eventually("the abandoned job to finish", || async {
        slow.document_embeds() == 40
    })
    .await;
    let index = provider
        .fit_for_owner(&alice, &corpus)
        .await
        .expect("next fit");
    assert_eq!(slow.document_embeds(), 40, "nothing is embedded twice");
    assert_eq!(index.fit_report().expect("report").embedded, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_users_stored_documents_are_never_read_for_another_user() {
    let root = Arc::new(InMemoryBackend::new());
    let alice = owner("alice");
    let bob = owner("bob");
    let mut alice_catalog = catalog();
    alice_catalog.push(tool(
        "private.secret_lookup",
        "Look up a secret in a private server",
    ));

    let embedder = CountingEmbedder::new("model-a");
    let provider = provider_over(&embedder, &root);
    provider
        .fit_for_owner(&alice, &alice_catalog)
        .await
        .expect("alice fits");
    provider
        .fit_for_owner(&bob, &catalog())
        .await
        .expect("bob fits");
    assert_eq!(manifest_len(&root, &alice).await, 5);
    // Bob's vectors were all in the memory cache already (the same text is
    // the same vector); they reach bob's own store in the background.
    eventually("bob's corpus to be persisted for bob", || async {
        manifest_len(&root, &bob).await == 4
    })
    .await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        manifest_len(&root, &bob).await,
        4,
        "bob's store holds only bob's corpus, never alice's private tool"
    );
    assert_eq!(
        embedder.document_embeds(),
        5,
        "bob's documents were not embedded twice"
    );
    drop(provider);

    // After a restart, bob's corpus gains a document alice stored. It is
    // embedded for bob, not read back from alice's store.
    let restarted = CountingEmbedder::new("model-a");
    let index = provider_over(&restarted, &root)
        .fit_for_owner(&bob, &alice_catalog)
        .await
        .expect("bob fits a wider corpus");
    assert_eq!(
        restarted.document_embeds(),
        1,
        "only the new document is embedded"
    );
    let report = index.fit_report().expect("report");
    assert_eq!((report.loaded, report.embedded), (4, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_indexing_leaves_nothing_for_the_first_fit() {
    let root = Arc::new(InMemoryBackend::new());
    let alice = owner("alice");
    let embedder = CountingEmbedder::new("model-a");
    let provider = provider_over(&embedder, &root);

    provider.index_in_background(&alice, &catalog());
    eventually("the background pass to store the catalog", || async {
        manifest_len(&root, &alice).await == 4
    })
    .await;
    let before = embedder.document_embeds();
    let index = provider
        .fit_for_owner(&alice, &catalog())
        .await
        .expect("fit");
    assert_eq!(embedder.document_embeds(), before, "the fit embeds nothing");
    assert_eq!(index.fit_report().expect("report").embedded, 0);

    // A new process finds the background work in the store.
    let restarted = CountingEmbedder::new("model-a");
    provider_over(&restarted, &root)
        .fit_for_owner(&alice, &catalog())
        .await
        .expect("fit after restart");
    assert_eq!(restarted.document_embeds(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_catalog_changes_runs_a_bounded_number_of_jobs() {
    let root = Arc::new(InMemoryBackend::new());
    let gate = Arc::new(Semaphore::new(0));
    let embedder = CountingEmbedder::gated("model-a", Arc::clone(&gate));
    let provider = provider_over(&embedder, &root);

    for user in 0..40 {
        let catalog = wide_catalog(3, &format!("user{user}"));
        provider.index_in_background(&owner(&format!("user-{user}")), &catalog);
    }
    eventually("jobs to reach the gate", || async {
        embedder.running.load(Ordering::SeqCst) >= 2
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        embedder.max_running.load(Ordering::SeqCst),
        2,
        "at most two jobs embed at once"
    );
    gate.add_permits(1_000);
    eventually("the accepted jobs to finish", || async {
        embedder.running.load(Ordering::SeqCst) == 0 && embedder.document_embeds() > 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        embedder.document_embeds() <= 16 * 3,
        "requests past the queue bound are dropped, not queued without limit: {}",
        embedder.document_embeds()
    );
    assert_eq!(embedder.max_running.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_owner_blind_fit_never_touches_the_store() {
    let root = Arc::new(InMemoryBackend::new());
    let embedder = CountingEmbedder::new("model-a");
    let provider = provider_over(&embedder, &root);
    provider.fit(&catalog()).await.expect("fit");
    assert_eq!(manifest_len(&root, &owner("alice")).await, 0);
    assert_eq!(embedder.document_embeds(), 4);
}

#[test]
fn background_indexing_outside_a_runtime_is_dropped_not_a_panic() {
    let root = Arc::new(InMemoryBackend::new());
    let embedder = CountingEmbedder::new("model-a");
    let provider = provider_over(&embedder, &root);
    provider.index_in_background(&owner("alice"), &catalog());
    assert_eq!(embedder.document_embeds(), 0);
}
