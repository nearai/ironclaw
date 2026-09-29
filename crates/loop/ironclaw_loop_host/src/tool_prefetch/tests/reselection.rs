//! Re-selection once the prompt cache can no longer be warm, driven through
//! the public decorator path over an in-memory thread store.

use std::time::Duration;

use chrono::Utc;
use ironclaw_llm::PromptCacheLifetime;
use ironclaw_threads::{
    CreateSummaryArtifactRequest, ModelCallMark, RecordToolSelectionActivityRequest,
    ToolSelectionActivityUpdate,
};

use super::*;

/// A bound classifier whose answer a test changes between turns.
#[derive(Debug)]
struct SwitchableClassifier {
    answer: Mutex<Result<Vec<(&'static str, f32)>, ToolSelectionError>>,
    requests: Mutex<Vec<ToolSelectionRequest>>,
}

impl SwitchableClassifier {
    fn answering(chosen: Vec<(&'static str, f32)>) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(Ok(chosen)),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn answer(&self, answer: Result<Vec<(&'static str, f32)>, ToolSelectionError>) {
        *self.answer.lock().expect("answer") = answer;
    }

    fn calls(&self) -> usize {
        self.requests.lock().expect("requests").len()
    }

    fn last_request(&self) -> ToolSelectionRequest {
        self.requests
            .lock()
            .expect("requests")
            .last()
            .cloned()
            .expect("a request")
    }
}

#[async_trait]
impl ToolSelectionClassifier for SwitchableClassifier {
    fn classifier_name(&self) -> &str {
        "switchable"
    }

    async fn classify(
        &self,
        request: &ToolSelectionRequest,
    ) -> Result<ironclaw_loop_contracts::ToolSelection, ToolSelectionError> {
        self.requests
            .lock()
            .expect("requests")
            .push(request.clone());
        self.answer.lock().expect("answer").clone().map(|chosen| {
            ironclaw_loop_contracts::ToolSelection {
                chosen: chosen
                    .into_iter()
                    .map(|(name, score)| ChosenTool::new(name, score))
                    .collect(),
                scorer: "switchable-probability".to_string(),
            }
        })
    }
}

/// The run's model and cache lifetime, changeable between turns.
struct FixedProfile(Mutex<HostManagedPromptCacheProfile>);

impl FixedProfile {
    fn new(model: &str, lifetime: PromptCacheLifetime) -> Arc<Self> {
        Arc::new(Self(Mutex::new(HostManagedPromptCacheProfile {
            model: Some(model.to_string()),
            lifetime,
        })))
    }

    fn set_model(&self, model: &str) {
        self.0.lock().expect("profile").model = Some(model.to_string());
    }
}

impl PromptCacheProfileSource for FixedProfile {
    fn prompt_cache_profile(&self, _run_context: &LoopRunContext) -> HostManagedPromptCacheProfile {
        self.0.lock().expect("profile").clone()
    }
}

fn github_answer() -> Vec<(&'static str, f32)> {
    vec![("github__create_issue", 0.9), ("github__list_issues", 0.8)]
}

fn calendar_answer() -> Vec<(&'static str, f32)> {
    vec![("calendar__list_events", 0.9)]
}

fn reselecting(
    conversation: &Conversation,
    classifier: &Arc<SwitchableClassifier>,
) -> ToolPrefetch {
    conversation.prefetch(
        config(ToolPrefetchRanking::Semantic, 100, &[])
            .with_classifier(Arc::clone(classifier) as Arc<dyn ToolSelectionClassifier>)
            .with_reselection(ToolReselectionConfig::new(
                true,
                Duration::from_secs(3_600),
                Duration::from_secs(60),
            )),
        CountingRanker::new(Vec::new()),
    )
}

impl Conversation {
    /// Record that `turn` called the model `ago` before now, on `model`.
    async fn model_called(&self, turn: &LoopRunContext, ago: Duration, model: Option<&str>) {
        self.threads
            .record_tool_selection_activity(RecordToolSelectionActivityRequest {
                scope: thread_scope(),
                thread_id: self.thread_id.clone(),
                update: ToolSelectionActivityUpdate::ModelCall(ModelCallMark {
                    turn_id: turn.turn_id,
                    called_at: Utc::now() - chrono::Duration::from_std(ago).expect("duration"),
                    model: model.map(str::to_string),
                }),
            })
            .await
            .expect("mark");
    }

