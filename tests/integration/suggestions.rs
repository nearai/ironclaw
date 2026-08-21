//! ProductSurface suggestion generation: this is deliberately a backend-only
//! caller-path test.  It must exercise composition, the canonical unbound
//! structured-output loop, and durable thread state without mounting WebUI.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_assistant::{THREADS_VIEW, TIMELINE_VIEW};
use ironclaw_composition::{
    RebornRuntime, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
    standalone_runtime_policy,
};
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{TurnRunId, TurnScope, TurnStatus};
use ironclaw_llm::testing::provider_chain_over;
use ironclaw_llm::{CompletionResponseFormat, LlmProvider, SessionConfig, create_session_manager};
use ironclaw_loop_host::{HostManagedModelGateway, LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_product_contracts::inbound_requests::{
    ProductListThreadsRequest, RebornSuggestionDismissRequest, RebornSuggestionStartRequest,
    RebornSuggestionsGenerateRequest, RebornSuggestionsListRequest,
};
use ironclaw_product_contracts::product_wire::{
    RebornSuggestionGenerationStatus, RebornSuggestionsResponse, RebornTimelineRequest,
};
use ironclaw_product_contracts::suggestions::{
    SUGGESTION_DISMISS_COMMAND, SUGGESTION_START_COMMAND, SUGGESTIONS_GENERATE_COMMAND,
    SUGGESTIONS_LIST_VIEW,
};
use ironclaw_product_contracts::surface::{BoundProductSurface, ProductSurfaceCaller};
use ironclaw_product_contracts::surface::{ProductSurfaceErrorCode, ProductSurfaceValidationCode};
use ironclaw_threads::{MessageKind, read_declarations_for_run_scope};
use ironclaw_turns::{GetRunStateRequest, TurnCoordinator};
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::{
    ErrLlm, ErrLlmKind, ParkingModelGate, SCRIPTED_MODEL_NAME, parking_trace_llm,
    scripted_trace_llm,
};
use serde_json::json;
use support::trace_llm::TraceLlm;
use tempfile::{TempDir, tempdir};

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

async fn build_suggestions_runtime(
    storage_root: &Path,
    tenant_id: &TenantId,
    agent_id: &AgentId,
    model_gateway: Arc<dyn HostManagedModelGateway>,
) -> HarnessResult<Box<RebornRuntime>> {
    let input = ironclaw_composition::local_filesystem_build_input(
        "suggestions-itest-user",
        storage_root.join("local-dev"),
    )
    .with_local_runtime_identity(tenant_id.clone(), agent_id.clone())
    .with_runtime_policy(standalone_runtime_policy()?)
    .with_bundled_first_party_for_test();
    Ok(Box::new(
        build_reborn_runtime(
            RebornRuntimeInput::from_build_input(input)
                .with_identity(RebornRuntimeIdentity {
                    tenant_id: tenant_id.as_str().to_string(),
                    agent_id: agent_id.as_str().to_string(),
                    source_binding_id: "suggestions-itest-source".to_string(),
                    reply_target_binding_id: "suggestions-itest-reply".to_string(),
                })
                .with_model_gateway_override(model_gateway),
        )
        .await?,
    ))
}

async fn build_suggestions_gateway(
    session_root: &Path,
    raw: Arc<dyn LlmProvider>,
) -> HarnessResult<Arc<dyn HostManagedModelGateway>> {
    let session = create_session_manager(SessionConfig {
        session_path: session_root.join("suggestions.session.json"),
        ..SessionConfig::default()
    })
    .await;
    let provider = provider_chain_over(
        raw,
        &ironclaw_llm::testing::nearai_test_config(SCRIPTED_MODEL_NAME),
        session,
    )
    .await?;
    let model_profile_id = ironclaw_loop_contracts::ModelProfileId::new("interactive_model")
        .map_err(|reason| format!("invalid model profile id: {reason}"))?;
    Ok(Arc::new(LlmProviderModelGateway::new(
        provider,
        LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, None),
    )))
}

fn bound_surface(
    runtime: &RebornRuntime,
    tenant_id: &TenantId,
    user_id: &UserId,
    agent_id: &AgentId,
) -> HarnessResult<BoundProductSurface> {
    Ok(BoundProductSurface::new(
        runtime.product_surface(None)?,
        ProductSurfaceCaller::new(
            tenant_id.clone(),
            user_id.clone(),
            Some(agent_id.clone()),
            None,
        ),
    ))
}

