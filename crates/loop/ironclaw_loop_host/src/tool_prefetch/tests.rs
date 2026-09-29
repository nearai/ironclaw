use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::{
    CapabilityResultWrite, CapabilityWriteResult, LoopCapabilityResultWriter,
    ToolDisclosureCapabilityDecorator,
};
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, ProjectId, ProviderToolName, TenantId, ThreadId},
    resolution::{Resolution, ResolutionBatch},
    turn::AcceptedMessageRef,
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, CapabilityCallCandidate, CapabilityDescriptorView, CapabilityInputRef,
    CapabilityProgress, CapabilitySurfaceVersion, InMemoryRunProfileResolver, LoopCapabilityPort,
    LoopRequest, LoopRequestBatch, ProviderToolCall, ProviderToolCallCapabilityIds, RankedTool,
    RegisterProviderToolCallRequest, RunProfileResolutionRequest, RunProfileResolver,
    ToolRetrievalError, ToolRetrievalIndex, ToolSearchOutcome, ToolSearchQueryClass,
    VisibleCapabilityRequest, VisibleCapabilitySurface, resolution,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
};
use ironclaw_turns::{LoopResultRef, TurnId, TurnRunId, TurnScope};
use serde_json::json;

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

fn definition(capability_id: &str, description: &str) -> ProviderToolDefinition {
    ProviderToolDefinition {
        capability_id: CapabilityId::new(capability_id).expect("capability id"),
        name: ProviderToolName::new(ProviderToolName::encode_capability_str(capability_id))
            .expect("provider tool name"),
        description: description.to_string(),
        description_trust: Default::default(),
        parameters: json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "additionalProperties": false
        }),
    }
}

fn catalog_definitions() -> Vec<ProviderToolDefinition> {
    vec![
        definition(
            "builtin.result_read",
            "Read the rest of a large tool result.",
        ),
        definition("builtin.read_file", "Read a file from the workspace."),
        definition("builtin.shell", "Run a shell command in the workspace."),
        definition(
            "builtin.outbound_deliver",
            "Deliver a message to another surface.",
        ),
        definition(
            "builtin.outbound_delivery_targets_list",
            "List delivery targets.",
        ),
        definition(
            "github.create_issue",
            "Create a GitHub issue in a repository.",
        ),
        definition("github.list_issues", "List GitHub issues in a repository."),
        definition("github.get_repo", "Get a GitHub repository."),
        definition("gmail.send_message", "Send an email message through Gmail."),
        definition("calendar.list_events", "List calendar events for a day."),
    ]
}

fn catalog_definitions_sorted() -> Vec<ProviderToolDefinition> {
    let mut definitions = catalog_definitions();
    definitions.sort_by(|left, right| left.name.cmp(&right.name));
    definitions
}

fn names(definitions: &[ProviderToolDefinition]) -> Vec<String> {
    definitions
        .iter()
        .map(|definition| definition.name.to_string())
        .collect()
}

/// Selection with explicit thresholds: the former cosine defaults, 0.35
/// absolute and 0.7 relative. These scenarios pin a narrow scripted
/// selection; the production defaults (no thresholds) are
/// [`ranked_config`].
fn config(ranking: ToolPrefetchRanking, max_tools: usize, always: &[&str]) -> ToolPrefetchConfig {
    ToolPrefetchConfig::new(
        ranking,
        max_tools,
        16_000,
        0.35,
        0.7,
        always.iter().map(|name| name.to_string()).collect(),
    )
    .expect("valid prefetch config")
}

/// Selection at the production defaults: no thresholds, a 32,000-token
/// budget, 16 messages of 2 KiB segments.
fn ranked_config(ranking: ToolPrefetchRanking, max_tools: usize) -> ToolPrefetchConfig {
    ToolPrefetchConfig::new(ranking, max_tools, 32_000, 0.0, 0.0, Vec::new())
        .expect("valid prefetch config")
}

fn thread_scope() -> ThreadScope {
    ThreadScope {
        tenant_id: TenantId::new("tenant-prefetch").expect("tenant"),
        agent_id: AgentId::new("agent-prefetch").expect("agent"),
        project_id: Some(ProjectId::new("project-prefetch").expect("project")),
        owner_user_id: None,
        mission_id: None,
    }
}

/// A conversation in an in-memory thread store: each `turn` accepts one user
/// message and returns the run context a runner would build for it.
struct Conversation {
    threads: Arc<InMemorySessionThreadService>,
    thread_id: ThreadId,
}

impl Conversation {
    async fn new(label: &str) -> Self {
        let threads = Arc::new(InMemorySessionThreadService::default());
        let thread_id = ThreadId::new(format!("thread-{label}")).expect("thread");
        threads
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "user".to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread");
        Self { threads, thread_id }
    }

    async fn turn(&self, text: &str) -> LoopRunContext {
        let accepted = self
            .threads
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope(),
                thread_id: self.thread_id.clone(),
                actor_id: "user".to_string(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text(text),
            })
            .await
            .expect("accepted message");
        self.run_context_for(TurnId::new())
            .await
            .with_accepted_message_ref(
                AcceptedMessageRef::new(format!("msg:{}", accepted.message_id))
                    .expect("message ref"),
            )
    }

    async fn run_context_for(&self, turn_id: TurnId) -> LoopRunContext {
        let scope = thread_scope();
        let turn_scope = TurnScope::new(
            scope.tenant_id,
            Some(scope.agent_id),
            scope.project_id,
            self.thread_id.clone(),
        );
        let profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("run profile");
        LoopRunContext::new(turn_scope, turn_id, TurnRunId::new(), profile)
    }

    async fn history(&self) -> Option<ToolSelectionHistory> {
        self.threads
            .read_tool_selection_history(&thread_scope(), &self.thread_id)
            .await
            .expect("history read")
    }

    fn prefetch(
        &self,
        config: ToolPrefetchConfig,
        ranker: Arc<dyn ToolRetrievalProvider>,
    ) -> ToolPrefetch {
        ToolPrefetch::new(
            config,
            ranker,
            Arc::clone(&self.threads) as Arc<dyn SessionThreadService>,
            thread_scope(),
        )
        .expect("prefetch binding")
    }
}

/// A ranker with a scripted cosine-scale ranking that counts its calls, so a
/// test can prove a later turn never ranks again. Every query gets the same
/// ranking; `searches` counts ranking calls (one `search`, or one
/// `search_many` batch), and `batches` records each batch's query count.
#[derive(Debug)]
struct CountingRanker {
    ranking: Vec<(&'static str, f32)>,
    fits: AtomicUsize,
    searches: Arc<AtomicUsize>,
    batches: Arc<Mutex<Vec<usize>>>,
}

impl CountingRanker {
    fn new(ranking: Vec<(&'static str, f32)>) -> Arc<Self> {
        Arc::new(Self {
            ranking,
            fits: AtomicUsize::new(0),
            searches: Arc::new(AtomicUsize::new(0)),
            batches: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn batches(&self) -> Vec<usize> {
        self.batches.lock().expect("batches").clone()
    }
}

#[async_trait]
impl ToolRetrievalProvider for CountingRanker {
    fn ranker_version(&self) -> &str {
        "dense-cosine-test"
    }

    async fn fit(
        &self,
        _definitions: &[ProviderToolDefinition],
    ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
        self.fits.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(CountingIndex {
            ranking: self
                .ranking
                .iter()
                .map(|(name, score)| RankedTool::new(*name, *score))
                .collect(),
            searches: Arc::clone(&self.searches),
            batches: Arc::clone(&self.batches),
        }))
    }
}

#[derive(Debug)]
struct CountingIndex {
    ranking: Vec<RankedTool>,
    searches: Arc<AtomicUsize>,
    batches: Arc<Mutex<Vec<usize>>>,
}

impl CountingIndex {
    fn outcome(&self, limit: usize) -> ToolSearchOutcome {
        ToolSearchOutcome {
            ranked: self.ranking.iter().take(limit).cloned().collect(),
            query_class: ToolSearchQueryClass::Lexical,
        }
    }
}

#[async_trait]
impl ToolRetrievalIndex for CountingIndex {
    async fn search(
        &self,
        _query: &str,
        limit: usize,
    ) -> Result<ToolSearchOutcome, ToolRetrievalError> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        Ok(self.outcome(limit))
    }

    async fn search_many(
        &self,
        queries: &[&str],
        limit: usize,
    ) -> Result<Vec<ToolSearchOutcome>, ToolRetrievalError> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        self.batches.lock().expect("batches").push(queries.len());
        Ok(queries.iter().map(|_| self.outcome(limit)).collect())
    }
}

/// An inner capability port over a mutable catalog: the surface version
/// follows the catalog, so a change shows up as a real surface refresh.
struct CatalogPort {
    definitions: Mutex<Vec<ProviderToolDefinition>>,
    invocations: Mutex<Vec<CapabilityId>>,
}

impl CatalogPort {
    fn new(definitions: Vec<ProviderToolDefinition>) -> Arc<Self> {
        Arc::new(Self {
            definitions: Mutex::new(definitions),
            invocations: Mutex::new(Vec::new()),
        })
    }

