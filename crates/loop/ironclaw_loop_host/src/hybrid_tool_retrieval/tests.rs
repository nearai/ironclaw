use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};

use ironclaw_host_api::{
    capability::CapabilityDescriptionTrust,
    ids::{CapabilityId, ProviderToolName},
};
use ironclaw_loop_contracts::LoopSafeSummary;
use serde_json::json;

use super::*;

const SHORT_TIMEOUT: Duration = Duration::from_millis(20);
const HANG: Duration = Duration::from_secs(30);

fn definition(capability_id: &str, description: &str) -> ProviderToolDefinition {
    ProviderToolDefinition {
        capability_id: CapabilityId::new(capability_id).expect("valid capability id"),
        name: ProviderToolName::new(ProviderToolName::encode_capability_str(capability_id))
            .expect("valid provider tool name"),
        description: description.to_string(),
        description_trust: CapabilityDescriptionTrust::Untrusted,
        parameters: json!({"type": "object", "properties": {}}),
    }
}

fn corpus() -> Vec<ProviderToolDefinition> {
    vec![
        definition("github.create_issue", "Open a new issue in a repository."),
        definition("slack.send_message", "Post a message to a channel."),
        definition("calendar.create_event", "Schedule an event on a calendar."),
        definition("files.read_file", "Read the contents of a file."),
        definition("files.list_directory", "List the entries of a directory."),
    ]
}

/// What the fake dense ranker does.
#[derive(Debug, Clone)]
enum DenseBehavior {
    /// Fit succeeds; each search answers from this query -> ranking table
    /// (an unknown query gets an empty ranking).
    Rank(BTreeMap<String, Vec<String>>),
    /// Fit succeeds; every search fails.
    SearchFails,
    /// Fit succeeds; every search outlives any test timeout.
    SearchHangs,
    /// Fit fails.
    FitFails,
    /// Fit outlives any test timeout.
    FitHangs,
}

#[derive(Debug)]
struct FakeDense {
    behavior: DenseBehavior,
    searches: Arc<AtomicUsize>,
    batches: Arc<AtomicUsize>,
}

impl FakeDense {
    fn bound(behavior: DenseBehavior) -> (Arc<dyn ToolRetrievalProvider>, Arc<AtomicUsize>) {
        let (provider, searches, _batches) = Self::bound_counting_batches(behavior);
        (provider, searches)
    }

    /// Also counts `search_many` calls, each of which searches every query.
    fn bound_counting_batches(
        behavior: DenseBehavior,
    ) -> (
        Arc<dyn ToolRetrievalProvider>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
    ) {
        let searches = Arc::new(AtomicUsize::new(0));
        let batches = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(Self {
            behavior,
            searches: Arc::clone(&searches),
            batches: Arc::clone(&batches),
        });
        (provider, searches, batches)
    }
}

#[async_trait]
impl ToolRetrievalProvider for FakeDense {
    fn ranker_version(&self) -> &str {
        "fake-dense-v1"
    }

    async fn fit(
        &self,
        _definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        match &self.behavior {
            DenseBehavior::FitFails => Err(ToolRetrievalError::Unavailable {
                reason: LoopSafeSummary::new("fake dense fit failure").expect("valid summary"),
            }),
            DenseBehavior::FitHangs => {
                tokio::time::sleep(HANG).await;
                Err(ToolRetrievalError::Timeout { elapsed: HANG })
            }
            behavior => Ok(Arc::new(FakeDenseIndex {
                behavior: behavior.clone(),
                searches: Arc::clone(&self.searches),
                batches: Arc::clone(&self.batches),
            })),
        }
    }
}

#[derive(Debug)]
struct FakeDenseIndex {
    behavior: DenseBehavior,
    searches: Arc<AtomicUsize>,
    batches: Arc<AtomicUsize>,
}