async fn wait_for_completed_run(
    coordinator: &Arc<dyn TurnCoordinator>,
    scope: TurnScope,
    run_id: TurnRunId,
) -> HarnessResult<()> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let state = coordinator
            .get_run_state(GetRunStateRequest {
                scope: scope.clone(),
                run_id,
            })
            .await?;
        if state.status == TurnStatus::Completed {
            return Ok(());
        }
        if state.status.is_terminal() || std::time::Instant::now() > deadline {
            return Err(format!(
                "suggestion-start run did not complete; status={:?} failure={:?}",
                state.status, state.failure
            )
            .into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_ready_suggestions(
    surface: &BoundProductSurface,
) -> HarnessResult<RebornSuggestionsResponse> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let response = SUGGESTIONS_LIST_VIEW
            .query_on(surface, RebornSuggestionsListRequest::default(), None)
            .await?;
        match response.status {
            RebornSuggestionGenerationStatus::Ready => return Ok(response),
            RebornSuggestionGenerationStatus::Failed => {
                return Err("suggestion generation failed".into());
            }
            RebornSuggestionGenerationStatus::Empty
            | RebornSuggestionGenerationStatus::Generating
                if std::time::Instant::now() <= deadline => {}
            RebornSuggestionGenerationStatus::Empty
            | RebornSuggestionGenerationStatus::Generating => {
                return Err("suggestion generation did not become ready".into());
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_failed_suggestions(
    surface: &BoundProductSurface,
) -> HarnessResult<RebornSuggestionsResponse> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let response = SUGGESTIONS_LIST_VIEW
            .query_on(surface, RebornSuggestionsListRequest::default(), None)
            .await?;
        match response.status {
            RebornSuggestionGenerationStatus::Failed => return Ok(response),
            RebornSuggestionGenerationStatus::Ready => {
                return Err("suggestion generation unexpectedly became ready".into());
            }
            RebornSuggestionGenerationStatus::Empty
            | RebornSuggestionGenerationStatus::Generating
                if std::time::Instant::now() <= deadline => {}
            RebornSuggestionGenerationStatus::Empty
            | RebornSuggestionGenerationStatus::Generating => {
                return Err("suggestion generation did not become failed".into());
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

struct SuggestionsFixture {
    root: TempDir,
    _session_root: TempDir,
    tenant_id: TenantId,
    agent_id: AgentId,
    user_id: UserId,
    generation_gate: ParkingModelGate,
    scripted_llm: Arc<TraceLlm>,
    gateway: Arc<dyn HostManagedModelGateway>,
}

impl SuggestionsFixture {
    async fn new() -> HarnessResult<Self> {
        Self::with_finalization(json!({
            "suggestions": [{
                "title": "Triage inbox",
                "description": "Review and prioritize unread email.",
                "suggested_prompt": "Triage my unread inbox.",
                "icon": "web",
                "sources": ["Web Search"],
            }]
        }))
        .await
    }

    async fn with_finalization(finalization: serde_json::Value) -> HarnessResult<Self> {
        let root = tempdir()?;
        let session_root = tempdir()?;
        let tenant_id = TenantId::new("suggestions-itest-tenant")?;
        let agent_id = AgentId::new("suggestions-itest-agent")?;
        let user_id = UserId::new("suggestions-itest-user")?;
        let replacement_cards = json!([{
            "title": "Review calendar",
            "description": "Review upcoming events and commitments.",
            "suggested_prompt": "Review my upcoming calendar.",
            "icon": "generic",
            "sources": ["Google Calendar"],
        }]);
        let scripted_llm: Arc<TraceLlm> = Arc::new(scripted_trace_llm([
            RebornScriptedReply::text("I recommend triaging the inbox."),
            RebornScriptedReply::text(serde_json::to_string(&finalization)?),
            RebornScriptedReply::text("I recommend reviewing the calendar."),
            RebornScriptedReply::text(serde_json::to_string(&json!({
                "suggestions": replacement_cards
            }))?),
            RebornScriptedReply::text("Inbox triage started."),
        ]));
        let generation_gate = ParkingModelGate::new();
        let raw: Arc<dyn LlmProvider> = Arc::new(parking_trace_llm(
            generation_gate.clone(),
            scripted_llm.clone(),
        ));
        let gateway = build_suggestions_gateway(session_root.path(), raw).await?;
        Ok(Self {
            root,
            _session_root: session_root,
            tenant_id,
            agent_id,
            user_id,
            generation_gate,
            scripted_llm,
            gateway,
        })
    }

    async fn start_runtime(&self) -> HarnessResult<Box<RebornRuntime>> {
        build_suggestions_runtime(
            self.root.path(),
            &self.tenant_id,
            &self.agent_id,
            Arc::clone(&self.gateway),
        )
        .await
    }

    fn surface(&self, runtime: &RebornRuntime) -> HarnessResult<BoundProductSurface> {
        bound_surface(runtime, &self.tenant_id, &self.user_id, &self.agent_id)
    }
}

async fn generate_initial_suggestions(
    fixture: &SuggestionsFixture,
    runtime: &RebornRuntime,
    surface: &BoundProductSurface,
) -> HarnessResult<RebornSuggestionsResponse> {
    let invalid_generation = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "\n".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await
        .expect_err("invalid generation action ids must be rejected before claiming");
    assert_eq!(
        invalid_generation.code,
        ProductSurfaceErrorCode::InvalidRequest
    );
    assert_eq!(
        invalid_generation.validation_code,
        Some(ProductSurfaceValidationCode::InvalidId)
    );
    let after_invalid_generation = SUGGESTIONS_LIST_VIEW
        .query_on(surface, RebornSuggestionsListRequest::default(), None)
        .await?;
    assert_eq!(
        after_invalid_generation.status,
        RebornSuggestionGenerationStatus::Empty,
        "pre-claim validation must not leave a generating lease behind"
    );

    let first_surface = surface.clone();
    let first_generate = tokio::spawn(async move {
        SUGGESTIONS_GENERATE_COMMAND
            .invoke_on(
                &first_surface,
                RebornSuggestionsGenerateRequest {
                    client_action_id: "suggestions-action-1".to_string(),
                },
                ironclaw_host_api::ids::ActivityId::new(),
            )
            .await
    });
    tokio::time::timeout(
        Duration::from_secs(10),
        fixture.generation_gate.wait_until_parked(),
    )
    .await?;
    let first_join = tokio::time::timeout(Duration::from_secs(10), first_generate)
        .await
        .map_err(|_| "generate request waited for the parked model")?;
    let first_result = first_join?;
    let first_response = first_result?;
    assert_eq!(
        first_response.status,
        RebornSuggestionGenerationStatus::Generating
    );
    let generation_id = first_response
        .generation_id
        .as_deref()
        .ok_or("generating response omitted generation id")?;
    let suggestion_thread_id = ThreadId::new(format!("suggestions-{generation_id}"))?;
    let suggestion_run_scope = TurnScope::new_with_owner(
        fixture.tenant_id.clone(),
        Some(fixture.agent_id.clone()),
        None,
        suggestion_thread_id,
        Some(fixture.user_id.clone()),
    );
    let declarations = read_declarations_for_run_scope(
        runtime.session_thread_service().as_ref(),
        &suggestion_run_scope,
    )
    .await?
    .ok_or("suggestion run omitted prepared declarations")?;
    let declared_tool_ids = declarations
        .tools
        .iter()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        declared_tool_ids,
        BTreeSet::from([
            "builtin.extension_search".to_string(),
            "ironclaw.memory.read".to_string(),
            "ironclaw.memory.search".to_string(),
            "ironclaw.memory.tree".to_string(),
        ]),
        "prepared suggestion context must carry the exact discovery-only allowlist"
    );

    let concurrent_surface = surface.clone();
    let concurrent_generate = tokio::spawn(async move {
        SUGGESTIONS_GENERATE_COMMAND
            .invoke_on(
                &concurrent_surface,
                RebornSuggestionsGenerateRequest {
                    client_action_id: "suggestions-action-1".to_string(),
                },
                ironclaw_host_api::ids::ActivityId::new(),
            )
            .await
    });
    let concurrent_response = concurrent_generate.await??;
    assert_eq!(
        concurrent_response.status,
        RebornSuggestionGenerationStatus::Generating
    );
    assert_eq!(
        first_response.generation_id,
        concurrent_response.generation_id
    );
    let conflicting = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "suggestions-action-conflict".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await
        .expect_err("a different action must conflict while generation is running");
    assert_eq!(conflicting.code, ProductSurfaceErrorCode::Conflict);
    fixture.generation_gate.release();
    let generated = wait_for_ready_suggestions(surface).await?;
    assert_eq!(generated.suggestions.len(), 1);
    assert_eq!(generated.suggestions[0].icon, "web");
    assert_eq!(
        generated.suggestions[0].sources,
        vec!["Web Search".to_string()]
    );
    assert_eq!(generated.suggestions[0].title, "Triage inbox");
    assert_eq!(
        generated.suggestions[0].description,
        "Review and prioritize unread email."
    );
    assert_eq!(
        generated.suggestions[0].suggested_prompt,
        "Triage my unread inbox."
    );
    assert!(generated.suggestions[0].thread_id.is_none());
    Ok(generated)
}

async fn replace_suggestions(
    fixture: &SuggestionsFixture,
    surface: &BoundProductSurface,
) -> HarnessResult<RebornSuggestionsResponse> {
    fixture.generation_gate.rearm();
    let replacement_surface = surface.clone();
    let replacement_generate = tokio::spawn(async move {
        SUGGESTIONS_GENERATE_COMMAND
            .invoke_on(
                &replacement_surface,
                RebornSuggestionsGenerateRequest {
                    client_action_id: "suggestions-action-2".to_string(),
                },
                ironclaw_host_api::ids::ActivityId::new(),
            )
            .await
    });
    tokio::time::timeout(
        Duration::from_secs(10),
        fixture.generation_gate.wait_until_parked(),
    )
    .await?;
    let replacement_join = tokio::time::timeout(Duration::from_secs(10), replacement_generate)
        .await
        .map_err(|_| "replacement request waited for the parked model")?;
    let replacement_result = replacement_join?;
    let replacing = replacement_result?;
    assert_eq!(
        replacing.status,
        RebornSuggestionGenerationStatus::Generating
    );
    assert!(
        replacing.suggestions.is_empty(),
        "old cards must be hidden as soon as a new generation starts"
    );
    fixture.generation_gate.release();
    let replaced = wait_for_ready_suggestions(surface).await?;
    assert_eq!(replaced.suggestions.len(), 1);
    assert_eq!(replaced.suggestions[0].title, "Review calendar");
    assert!(
        !replaced
            .suggestions
            .iter()
            .any(|suggestion| suggestion.title == "Triage inbox")
    );
    assert_eq!(fixture.scripted_llm.captured_requests().len(), 4);
    Ok(replaced)
}

async fn start_replacement_suggestion(
    fixture: &SuggestionsFixture,
    runtime: &RebornRuntime,
    surface: &BoundProductSurface,
    suggestion_id: String,
) -> HarnessResult<ironclaw_product_contracts::product_wire::RebornSuggestionStartResponse> {
    let first_start = SUGGESTION_START_COMMAND.invoke_on(
        surface,
        RebornSuggestionStartRequest {
            suggestion_id: suggestion_id.clone(),
        },
        ironclaw_host_api::ids::ActivityId::new(),
    );
    let concurrent_start = SUGGESTION_START_COMMAND.invoke_on(
        surface,
        RebornSuggestionStartRequest {
            suggestion_id: suggestion_id.clone(),
        },
        ironclaw_host_api::ids::ActivityId::new(),
    );
    let (first_result, second_result) = tokio::join!(first_start, concurrent_start);
    let started = match (first_result, second_result) {
        (Ok(first), Ok(second)) => {
            assert_eq!(second, first);
            first
        }
        (Ok(first), Err(second)) => {
            assert_eq!(second.code, ProductSurfaceErrorCode::Unavailable);
            assert!(second.retryable);
            let replay = SUGGESTION_START_COMMAND
                .invoke_on(
                    surface,
                    RebornSuggestionStartRequest {
                        suggestion_id: suggestion_id.clone(),
                    },
                    ironclaw_host_api::ids::ActivityId::new(),
                )
                .await?;
            assert_eq!(replay, first);
            first
        }
        (Err(first), Ok(second)) => {
            assert_eq!(first.code, ProductSurfaceErrorCode::Unavailable);
            assert!(first.retryable);
            let replay = SUGGESTION_START_COMMAND
                .invoke_on(
                    surface,
                    RebornSuggestionStartRequest {
                        suggestion_id: suggestion_id.clone(),
                    },
                    ironclaw_host_api::ids::ActivityId::new(),
                )
                .await?;
            assert_eq!(replay, second);
            second
        }
        (Err(first), Err(second)) => {
            return Err(format!(
                "both concurrent starts failed: first={first:?}, second={second:?}"
            )
            .into());
        }
    };

    let started_thread_id = ThreadId::new(started.thread_id.clone())?;
    let started_run_id = TurnRunId::parse(&started.run_id)?;
    wait_for_completed_run(
        &runtime.product_turn_coordinator_for_test(),
        TurnScope::new_with_owner(
            fixture.tenant_id.clone(),
            Some(fixture.agent_id.clone()),
            None,
            started_thread_id,
            Some(fixture.user_id.clone()),
        ),
        started_run_id,
    )
    .await?;
    Ok(started)
}

/// The first caller-path proof for the backend-only suggestions contract.
///
/// The generation path, native structured-output contract, and idempotent replay
/// are kept together because they all exercise the same initial durable run.
#[tokio::test(flavor = "multi_thread")]
async fn generate_suggestions_returns_cards_and_cached_replay() -> HarnessResult<()> {
    let fixture = SuggestionsFixture::new().await?;
    let runtime = fixture.start_runtime().await?;
    let surface = fixture.surface(&runtime)?;

    let generated = generate_initial_suggestions(&fixture, &runtime, &surface).await?;
    let cached = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "suggestions-action-1".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await?;
    assert_eq!(cached, generated);
    assert_eq!(fixture.scripted_llm.captured_requests().len(), 2);
    let generation_tools = fixture.scripted_llm.captured_tool_definitions();
    assert_eq!(generation_tools.len(), 2);
    let expected_generation_tool_names = BTreeSet::from([
        "builtin__extension_search".to_string(),
        "ironclaw__memory__read".to_string(),
        "ironclaw__memory__search".to_string(),
        "ironclaw__memory__tree".to_string(),
    ]);
    let observed_generation_tool_names = generation_tools[0]
        .iter()
        .map(|definition| definition.name.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        observed_generation_tool_names, expected_generation_tool_names,
        "suggestion work phase must expose exactly the read-only discovery allowlist"
    );
    assert!(
        generation_tools[1].is_empty(),
        "provider-native structured finalization must receive zero tools"
    );
    assert!(
        generation_tools
            .iter()
            .flatten()
            .all(|definition| definition.name != "builtin__structured_result"),
        "native structured generation must expose zero synthetic result tools"
    );
    let response_formats = fixture.scripted_llm.captured_response_formats();
    assert_eq!(response_formats.len(), 2);
    assert!(response_formats[0].is_none());
    let native_schema = match response_formats[1].as_ref() {
        Some(CompletionResponseFormat::JsonSchema(format)) => format,
        Some(CompletionResponseFormat::JsonObject) => {
            return Err("suggestion finalization must request a JSON schema".into());
        }
        None => return Err("suggestion finalization must request a JSON schema".into()),
    };
    assert_eq!(native_schema.name, "suggestions");
    assert!(native_schema.is_strict());
    assert_eq!(
        native_schema.schema["properties"]["suggestions"]["minItems"],
        json!(1)
    );
    assert_eq!(
        native_schema.schema["properties"]["suggestions"]["maxItems"],
        json!(5)
    );
    assert_eq!(
        native_schema.schema["properties"]["suggestions"]["items"]["additionalProperties"],
        json!(false)
    );
    assert_eq!(
        native_schema.schema["properties"]["suggestions"]["items"]["required"],
        json!([
            "title",
            "description",
            "suggested_prompt",
            "icon",
            "sources"
        ])
    );

    runtime.shutdown().await?;
    Ok(())
}

/// Cross-context reservations, restart recovery, replacement, and starting a
/// suggestion are a separate lifecycle scenario from initial generation.
#[tokio::test(flavor = "multi_thread")]
async fn replacement_generation_preserves_reservations_and_replaces_cards() -> HarnessResult<()> {
    let fixture = SuggestionsFixture::new().await?;
    let runtime = fixture.start_runtime().await?;
    let surface = fixture.surface(&runtime)?;
    let generated = generate_initial_suggestions(&fixture, &runtime, &surface).await?;

    let failing_surface = BoundProductSurface::new(
        runtime.product_surface(None)?,
        ProductSurfaceCaller::new(
            fixture.tenant_id.clone(),
            fixture.user_id.clone(),
            Some(fixture.agent_id.clone()),
            Some(ProjectId::new("suggestions-missing-project")?),
        ),
    );
    let changed_context_start = SUGGESTION_START_COMMAND
        .invoke_on(
            &failing_surface,
            RebornSuggestionStartRequest {
                suggestion_id: generated.suggestions[0].id.clone(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await
        .expect_err("an unauthorized alternate project must fail after reserving the start");
    assert_eq!(
        changed_context_start.code,
        ProductSurfaceErrorCode::NotFound
    );

    let original_context_retry = SUGGESTION_START_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionStartRequest {
                suggestion_id: generated.suggestions[0].id.clone(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await
        .expect_err("a changed context reservation cannot be replayed elsewhere");
    assert_eq!(
        original_context_retry.code,
        ProductSurfaceErrorCode::Conflict
    );
    assert!(original_context_retry.retryable);

    drop(surface);
    runtime.shutdown().await?;
    let runtime = fixture.start_runtime().await?;
    let surface = fixture.surface(&runtime)?;
    let reopened_cached = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "suggestions-action-1".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await?;
    assert_eq!(reopened_cached, generated);
    assert_eq!(fixture.scripted_llm.captured_requests().len(), 2);

    let replaced = replace_suggestions(&fixture, &surface).await?;
    assert_eq!(replaced.suggestions.len(), 1);
    assert_eq!(replaced.suggestions[0].title, "Review calendar");

    // A delayed retry from the replaced generation is still an idempotent
    // replay: it returns its original durable response without acquiring a
    // third generation, while the current list remains generation B.
    let delayed_retry = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "suggestions-action-1".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await?;
    assert_eq!(delayed_retry, generated);
    let current_after_delayed_retry = SUGGESTIONS_LIST_VIEW
        .query_on(&surface, RebornSuggestionsListRequest::default(), None)
        .await?;
    assert_eq!(current_after_delayed_retry, replaced);
    assert_eq!(fixture.scripted_llm.captured_requests().len(), 4);

    // The same historical replay remains durable after a runtime restart, and
    // still cannot displace the current generation.
    drop(surface);

    runtime.shutdown().await?;
    let runtime = fixture.start_runtime().await?;
    let surface = fixture.surface(&runtime)?;
    let delayed_retry_after_restart = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "suggestions-action-1".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await?;
    assert_eq!(delayed_retry_after_restart, generated);
    let current_after_restart = SUGGESTIONS_LIST_VIEW
        .query_on(&surface, RebornSuggestionsListRequest::default(), None)
        .await?;
    assert_eq!(current_after_restart, replaced);
    assert_eq!(fixture.scripted_llm.captured_requests().len(), 4);

    drop(surface);
    runtime.shutdown().await?;
    Ok(())
}

/// Starting and dismissing are independent product actions after replacement.
#[tokio::test(flavor = "multi_thread")]
async fn starting_a_replacement_suggestion_creates_one_thread() -> HarnessResult<()> {
    let fixture = SuggestionsFixture::new().await?;
    let runtime = fixture.start_runtime().await?;
    let surface = fixture.surface(&runtime)?;
    generate_initial_suggestions(&fixture, &runtime, &surface).await?;
    let replaced = replace_suggestions(&fixture, &surface).await?;
    let started = start_replacement_suggestion(
        &fixture,
        &runtime,
        &surface,
        replaced.suggestions[0].id.clone(),
    )
    .await?;

    let started_thread_id = ThreadId::new(started.thread_id.clone())?;
    let threads = THREADS_VIEW
        .query_on(&surface, ProductListThreadsRequest::default(), None)
        .await?;
    assert_eq!(
        threads
            .threads
            .iter()
            .filter(|thread| thread.thread_id == started_thread_id)
            .count(),
        1,
        "starting a suggestion must create one visible canonical thread"
    );

    let timeline = TIMELINE_VIEW
        .query_on(
            &surface,
            RebornTimelineRequest {
                thread_id: started.thread_id,
                ..RebornTimelineRequest::default()
            },
            None,
        )
        .await?;
    assert!(timeline.messages.iter().any(|message| {
        message.kind == MessageKind::User
            && message.content.as_deref() == Some("Review my upcoming calendar.")
    }));

    runtime.shutdown().await?;
    Ok(())
}

/// Suggestion documents are owned by the authenticated tenant/user scope.
/// Reads from another scope are empty, and start/dismiss cannot mutate the
/// owner's card.
#[tokio::test(flavor = "multi_thread")]
async fn suggestions_are_isolated_by_authenticated_scope() -> HarnessResult<()> {
    let fixture = SuggestionsFixture::new().await?;
    let runtime = fixture.start_runtime().await?;
    let owner = fixture.surface(&runtime)?;
    let generated = generate_initial_suggestions(&fixture, &runtime, &owner).await?;
    let suggestion_id = generated.suggestions[0].id.clone();
    let other_user = UserId::new("suggestions-other-user")?;
    let other_tenant = TenantId::new("suggestions-other-tenant")?;
    let alternate_surfaces = vec![
        (
            "other user in the tenant",
            bound_surface(&runtime, &fixture.tenant_id, &other_user, &fixture.agent_id)?,
        ),
        (
            "other tenant",
            bound_surface(&runtime, &other_tenant, &fixture.user_id, &fixture.agent_id)?,
        ),
    ];

    for (scope, alternate) in alternate_surfaces {
        let listed = SUGGESTIONS_LIST_VIEW
            .query_on(&alternate, RebornSuggestionsListRequest::default(), None)
            .await?;
        assert_eq!(
            listed.status,
            RebornSuggestionGenerationStatus::Empty,
            "{scope}"
        );
        assert!(listed.suggestions.is_empty(), "{scope}");

        let start = SUGGESTION_START_COMMAND
            .invoke_on(
                &alternate,
                RebornSuggestionStartRequest {
                    suggestion_id: suggestion_id.clone(),
                },
                ironclaw_host_api::ids::ActivityId::new(),
            )
            .await
            .expect_err("an alternate scope cannot start the owner's card");
        assert_eq!(start.code, ProductSurfaceErrorCode::NotFound, "{scope}");

        let dismiss = SUGGESTION_DISMISS_COMMAND
            .invoke_on(
                &alternate,
                RebornSuggestionDismissRequest {
                    suggestion_id: suggestion_id.clone(),
                },
                ironclaw_host_api::ids::ActivityId::new(),
            )
            .await
            .expect_err("an alternate scope cannot dismiss the owner's card");
        assert_eq!(dismiss.code, ProductSurfaceErrorCode::NotFound, "{scope}");

        let owner_after = SUGGESTIONS_LIST_VIEW
            .query_on(&owner, RebornSuggestionsListRequest::default(), None)
            .await?;
        assert_eq!(owner_after, generated, "owner changed after {scope}");
    }

    runtime.shutdown().await?;
    Ok(())
}

/// Dismissal removes the card but does not delete the started thread or its
/// timeline, including after a runtime restart.
#[tokio::test(flavor = "multi_thread")]
async fn dismissing_a_started_suggestion_persists_across_restart() -> HarnessResult<()> {
    let fixture = SuggestionsFixture::new().await?;
    let runtime = fixture.start_runtime().await?;
    let surface = fixture.surface(&runtime)?;
    generate_initial_suggestions(&fixture, &runtime, &surface).await?;
    let replaced = replace_suggestions(&fixture, &surface).await?;
    let started = start_replacement_suggestion(
        &fixture,
        &runtime,
        &surface,
        replaced.suggestions[0].id.clone(),
    )
    .await?;

    let model_calls_before_dismiss = fixture.scripted_llm.captured_requests().len();
    let suggestion_id = replaced.suggestions[0].id.clone();
    let uppercase_suggestion_id = suggestion_id.to_uppercase();
    let dismissed = SUGGESTION_DISMISS_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionDismissRequest {
                suggestion_id: uppercase_suggestion_id,
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await?;
    assert!(dismissed.dismissed);
    assert_eq!(dismissed.suggestion_id, suggestion_id);

    let after_dismiss = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "suggestions-action-2".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await?;
    assert!(after_dismiss.suggestions.is_empty());
    assert_eq!(
        fixture.scripted_llm.captured_requests().len(),
        model_calls_before_dismiss,
        "an all-dismissed ready document must not regenerate"
    );

    let retained_timeline = TIMELINE_VIEW
        .query_on(
            &surface,
            RebornTimelineRequest {
                thread_id: started.thread_id.clone(),
                ..RebornTimelineRequest::default()
            },
            None,
        )
        .await?;
    assert!(retained_timeline.messages.iter().any(|message| {
        message.kind == MessageKind::User
            && message.content.as_deref() == Some("Review my upcoming calendar.")
    }));

    let invalid_id = SUGGESTION_START_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionStartRequest {
                suggestion_id: "not-a-suggestion-id".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await
        .expect_err("malformed external ids must be rejected before lookup");
    assert_eq!(invalid_id.code, ProductSurfaceErrorCode::InvalidRequest);
    assert_eq!(
        invalid_id.validation_code,
        Some(ProductSurfaceValidationCode::InvalidId)
    );

    drop(surface);
    runtime.shutdown().await?;
    let runtime = fixture.start_runtime().await?;
    let surface = fixture.surface(&runtime)?;
    let reopened_after_dismiss = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "suggestions-action-2".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await?;
    assert!(reopened_after_dismiss.suggestions.is_empty());
    assert_eq!(
        fixture.scripted_llm.captured_requests().len(),
        model_calls_before_dismiss
    );
    let reopened_timeline = TIMELINE_VIEW
        .query_on(
            &surface,
            RebornTimelineRequest {
                thread_id: started.thread_id,
                ..RebornTimelineRequest::default()
            },
            None,
        )
        .await?;
    assert!(reopened_timeline.messages.iter().any(|message| {
        message.kind == MessageKind::User
            && message.content.as_deref() == Some("Review my upcoming calendar.")
    }));

    runtime.shutdown().await?;
    Ok(())
}

/// A generation that is in flight when the host restarts must remain owned by
/// the durable AgentTurn process.  The restarted client does not replay the
/// generate command (and therefore has no idempotency key); it only reads the
/// list view while the recovered scheduler resumes the same run.
#[tokio::test(flavor = "multi_thread")]
async fn generation_in_progress_survives_runtime_restart_and_recovers_via_list_view()
-> HarnessResult<()> {
    let root = tempdir()?;
    let session_root = tempdir()?;
    let tenant_id = TenantId::new("suggestions-restart-tenant")?;
    let agent_id = AgentId::new("suggestions-restart-agent")?;
    let user_id = UserId::new("suggestions-restart-user")?;
    let scripted_llm: Arc<TraceLlm> = Arc::new(scripted_trace_llm([
        RebornScriptedReply::text("I recommend reviewing the inbox."),
        RebornScriptedReply::text(serde_json::to_string(&json!({
            "suggestions": [{
                "title": "Review inbox",
                "description": "Review unread messages.",
                "suggested_prompt": "Review my unread inbox.",
                "icon": "generic",
                "sources": ["Gmail"]
            }]
        }))?),
    ]));
    let generation_gate = ParkingModelGate::new();
    let raw: Arc<dyn LlmProvider> =
        Arc::new(parking_trace_llm(generation_gate.clone(), scripted_llm));
    let gateway = build_suggestions_gateway(session_root.path(), raw).await?;

    let runtime =
        build_suggestions_runtime(root.path(), &tenant_id, &agent_id, gateway.clone()).await?;
    let surface = bound_surface(&runtime, &tenant_id, &user_id, &agent_id)?;
    let generating_surface = surface.clone();
    let generating = tokio::spawn(async move {
        SUGGESTIONS_GENERATE_COMMAND
            .invoke_on(
                &generating_surface,
                RebornSuggestionsGenerateRequest {
                    client_action_id: "suggestions-restart-action".to_string(),
                },
                ironclaw_host_api::ids::ActivityId::new(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), generation_gate.wait_until_parked()).await?;
    let generation_join = tokio::time::timeout(Duration::from_secs(10), generating)
        .await
        .map_err(|_| "generation request did not return while the model was parked")?;
    let generation_result = generation_join?;
    let before_restart = generation_result?;
    assert_eq!(
        before_restart.status,
        RebornSuggestionGenerationStatus::Generating
    );

    drop(surface);
    tokio::time::timeout(Duration::from_secs(10), runtime.shutdown())
        .await
        .map_err(|_| "runtime shutdown did not relinquish the in-flight generation")??;

    // The old model call was cancelled by shutdown.  The next model call is
    // deliberately parked again so the post-restart assertion observes the
    // recovered run before its terminal structured finalization.
    generation_gate.rearm();
    let runtime = build_suggestions_runtime(root.path(), &tenant_id, &agent_id, gateway).await?;
    let surface = bound_surface(&runtime, &tenant_id, &user_id, &agent_id)?;
    tokio::time::timeout(Duration::from_secs(10), generation_gate.wait_until_parked()).await?;
    let after_restart = SUGGESTIONS_LIST_VIEW
        .query_on(&surface, RebornSuggestionsListRequest::default(), None)
        .await?;
    assert_eq!(
        after_restart.status,
        RebornSuggestionGenerationStatus::Generating,
        "restart recovery must remain visible through GET/list without replaying generate"
    );
    assert_eq!(after_restart.suggestions, Vec::new());

    generation_gate.release();
    let recovered = wait_for_ready_suggestions(&surface).await?;
    assert_eq!(recovered.suggestions.len(), 1);
    assert_eq!(recovered.suggestions[0].title, "Review inbox");
    assert_eq!(
        recovered.suggestions[0].suggested_prompt,
        "Review my unread inbox."
    );

    runtime.shutdown().await?;
    Ok(())
}

/// A terminal AgentTurn failure is materialized by the process-commit
/// observer, not by the generate request. The list view must expose a durable
/// retryable Failed state and never retain cards from the prior snapshot.
#[tokio::test(flavor = "multi_thread")]
async fn failed_suggestion_run_settles_failed_and_retryable_via_list_view() -> HarnessResult<()> {
    let root = tempdir()?;
    let session_root = tempdir()?;
    let tenant_id = TenantId::new("suggestions-failed-tenant")?;
    let agent_id = AgentId::new("suggestions-failed-agent")?;
    let user_id = UserId::new("suggestions-failed-user")?;
    let (failing_provider, _) = ErrLlm::new(ErrLlmKind::ContextLength);
    let gateway = build_suggestions_gateway(
        session_root.path(),
        Arc::new(failing_provider) as Arc<dyn LlmProvider>,
    )
    .await?;
    let runtime = build_suggestions_runtime(root.path(), &tenant_id, &agent_id, gateway).await?;
    let surface = bound_surface(&runtime, &tenant_id, &user_id, &agent_id)?;

    let started = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "suggestions-failed-action".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await?;
    let failed = wait_for_failed_suggestions(&surface).await?;
    assert_eq!(failed.generation_id, started.generation_id);
    assert!(failed.suggestions.is_empty());
    assert_eq!(failed.retry_after_seconds, Some(1));

    runtime.shutdown().await?;
    Ok(())
}

/// A completed structured run can still carry a semantically invalid payload
/// (JSON-schema strings permit whitespace). The durable observer must reject
/// it and settle Failed instead of publishing Ready cards or leaving the
/// generation Pending forever.
#[tokio::test(flavor = "multi_thread")]
async fn semantically_invalid_completed_suggestion_output_settles_failed() -> HarnessResult<()> {
    let root = tempdir()?;
    let session_root = tempdir()?;
    let tenant_id = TenantId::new("suggestions-invalid-output-tenant")?;
    let agent_id = AgentId::new("suggestions-invalid-output-agent")?;
    let user_id = UserId::new("suggestions-invalid-output-user")?;
    let scripted_llm: Arc<TraceLlm> = Arc::new(scripted_trace_llm([
        RebornScriptedReply::text("I recommend reviewing the inbox."),
        RebornScriptedReply::text(serde_json::to_string(&json!({
            "suggestions": [{
                "title": "   ",
                "description": "Review unread messages.",
                "suggested_prompt": "Review my unread inbox.",
                "icon": "generic",
                "sources": ["Gmail"]
            }]
        }))?),
    ]));
    let gateway =
        build_suggestions_gateway(session_root.path(), scripted_llm as Arc<dyn LlmProvider>)
            .await?;
    let runtime = build_suggestions_runtime(root.path(), &tenant_id, &agent_id, gateway).await?;
    let surface = bound_surface(&runtime, &tenant_id, &user_id, &agent_id)?;

    let started = SUGGESTIONS_GENERATE_COMMAND
        .invoke_on(
            &surface,
            RebornSuggestionsGenerateRequest {
                client_action_id: "suggestions-invalid-output-action".to_string(),
            },
            ironclaw_host_api::ids::ActivityId::new(),
        )
        .await?;
    let failed = wait_for_failed_suggestions(&surface).await?;
    assert_eq!(failed.generation_id, started.generation_id);
    assert!(failed.suggestions.is_empty());
    assert_eq!(failed.retry_after_seconds, Some(1));

    runtime.shutdown().await?;
    Ok(())
}

/// A completed finalization can be valid JSON while violating the suggestion
/// contract. The observer must settle Failed and publish no cards.
#[tokio::test(flavor = "multi_thread")]
async fn unknown_field_in_completed_suggestion_output_settles_failed() -> HarnessResult<()> {
    let fixture = SuggestionsFixture::with_finalization(json!({
        "suggestions": [{
            "title": "Triage inbox",
            "description": "Review and prioritize unread email.",
            "suggested_prompt": "Triage my unread inbox.",
            "icon": "web",
            "sources": ["Web Search"],
            "unexpected": true,
        }]
    }))
    .await?;
    let runtime = fixture.start_runtime().await?;
    let surface = fixture.surface(&runtime)?;
    let generating_surface = surface.clone();
    let generating = tokio::spawn(async move {
        SUGGESTIONS_GENERATE_COMMAND
            .invoke_on(
                &generating_surface,
                RebornSuggestionsGenerateRequest {
                    client_action_id: "suggestions-unknown-field-action".to_string(),
                },
                ironclaw_host_api::ids::ActivityId::new(),
            )
            .await
    });
    tokio::time::timeout(
        Duration::from_secs(10),
        fixture.generation_gate.wait_until_parked(),
    )
    .await?;
    let generation_join = tokio::time::timeout(Duration::from_secs(10), generating)
        .await
        .map_err(|_| "generation request did not return while the model was parked")?;
    let generation_result = generation_join?;
    let started = generation_result?;
    assert_eq!(started.status, RebornSuggestionGenerationStatus::Generating);

    fixture.generation_gate.release();
    let failed = wait_for_failed_suggestions(&surface).await?;
    assert_eq!(failed.generation_id, started.generation_id);
    assert!(failed.suggestions.is_empty());
    assert_eq!(failed.retry_after_seconds, Some(1));
    assert_eq!(fixture.scripted_llm.captured_requests().len(), 2);

    runtime.shutdown().await?;
    Ok(())
}