    fn definitions(&self) -> Vec<ProviderToolDefinition> {
        self.definitions.lock().expect("definitions lock").clone()
    }

    fn version(&self) -> CapabilitySurfaceVersion {
        CapabilitySurfaceVersion::new(format!("surface:{}", self.definitions().len()))
            .expect("surface version")
    }
}

#[async_trait]
impl LoopCapabilityPort for CatalogPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        Ok(self.definitions())
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        self.definitions()
            .into_iter()
            .find(|definition| definition.name == tool_call.name)
            .map(|definition| ProviderToolCallCapabilityIds::single(definition.capability_id))
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    ironclaw_loop_contracts::AgentLoopHostErrorKind::InvalidInvocation,
                    "unknown tool",
                )
            })
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.provider_tool_call_capability_ids(tool_call)
            .map(|_| ())
    }

    async fn register_provider_tool_call(
        &self,
        request: RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let capability_id = self
            .provider_tool_call_capability_ids(&request.tool_call)?
            .provider_capability_id;
        Ok(CapabilityCallCandidate {
            activity_id: request.activity_id.unwrap_or_default(),
            surface_version: self.version(),
            capability_id,
            input_ref: CapabilityInputRef::new(format!("input:{}", request.tool_call.name))
                .expect("input ref"),
            effective_capability_ids: Vec::new(),
            provider_replay: None,
        })
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            advertised_choice: Default::default(),
            version: self.version(),
            descriptors: self
                .definitions()
                .into_iter()
                .map(|definition| CapabilityDescriptorView {
                    capability_id: definition.capability_id,
                    provider: None,
                    runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
                    safe_name: definition.name.to_string(),
                    safe_description: definition.description,
                    description_trust: definition.description_trust,
                    parameters_schema: definition.parameters,
                })
                .collect(),
            callable_capability_ids: None,
        })
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        self.invocations
            .lock()
            .expect("invocations lock")
            .push(request.capability_id);
        Ok(resolution::completed(
            LoopResultRef::new("result:done").expect("result ref"),
            "done".to_string(),
            CapabilityProgress::MadeProgress,
            false,
            4,
            None,
            None,
        ))
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        let mut resolutions = Vec::new();
        for invocation in request.invocations {
            resolutions.push(self.invoke_capability(invocation).await?);
        }
        Ok(ResolutionBatch {
            resolutions,
            stopped_on_suspension: false,
        })
    }
}

struct DiscardingWriter;

#[async_trait]
impl LoopCapabilityResultWriter for DiscardingWriter {
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        let digest =
            ironclaw_host_api::approval::sha256_digest_token(write.input_ref.as_str().as_bytes())
                .replace(':', ".");
        Ok(CapabilityWriteResult::without_output_digest(
            LoopResultRef::new(format!("result:{digest}")).expect("result ref"),
            write.output.to_string().len() as u64,
        ))
    }
}

fn decorator(prefetch: Option<ToolPrefetch>) -> ToolDisclosureCapabilityDecorator {
    let decorator = ToolDisclosureCapabilityDecorator::new(
        Arc::new(DiscardingWriter),
        ToolDisclosureMode::Namespaces,
    );
    let mut decorator = decorator;
    decorator.prefetch = prefetch;
    decorator
}

/// Build one run's disclosure port and return the `tools` array it
/// advertises (as a runner does at host build, before the first model call).
async fn advertised_tools(
    decorator: &ToolDisclosureCapabilityDecorator,
    run_context: &LoopRunContext,
    inner: &Arc<CatalogPort>,
    policy: CapabilitySurfacePolicy,
) -> (Arc<dyn LoopCapabilityPort>, Vec<ProviderToolDefinition>) {
    let port = decorator.decorate_with_policy(
        run_context,
        Arc::clone(inner) as Arc<dyn LoopCapabilityPort>,
        Arc::new(policy),
    );
    port.visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("visible capabilities");
    let tools = port.tool_definitions().expect("tool definitions");
    (port, tools)
}

fn provider_call(name: &str, arguments: serde_json::Value) -> ProviderToolCall {
    ProviderToolCall {
        provider_id: "provider".to_string(),
        provider_model_id: "model".to_string(),
        turn_id: Some("provider-turn".to_string()),
        id: format!("call-{name}"),
        name: ProviderToolName::new(name).expect("provider tool name"),
        arguments,
        response_reasoning: None,
        reasoning: None,
        signature: None,
    }
}

fn request_for(candidate: CapabilityCallCandidate) -> LoopRequest {
    LoopRequest {
        activity_id: candidate.activity_id,
        surface_version: candidate.surface_version,
        capability_id: candidate.capability_id,
        input_ref: candidate.input_ref,
        approval_resume: None,
        auth_resume: None,
    }
}

fn scripted_ranking() -> Vec<(&'static str, f32)> {
    vec![
        ("github__create_issue", 0.82),
        ("github__list_issues", 0.71),
        ("github__get_repo", 0.66),
        ("builtin__read_file", 0.60),
        ("gmail__send_message", 0.30),
        ("calendar__list_events", 0.12),
    ]
}

// ---------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------

#[test]
fn config_refuses_a_floor_and_extras_larger_than_max_tools() {
    let error = ToolPrefetchConfig::new(
        ToolPrefetchRanking::Lexical,
        5,
        16_000,
        0.0,
        0.0,
        vec!["outbound_deliver".to_string(), "trigger_create".to_string()],
    )
    .expect_err("six floor tools do not fit in five");
    assert_eq!(
        error,
        ToolPrefetchConfigError::FloorExceedsMaxTools {
            floor: 6,
            mandatory: 4,
            max_tools: 5,
        }
    );
    assert!(error.to_string().contains("exceeds the maximum of 5"));
}

#[test]
fn config_counts_repeated_and_mandatory_extras_once() {
    let config = ToolPrefetchConfig::new(
        ToolPrefetchRanking::Lexical,
        5,
        16_000,
        0.0,
        0.0,
        vec![
            " outbound_deliver ".to_string(),
            "outbound_deliver".to_string(),
            "tool_search".to_string(),
            String::new(),
        ],
    )
    .expect("four mandatory plus one extra fit in five");
    assert_eq!(config.always, vec!["outbound_deliver", "tool_search"]);
}

#[test]
fn config_rejects_thresholds_outside_the_unit_interval() {
    for (min_similarity, min_relative) in [(1.5, 0.0), (0.35, -0.1), (f32::NAN, 0.0), (0.0, 1.5)] {
        assert!(matches!(
            ToolPrefetchConfig::new(
                ToolPrefetchRanking::Semantic,
                100,
                16_000,
                min_similarity,
                min_relative,
                Vec::new(),
            ),
            Err(ToolPrefetchConfigError::ThresholdOutOfRange { .. })
        ));
    }
}

#[test]
fn config_bounds_the_conversation_context() {
    let defaults = ranked_config(ToolPrefetchRanking::Lexical, 100);
    assert_eq!(defaults.context_messages(), 16);
    assert_eq!(defaults.segment_bytes(), 2_048);
    assert_eq!(defaults.min_similarity(), 0.0);
    assert_eq!(defaults.min_relative(), 0.0);
    let bounded = defaults
        .clone()
        .with_context(4, 512)
        .expect("valid context bounds");
    assert_eq!(
        (bounded.context_messages(), bounded.segment_bytes()),
        (4, 512)
    );
    for (messages, segment_bytes) in [
        (0, 2_048),
        (MAX_CONTEXT_MESSAGES + 1, 2_048),
        (16, MIN_CONTEXT_SEGMENT_BYTES - 1),
        (16, MAX_CONTEXT_SEGMENT_BYTES + 1),
    ] {
        assert!(
            defaults
                .clone()
                .with_context(messages, segment_bytes)
                .is_err(),
            "{messages} messages of {segment_bytes} bytes"
        );
    }
}

// ---------------------------------------------------------------------
// Selection plan (pure)
// ---------------------------------------------------------------------

fn candidates(definitions: &[ProviderToolDefinition]) -> Vec<(&ProviderToolDefinition, u32)> {
    definitions
        .iter()
        .map(|definition| (definition, 100))
        .collect()
}

fn ranked(ranking: &[(&str, f32)]) -> Vec<RankedTool> {
    ranking
        .iter()
        .map(|(name, score)| RankedTool::new(*name, *score))
        .collect()
}

fn selected_names(plan: &ToolSelectionPlan) -> Vec<&str> {
    plan.selected
        .iter()
        .map(|tool| tool.name.as_str())
        .collect()
}

