//! Turn-start selection driven through its caller, the tool-disclosure port:
//! what a run advertises is read from `tool_definitions()`.

use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_host_api::{
    ids::{AgentId, CapabilityId, ProviderToolName, TenantId, ThreadId},
    resolution::{Resolution, ResolutionBatch},
    turn::{AcceptedMessageRef, TurnId, TurnRunId, TurnScope},
};
use ironclaw_loop_contracts::{
    AgentLoopHostError, CapabilityDescriptorView, CapabilitySurfaceVersion,
    InMemoryRunProfileResolver, LoopCapabilityPort, LoopRequest, LoopRequestBatch, LoopSafeSummary,
    ProviderToolDefinition, RunProfileResolutionRequest, RunProfileResolver, ToolSelection,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
};
use serde_json::json;

use super::*;
use crate::{
    CapabilityResultWrite, CapabilityWriteResult, LoopCapabilityResultWriter,
    ToolDisclosureCapabilityDecorator, ToolDisclosureMode,
};

const DEFERRED_TOOLS: usize = 40;

/// The authorized catalog: one core tool and enough deferred tools that the
/// surface defers.
struct CatalogPort {
    definitions: Vec<ProviderToolDefinition>,
}

impl CatalogPort {
    fn wide() -> Self {
        let mut definitions = vec![definition("builtin.read_file", "builtin__read_file")];
        definitions.extend((0..DEFERRED_TOOLS).map(|index| {
            definition(
                &format!("fixture.tool_{index:02}"),
                &format!("tool_{index:02}"),
            )
        }));
        Self { definitions }
    }
}

fn definition(capability_id: &str, name: &str) -> ProviderToolDefinition {
    ProviderToolDefinition {
        capability_id: CapabilityId::new(capability_id).expect("valid capability id"),
        name: ProviderToolName::new(name).expect("valid tool name"),
        description: format!("Fixture tool {name}."),
        description_trust: Default::default(),
        parameters: json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"]
        }),
    }
}

#[async_trait]
impl LoopCapabilityPort for CatalogPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        Ok(self.definitions.clone())
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("surface:selection").expect("valid version"),
            descriptors: self
                .definitions
                .iter()
                .cloned()
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
        _request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        unreachable!("selection tests do not dispatch")
    }

    async fn invoke_capability_batch(
        &self,
        _request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        unreachable!("selection tests do not dispatch")
    }
}

struct NoopWriter;

#[async_trait]
impl LoopCapabilityResultWriter for NoopWriter {
    async fn write_capability_result(
        &self,
        _write: CapabilityResultWrite<'_>,
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        unreachable!("selection tests do not write results")
    }
}

/// A classifier that answers with a script and records what it was asked.
#[derive(Debug)]
struct ScriptedClassifier {
    answer: Result<Vec<ChosenTool>, ToolSelectionError>,
    calls: AtomicUsize,
    last_request: Mutex<Option<ToolSelectionRequest>>,
}

impl ScriptedClassifier {
    fn new(answer: Result<Vec<ChosenTool>, ToolSelectionError>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            calls: AtomicUsize::new(0),
            last_request: Mutex::new(None),
        })
    }

    fn choosing(names: &[&str]) -> Arc<Self> {
        Self::new(Ok(names
            .iter()
            .map(|name| ChosenTool::new(*name, 0.9))
            .collect()))
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
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
    ) -> Result<ToolSelection, ToolSelectionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_request.lock().expect("request lock") = Some(request.clone());
        self.answer.clone().map(|chosen| ToolSelection {
            chosen,
            scorer: "scripted:v1".to_string(),
        })
    }
}

/// One conversation: a thread store, its scope, and runs over it.
struct Conversation {
    threads: Arc<InMemorySessionThreadService>,
    scope: ThreadScope,
    thread_id: ThreadId,
}