#[async_trait]
impl ToolRetrievalIndex for FakeDenseIndex {
    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<ToolSearchOutcome, ToolRetrievalError> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        match &self.behavior {
            DenseBehavior::Rank(table) => {
                let ranked: Vec<RankedTool> = table
                    .get(query)
                    .into_iter()
                    .flatten()
                    .take(limit)
                    .enumerate()
                    .map(|(index, name)| RankedTool::new(name.clone(), 1.0 / (index as f32 + 1.0)))
                    .collect();
                Ok(ToolSearchOutcome {
                    query_class: if ranked.is_empty() {
                        ToolSearchQueryClass::NoMatch
                    } else {
                        ToolSearchQueryClass::Lexical
                    },
                    ranked,
                })
            }
            DenseBehavior::SearchFails => Err(ToolRetrievalError::InvalidOutput {
                reason: LoopSafeSummary::new("fake dense search failure").expect("valid summary"),
            }),
            DenseBehavior::SearchHangs => {
                tokio::time::sleep(HANG).await;
                Err(ToolRetrievalError::Timeout { elapsed: HANG })
            }
            DenseBehavior::FitFails | DenseBehavior::FitHangs => {
                unreachable!("a failed fit never yields an index")
            }
        }
    }

    async fn search_many(
        &self,
        queries: &[&str],
        limit: usize,
    ) -> Result<Vec<ToolSearchOutcome>, ToolRetrievalError> {
        self.batches.fetch_add(1, Ordering::SeqCst);
        let mut outcomes = Vec::new();
        for query in queries {
            outcomes.push(self.search(query, limit).await?);
        }
        Ok(outcomes)
    }
}

fn table(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(query, names)| {
            (
                (*query).to_string(),
                names.iter().map(|name| (*name).to_string()).collect(),
            )
        })
        .collect()
}

fn native_outcome(query: &str, limit: usize) -> ToolSearchOutcome {
    AuthorizedToolSearchIndex::new(corpus().iter()).search(query, limit)
}

async fn hybrid_search(
    hybrid: &HybridToolRetrieval,
    query: &str,
    limit: usize,
) -> ToolSearchOutcome {
    hybrid
        .fit(&corpus())
        .await
        .expect("hybrid fit never fails")
        .search(query, limit)
        .await
        .expect("hybrid search never fails")
}