#[test]
fn plan_keeps_the_floor_and_caps_the_array_at_max_tools_including_it() {
    let definitions = catalog_definitions();
    let candidates = candidates(&definitions);
    let config = config(ToolPrefetchRanking::Semantic, 6, &["outbound_deliver"]);
    let plan = plan_selection(SelectionInputs {
        config: &config,
        candidates: &candidates,
        bridge_tokens: 300,
        rankings: &[ranked(&scripted_ranking())],
        ranker_version: "dense-cosine-v1",
    });
    // 3 bridges + result_read + one extra leave room for one ranked tool.
    assert_eq!(plan.advertised.len(), 6, "{:?}", plan.advertised);
    assert_eq!(
        plan.floor,
        vec!["builtin__result_read", "builtin__outbound_deliver"]
    );
    assert_eq!(selected_names(&plan), vec!["github__create_issue"]);
    assert!(
        plan.skipped
            .iter()
            .any(|tool| tool.name == "github__list_issues" && tool.reason == SkipReason::MaxTools)
    );
    for bridge in [TOOL_SEARCH_NAME, TOOL_DESCRIBE_NAME, TOOL_CALL_NAME] {
        assert!(plan.advertised.iter().any(|name| name == bridge));
    }
}

#[test]
fn plan_token_budget_trims_when_it_is_reached_before_max_tools() {
    let definitions = catalog_definitions();
    let candidates = candidates(&definitions);
    // Bridges 300 + result_read 100 = 400; two ranked tools fit in 600.
    let config = ToolPrefetchConfig::new(
        ToolPrefetchRanking::Semantic,
        100,
        600,
        0.0,
        0.0,
        Vec::new(),
    )
    .expect("config");
    let plan = plan_selection(SelectionInputs {
        config: &config,
        candidates: &candidates,
        bridge_tokens: 300,
        rankings: &[ranked(&scripted_ranking())],
        ranker_version: "dense-cosine-v1",
    });
    assert_eq!(selected_names(&plan).len(), 2);
    assert_eq!(plan.est_schema_tokens, 600);
    assert_eq!(
        plan.skipped.first().map(|tool| tool.reason),
        Some(SkipReason::TokenBudget)
    );
}

#[test]
fn plan_ignores_unknown_and_unauthorized_extras() {
    // `gmail.send_message` is not a candidate: it is unauthorized for this
    // run, so naming it as an extra must not advertise it.
    let definitions: Vec<_> = catalog_definitions()
        .into_iter()
        .filter(|definition| definition.capability_id.as_str() != "gmail.send_message")
        .collect();
    let candidates = candidates(&definitions);
    let config = config(
        ToolPrefetchRanking::Lexical,
        100,
        &[
            "gmail.send_message",
            "no_such_tool",
            "outbound_delivery_targets_list",
        ],
    );
    let plan = plan_selection(SelectionInputs {
        config: &config,
        candidates: &candidates,
        bridge_tokens: 300,
        rankings: &[],
        ranker_version: LEXICAL_SELECTION_RANKER_VERSION,
    });
    assert_eq!(
        plan.floor,
        vec![
            "builtin__result_read",
            "builtin__outbound_delivery_targets_list"
        ]
    );
    assert!(!plan.advertised.iter().any(|name| name.starts_with("gmail")));
    assert!(plan.selected.is_empty(), "an empty ranking selects nothing");
}

#[test]
fn plan_applies_the_absolute_threshold_only_to_cosine_scores() {
    let definitions = catalog_definitions();
    let candidates = candidates(&definitions);
    let config = ToolPrefetchConfig::new(
        ToolPrefetchRanking::Semantic,
        100,
        16_000,
        0.5,
        0.0,
        Vec::new(),
    )
    .expect("config");
    let cosine = plan_selection(SelectionInputs {
        config: &config,
        candidates: &candidates,
        bridge_tokens: 0,
        rankings: &[ranked(&scripted_ranking())],
        ranker_version: "dense-cosine-v1",
    });
    assert_eq!(
        selected_names(&cosine).len(),
        4,
        "cosine scores below 0.5 are dropped"
    );
    assert!(
        cosine
            .skipped
            .iter()
            .all(|tool| tool.reason == SkipReason::BelowMinSimilarity)
    );

    // The same numbers on a reciprocal-rank scale are not similarities.
    let fused = plan_selection(SelectionInputs {
        config: &config,
        candidates: &candidates,
        bridge_tokens: 0,
        rankings: &[ranked(&scripted_ranking())],
        ranker_version: "hybrid-rrf-v1(bounded-bm25f-v1,dense-cosine-v1)",
    });
    assert_eq!(selected_names(&fused).len(), 6);
}

#[test]
fn plan_fills_max_tools_by_rank_without_thresholds_by_default() {
    let definitions = catalog_definitions();
    let candidates = candidates(&definitions);
    let settings = ranked_config(ToolPrefetchRanking::Semantic, 100);
    let plan = plan_selection(SelectionInputs {
        config: &settings,
        candidates: &candidates,
        bridge_tokens: 300,
        rankings: &[ranked(&scripted_ranking())],
        ranker_version: "dense-cosine-v1",
    });
    // Gmail (0.30) and calendar (0.12) are below the former cosine defaults
    // (0.35 absolute, 0.7 relative) and are ranked in now.
    assert_eq!(
        selected_names(&plan),
        vec![
            "github__create_issue",
            "github__list_issues",
            "github__get_repo",
            "builtin__read_file",
            "gmail__send_message",
            "calendar__list_events",
        ]
    );
    assert!(plan.skipped.is_empty());

    // The same ranking with the former defaults set explicitly still cuts.
    let thresholded = config(ToolPrefetchRanking::Semantic, 100, &[]);
    let plan = plan_selection(SelectionInputs {
        config: &thresholded,
        candidates: &candidates,
        bridge_tokens: 300,
        rankings: &[ranked(&scripted_ranking())],
        ranker_version: "dense-cosine-v1",
    });
    assert_eq!(selected_names(&plan).len(), 4);
    assert!(plan.skipped.iter().all(|tool| matches!(
        tool.reason,
        SkipReason::BelowMinSimilarity | SkipReason::BelowMinRelative
    )));

    // And a cap still binds: room for two ranked tools past the floor.
    let tight = ranked_config(ToolPrefetchRanking::Semantic, 6);
    let plan = plan_selection(SelectionInputs {
        config: &tight,
        candidates: &candidates,
        bridge_tokens: 300,
        rankings: &[ranked(&scripted_ranking())],
        ranker_version: "dense-cosine-v1",
    });
    assert_eq!(
        selected_names(&plan),
        vec!["github__create_issue", "github__list_issues"]
    );
    assert!(
        plan.skipped
            .iter()
            .all(|tool| tool.reason == SkipReason::MaxTools)
    );
}

/// Three segments: the newest about GitHub (three tools), the next about
/// mail and GitHub, the oldest about the calendar alone.
fn segment_rankings() -> Vec<Vec<RankedTool>> {
    vec![
        ranked(&[
            ("github__create_issue", 9.0),
            ("github__list_issues", 7.0),
            ("github__get_repo", 6.0),
        ]),
        ranked(&[
            ("gmail__send_message", 4.0),
            ("github__create_issue", 3.5),
            ("builtin__read_file", 1.0),
        ]),
        ranked(&[("calendar__list_events", 0.5)]),
    ]
}

#[test]
fn plan_merges_the_segment_rankings_round_robin_by_rank() {
    let definitions = catalog_definitions();
    let candidates = candidates(&definitions);
    let settings = ranked_config(ToolPrefetchRanking::Lexical, 100);
    let plan = || {
        plan_selection(SelectionInputs {
            config: &settings,
            candidates: &candidates,
            bridge_tokens: 300,
            rankings: &segment_rankings(),
            ranker_version: LEXICAL_SELECTION_RANKER_VERSION,
        })
    };
    let merged = plan();
    // Every segment's first tool, newest segment first; then every
    // segment's second (create_issue is taken already), then the thirds.
    assert_eq!(
        merged
            .selected
            .iter()
            .map(|tool| (tool.name.as_str(), tool.rank, tool.segment))
            .collect::<Vec<_>>(),
        vec![
            ("github__create_issue", 1, 0),
            ("gmail__send_message", 1, 1),
            ("calendar__list_events", 1, 2),
            ("github__list_issues", 2, 0),
            ("github__get_repo", 3, 0),
            ("builtin__read_file", 3, 1),
        ]
    );
    // A tool ranked by several segments keeps its best score.
    assert_eq!(merged.selected[0].score, 9.0);
    // The calendar tool scores far below the GitHub ones and is still in:
    // no threshold compares scores across segments.
    assert_eq!(plan(), merged, "the merge is deterministic");

    // The minority topic gets in however tight the cap: room for three.
    let tight = ranked_config(ToolPrefetchRanking::Lexical, 7);
    let plan = plan_selection(SelectionInputs {
        config: &tight,
        candidates: &candidates,
        bridge_tokens: 300,
        rankings: &segment_rankings(),
        ranker_version: LEXICAL_SELECTION_RANKER_VERSION,
    });
    assert_eq!(
        selected_names(&plan),
        vec![
            "github__create_issue",
            "gmail__send_message",
            "calendar__list_events"
        ]
    );
    assert_eq!(
        plan.skipped
            .iter()
            .map(|tool| (tool.name.as_str(), tool.reason))
            .collect::<Vec<_>>(),
        vec![
            ("github__list_issues", SkipReason::MaxTools),
            ("github__get_repo", SkipReason::MaxTools),
            ("builtin__read_file", SkipReason::MaxTools),
        ]
    );

    // The token budget trims from the bottom of the merged order: bridges
    // 300 + result_read 100 leave room for four tools of 100 in 800.
    let budget =
        ToolPrefetchConfig::new(ToolPrefetchRanking::Lexical, 100, 800, 0.0, 0.0, Vec::new())
            .expect("config");
    let plan = plan_selection(SelectionInputs {
        config: &budget,
        candidates: &candidates,
        bridge_tokens: 300,
        rankings: &segment_rankings(),
        ranker_version: LEXICAL_SELECTION_RANKER_VERSION,
    });
    assert_eq!(
        selected_names(&plan),
        vec![
            "github__create_issue",
            "gmail__send_message",
            "calendar__list_events",
            "github__list_issues",
        ]
    );
    assert_eq!(plan.est_schema_tokens, 800);
    assert_eq!(
        plan.skipped.first().map(|tool| tool.reason),
        Some(SkipReason::TokenBudget)
    );
}