impl Conversation {
    async fn new() -> Self {
        let threads = Arc::new(InMemorySessionThreadService::default());
        let scope = ThreadScope {
            tenant_id: TenantId::new("tenant-selection").expect("tenant"),
            agent_id: AgentId::new("agent-selection").expect("agent"),
            project_id: None,
            owner_user_id: None,
            mission_id: None,
        };
        let thread_id = threads
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(ThreadId::new("thread-selection").expect("thread")),
                created_by_actor_id: "actor".into(),
                title: None,
                metadata_json: None,
            })
            .await
            .expect("thread")
            .thread_id;
        Self {
            threads,
            scope,
            thread_id,
        }
    }

    /// A run for a new turn. With `text`, the turn has an accepted user
    /// message; without, it has none (a trigger fire).
    async fn run(&self, text: Option<&str>) -> LoopRunContext {
        let turn_scope = TurnScope::new(
            self.scope.tenant_id.clone(),
            Some(self.scope.agent_id.clone()),
            None,
            self.thread_id.clone(),
        );
        let profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("run profile resolves");
        let run = LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), profile);
        let Some(text) = text else {
            return run;
        };
        let accepted = self
            .threads
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: self.scope.clone(),
                thread_id: self.thread_id.clone(),
                actor_id: "actor".into(),
                source_binding_id: None,
                reply_target_binding_id: None,
                external_event_id: None,
                content: MessageContent::text(text),
            })
            .await
            .expect("accepted message");
        run.with_accepted_message_ref(
            AcceptedMessageRef::new(format!("msg:{}", accepted.message_id)).expect("message ref"),
        )
    }

    /// The tool names one run advertises with selection bound to
    /// `classifier`, under `policy`.
    async fn advertised_under(
        &self,
        classifier: &Arc<ScriptedClassifier>,
        run: &LoopRunContext,
        policy: CapabilitySurfacePolicy,
    ) -> Vec<String> {
        let config = ToolSelectionConfig::new(3, 10_000, classifier.clone()).expect("config");
        let port = ToolDisclosureCapabilityDecorator::new(
            Arc::new(NoopWriter),
            ToolDisclosureMode::Bridged,
        )
        .with_tool_selection(config, self.threads.clone(), self.scope.clone())
        .decorate_with_policy(run, Arc::new(CatalogPort::wide()), Arc::new(policy));
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        port.tool_definitions()
            .expect("tool definitions")
            .into_iter()
            .map(|definition| definition.name.to_string())
            .collect()
    }

    async fn advertised(
        &self,
        classifier: &Arc<ScriptedClassifier>,
        run: &LoopRunContext,
    ) -> Vec<String> {
        self.advertised_under(classifier, run, CapabilitySurfacePolicy::allow_all())
            .await
    }

    async fn record(&self) -> Option<ToolSelectionRecord> {
        self.threads
            .read_tool_selection(&self.scope, &self.thread_id)
            .await
            .expect("record read")
    }
}

/// What the ordinary deferred surface advertises: the core tool and bridges.
const ORDINARY: [&str; 4] = [
    "builtin__read_file",
    "tool_search",
    "tool_describe",
    "tool_call",
];

fn ordinary_plus(selected: &[&str]) -> Vec<String> {
    ORDINARY
        .iter()
        .chain(selected)
        .map(|name| name.to_string())
        .collect()
}

#[tokio::test]
async fn the_first_turn_selects_and_later_turns_reuse_the_record() {
    let conversation = Conversation::new().await;
    let classifier = ScriptedClassifier::choosing(&["tool_07", "tool_02"]);

    let first = conversation
        .run(Some("file an issue about the outage"))
        .await;
    assert_eq!(
        conversation.advertised(&classifier, &first).await,
        ordinary_plus(&["tool_07", "tool_02"]),
        "selected tools follow the ordinary surface, in the classifier's order"
    );
    let request = classifier
        .last_request
        .lock()
        .expect("request lock")
        .clone()
        .expect("the classifier was asked");
    assert_eq!(
        request.context.user_messages(),
        ["file an issue about the outage"]
    );
    assert_eq!(
        request.candidates.len(),
        DEFERRED_TOOLS,
        "core tools and bridges are not candidates"
    );
    assert_eq!((request.max_tools, request.token_budget), (3, 10_000));
    let record = conversation.record().await.expect("recorded");
    assert_eq!(record.turn_id, first.turn_id);
    assert_eq!(record.scorer.as_deref(), Some("scripted:v1"));

    // A later turn, and a fresh port for it, rebuilds the list from the
    // record without asking the classifier, whatever it would say now.
    let changed = ScriptedClassifier::choosing(&["tool_30"]);
    let second = conversation.run(Some("now something unrelated")).await;
    assert_eq!(
        conversation.advertised(&changed, &second).await,
        ordinary_plus(&["tool_07", "tool_02"])
    );
    assert_eq!((classifier.calls(), changed.calls()), (1, 0));
}

#[tokio::test]
async fn an_untrusted_answer_is_repaired_before_it_is_recorded() {
    let conversation = Conversation::new().await;
    let classifier = ScriptedClassifier::new(Ok(vec![
        ChosenTool::new("not_in_the_catalog", 0.99),
        ChosenTool::new("builtin__read_file", 0.98),
        ChosenTool::new("tool_01", f32::NAN),
        ChosenTool::new("tool_05", 0.9),
        ChosenTool::new("tool_05", 0.8),
        ChosenTool::new("tool_06", 0.7),
        ChosenTool::new("tool_07", 0.6),
        ChosenTool::new("tool_08", 0.5),
    ]));
    let run = conversation.run(Some("do the thing")).await;

    assert_eq!(
        conversation.advertised(&classifier, &run).await,
        ordinary_plus(&["tool_05", "tool_06", "tool_07"]),
        "unknown, core, invalid and duplicate answers are dropped, then max_tools applies"
    );
}