#[tokio::test]
async fn exact_name_and_paraphrase_queries_both_rank_the_right_tool() {
    // The paraphrase shares no word with the Slack tool, so BM25F alone
    // cannot find it; the dense side can.
    let paraphrase = "ping coworkers over chat";
    assert_eq!(
        native_outcome(paraphrase, 5).query_class,
        ToolSearchQueryClass::NoMatch
    );
    let (dense, searches) = FakeDense::bound(DenseBehavior::Rank(table(&[
        (
            paraphrase,
            &["slack__send_message", "calendar__create_event"],
        ),
        (
            "slack__send_message",
            &["calendar__create_event", "slack__send_message"],
        ),
    ])));
    let hybrid = HybridToolRetrieval::new(Some(dense));
    let index = hybrid.fit(&corpus()).await.expect("fit");

    let exact = index
        .search("slack__send_message", 5)
        .await
        .expect("search");
    assert_eq!(exact.query_class, ToolSearchQueryClass::ExactIdentifier);
    assert_eq!(exact.names()[0], "slack__send_message");
    assert_eq!(
        searches.load(Ordering::SeqCst),
        0,
        "an exact identifier is answered without consulting the dense side"
    );

    let fused = index.search(paraphrase, 5).await.expect("search");
    assert_eq!(fused.query_class, ToolSearchQueryClass::Lexical);
    assert_eq!(fused.names()[0], "slack__send_message");
    assert_eq!(searches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fusion_rewards_agreement_between_the_two_rankers() {
    // BM25F prefers `files__read_file` for "read file contents", the dense
    // side prefers `files__list_directory`; a tool both rank highly wins.
    let query = "read file contents";
    let lexical = native_outcome(query, 5);
    assert_eq!(lexical.names()[0], "files__read_file");
    let (dense, _searches) = FakeDense::bound(DenseBehavior::Rank(table(&[(
        query,
        &[
            "files__list_directory",
            "files__read_file",
            "not__in_corpus",
        ],
    )])));
    let outcome = hybrid_search(&HybridToolRetrieval::new(Some(dense)), query, 5).await;

    assert_eq!(
        outcome.names(),
        vec!["files__read_file", "files__list_directory"],
        "dense names outside the fitted corpus are dropped before fusion"
    );
    let expected_top = 1.0 / 61.0 + 1.0 / 62.0;
    assert!((f64::from(outcome.ranked[0].score) - expected_top).abs() < 1e-6);
    assert!(
        outcome
            .ranked
            .iter()
            .all(|tool| tool.score.is_finite() && tool.score > 0.0)
    );
}

#[tokio::test]
async fn a_dense_search_timeout_gives_the_lexical_result() {
    let (dense, searches) = FakeDense::bound(DenseBehavior::SearchHangs);
    let hybrid = HybridToolRetrieval::new(Some(dense)).with_dense_search_timeout(SHORT_TIMEOUT);
    let started = Instant::now();
    for (query, limit) in [("create issue", 5), ("read file", 1), ("zzz", 5)] {
        assert_eq!(
            hybrid_search(&hybrid, query, limit).await,
            native_outcome(query, limit),
            "{query:?}"
        );
    }
    assert_eq!(searches.load(Ordering::SeqCst), 3);
    assert!(
        started.elapsed() < HANG,
        "the timeout must cut the search off"
    );
}

#[tokio::test]
async fn a_dense_search_error_gives_the_lexical_result() {
    let (dense, searches) = FakeDense::bound(DenseBehavior::SearchFails);
    let hybrid = HybridToolRetrieval::new(Some(dense));
    for (query, limit) in [("create issue", 5), ("send message", 2), ("zzz", 5)] {
        assert_eq!(
            hybrid_search(&hybrid, query, limit).await,
            native_outcome(query, limit),
            "{query:?}"
        );
    }
    assert_eq!(searches.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_failed_or_slow_dense_fit_and_an_absent_dense_side_give_the_lexical_result() {
    let (failing, failing_searches) = FakeDense::bound(DenseBehavior::FitFails);
    let (hanging, hanging_searches) = FakeDense::bound(DenseBehavior::FitHangs);
    let hybrids = [
        HybridToolRetrieval::new(Some(failing)),
        HybridToolRetrieval::new(Some(hanging)).with_dense_fit_timeout(SHORT_TIMEOUT),
        HybridToolRetrieval::new(None),
    ];
    for hybrid in &hybrids {
        for query in ["create issue", "schedule event", "calendar__create_event"] {
            assert_eq!(
                hybrid_search(hybrid, query, 5).await,
                native_outcome(query, 5),
                "{query:?} via {}",
                hybrid.ranker_version()
            );
        }
    }
    assert_eq!(failing_searches.load(Ordering::SeqCst), 0);
    assert_eq!(hanging_searches.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn limit_zero_and_empty_queries_return_no_match() {
    let (dense, searches) = FakeDense::bound(DenseBehavior::Rank(BTreeMap::new()));
    let hybrid = HybridToolRetrieval::new(Some(dense));
    assert_eq!(
        hybrid_search(&hybrid, "create issue", 0).await,
        ToolSearchOutcome::no_match()
    );
    assert_eq!(
        hybrid_search(&hybrid, "   ", 5).await,
        ToolSearchOutcome::no_match()
    );
    assert_eq!(
        searches.load(Ordering::SeqCst),
        1,
        "limit zero never searches"
    );
}

#[test]
fn ranker_version_differs_from_native_and_names_both_inputs() {
    let native = NativeBm25fToolRetrieval.ranker_version().to_string();
    let (dense, _searches) = FakeDense::bound(DenseBehavior::SearchFails);
    let with_dense = HybridToolRetrieval::new(Some(dense));
    let without_dense = HybridToolRetrieval::new(None);

    assert_eq!(
        with_dense.ranker_version(),
        format!("hybrid-rrf-v1({native},fake-dense-v1)")
    );
    assert_eq!(
        without_dense.ranker_version(),
        format!("hybrid-rrf-v1({native},none)")
    );
    let corpus = corpus();
    let fingerprints: BTreeSet<u64> = [
        native.as_str(),
        "fake-dense-v1",
        with_dense.ranker_version(),
        without_dense.ranker_version(),
    ]
    .into_iter()
    .map(|version| crate::tool_search::definitions_fingerprint(version, &corpus))
    .collect();
    assert_eq!(
        fingerprints.len(),
        4,
        "every ranker keys its own fitted index"
    );
}

#[test]
fn fusion_is_deterministic_and_breaks_ties_by_lexical_rank_then_name() {
    let tools = |names: &[&str]| -> Vec<RankedTool> {
        names
            .iter()
            .map(|name| RankedTool::new(*name, 1.0))
            .collect()
    };
    // `b` and `c` swap ranks across the lists, so they tie on fused score
    // and BM25F rank decides; `w` is dense-only and comes last.
    let lexical = tools(&["b", "c"]);
    let dense = tools(&["c", "b", "w"]);
    let fused = fuse(&lexical, &dense, 10);
    assert_eq!(
        fused
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "c", "w"]
    );
    assert_eq!(fused[0].score, fused[1].score);
    assert_eq!(
        fuse(&lexical, &dense, 10),
        fused,
        "fusion is a pure function"
    );

    let both = fuse(&tools(&["a"]), &tools(&["q", "a"]), 1);
    assert_eq!(both.len(), 1, "fusion honours the limit");
    assert_eq!(both[0].name, "a");
}

#[tokio::test]
async fn several_queries_reach_the_dense_side_in_one_batch_and_fuse_separately() {
    let paraphrase = "ping coworkers over chat";
    let (dense, searches, batches) =
        FakeDense::bound_counting_batches(DenseBehavior::Rank(table(&[(
            paraphrase,
            &["slack__send_message", "calendar__create_event"],
        )])));
    let index = HybridToolRetrieval::new(Some(dense))
        .fit(&corpus())
        .await
        .expect("fit");
    let queries = [paraphrase, "slack__send_message", "read a file"];
    let outcomes = index.search_many(&queries, 5).await.expect("searched");

    assert_eq!(batches.load(Ordering::SeqCst), 1, "one dense batch");
    assert_eq!(
        searches.load(Ordering::SeqCst),
        2,
        "the exact identifier is answered lexically"
    );
    for (outcome, query) in outcomes.iter().zip(queries) {
        assert_eq!(outcome, &index.search(query, 5).await.expect("search"));
    }
    assert_eq!(outcomes[0].ranked[0].name, "slack__send_message");
    assert_eq!(outcomes[1], native_outcome("slack__send_message", 5));

    // A failed batch falls back to BM25F for every query.
    let (dense, _, _) = FakeDense::bound_counting_batches(DenseBehavior::SearchFails);
    let index = HybridToolRetrieval::new(Some(dense))
        .fit(&corpus())
        .await
        .expect("fit");
    let outcomes = index.search_many(&queries, 5).await.expect("searched");
    for (outcome, query) in outcomes.iter().zip(queries) {
        assert_eq!(outcome, &native_outcome(query, 5));
    }
}

/// A dense side that records the owners it is fitted and indexed for, and
/// reports a fixed fit.
#[derive(Debug, Default)]
struct OwnerRecordingDense {
    fitted_for: std::sync::Mutex<Vec<Option<ToolCorpusOwner>>>,
    indexed_for: std::sync::Mutex<Vec<(ToolCorpusOwner, usize)>>,
}

const RECORDED_REPORT: ToolIndexFitReport = ToolIndexFitReport {
    stored: 3,
    loaded: 2,
    embedded: 2,
    missing: 0,
    dense_fallback: None,
};

#[derive(Debug)]
struct ReportingIndex;

#[async_trait]
impl ToolRetrievalIndex for ReportingIndex {
    async fn search(
        &self,
        _query: &str,
        _limit: usize,
    ) -> Result<ToolSearchOutcome, ToolRetrievalError> {
        Ok(ToolSearchOutcome::no_match())
    }

    fn fit_report(&self) -> Option<ToolIndexFitReport> {
        Some(RECORDED_REPORT)
    }
}

#[async_trait]
impl ToolRetrievalProvider for OwnerRecordingDense {
    fn ranker_version(&self) -> &str {
        "owner-recording-dense-v1"
    }

    async fn fit(
        &self,
        _definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        self.fitted_for.lock().expect("lock").push(None);
        Ok(Arc::new(ReportingIndex))
    }

    async fn fit_for_owner(
        &self,
        owner: &ToolCorpusOwner,
        _definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        self.fitted_for
            .lock()
            .expect("lock")
            .push(Some(owner.clone()));
        Ok(Arc::new(ReportingIndex))
    }

    fn index_in_background(&self, owner: &ToolCorpusOwner, definitions: &[ProviderToolDefinition]) {
        self.indexed_for
            .lock()
            .expect("lock")
            .push((owner.clone(), definitions.len()));
    }
}

fn corpus_owner() -> ToolCorpusOwner {
    ToolCorpusOwner::new(
        ironclaw_host_api::ids::TenantId::new("tenant").expect("tenant"),
        ironclaw_host_api::ids::UserId::new("alice").expect("user"),
    )
}

#[tokio::test]
async fn owner_scoped_fits_and_background_indexing_reach_the_dense_side() {
    let dense = Arc::new(OwnerRecordingDense::default());
    let hybrid =
        HybridToolRetrieval::new(Some(Arc::clone(&dense) as Arc<dyn ToolRetrievalProvider>));
    let owner = corpus_owner();

    let scoped = hybrid
        .fit_for_owner(&owner, &corpus())
        .await
        .expect("hybrid fits");
    hybrid.fit(&corpus()).await.expect("owner-blind fit");
    hybrid.index_in_background(&owner, &corpus());

    assert_eq!(
        *dense.fitted_for.lock().expect("lock"),
        vec![Some(owner.clone()), None],
        "the dense side is fitted for the same owner, or none"
    );
    assert_eq!(
        *dense.indexed_for.lock().expect("lock"),
        vec![(owner, corpus().len())]
    );
    assert_eq!(
        scoped.fit_report(),
        Some(RECORDED_REPORT),
        "a used dense side's fit report is the index's"
    );
}

#[tokio::test]
async fn the_fit_report_names_why_the_dense_side_is_absent() {
    let (failing, _) = FakeDense::bound(DenseBehavior::FitFails);
    let (hanging, _) = FakeDense::bound(DenseBehavior::FitHangs);
    let cases = [
        (HybridToolRetrieval::new(Some(failing)), "unavailable"),
        (
            HybridToolRetrieval::new(Some(hanging)).with_dense_fit_timeout(SHORT_TIMEOUT),
            "timeout",
        ),
        (HybridToolRetrieval::new(None), "absent"),
    ];
    for (hybrid, expected) in cases {
        let report = hybrid
            .fit(&corpus())
            .await
            .expect("hybrid always fits")
            .fit_report()
            .expect("hybrid reports its dense side");
        assert_eq!(report.dense_fallback, Some(expected));
        if expected != "absent" {
            assert_eq!(
                report.missing,
                corpus().len(),
                "no dense vector is in the index"
            );
        }
    }
}