#[test]
fn an_explicit_relative_threshold_applies_within_each_segment() {
    let definitions = catalog_definitions();
    let candidates = candidates(&definitions);
    // Half the top score of the same segment's ranking.
    let halved = ToolPrefetchConfig::new(
        ToolPrefetchRanking::Lexical,
        100,
        32_000,
        0.0,
        0.5,
        Vec::new(),
    )
    .expect("config");
    let plan = plan_selection(SelectionInputs {
        config: &halved,
        candidates: &candidates,
        bridge_tokens: 300,
        rankings: &segment_rankings(),
        ranker_version: LEXICAL_SELECTION_RANKER_VERSION,
    });
    // read_file (1.0 of 4.0) is cut in its segment; the calendar tool is
    // the top of its own segment, so its low absolute score does not matter.
    assert_eq!(
        selected_names(&plan),
        vec![
            "github__create_issue",
            "gmail__send_message",
            "calendar__list_events",
            "github__list_issues",
            "github__get_repo",
        ]
    );
    assert_eq!(
        plan.skipped
            .iter()
            .map(|tool| (tool.name.as_str(), tool.reason))
            .collect::<Vec<_>>(),
        vec![("builtin__read_file", SkipReason::BelowMinRelative)]
    );
}

// ---------------------------------------------------------------------
// The frozen surface, driven through the public decorator path
// ---------------------------------------------------------------------