    async fn reasons(&self) -> Vec<ToolSelectionReason> {
        self.history()
            .await
            .map(|history| history.entries.iter().map(|entry| entry.reason).collect())
            .unwrap_or_default()
    }
}

async fn serve(
    prefetch: &ToolPrefetch,
    run: &LoopRunContext,
    inner: &Arc<CatalogPort>,
) -> Vec<ProviderToolDefinition> {
    advertised_tools(
        &decorator(Some(prefetch.clone())),
        run,
        inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await
    .1
}

fn encoded(tools: &[ProviderToolDefinition]) -> Vec<u8> {
    serde_json::to_vec(tools).expect("encode")
}

const HOUR: Duration = Duration::from_secs(3_600);
const MINUTE: Duration = Duration::from_secs(60);

#[tokio::test]
async fn while_the_cache_could_be_warm_the_array_stays_byte_identical() {
    let conversation = Conversation::new("warm").await;
    let inner = CatalogPort::new(catalog_definitions());
    let classifier = SwitchableClassifier::answering(github_answer());
    let prefetch = reselecting(&conversation, &classifier);

    let opening = conversation.turn("open a github issue").await;
    let first = serve(&prefetch, &opening, &inner).await;
    classifier.answer(Ok(calendar_answer()));

    // Idle below lifetime + margin (unknown provider: 1 h + 60 s), and no
    // record of a model call at all: both count as warm.
    let unrecorded = conversation.turn("what is on my calendar?").await;
    assert_eq!(
        encoded(&serve(&prefetch, &unrecorded, &inner).await),
        encoded(&first)
    );
    conversation.model_called(&unrecorded, HOUR, None).await;
    let within = conversation.turn("and tomorrow?").await;
    assert_eq!(
        encoded(&serve(&prefetch, &within, &inner).await),
        encoded(&first)
    );

    assert_eq!(classifier.calls(), 1, "never classified again while warm");
    assert_eq!(conversation.reasons().await, [ToolSelectionReason::Initial]);
}

#[tokio::test]
async fn after_an_idle_gap_the_tools_follow_the_conversation_so_far() {
    let conversation = Conversation::new("cold").await;
    let inner = CatalogPort::new(catalog_definitions());
    let classifier = SwitchableClassifier::answering(github_answer());
    let prefetch = reselecting(&conversation, &classifier);

    let opening = conversation.turn("open a github issue").await;
    let first = serve(&prefetch, &opening, &inner).await;
    assert!(names(&first).contains(&"github__create_issue".to_string()));
    conversation
        .model_called(&opening, HOUR + 2 * MINUTE, None)
        .await;

    classifier.answer(Ok(calendar_answer()));
    let drifted = conversation.turn("what is on my calendar today?").await;
    let second = serve(&prefetch, &drifted, &inner).await;
    assert!(names(&second).contains(&"calendar__list_events".to_string()));
    assert!(!names(&second).contains(&"github__create_issue".to_string()));

    let request = classifier.last_request();
    assert_eq!(
        request.context.user_messages(),
        ["open a github issue", "what is on my calendar today?"],
        "ranked against the conversation window, oldest first"
    );
    let history = conversation.history().await.expect("history");
    assert_eq!(
        conversation.reasons().await,
        [ToolSelectionReason::Initial, ToolSelectionReason::CacheCold]
    );
    let entry = &history.entries[1];
    assert_eq!(entry.effective_from.turn_id, drifted.turn_id);
    assert_eq!(entry.advertised, names(&second));
    assert_eq!(
        entry.ranker_version.as_deref(),
        Some("switchable-probability")
    );

    // Mid-turn (a surface refresh after this turn's first model call) and
    // on the next warm turn nothing changes again.
    conversation
        .model_called(&drifted, Duration::ZERO, None)
        .await;
    let refreshed = conversation.run_context_for(drifted.turn_id).await;
    classifier.answer(Ok(github_answer()));
    assert_eq!(
        encoded(&serve(&prefetch, &refreshed, &inner).await),
        encoded(&second)
    );
    let next = conversation.turn("thanks").await;
    assert_eq!(
        encoded(&serve(&prefetch, &next, &inner).await),
        encoded(&second)
    );
    assert_eq!(classifier.calls(), 2);

    // Replay serves the entry in force at each turn.
    let replay_opening = conversation.run_context_for(opening.turn_id).await;
    assert_eq!(
        encoded(&serve(&prefetch, &replay_opening, &inner).await),
        encoded(&first)
    );
    let replay_drifted = conversation.run_context_for(drifted.turn_id).await;
    assert_eq!(
        encoded(&serve(&prefetch, &replay_drifted, &inner).await),
        encoded(&second)
    );
    // Ordered by message: a turn after the re-selection that has no entry
    // of its own replays the re-selected list.
    let history = conversation.history().await.expect("history");
    let drifted_sequence = history.entries[1]
        .effective_from
        .message_sequence
        .expect("sequence");
    assert_eq!(
        history
            .in_force_at(TurnId::new(), Some(drifted_sequence + 1))
            .map(|entry| entry.reason),
        Some(ToolSelectionReason::CacheCold)
    );
    assert_eq!(
        history
            .in_force_at(TurnId::new(), Some(drifted_sequence - 1))
            .map(|entry| entry.reason),
        Some(ToolSelectionReason::Initial)
    );
}

/// Whether a conversation re-selects after `idle` with `lifetime`.
async fn reselects_after(lifetime: PromptCacheLifetime, idle: Duration) -> bool {
    let conversation = Conversation::new("lifetime").await;
    let inner = CatalogPort::new(catalog_definitions());
    let classifier = SwitchableClassifier::answering(github_answer());
    let prefetch = reselecting(&conversation, &classifier)
        .with_prompt_cache_profiles(FixedProfile::new("anthropic/claude", lifetime));
    let opening = conversation.turn("open a github issue").await;
    serve(&prefetch, &opening, &inner).await;
    conversation
        .model_called(&opening, idle, Some("anthropic/claude"))
        .await;
    classifier.answer(Ok(calendar_answer()));
    let next = conversation.turn("what is on my calendar?").await;
    serve(&prefetch, &next, &inner).await;
    conversation.reasons().await.len() == 2
}

#[tokio::test]
async fn a_known_cache_lifetime_decides_when_the_cache_is_cold() {
    let short = PromptCacheLifetime::Known(5 * MINUTE);
    let long = PromptCacheLifetime::Known(HOUR);
    // Lifetime plus the 60 s margin.
    assert!(!reselects_after(short, 5 * MINUTE + 30 * Duration::from_secs(1)).await);
    assert!(reselects_after(short, 7 * MINUTE).await);
    assert!(!reselects_after(long, 30 * MINUTE).await);
    assert!(reselects_after(long, HOUR + 2 * MINUTE).await);
    // No cache at all: every turn boundary may re-select.
    assert!(reselects_after(PromptCacheLifetime::Disabled, Duration::from_secs(1)).await);
    // Unknown: the configured hour.
    assert!(!reselects_after(PromptCacheLifetime::Unknown, 30 * MINUTE).await);
    assert!(reselects_after(PromptCacheLifetime::Unknown, 2 * HOUR).await);
}

#[tokio::test]
async fn a_model_change_reselects_and_compaction_does_not() {
    let conversation = Conversation::new("model-change").await;
    let inner = CatalogPort::new(catalog_definitions());
    let classifier = SwitchableClassifier::answering(github_answer());
    let profile = FixedProfile::new("anthropic/claude-a", PromptCacheLifetime::Known(HOUR));
    let prefetch = reselecting(&conversation, &classifier)
        .with_prompt_cache_profiles(Arc::clone(&profile) as Arc<dyn PromptCacheProfileSource>);
    let opening = conversation.turn("open a github issue").await;
    let first = serve(&prefetch, &opening, &inner).await;
    conversation
        .model_called(&opening, MINUTE, Some("anthropic/claude-a"))
        .await;
    classifier.answer(Ok(calendar_answer()));

    // Compaction rewrites the messages behind the tools; the cache of the
    // tools part stays warm.
    conversation
        .threads
        .create_summary_artifact(CreateSummaryArtifactRequest {
            scope: thread_scope(),
            thread_id: conversation.thread_id.clone(),
            start_sequence: 1,
            end_sequence: 1,
            summary_kind: ironclaw_threads::SummaryKind::Compaction,
            content: MessageContent::text("summary"),
            model_context_policy: None,
            context_mode: None,
        })
        .await
        .expect("summary");
    let compacted = conversation.turn("what is on my calendar?").await;
    assert_eq!(
        encoded(&serve(&prefetch, &compacted, &inner).await),
        encoded(&first)
    );
    conversation
        .model_called(&compacted, Duration::ZERO, Some("anthropic/claude-a"))
        .await;

    profile.set_model("anthropic/claude-b");
    let switched = conversation.turn("and tomorrow?").await;
    let tools = serve(&prefetch, &switched, &inner).await;
    assert!(names(&tools).contains(&"calendar__list_events".to_string()));
    assert_eq!(
        conversation.reasons().await,
        [
            ToolSelectionReason::Initial,
            ToolSelectionReason::ModelChange
        ]
    );
}

#[tokio::test]
async fn sticky_tools_stay_selected_and_count_first() {
    let conversation = Conversation::new("sticky").await;
    let inner = CatalogPort::new(catalog_definitions());
    let classifier = SwitchableClassifier::answering(github_answer());
    let prefetch = reselecting(&conversation, &classifier);
    let decorator = decorator(Some(prefetch.clone()));
    let opening = conversation
        .turn("open a github issue and email the team")
        .await;
    let (port, first) = advertised_tools(
        &decorator,
        &opening,
        &inner,
        CapabilitySurfacePolicy::allow_all(),
    )
    .await;
    assert!(!names(&first).contains(&"gmail__send_message".to_string()));

    // The model reaches the deferred gmail tool through `tool_call`.
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
    conversation.model_called(&opening, 2 * HOUR, None).await;

    classifier.answer(Ok(calendar_answer()));
    let later = conversation.turn("what is on my calendar?").await;
    let tools = serve(&prefetch, &later, &inner).await;
    assert!(
        names(&tools).contains(&"gmail__send_message".to_string()),
        "sticky"
    );
    assert!(names(&tools).contains(&"calendar__list_events".to_string()));
    let request = classifier.last_request();
    assert_eq!(request.called_tools, ["gmail__send_message"]);
    assert!(request.is_pinned("gmail__send_message"));
    // Three bridges, result_read, and the sticky tool.
    assert_eq!(request.reserved_tools, 5);
    let entry = conversation
        .history()
        .await
        .expect("history")
        .entries
        .remove(1);
    assert!(
        !entry
            .scores
            .iter()
            .any(|score| score.name == "gmail__send_message"),
        "a sticky tool is kept, not scored"
    );
}

#[tokio::test]
#[tracing_test::traced_test]
async fn an_unchanged_or_failed_reselection_keeps_the_selection_and_logs_no_text() {
    let conversation = Conversation::new("unchanged").await;
    let inner = CatalogPort::new(catalog_definitions());
    let classifier = SwitchableClassifier::answering(github_answer());
    let prefetch = reselecting(&conversation, &classifier);
    let opening = conversation.turn("open a github issue").await;
    let first = serve(&prefetch, &opening, &inner).await;

    // Unchanged: classified again, nothing appended.
    conversation.model_called(&opening, 2 * HOUR, None).await;
    let same = conversation
        .turn("file one more about the zebra-quartz-secret")
        .await;
    assert_eq!(
        encoded(&serve(&prefetch, &same, &inner).await),
        encoded(&first)
    );
    assert_eq!(classifier.calls(), 2);
    assert_eq!(conversation.reasons().await, [ToolSelectionReason::Initial]);
    assert!(logs_contain(
        "tool re-selection chose the selection in force"
    ));

    // Failed: the selection in force is kept and nothing is appended.
    conversation.model_called(&same, 2 * HOUR, None).await;
    classifier.answer(Err(ToolSelectionError::RateLimited));
    let failed = conversation.turn("what is on my calendar?").await;
    assert_eq!(
        encoded(&serve(&prefetch, &failed, &inner).await),
        encoded(&first)
    );
    assert_eq!(conversation.reasons().await, [ToolSelectionReason::Initial]);
    assert!(logs_contain(
        "tool re-selection failed; keeping the selection in force"
    ));
    assert!(logs_contain("rate_limited"));

    // A successful re-selection logs counts and names, never the window.
    conversation.model_called(&failed, 2 * HOUR, None).await;
    classifier.answer(Ok(calendar_answer()));
    let drifted = conversation
        .turn("calendar for the zebra-quartz-secret")
        .await;
    serve(&prefetch, &drifted, &inner).await;
    assert!(logs_contain(
        "re-selected the conversation's tools from the conversation so far"
    ));
    assert!(!logs_contain("zebra-quartz-secret"));
    assert!(!logs_contain("calendar for the"));
}

#[tokio::test]
async fn a_revocation_reselects_at_the_same_moment_or_still_removes_on_failure() {
    let conversation = Conversation::new("revoked-reselect").await;
    let inner = CatalogPort::new(catalog_definitions());
    let classifier = SwitchableClassifier::answering(github_answer());
    let prefetch = reselecting(&conversation, &classifier);
    let opening = conversation.turn("open a github issue").await;
    serve(&prefetch, &opening, &inner).await;
    // The cache is warm; the revocation alone re-selects.
    conversation.model_called(&opening, MINUTE, None).await;

    classifier.answer(Ok(vec![
        ("github__create_issue", 0.9),
        ("github__get_repo", 0.7),
    ]));
    let policy = || {
        CapabilitySurfacePolicy::allow_all()
            .deny_capability_ids([CapabilityId::new("github.list_issues").expect("id")])
    };
    let revoked_turn = conversation.turn("and the repo?").await;
    let (_, tools) = advertised_tools(
        &decorator(Some(prefetch.clone())),
        &revoked_turn,
        &inner,
        policy(),
    )
    .await;
    assert!(!names(&tools).contains(&"github__list_issues".to_string()));
    assert!(
        names(&tools).contains(&"github__get_repo".to_string()),
        "re-selected"
    );
    assert_eq!(
        conversation.reasons().await,
        [ToolSelectionReason::Initial, ToolSelectionReason::Revoked]
    );

    // A failed re-selection still removes the revoked tool.
    classifier.answer(Err(ToolSelectionError::RateLimited));
    let policy = || {
        CapabilitySurfacePolicy::allow_all().deny_capability_ids([
            CapabilityId::new("github.list_issues").expect("id"),
            CapabilityId::new("github.get_repo").expect("id"),
        ])
    };
    let again = conversation.turn("never mind the repo").await;
    let (_, tools) = advertised_tools(&decorator(Some(prefetch)), &again, &inner, policy()).await;
    assert!(!names(&tools).contains(&"github__get_repo".to_string()));
    assert!(names(&tools).contains(&"github__create_issue".to_string()));
    assert_eq!(
        conversation.reasons().await,
        [
            ToolSelectionReason::Initial,
            ToolSelectionReason::Revoked,
            ToolSelectionReason::Revoked
        ]
    );
}

#[test]
fn reselection_is_off_unless_configured() {
    let reselection = ToolReselectionConfig::new(true, HOUR, MINUTE);
    assert!(reselection.enabled());
    assert!(!ToolReselectionConfig::disabled().enabled());
    assert!(
        !config(ToolPrefetchRanking::Lexical, 100, &[])
            .reselection()
            .enabled(),
        "re-selection is off unless configured"
    );
}

/// Filler prose that shares no term with the test catalog: it pushes each
/// message well past the 32 query terms `tool_search` reads.
fn filler(words: usize) -> String {
    const LOREM: [&str; 12] = [
        "lorem",
        "ipsum",
        "dolor",
        "amet",
        "consectetur",
        "adipiscing",
        "elit",
        "sed",
        "tempor",
        "incididunt",
        "labore",
        "magna",
    ];
    (0..words)
        .map(|index| format!("{}{}", LOREM[index % LOREM.len()], index / LOREM.len()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Two topics, one message each, each far past 32 terms. A single query
/// joined newest first would read only the calendar message's opening
/// terms; ranking each message separately and merging by rank gives each
/// topic its best tool, even with room for only two ranked tools.
#[tokio::test]
async fn a_two_topic_conversation_gets_tools_for_both_topics() {
    let conversation = Conversation::new("two-topics").await;
    let inner = CatalogPort::new(catalog_definitions());
    // Floor: three bridges plus result_read, so two slots are left.
    let prefetch = conversation.prefetch(
        ranked_config(ToolPrefetchRanking::Lexical, 6)
            .with_reselection(ToolReselectionConfig::new(true, HOUR, MINUTE)),
        Arc::new(crate::tool_search::NativeBm25fToolRetrieval),
    );
    let github = format!("Create a GitHub issue in the repository. {}", filler(60));
    let opening = conversation.turn(&github).await;
    let first = serve(&prefetch, &opening, &inner).await;
    assert!(names(&first).contains(&"github__create_issue".to_string()));
    conversation
        .model_called(&opening, HOUR + 2 * MINUTE, None)
        .await;

    let calendar = format!("List my calendar events for the day. {}", filler(60));
    let drifted = conversation.turn(&calendar).await;
    let second = names(&serve(&prefetch, &drifted, &inner).await);
    assert!(
        second.contains(&"calendar__list_events".to_string()),
        "{second:?}"
    );
    assert!(
        second.iter().any(|name| name.starts_with("github__")),
        "the older topic keeps its best tool: {second:?}"
    );
    assert_eq!(second.len(), 6, "{second:?}");
    assert_eq!(
        conversation.reasons().await,
        [ToolSelectionReason::Initial, ToolSelectionReason::CacheCold]
    );
}