#[tokio::test]
async fn a_classifier_failure_is_recorded_and_keeps_the_ordinary_surface() {
    let conversation = Conversation::new().await;
    let failing = ScriptedClassifier::new(Err(ToolSelectionError::Unavailable {
        reason: LoopSafeSummary::new("classifier backend unreachable").expect("summary"),
    }));
    let first = conversation.run(Some("do the thing")).await;
    assert_eq!(
        conversation.advertised(&failing, &first).await,
        ordinary_plus(&[])
    );
    let record = conversation
        .record()
        .await
        .expect("the failure is recorded");
    assert_eq!(
        record.fallback_reason,
        Some(ToolSelectionFallbackReason::Unavailable)
    );

    // The conversation does not switch surfaces once the classifier recovers.
    let recovered = ScriptedClassifier::choosing(&["tool_03"]);
    let second = conversation.run(Some("try again")).await;
    assert_eq!(
        conversation.advertised(&recovered, &second).await,
        ordinary_plus(&[])
    );
    assert_eq!(recovered.calls(), 0);
}

#[tokio::test]
async fn a_run_without_user_text_records_nothing_and_a_later_turn_selects() {
    let conversation = Conversation::new().await;
    let classifier = ScriptedClassifier::choosing(&["tool_04"]);

    for run in [
        conversation.run(None).await,
        conversation.run(Some("   ")).await,
    ] {
        assert_eq!(
            conversation.advertised(&classifier, &run).await,
            ordinary_plus(&[])
        );
    }
    assert_eq!(classifier.calls(), 0);
    assert_eq!(conversation.record().await, None);

    let run = conversation.run(Some("a real request")).await;
    assert_eq!(
        conversation.advertised(&classifier, &run).await,
        ordinary_plus(&["tool_04"])
    );
}

#[tokio::test]
async fn a_record_that_cannot_be_read_keeps_the_ordinary_surface_without_selecting() {
    let conversation = Conversation::new().await;
    let classifier = ScriptedClassifier::choosing(&["tool_04"]);
    // The store knows no such thread, so the record read fails.
    let mut run = conversation.run(Some("do the thing")).await;
    run.thread_id = ThreadId::new("thread-unknown").expect("thread");

    assert_eq!(
        conversation.advertised(&classifier, &run).await,
        ordinary_plus(&[])
    );
    assert_eq!(
        classifier.calls(),
        0,
        "no list is chosen that cannot be kept"
    );
    assert_eq!(conversation.record().await, None);
}

#[tokio::test]
async fn a_selected_tool_that_loses_authorization_is_no_longer_advertised() {
    let conversation = Conversation::new().await;
    let classifier = ScriptedClassifier::choosing(&["tool_07", "tool_02"]);
    let first = conversation.run(Some("do the thing")).await;
    conversation.advertised(&classifier, &first).await;

    let revoked = CapabilitySurfacePolicy::allow_all()
        .deny_capability_ids([CapabilityId::new("fixture.tool_07").expect("capability id")]);
    let second = conversation.run(Some("and again")).await;
    assert_eq!(
        conversation
            .advertised_under(&classifier, &second, revoked)
            .await,
        ordinary_plus(&["tool_02"])
    );
}

#[test]
fn settings_outside_their_bounds_are_refused() {
    let classifier = ScriptedClassifier::choosing(&[]);
    for max_tools in [0, MAX_TOOL_SELECTION_TOOLS + 1] {
        assert!(matches!(
            ToolSelectionConfig::new(max_tools, 1_000, classifier.clone()),
            Err(ToolSelectionConfigError::MaxToolsOutOfRange { .. })
        ));
    }
    assert_eq!(
        ToolSelectionConfig::new(8, 0, classifier.clone()).err(),
        Some(ToolSelectionConfigError::ZeroTokenBudget)
    );
}

#[test]
fn a_long_request_is_cut_on_a_character_boundary() {
    let text = "é".repeat(MAX_CONVERSATION_CONTEXT_BYTES);
    let cut = bounded(&text);
    assert!(cut.len() <= MAX_CONVERSATION_CONTEXT_BYTES);
    assert!(cut.chars().all(|c| c == 'é'));
    assert_eq!(bounded("short"), "short");
}