#[tokio::test]
async fn opening_selection_is_recorded_and_every_later_turn_reuses_it_without_ranking() {
    let conversation = Conversation::new("frozen").await;
    let inner = CatalogPort::new(catalog_definitions());
    let ranker = CountingRanker::new(scripted_ranking());
    let prefetch = conversation.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        Arc::clone(&ranker) as Arc<dyn ToolRetrievalProvider>,
    );

    let opening = conversation
        .turn("open an issue about the flaky test")
        .await;
    let (_, first) = advertised_tools(
        &decorator(Some(prefetch.clone())),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    // Cosine 0.35 absolute and 0.7 relative (0.574 here): the three GitHub
    // tools and read_file are kept; gmail and calendar are deferred.
    assert_eq!(
        names(&first),
        vec![
            "builtin__read_file",
            "builtin__result_read",
            "github__create_issue",
            "github__get_repo",
            "github__list_issues",
            "tool_search",
            "tool_describe",
            "tool_call",
        ]
    );
    assert_eq!(ranker.searches.load(Ordering::SeqCst), 1);

    let history = conversation
        .history()
        .await
        .expect("initial entry recorded");
    assert_eq!(history.entries.len(), 1);
    let entry = &history.entries[0];
    assert_eq!(entry.reason, ToolSelectionReason::Initial);
    assert_eq!(entry.effective_from.turn_id, opening.turn_id);
    assert_eq!(entry.advertised, names(&first));
    assert_eq!(entry.ranker_version.as_deref(), Some("dense-cosine-test"));
    assert_eq!(
        entry
            .scores
            .iter()
            .map(|score| (score.name.as_str(), score.score))
            .collect::<Vec<_>>(),
        vec![
            ("github__create_issue", 0.82),
            ("github__list_issues", 0.71),
            ("github__get_repo", 0.66),
            ("builtin__read_file", 0.60),
        ]
    );
    let index = entry
        .tool_search_description
        .as_deref()
        .expect("frozen tool_search index");
    assert!(index.contains("gmail__send_message"), "{index}");
    assert!(!index.contains("github__create_issue"), "{index}");

    // Later turns, each through a fresh decorator bound the way the runtime
    // binds it (as after a restart), advertise the byte-identical array and
    // never rank again.
    for text in ["now email the team", "and what is on my calendar?"] {
        let later = conversation.turn(text).await;
        let restarted = ToolDisclosureCapabilityDecorator::new(
            Arc::new(DiscardingWriter),
            ToolDisclosureMode::Namespaces,
        )
        .with_retrieval_provider(Arc::clone(&ranker) as Arc<dyn ToolRetrievalProvider>)
        .with_tool_prefetch(
            config(ToolPrefetchRanking::Semantic, 100, &[]),
            Arc::clone(&conversation.threads) as Arc<dyn SessionThreadService>,
            thread_scope(),
        )
        .expect("the runtime binding accepts a semantic ranker");
        let (_, again) = advertised_tools(
            &restarted,
            &later,
            &inner,
            CapabilitySurfacePolicy::allow_all(),
        )
        .await;
        assert_eq!(
            serde_json::to_vec(&again).expect("encode"),
            serde_json::to_vec(&first).expect("encode"),
            "the tools array is frozen for the conversation"
        );
    }
    // The restarted decorators also bind the ranker behind `tool_search`,
    // which fits its own index each run; the selection never searches again.
    assert_eq!(ranker.searches.load(Ordering::SeqCst), 1);
    assert_eq!(
        conversation.history().await.expect("history").entries.len(),
        1
    );

    // Resuming the opening turn replays the entry recorded for it.
    let resumed = conversation.run_context_for(opening.turn_id).await;
    let (_, replayed) = advertised_tools(
        &decorator(Some(prefetch)),
        &resumed,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert_eq!(
        serde_json::to_vec(&replayed).expect("encode"),
        serde_json::to_vec(&first).expect("encode")
    );
    assert_eq!(ranker.searches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn searching_and_calling_a_deferred_tool_promotes_nothing() {
    let conversation = Conversation::new("no-promotion").await;
    let inner = CatalogPort::new(catalog_definitions());
    let prefetch = conversation.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        CountingRanker::new(scripted_ranking()),
    );
    let decorator = decorator(Some(prefetch));

    let opening = conversation.turn("open an issue").await;
    let (port, first) = advertised_tools(
        &decorator,
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert!(!names(&first).contains(&"gmail__send_message".to_string()));

    // The model describes and then calls the deferred gmail tool.
    let describe = port
        .register_provider_tool_call(RegisterProviderToolCallRequest {
            tool_call: provider_call(TOOL_DESCRIBE_NAME, json!({"name": "gmail__send_message"})),
            activity_id: None,
        })
        .await
        .expect("describe registers");
    port.invoke_capability(request_for(describe))
        .await
        .expect("describe runs");
    let call = port
        .register_provider_tool_call(RegisterProviderToolCallRequest {
            tool_call: provider_call(
                TOOL_CALL_NAME,
                json!({"name": "gmail__send_message", "arguments": "{\"query\":\"hi\"}"}),
            ),
            activity_id: None,
        })
        .await
        .expect("tool_call registers");
    port.invoke_capability(request_for(call))
        .await
        .expect("tool_call dispatches");
    assert_eq!(
        inner.invocations.lock().expect("invocations").as_slice(),
        [CapabilityId::new("gmail.send_message").expect("id")]
    );

    // The visible surface stays the advertised tools: describing a tool does
    // not add it to the prompt's capability list either.
    let surface = port
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("surface");
    assert!(
        !surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id.as_str() == "gmail.send_message")
    );
    assert_eq!(port.tool_definitions().expect("tools"), first);

    let next = conversation.turn("thanks").await;
    let (_, again) = advertised_tools(
        &decorator,
        &next,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert_eq!(names(&again), names(&first), "nothing is promoted");
}

#[tokio::test]
async fn revoking_a_selected_tool_removes_it_and_nothing_is_ever_added() {
    let conversation = Conversation::new("revoked").await;
    let inner = CatalogPort::new(catalog_definitions());
    let prefetch = conversation.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        CountingRanker::new(scripted_ranking()),
    );
    let decorator = decorator(Some(prefetch));
    let opening = conversation.turn("open an issue").await;
    let (_, first) = advertised_tools(
        &decorator,
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert!(names(&first).contains(&"github__get_repo".to_string()));

    // A new tool becomes authorized and a selected one is revoked.
    inner
        .definitions
        .lock()
        .expect("definitions")
        .push(definition(
            "github.merge_pull",
            "Merge a GitHub pull request.",
        ));
    let revoked_turn = conversation.turn("merge it").await;
    let policy = CapabilitySurfacePolicy::allow_all()
        .deny_capability_ids([CapabilityId::new("github.get_repo").expect("id")]);
    let (_, narrowed) = advertised_tools(&decorator, &revoked_turn, &inner, policy).await;
    let mut expected = names(&first);
    expected.retain(|name| name != "github__get_repo");
    assert_eq!(names(&narrowed), expected);

    let history = conversation.history().await.expect("history");
    assert_eq!(
        history
            .entries
            .iter()
            .map(|entry| entry.reason)
            .collect::<Vec<_>>(),
        vec![ToolSelectionReason::Initial, ToolSelectionReason::Revoked]
    );
    let revoked = &history.entries[1];
    assert_eq!(revoked.effective_from.turn_id, revoked_turn.turn_id);
    assert!(revoked.effective_from.message_sequence.is_some());
    assert_eq!(revoked.advertised, expected);
    assert!(
        !revoked
            .scores
            .iter()
            .any(|score| score.name == "github__get_repo")
    );
    assert!(
        !revoked
            .tool_search_description
            .as_deref()
            .unwrap_or_default()
            .contains("github__get_repo"),
        "a revoked tool is not named in the tool_search index either"
    );

    // Authorizing it again does not bring it back.
    let later = conversation.turn("again").await;
    let (_, after) = advertised_tools(
        &decorator,
        &later,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert_eq!(names(&after), expected);
    assert_eq!(
        conversation.history().await.expect("history").entries.len(),
        2
    );
}

#[tokio::test]
async fn lexical_selection_picks_github_tools_for_a_github_request() {
    let conversation = Conversation::new("lexical").await;
    let inner = CatalogPort::new(catalog_definitions());
    let prefetch = conversation.prefetch(
        config(ToolPrefetchRanking::Lexical, 100, &[]),
        Arc::new(crate::tool_search::NativeBm25fToolRetrieval),
    );
    let opening = conversation
        .turn("Create a GitHub issue in the ironclaw repository about flaky tests")
        .await;
    let (_, tools) = advertised_tools(
        &decorator(Some(prefetch)),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    let tools = names(&tools);
    assert!(
        tools.contains(&"github__create_issue".to_string()),
        "{tools:?}"
    );
    assert!(
        !tools.contains(&"calendar__list_events".to_string()),
        "{tools:?}"
    );
    assert!(!tools.contains(&"builtin__shell".to_string()), "{tools:?}");
    let entry = conversation
        .history()
        .await
        .expect("history")
        .entries
        .remove(0);
    assert_eq!(
        entry.ranker_version.as_deref(),
        Some(LEXICAL_SELECTION_RANKER_VERSION)
    );
}

#[tokio::test]
async fn lexical_selection_ignores_function_words() {
    let index = LexicalSelectionRetrieval
        .fit(&catalog_definitions())
        .await
        .expect("fit");
    let prose = index
        .search("What is the capital of France?", 10)
        .await
        .expect("search");
    assert!(
        prose.ranked.is_empty(),
        "a request sharing only function words with the catalog predicts no tool: {:?}",
        prose.names()
    );
    let request = index
        .search("Please send an email to the team", 10)
        .await
        .expect("search");
    assert_eq!(request.names().first(), Some(&"gmail__send_message"));
}

/// Paragraphs of filler prose that share no term with the test catalog.
fn filler_paragraphs(count: usize) -> Vec<String> {
    (0..count)
        .map(|paragraph| {
            (0..60)
                .map(|word| format!("lorem{paragraph}x{word}"))
                .collect::<Vec<_>>()
                .join(" ")
                + "."
        })
        .collect()
}

#[tokio::test]
async fn default_selection_fills_max_tools_by_rank() {
    let conversation = Conversation::new("ranked").await;
    let inner = CatalogPort::new(catalog_definitions());
    let prefetch = conversation.prefetch(
        ranked_config(ToolPrefetchRanking::Semantic, 100),
        CountingRanker::new(scripted_ranking()),
    );
    let opening = conversation.turn("open an issue").await;
    let (_, tools) = advertised_tools(
        &decorator(Some(prefetch)),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    // Every ranked tool, gmail (0.30) and calendar (0.12) included: the
    // former cosine defaults would have deferred both.
    let tools = names(&tools);
    for name in scripted_ranking().iter().map(|(name, _)| *name) {
        assert!(tools.contains(&name.to_string()), "{name}: {tools:?}");
    }
    assert!(!tools.contains(&"builtin__shell".to_string()), "{tools:?}");
}

/// A long pasted opening message is ranked segment by segment, so a tool
/// named only in its last paragraph, kilobytes in, is still selected. (The
/// former 1 KiB opening cap cut that paragraph off before any ranker saw it.)
#[tokio::test]
async fn a_tool_named_only_at_the_end_of_a_long_opening_message_is_selected() {
    let conversation = Conversation::new("long-opening").await;
    let inner = CatalogPort::new(catalog_definitions());
    let prefetch = conversation.prefetch(
        ranked_config(ToolPrefetchRanking::Lexical, 100),
        Arc::new(crate::tool_search::NativeBm25fToolRetrieval),
    );
    let mut paragraphs = vec!["Create a GitHub issue in the repository.".to_string()];
    paragraphs.extend(filler_paragraphs(14));
    paragraphs.push("Finally send an email message through Gmail to the team.".to_string());
    let message = paragraphs.join("\n\n");
    assert!(message.len() > 8 * 1_024, "{}", message.len());
    let opening = conversation.turn(&message).await;
    let (_, tools) = advertised_tools(
        &decorator(Some(prefetch)),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    let tools = names(&tools);
    assert!(
        tools.contains(&"gmail__send_message".to_string()),
        "{tools:?}"
    );
    assert!(
        tools.contains(&"github__create_issue".to_string()),
        "{tools:?}"
    );
    assert!(
        !tools.contains(&"calendar__list_events".to_string()),
        "lexical selection does not pad with unrelated tools: {tools:?}"
    );
}

/// Every segment is ranked in one `search_many` call on one fitted index,
/// which a dense ranker answers with one embedding request.
#[tokio::test]
async fn every_segment_is_ranked_in_one_batch() {
    let conversation = Conversation::new("one-batch").await;
    let inner = CatalogPort::new(catalog_definitions());
    let ranker = CountingRanker::new(scripted_ranking());
    let prefetch = conversation.prefetch(
        ranked_config(ToolPrefetchRanking::Semantic, 100)
            .with_context(16, 512)
            .expect("context bounds"),
        Arc::clone(&ranker) as Arc<dyn ToolRetrievalProvider>,
    );
    let message = filler_paragraphs(6).join("\n\n");
    let opening = conversation.turn(&message).await;
    let expected_segments =
        conversation::segments(&conversation::opening_request(&message), 512, 16).len();
    assert!(expected_segments > 1, "{expected_segments}");
    advertised_tools(
        &decorator(Some(prefetch)),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert_eq!(ranker.fits.load(Ordering::SeqCst), 1);
    assert_eq!(ranker.batches(), vec![expected_segments]);
    assert_eq!(
        ranker.searches.load(Ordering::SeqCst),
        1,
        "no segment is searched on its own"
    );
}

#[tokio::test]
async fn prefetch_off_keeps_the_ordinary_surface_and_never_ranks() {
    let conversation = Conversation::new("off").await;
    let inner = CatalogPort::new(catalog_definitions());
    let ranker = CountingRanker::new(scripted_ranking());
    let decorator = ToolDisclosureCapabilityDecorator::new(
        Arc::new(DiscardingWriter),
        ToolDisclosureMode::Namespaces,
    )
    .with_retrieval_provider(Arc::clone(&ranker) as Arc<dyn ToolRetrievalProvider>);
    let opening = conversation.turn("open an issue").await;
    let (_, tools) = advertised_tools(
        &decorator,
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    // Ten tools stay under the ordinary caps: the flat list, no bridges.
    assert_eq!(names(&tools), names(&catalog_definitions_sorted()));
    assert_eq!(ranker.searches.load(Ordering::SeqCst), 0);
    assert!(conversation.history().await.is_none());
}

#[tokio::test]
async fn a_run_without_an_accepted_message_keeps_the_ordinary_surface_unrecorded() {
    let conversation = Conversation::new("no-message").await;
    let inner = CatalogPort::new(catalog_definitions());
    let ranker = CountingRanker::new(scripted_ranking());
    let prefetch = conversation.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        Arc::clone(&ranker) as Arc<dyn ToolRetrievalProvider>,
    );
    let trigger_run = conversation.run_context_for(TurnId::new()).await;
    let (_, tools) = advertised_tools(
        &decorator(Some(prefetch)),
        &trigger_run,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert_eq!(names(&tools), names(&catalog_definitions_sorted()));
    assert_eq!(ranker.searches.load(Ordering::SeqCst), 0);
    assert!(conversation.history().await.is_none());
}

#[tokio::test]
async fn semantic_selection_refuses_the_native_ranker() {
    let conversation = Conversation::new("native").await;
    let error = ToolPrefetch::new(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        Arc::new(crate::tool_search::NativeBm25fToolRetrieval),
        Arc::clone(&conversation.threads) as Arc<dyn SessionThreadService>,
        thread_scope(),
    )
    .expect_err("semantic over BM25F is refused");
    assert_eq!(error, ToolPrefetchConfigError::SemanticNeedsBoundRanker);
}

/// A scripted [`ToolAvailabilityPredicate`]: tools it has no answer for are
/// available (as a tool needing no credential is), and it can be made slow.
#[derive(Debug, Default)]
struct ScriptedAvailability {
    answers: BTreeMap<CapabilityId, ToolAvailability>,
    delay: Option<Duration>,
    omit_answers: bool,
}

impl ScriptedAvailability {
    fn with(answers: &[(&str, ToolAvailability)]) -> Arc<Self> {
        Arc::new(Self {
            answers: answers
                .iter()
                .map(|(id, answer)| (CapabilityId::new(*id).expect("capability id"), *answer))
                .collect(),
            ..Self::default()
        })
    }
}

#[async_trait]
impl ToolAvailabilityPredicate for ScriptedAvailability {
    async fn availability(
        &self,
        _run_context: &LoopRunContext,
        capability_ids: &[CapabilityId],
    ) -> BTreeMap<CapabilityId, ToolAvailability> {
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        if self.omit_answers {
            return BTreeMap::new();
        }
        capability_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    self.answers
                        .get(id)
                        .copied()
                        .unwrap_or(ToolAvailability::Available),
                )
            })
            .collect()
    }
}

const ACCOUNT_MISSING: ToolAvailability = ToolAvailability::Unavailable(
    ironclaw_loop_contracts::ToolUnavailableReason::CredentialMissing,
);

fn latest_reason(history: &ToolSelectionHistory) -> Option<ToolSelectionReason> {
    history.latest().map(|entry| entry.reason)
}

#[tokio::test]
async fn availability_keeps_unusable_tools_out_and_revokes_them_later() {
    let conversation = Conversation::new("filter").await;
    let inner = CatalogPort::new(catalog_definitions());
    let base = conversation.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        CountingRanker::new(scripted_ranking()),
    );
    let opening = conversation.turn("open an issue").await;
    // list_issues's account is missing; create_issue's is configured; the
    // builtin tools need no credential at all.
    let (_, tools) = advertised_tools(
        &decorator(Some(base.clone().with_availability(
            ScriptedAvailability::with(&[
                ("github.list_issues", ACCOUNT_MISSING),
                ("github.create_issue", ToolAvailability::Available),
            ]),
        ))),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    let advertised = names(&tools);
    assert!(!advertised.contains(&"github__list_issues".to_string()));
    assert!(advertised.contains(&"github__create_issue".to_string()));
    assert!(
        advertised.contains(&"builtin__result_read".to_string()),
        "a tool with no credential requirement is admitted"
    );
    let initial = conversation.history().await.expect("history");
    assert!(
        !initial.entries[0]
            .advertised
            .contains(&"github__list_issues".to_string())
    );

    // The account behind create_issue is disconnected: a definite answer, so
    // the frozen tool is revoked.
    let later = conversation.turn("and now").await;
    let (_, narrowed) = advertised_tools(
        &decorator(Some(base.with_availability(ScriptedAvailability::with(&[
            ("github.create_issue", ACCOUNT_MISSING),
        ])))),
        &later,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert!(!names(&narrowed).contains(&"github__create_issue".to_string()));
    assert!(
        !names(&narrowed).contains(&"github__list_issues".to_string()),
        "connecting list_issues's account later does not add it to the frozen list"
    );
    assert_eq!(
        latest_reason(&conversation.history().await.expect("history")),
        Some(ToolSelectionReason::Revoked)
    );
}

#[tokio::test]
async fn unknown_availability_keeps_a_new_candidate_out_but_never_revokes_a_frozen_tool() {
    let conversation = Conversation::new("unknown").await;
    let inner = CatalogPort::new(catalog_definitions());
    let base = conversation.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        CountingRanker::new(scripted_ranking()),
    );
    // Admission fails closed: a lookup that did not answer excludes the tool.
    let opening = conversation.turn("open an issue").await;
    let (_, first) = advertised_tools(
        &decorator(Some(base.clone().with_availability(
            ScriptedAvailability::with(&[("github.list_issues", ToolAvailability::Unknown)]),
        ))),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert!(!names(&first).contains(&"github__list_issues".to_string()));
    assert!(names(&first).contains(&"github__create_issue".to_string()));

    // Revocation does not: a frozen tool whose lookup fails stays, and the
    // cached `tools` array is unchanged.
    let later = conversation.turn("and now").await;
    let (_, again) = advertised_tools(
        &decorator(Some(base.with_availability(ScriptedAvailability::with(&[
            ("github.create_issue", ToolAvailability::Unknown),
        ])))),
        &later,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert_eq!(again, first);
    let history = conversation.history().await.expect("history");
    assert_eq!(history.entries.len(), 1);
    assert_eq!(latest_reason(&history), Some(ToolSelectionReason::Initial));
}

#[tokio::test]
async fn a_predicate_without_an_answer_or_past_its_deadline_leaves_candidates_unknown() {
    let conversation = Conversation::new("deadline").await;
    let run = conversation.run_context_for(TurnId::new()).await;
    let definitions = catalog_definitions();
    let candidates: Vec<&ProviderToolDefinition> = definitions.iter().collect();
    let all: BTreeSet<String> = names(&definitions).into_iter().collect();

    let silent = PredicateCandidateFilter::new(Arc::new(ScriptedAvailability {
        omit_answers: true,
        ..ScriptedAvailability::default()
    }));
    let sorted = silent.availability(&run, &candidates).await;
    assert!(sorted.available.is_empty() && sorted.unavailable.is_empty());
    assert_eq!(sorted.unknown, all, "a missing answer counts as unknown");

    let mut slow = PredicateCandidateFilter::new(Arc::new(ScriptedAvailability {
        delay: Some(Duration::from_secs(30)),
        ..ScriptedAvailability::default()
    }));
    slow.backstop = Duration::from_millis(20);
    let sorted = slow.availability(&run, &candidates).await;
    assert!(sorted.available.is_empty() && sorted.unavailable.is_empty());
    assert_eq!(
        sorted.unknown, all,
        "an overrun counts every candidate unknown"
    );
}

#[tokio::test]
async fn a_tool_left_out_for_availability_is_still_callable_through_tool_call() {
    let conversation = Conversation::new("setup-flow").await;
    let inner = CatalogPort::new(catalog_definitions());
    let prefetch = conversation
        .prefetch(
            config(ToolPrefetchRanking::Semantic, 100, &[]),
            CountingRanker::new(scripted_ranking()),
        )
        .with_availability(ScriptedAvailability::with(&[(
            "github.list_issues",
            ACCOUNT_MISSING,
        )]));
    let opening = conversation.turn("list my issues").await;
    let (port, tools) = advertised_tools(
        &decorator(Some(prefetch)),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert!(!names(&tools).contains(&"github__list_issues".to_string()));

    // Calling it reaches the host, which is where its connect prompt comes
    // from: availability narrows what is advertised, never what is callable.
    let call = port
        .register_provider_tool_call(RegisterProviderToolCallRequest {
            tool_call: provider_call(
                TOOL_CALL_NAME,
                json!({"name": "github__list_issues", "arguments": "{\"query\":\"mine\"}"}),
            ),
            activity_id: None,
        })
        .await
        .expect("tool_call registers");
    port.invoke_capability(request_for(call))
        .await
        .expect("tool_call dispatches");
    assert_eq!(
        inner.invocations.lock().expect("invocations").as_slice(),
        [CapabilityId::new("github.list_issues").expect("id")]
    );
}

#[tokio::test]
async fn the_decorator_applies_availability_whichever_builder_runs_first() {
    for availability_first in [true, false] {
        let conversation = Conversation::new(if availability_first {
            "a-first"
        } else {
            "p-first"
        })
        .await;
        let inner = CatalogPort::new(catalog_definitions());
        let predicate = ScriptedAvailability::with(&[("github.list_issues", ACCOUNT_MISSING)]);
        let mut decorator = ToolDisclosureCapabilityDecorator::new(
            Arc::new(DiscardingWriter),
            ToolDisclosureMode::Namespaces,
        );
        let bind_prefetch = |decorator: ToolDisclosureCapabilityDecorator| {
            decorator
                .with_tool_prefetch(
                    config(ToolPrefetchRanking::Lexical, 100, &[]),
                    Arc::clone(&conversation.threads) as Arc<dyn SessionThreadService>,
                    thread_scope(),
                )
                .expect("prefetch binding")
        };
        if availability_first {
            decorator = bind_prefetch(decorator.with_tool_availability(predicate));
        } else {
            decorator = bind_prefetch(decorator).with_tool_availability(predicate);
        }
        let opening = conversation.turn("list the github issues").await;
        let (_, tools) = advertised_tools(
            &decorator,
            &opening,
            &inner,
            CapabilitySurfacePolicy::allow_all(),
        )
        .await;
        assert!(
            names(&tools).contains(&"github__get_repo".to_string()),
            "the github request still selects github tools"
        );
        assert!(
            !names(&tools).contains(&"github__list_issues".to_string()),
            "availability applies (availability bound first: {availability_first})"
        );
    }
}

#[tokio::test]
#[tracing_test::traced_test]
async fn selection_logs_never_contain_the_query() {
    let conversation = Conversation::new("logs").await;
    let inner = CatalogPort::new(catalog_definitions());
    let prefetch = conversation.prefetch(
        config(ToolPrefetchRanking::Lexical, 100, &[]),
        Arc::new(crate::tool_search::NativeBm25fToolRetrieval),
    );
    // Several segments, each carrying words that must never be logged.
    let mut paragraphs = vec!["create a github issue titled zebra-quartz-secret".to_string()];
    paragraphs.extend(filler_paragraphs(6));
    paragraphs.push("then send an email about the okapi-mango-secret".to_string());
    let message = paragraphs.join("\n\n");
    let segments =
        conversation::segments(&conversation::opening_request(&message), 2_048, 16).len();
    assert!(segments > 1, "{segments}");
    let opening = conversation.turn(&message).await;
    advertised_tools(
        &decorator(Some(prefetch.clone())),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert!(logs_contain(
        "selected the conversation's tools from its opening request"
    ));
    assert!(logs_contain(
        "ranked the candidates against each conversation segment and merged by rank"
    ));
    assert!(logs_contain(&format!("segment_count={segments}")));
    assert!(logs_contain("github__create_issue"));
    for secret in [
        "zebra-quartz-secret",
        "okapi-mango-secret",
        "titled",
        "lorem0x1",
    ] {
        assert!(!logs_contain(secret), "{secret} was logged");
    }

    // The line that records a selection and the line that serves it later
    // estimate the same array, so they report the same size.
    let later = conversation.turn("and another one").await;
    advertised_tools(
        &decorator(Some(prefetch)),
        &later,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    logs_assert(|lines: &[&str]| {
        let tokens = |message: &str| -> Result<String, String> {
            let line = lines
                .iter()
                .find(|line| line.contains(message))
                .ok_or_else(|| format!("no {message:?} line"))?;
            line.split_whitespace()
                .find_map(|field| field.strip_prefix("est_schema_tokens="))
                .map(str::to_string)
                .ok_or_else(|| format!("{message:?} logs no est_schema_tokens"))
        };
        let selected = tokens("selected the conversation's tools from its opening request")?;
        let served = tokens("serving the conversation's recorded tool selection")?;
        if selected == served {
            Ok(())
        } else {
            Err(format!(
                "selected {selected} estimated tokens but served {served} for the same entry"
            ))
        }
    });
}

// ---------------------------------------------------------------------
// Classifiers behind the port
// ---------------------------------------------------------------------

#[tokio::test]
async fn the_local_classifier_records_exactly_what_the_pure_plan_selects() {
    let conversation = Conversation::new("local-identical").await;
    let inner = CatalogPort::new(catalog_definitions());
    let local_config = config(ToolPrefetchRanking::Semantic, 7, &["outbound_deliver"]);
    let prefetch = conversation.prefetch(
        local_config.clone(),
        CountingRanker::new(scripted_ranking()),
    );
    let opening = conversation
        .turn("open an issue about the flaky test")
        .await;
    let (_, tools) = advertised_tools(
        &decorator(Some(prefetch)),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;

    // The same inputs through the selection function the local classifier is
    // built from, with nothing in between.
    let catalog = CapabilityCatalog::new(&catalog_definitions(), &[]);
    let policy = CapabilitySurfacePolicy::allow_all();
    let candidates: Vec<(&ProviderToolDefinition, u32)> =
        catalog.effective_definitions_with_tokens(&policy).collect();
    let plan = plan_selection(SelectionInputs {
        config: &local_config,
        candidates: &candidates,
        bridge_tokens: advertised_bridge_tokens(&catalog, &policy, ToolDisclosureMode::Namespaces),
        rankings: &[ranked(&scripted_ranking())],
        ranker_version: "dense-cosine-test",
    });
    let entry = conversation
        .history()
        .await
        .expect("history")
        .entries
        .remove(0);
    assert_eq!(entry.advertised, plan.advertised);
    assert_eq!(names(&tools), plan.advertised);
    assert_eq!(
        entry
            .scores
            .iter()
            .map(|score| (score.name.clone(), score.score))
            .collect::<Vec<_>>(),
        plan.selected
            .iter()
            .map(|tool| (tool.name.clone(), tool.score))
            .collect::<Vec<_>>()
    );
    assert_eq!(entry.ranker_version.as_deref(), Some("dense-cosine-test"));
    assert_eq!(entry.fallback_reason, None);
    assert!(
        plan.skipped
            .iter()
            .any(|tool| tool.reason == SkipReason::MaxTools),
        "the cap was exercised"
    );
}

/// A bound classifier with a scripted answer that records every request.
#[derive(Debug)]
struct ScriptedClassifier {
    answer: Result<Vec<(&'static str, f32)>, ToolSelectionError>,
    requests: Mutex<Vec<ToolSelectionRequest>>,
}

impl ScriptedClassifier {
    fn answering(chosen: Vec<(&'static str, f32)>) -> Arc<Self> {
        Arc::new(Self {
            answer: Ok(chosen),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn failing(error: ToolSelectionError) -> Arc<Self> {
        Arc::new(Self {
            answer: Err(error),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.requests.lock().expect("requests").len()
    }
}

#[async_trait]
impl ToolSelectionClassifier for ScriptedClassifier {
    fn classifier_name(&self) -> &str {
        "scripted"
    }

    async fn classify(
        &self,
        request: &ToolSelectionRequest,
    ) -> Result<ironclaw_loop_contracts::ToolSelection, ToolSelectionError> {
        self.requests
            .lock()
            .expect("requests")
            .push(request.clone());
        self.answer
            .clone()
            .map(|chosen| ironclaw_loop_contracts::ToolSelection {
                chosen: chosen
                    .into_iter()
                    .map(|(name, score)| ChosenTool::new(name, score))
                    .collect(),
                scorer: "scripted-probability".to_string(),
            })
    }
}

fn bound(
    conversation: &Conversation,
    classifier: &Arc<ScriptedClassifier>,
    ranker: &Arc<CountingRanker>,
    max_tools: usize,
) -> ToolPrefetch {
    conversation.prefetch(
        config(
            ToolPrefetchRanking::Semantic,
            max_tools,
            &["outbound_deliver"],
        )
        .with_classifier(Arc::clone(classifier) as Arc<dyn ToolSelectionClassifier>),
        Arc::clone(ranker) as Arc<dyn ToolRetrievalProvider>,
    )
}

#[tokio::test]
async fn a_bound_classifier_chooses_alone_and_its_answer_is_checked() {
    let conversation = Conversation::new("bound").await;
    let inner = CatalogPort::new(catalog_definitions());
    let ranker = CountingRanker::new(scripted_ranking());
    let classifier = ScriptedClassifier::answering(vec![
        ("github__create_issue", 0.97),
        // Not a candidate: unknown, and denied by the policy below.
        ("nope__missing", 0.95),
        ("gmail__send_message", 0.93),
        // Pinned: the host advertises it anyway.
        ("builtin__result_read", 0.9),
        ("github__create_issue", 0.8),
        ("calendar__list_events", f32::NAN),
        ("github__list_issues", 0.4),
    ]);
    let prefetch = bound(&conversation, &classifier, &ranker, 100);
    let policy = CapabilitySurfacePolicy::allow_all()
        .deny_capability_ids([CapabilityId::new("gmail.send_message").expect("capability id")]);
    let opening = conversation
        .turn("open an issue about the flaky test")
        .await;
    let (_, tools) = advertised_tools(&decorator(Some(prefetch)), &opening, &inner, policy).await;

    assert_eq!(
        names(&tools),
        vec![
            "builtin__outbound_deliver",
            "builtin__result_read",
            "github__create_issue",
            "github__list_issues",
            "tool_search",
            "tool_describe",
            "tool_call",
        ]
    );
    assert_eq!(
        ranker.fits.load(Ordering::SeqCst),
        0,
        "the ranker never runs"
    );
    assert_eq!(ranker.searches.load(Ordering::SeqCst), 0);

    let requests = classifier.requests.lock().expect("requests").clone();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    let candidate_names: Vec<&str> = request.candidates.iter().map(|c| c.name()).collect();
    assert!(!candidate_names.contains(&"gmail__send_message"), "denied");
    assert_eq!(candidate_names.len(), catalog_definitions().len() - 1);
    assert_eq!(
        request.pinned,
        vec!["builtin__result_read", "builtin__outbound_deliver"]
    );
    assert_eq!(request.reserved_tools, 5);
    assert!(request.called_tools.is_empty());
    assert_eq!(
        request.context.user_messages(),
        ["open an issue about the flaky test"]
    );

    let entry = conversation
        .history()
        .await
        .expect("history")
        .entries
        .remove(0);
    assert_eq!(entry.advertised, names(&tools));
    assert_eq!(
        entry
            .scores
            .iter()
            .map(|score| (score.name.as_str(), score.score))
            .collect::<Vec<_>>(),
        vec![("github__create_issue", 0.97), ("github__list_issues", 0.4)]
    );
    assert_eq!(
        entry.ranker_version.as_deref(),
        Some("scripted-probability")
    );
    assert_eq!(entry.fallback_reason, None);
}

#[tokio::test]
async fn a_bound_classifier_cannot_exceed_max_tools() {
    let conversation = Conversation::new("bound-cap").await;
    let inner = CatalogPort::new(catalog_definitions());
    let ranker = CountingRanker::new(scripted_ranking());
    let classifier = ScriptedClassifier::answering(vec![
        ("github__create_issue", 0.9),
        ("github__list_issues", 0.8),
        ("github__get_repo", 0.7),
    ]);
    // Floor of five (three bridges, result_read, outbound_deliver): room
    // for two.
    let prefetch = bound(&conversation, &classifier, &ranker, 7);
    let opening = conversation.turn("issues").await;
    let (_, tools) = advertised_tools(
        &decorator(Some(prefetch)),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert_eq!(tools.len(), 7);
    assert!(!names(&tools).contains(&"github__get_repo".to_string()));
}

#[tokio::test]
async fn a_failing_bound_classifier_freezes_the_core_set_and_is_never_retried() {
    let conversation = Conversation::new("bound-failure").await;
    let inner = CatalogPort::new(catalog_definitions());
    let ranker = CountingRanker::new(scripted_ranking());
    let classifier = ScriptedClassifier::failing(ToolSelectionError::Timeout {
        elapsed: std::time::Duration::from_millis(500),
    });
    let prefetch = bound(&conversation, &classifier, &ranker, 100);
    let policy = || {
        CapabilitySurfacePolicy::allow_all()
            .deny_capability_ids([CapabilityId::new("builtin.shell").expect("capability id")])
    };
    let opening = conversation.turn("open an issue").await;
    let (_, first) = advertised_tools(
        &decorator(Some(prefetch.clone())),
        &opening,
        &inner,
        policy(),
    )
    .await;
    // The authorized core tools (shell is denied) plus the floor and the
    // extra: what the conversation advertises without selection.
    assert_eq!(
        names(&first),
        vec![
            "builtin__outbound_deliver",
            "builtin__outbound_delivery_targets_list",
            "builtin__read_file",
            "builtin__result_read",
            "tool_search",
            "tool_describe",
            "tool_call",
        ]
    );
    assert_eq!(ranker.fits.load(Ordering::SeqCst), 0, "no local fallback");
    let entry = conversation
        .history()
        .await
        .expect("the fallback is recorded")
        .entries
        .remove(0);
    assert_eq!(entry.reason, ToolSelectionReason::Initial);
    assert_eq!(entry.advertised, names(&first));
    assert_eq!(entry.fallback_reason.as_deref(), Some("timeout"));
    assert!(entry.scores.is_empty());
    assert_eq!(entry.ranker_version, None);

    // A later turn and a resume of the opening turn serve the recorded
    // fallback; the classifier is never asked again.
    let later = conversation.turn("now list them").await;
    let resumed = conversation.run_context_for(opening.turn_id).await;
    for run in [later, resumed] {
        let (_, again) =
            advertised_tools(&decorator(Some(prefetch.clone())), &run, &inner, policy()).await;
        assert_eq!(
            serde_json::to_vec(&again).expect("encode"),
            serde_json::to_vec(&first).expect("encode")
        );
    }
    assert_eq!(classifier.calls(), 1);
    assert_eq!(ranker.fits.load(Ordering::SeqCst), 0);
    assert_eq!(
        conversation.history().await.expect("history").entries.len(),
        1
    );
}

#[tokio::test]
async fn a_failing_local_ranker_keeps_the_ordinary_surface_unrecorded() {
    #[derive(Debug)]
    struct BrokenRanker;

    #[async_trait]
    impl ToolRetrievalProvider for BrokenRanker {
        fn ranker_version(&self) -> &str {
            "dense-cosine-broken"
        }

        async fn fit(
            &self,
            _definitions: &[ProviderToolDefinition],
        ) -> Result<Arc<dyn ToolRetrievalIndex>, ToolRetrievalError> {
            Err(ToolRetrievalError::Timeout {
                elapsed: std::time::Duration::from_millis(1),
            })
        }
    }

    let conversation = Conversation::new("local-failure").await;
    let inner = CatalogPort::new(catalog_definitions());
    let prefetch = conversation.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        Arc::new(BrokenRanker),
    );
    let opening = conversation.turn("open an issue").await;
    let (_, tools) = advertised_tools(
        &decorator(Some(prefetch)),
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert!(names(&tools).contains(&"builtin__shell".to_string()));
    assert!(conversation.history().await.is_none(), "nothing recorded");
}

// ---------------------------------------------------------------------
// Prompt text follows the advertised tools
// ---------------------------------------------------------------------

async fn surface_for(
    decorator: &ToolDisclosureCapabilityDecorator,
    run_context: &LoopRunContext,
    inner: &Arc<CatalogPort>,
) -> VisibleCapabilitySurface {
    decorator
        .decorate_with_policy(
            run_context,
            Arc::clone(inner) as Arc<dyn LoopCapabilityPort>,
            Arc::new(CapabilitySurfacePolicy::allow_all()),
        )
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .expect("visible capabilities")
}

/// A selected run's surface says so, and its advertised set is exactly the
/// frozen `tools` array (bridges included), which is what tool-naming prompt
/// blocks are gated on. A run the selection did not apply to, and a runtime
/// with selection off, keep the ordinary surface, whose prompt text renders
/// as it always has.
#[tokio::test]
async fn only_a_selected_surface_is_marked_for_prompt_gating() {
    let conversation = Conversation::new("prompt-gating").await;
    let inner = CatalogPort::new(catalog_definitions());
    let ranker = CountingRanker::new(scripted_ranking());
    let prefetch = conversation.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        Arc::clone(&ranker) as Arc<dyn ToolRetrievalProvider>,
    );
    let selecting = decorator(Some(prefetch));

    let opening = conversation
        .turn("open an issue about the flaky test")
        .await;
    let selected = surface_for(&selecting, &opening, &inner).await;
    assert_eq!(
        selected.advertised_choice,
        ironclaw_loop_contracts::AdvertisedToolChoice::TurnStartSelection
    );
    let advertised = ironclaw_loop_contracts::AdvertisedTools::from_surface(&selected);
    assert!(advertised.may_name(&[
        "github.create_issue",
        "ironclaw.tool_search",
        "ironclaw.tool_call"
    ]));
    assert!(
        !advertised.may_name(&["gmail.send_message"]),
        "a deferred tool is not advertised: {advertised:?}"
    );

    // A fresh conversation whose first run has no accepted user text: the
    // selection does not apply, and nothing is recorded.
    let unselected = Conversation::new("prompt-gating-fallback").await;
    let fallback_prefetch = unselected.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[]),
        Arc::clone(&ranker) as Arc<dyn ToolRetrievalProvider>,
    );
    let trigger_run = unselected.run_context_for(TurnId::new()).await;
    let fallback = surface_for(&decorator(Some(fallback_prefetch)), &trigger_run, &inner).await;
    assert_eq!(
        ironclaw_loop_contracts::AdvertisedTools::from_surface(&fallback),
        ironclaw_loop_contracts::AdvertisedTools::Ordinary
    );

    let off = surface_for(&decorator(None), &opening, &inner).await;
    assert_eq!(
        off.advertised_choice,
        ironclaw_loop_contracts::AdvertisedToolChoice::Ordinary
    );
}

mod reselection;
