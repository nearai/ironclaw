//! Product-surface orchestration for durable backend suggestions.

use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_host_api::{ids::InvocationId, output::OutputContract, resource::ResourceScope};
use ironclaw_product_contracts::{
    inbound::ProductInboundAck,
    inbound_requests::{
        ProductCreateThreadRequest, ProductSubmitTurnRequest, RebornSuggestionDismissRequest,
        RebornSuggestionStartRequest, RebornSuggestionsGenerateRequest,
    },
    product_wire::{
        RebornSubmitTurnResponse, RebornSuggestion, RebornSuggestionDismissResponse,
        RebornSuggestionGenerationStatus, RebornSuggestionStartResponse, RebornSuggestionsResponse,
    },
    surface::{
        ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode,
        ProductSurfaceErrorKind, ProductSurfaceValidationCode,
    },
    views::RebornViewProvider,
};
use ironclaw_threads::agent_message::{AgentMessage, AgentMessageRole, ContentPart};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    ProductCapabilityInvoker, RebornServices,
    suggestions_store::{
        BeginGenerationRequest, GenerationId, GenerationState, SuggestionBinding,
        SuggestionDocument, SuggestionId, SuggestionRecord, SuggestionStartClaim,
        SuggestionStartReservation, SuggestionsStore, SuggestionsStoreError,
    },
    unbound_turn::{UnboundTurnError, UnboundTurnSubmission},
};

const PROMPT_SCHEMA_VERSION: u32 = 1;
const GENERATION_LEASE_DURATION_SECONDS: i64 = 30;
const GENERATION_RETRY_AFTER_SECONDS: u32 = 1;
pub(crate) const SUGGESTIONS_OUTPUT_NAME: &str = "suggestions";
const MIN_SUGGESTIONS: usize = 1;
const MAX_SUGGESTIONS: usize = 5;
const MIN_GENERATED_FIELD_LENGTH: usize = 1;
const MAX_TITLE_LENGTH: usize = 48;
const MAX_DESCRIPTION_LENGTH: usize = 240;
const MAX_PROMPT_LENGTH: usize = 2_000;
const MAX_ICON_LENGTH: usize = 128;
const MIN_SOURCES: usize = 1;
const MAX_SOURCES: usize = 5;
const MAX_SOURCE_LENGTH: usize = 128;
const SAFE_PRE_SUBMIT_FAILURE_REASON: &str = "suggestion generation submission failed";
const SUGGESTION_SYSTEM_PROMPT: &str = include_str!("../prompts/suggestion_generation.md");
const SUGGESTIONS_OUTPUT_SCHEMA: &str = include_str!("../schemas/suggestions.output.json");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GeneratedSuggestions {
    suggestions: Vec<GeneratedSuggestion>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GeneratedSuggestion {
    title: String,
    description: String,
    suggested_prompt: String,
    icon: String,
    sources: Vec<String>,
}

impl<I, V> RebornServices<I, V>
where
    I: ProductCapabilityInvoker + Clone + 'static,
    V: RebornViewProvider + Clone + 'static,
{
    /// Begin or observe one durable generation.  The request returns as soon
    /// as the canonical unbound run is durably bound; terminal materialization
    /// is performed by the process-journal observer.
    pub async fn generate_suggestions(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornSuggestionsGenerateRequest,
    ) -> Result<RebornSuggestionsResponse, ProductSurfaceError> {
        validate_client_action_id(&request.client_action_id)?;
        let suggestions = self
            .suggestions
            .as_ref()
            .ok_or_else(|| ProductSurfaceError::service_unavailable(false))?;
        let store = suggestions.store.as_ref();
        // Validate every local prerequisite before claiming the durable
        // generation. Once the claim exists, only the accept door and its
        // bounded lease window remain, so a crash can be recovered safely.
        let output = suggestion_output_contract()?;
        let scope = suggestion_scope(&caller);
        let proposed_generation_id = GenerationId::new(Uuid::new_v4().to_string())
            .map_err(ProductSurfaceError::internal_from)?;
        let lease_owner = Uuid::new_v4().to_string();
        let now = Utc::now();
        let claim = match store
            .begin_generation(
                &scope,
                BeginGenerationRequest {
                    generation_id: proposed_generation_id.clone(),
                    public_id: format!("suggestions-{proposed_generation_id}"),
                    accept_key: format!("suggestions-{proposed_generation_id}"),
                    client_action_id: Some(request.client_action_id),
                    prompt_schema_version: PROMPT_SCHEMA_VERSION,
                    lease_owner: lease_owner.clone(),
                    lease_expires_at: now
                        + ChronoDuration::seconds(GENERATION_LEASE_DURATION_SECONDS),
                    now,
                },
            )
            .await
        {
            Ok(claim) => claim,
            Err(error) => return Err(map_store_error(error)),
        };
        if !claim.is_acquired() {
            let document = store.read(&scope).await.map_err(map_store_error)?;
            return Ok(document
                .as_ref()
                .map(|document| {
                    if claim.is_historical_replay() {
                        suggestions_for_generation(document, claim.state())
                    } else {
                        active_suggestions(document)
                    }
                })
                .unwrap_or_else(empty_suggestions_response));
        }
        let GenerationState::Generating {
            generation_id,
            public_id,
            accept_key,
            prompt_schema_version,
            run_id,
            ..
        } = claim.state().clone()
        else {
            return Ok(suggestions_response_from_state(claim.state()));
        };
        let unbound = suggestions.unbound.as_ref();
        if prompt_schema_version != PROMPT_SCHEMA_VERSION {
            record_invalid_generation_output(
                store,
                &scope,
                &generation_id,
                &lease_owner,
                "suggestion prompt/schema version is no longer supported",
            )
            .await;
            return Err(ProductSurfaceError::service_unavailable(true));
        }

        submit_suggestion_generation(
            store,
            unbound,
            SuggestionGenerationSubmission {
                caller: &caller,
                scope: &scope,
                generation_id: &generation_id,
                public_id: &public_id,
                accept_key: &accept_key,
                existing_run_id: run_id,
                lease_owner: &lease_owner,
                output: &output,
            },
        )
        .await
    }

    pub async fn list_suggestions(
        &self,
        caller: ProductSurfaceCaller,
    ) -> Result<RebornSuggestionsResponse, ProductSurfaceError> {
        let suggestions = self
            .suggestions
            .as_ref()
            .ok_or_else(|| ProductSurfaceError::service_unavailable(false))?;
        let store = suggestions.store.as_ref();
        let scope = suggestion_scope(&caller);
        Ok(store
            .read(&scope)
            .await
            .map_err(map_store_error)?
            .map(|document| active_suggestions(&document))
            .unwrap_or_else(empty_suggestions_response))
    }

    pub async fn start_suggestion(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornSuggestionStartRequest,
    ) -> Result<RebornSuggestionStartResponse, ProductSurfaceError> {
        let requested_suggestion_id = parse_suggestion_id(&request.suggestion_id)?;
        let suggestions = self
            .suggestions
            .as_ref()
            .ok_or_else(|| ProductSurfaceError::service_unavailable(false))?;
        let store = suggestions.store.as_ref();
        let scope = suggestion_scope(&caller);
        let document = store
            .read(&scope)
            .await
            .map_err(map_store_error)?
            .ok_or_else(ProductSurfaceError::not_found)?;
        let current_generation_id = match &document.generation {
            GenerationState::Ready { generation_id, .. } => generation_id,
            _ => return Err(ProductSurfaceError::not_found()),
        };
        let suggestion = document
            .suggestions
            .iter()
            .find(|suggestion| suggestion.id == requested_suggestion_id)
            .filter(|suggestion| {
                suggestion.dismissed_at.is_none()
                    && suggestion.generation_id == *current_generation_id
            })
            .ok_or_else(ProductSurfaceError::not_found)?;
        let suggestion_id = suggestion.id.clone();
        let suggested_prompt = suggestion.suggested_prompt.clone();
        let reservation = suggestion_start_reservation(&caller, &suggestion_id);
        let reservation = match store
            .reserve_start(&scope, &suggestion_id, reservation, Utc::now())
            .await
            .map_err(map_store_error)?
        {
            SuggestionStartClaim::Bound(binding) => {
                return Ok(start_response(&suggestion_id, &binding));
            }
            SuggestionStartClaim::Reserved(reservation) => reservation,
        };
        let thread = self
            .create_thread(
                caller.clone(),
                ProductCreateThreadRequest {
                    client_action_id: Some(reservation.thread_action_id.clone()),
                    requested_thread_id: None,
                    project_id: caller.project_id.as_ref().map(ToString::to_string),
                },
            )
            .await?;
        let submitted = self
            .submit_turn(
                caller,
                ProductSubmitTurnRequest {
                    extension_id: None,
                    client_action_id: Some(reservation.turn_action_id.clone()),
                    thread_id: Some(thread.thread.thread_id.to_string()),
                    content: Some(suggested_prompt),
                    attachments: Vec::new(),
                    model: None,
                },
            )
            .await?;
        let binding = match submitted {
            RebornSubmitTurnResponse::Submitted {
                thread_id, run_id, ..
            }
            | RebornSubmitTurnResponse::AlreadySubmitted {
                thread_id, run_id, ..
            } => SuggestionBinding { thread_id, run_id },
            RebornSubmitTurnResponse::RejectedBusy { .. }
            | RebornSubmitTurnResponse::DeferredBusy { .. } => {
                return Err(ProductSurfaceError::service_unavailable(true));
            }
        };
        let binding = finish_start_binding(
            store
                .complete_start(
                    &scope,
                    &suggestion_id,
                    &reservation,
                    binding.clone(),
                    Utc::now(),
                )
                .await,
            binding,
        )?;
        Ok(start_response(&suggestion_id, &binding))
    }

    pub async fn dismiss_suggestion(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornSuggestionDismissRequest,
    ) -> Result<RebornSuggestionDismissResponse, ProductSurfaceError> {
        let suggestion_id = parse_suggestion_id(&request.suggestion_id)?;
        let suggestions = self
            .suggestions
            .as_ref()
            .ok_or_else(|| ProductSurfaceError::service_unavailable(false))?;
        let store = suggestions.store.as_ref();
        store
            .dismiss(&suggestion_scope(&caller), &suggestion_id, Utc::now())
            .await
            .map_err(map_store_error)?;
        Ok(RebornSuggestionDismissResponse {
            suggestion_id: suggestion_id.as_str().to_string(),
            dismissed: true,
        })
    }
}

struct SuggestionGenerationSubmission<'a> {
    caller: &'a ProductSurfaceCaller,
    scope: &'a ResourceScope,
    generation_id: &'a GenerationId,
    public_id: &'a str,
    accept_key: &'a str,
    existing_run_id: Option<ironclaw_host_api::turn::TurnRunId>,
    lease_owner: &'a str,
    output: &'a OutputContract,
}

async fn submit_suggestion_generation(
    store: &dyn SuggestionsStore,
    unbound: &crate::unbound_turn::UnboundTurnService,
    submission_context: SuggestionGenerationSubmission<'_>,
) -> Result<RebornSuggestionsResponse, ProductSurfaceError> {
    let SuggestionGenerationSubmission {
        caller,
        scope,
        generation_id,
        public_id,
        accept_key,
        existing_run_id,
        lease_owner,
        output,
    } = submission_context;
    let submission = UnboundTurnSubmission {
        caller: caller.clone(),
        public_id: public_id.to_string(),
        system_prompt: SUGGESTION_SYSTEM_PROMPT.to_string(),
        messages: vec![AgentMessage {
            role: AgentMessageRole::User,
            content: vec![ContentPart::text(
                "Generate useful suggestions for what I can ask IronClaw to do next.",
            )],
        }],
        // No declared tool list: the run takes the profile's surface,
        // narrowed by `require_no_approval` to what the user's own permissions
        // already auto-run. That narrowing does not restrict effects — the
        // surface can include write-effect capabilities the user has
        // auto-approved; read/list-only behaviour is carried by
        // SUGGESTION_SYSTEM_PROMPT alone, not enforced here. Declaring ids
        // here would mean re-deriving the approval decision in product code.
        tools: Vec::new(),
        require_no_approval: true,
        output: output.clone(),
        requested_model: None,
        idempotency_key: accept_key.to_string(),
    };
    let run_id = match existing_run_id {
        Some(run_id) => run_id,
        None => {
            let ack = match unbound.accept_and_submit(submission).await {
                Ok(ack) => ack,
                Err(error) => {
                    record_generation_failure(store, scope, generation_id, lease_owner, &error)
                        .await;
                    return Err(map_unbound_error(error));
                }
            };
            match ack {
                ProductInboundAck::Accepted {
                    submitted_run_id, ..
                } => submitted_run_id,
                _ => {
                    let error = UnboundTurnError::Internal {
                        reason: "suggestion generation accept returned a non-run acknowledgement"
                            .to_string(),
                    };
                    record_generation_failure(store, scope, generation_id, lease_owner, &error)
                        .await;
                    return Err(map_unbound_error(error));
                }
            }
        }
    };

    if let Err(error) = store
        .bind_generation_run(scope, generation_id, lease_owner, run_id, Utc::now())
        .await
    {
        if matches!(error, SuggestionsStoreError::GenerationNotCurrent { .. })
            && let Some(document) = store.read(scope).await.map_err(map_store_error)?
        {
            return Ok(active_suggestions(&document));
        }
        return Err(map_store_error(error));
    }
    let document = store.read(scope).await.map_err(map_store_error)?;
    Ok(document
        .as_ref()
        .map(active_suggestions)
        .unwrap_or_else(empty_suggestions_response))
}

fn suggestion_output_contract() -> Result<OutputContract, ProductSurfaceError> {
    let schema = serde_json::from_str(SUGGESTIONS_OUTPUT_SCHEMA)
        .map_err(ProductSurfaceError::internal_from)?;
    OutputContract::try_json_schema(SUGGESTIONS_OUTPUT_NAME, schema)
        .map_err(ProductSurfaceError::internal_from)
}

fn suggestion_scope(caller: &ProductSurfaceCaller) -> ResourceScope {
    // `/suggestions` is a per-user mount by design: a user's cards follow them
    // across product surfaces and current project selection. Agent/project are
    // still carried for filesystem authorization and audit context.
    ResourceScope {
        tenant_id: caller.tenant_id.clone(),
        user_id: caller.user_id.clone(),
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn active_suggestions(document: &SuggestionDocument) -> RebornSuggestionsResponse {
    suggestions_for_generation(document, &document.generation)
}

fn suggestions_for_generation(
    document: &SuggestionDocument,
    state: &GenerationState,
) -> RebornSuggestionsResponse {
    let active_generation_id = match state {
        GenerationState::Ready { generation_id, .. } => Some(generation_id),
        _ => None,
    };
    RebornSuggestionsResponse {
        status: generation_status(state),
        generation_id: match state {
            GenerationState::Generating { generation_id, .. }
            | GenerationState::Pending { generation_id, .. }
            | GenerationState::Ready { generation_id, .. }
            | GenerationState::Failed { generation_id, .. } => {
                Some(generation_id.as_str().to_string())
            }
            GenerationState::Never => None,
        },
        retry_after_seconds: match state {
            GenerationState::Generating { .. }
            | GenerationState::Pending { .. }
            | GenerationState::Failed { .. } => Some(GENERATION_RETRY_AFTER_SECONDS),
            GenerationState::Never | GenerationState::Ready { .. } => None,
        },
        suggestions: document
            .suggestions
            .iter()
            .filter(|suggestion| {
                suggestion.dismissed_at.is_none()
                    && Some(&suggestion.generation_id) == active_generation_id
            })
            .map(|suggestion| RebornSuggestion {
                id: suggestion.id.as_str().to_string(),
                title: suggestion.title.clone(),
                description: suggestion.description.clone(),
                suggested_prompt: suggestion.suggested_prompt.clone(),
                icon: suggestion.icon.clone(),
                sources: suggestion.sources.clone(),
                thread_id: suggestion
                    .binding
                    .as_ref()
                    .map(|binding| binding.thread_id.to_string()),
                run_id: suggestion
                    .binding
                    .as_ref()
                    .map(|binding| binding.run_id.to_string()),
            })
            .collect(),
    }
}

pub(crate) fn generated_records(
    generation_id: &GenerationId,
    generated: GeneratedSuggestions,
) -> Result<Vec<SuggestionRecord>, ProductSurfaceError> {
    if generated.suggestions.len() < MIN_SUGGESTIONS
        || generated.suggestions.len() > MAX_SUGGESTIONS
    {
        return Err(ProductSurfaceError::internal());
    }
    let now = Utc::now();
    generated
        .suggestions
        .into_iter()
        .enumerate()
        .map(|(index, suggestion)| {
            validate_generated_field(&suggestion.title, MAX_TITLE_LENGTH)?;
            validate_generated_field(&suggestion.description, MAX_DESCRIPTION_LENGTH)?;
            validate_generated_field(&suggestion.suggested_prompt, MAX_PROMPT_LENGTH)?;
            validate_bounded_semantic_key(&suggestion.icon)?;
            validate_sources(&suggestion.sources)?;
            let identity = format!(
                "{generation_id}:{index}:{}:{}",
                suggestion.title, suggestion.suggested_prompt
            );
            Ok(SuggestionRecord {
                id: SuggestionId::new(
                    Uuid::new_v5(&Uuid::NAMESPACE_OID, identity.as_bytes()).to_string(),
                )
                .map_err(ProductSurfaceError::internal_from)?,
                title: suggestion.title,
                description: suggestion.description,
                suggested_prompt: suggestion.suggested_prompt,
                icon: suggestion.icon,
                sources: suggestion.sources,
                generation_id: generation_id.clone(),
                created_at: now,
                updated_at: now,
                dismissed_at: None,
                start_reservation: None,
                binding: None,
            })
        })
        .collect()
}

fn validate_generated_field(value: &str, max_length: usize) -> Result<(), ProductSurfaceError> {
    let has_invalid_control = value.chars().any(|character| {
        character == '\0' || (character.is_control() && character != '\n' && character != '\t')
    });
    if value.chars().count() < MIN_GENERATED_FIELD_LENGTH
        || value.trim().is_empty()
        || value.chars().count() > max_length
        || has_invalid_control
    {
        return Err(ProductSurfaceError::internal());
    }
    Ok(())
}

fn validate_bounded_semantic_key(value: &str) -> Result<(), ProductSurfaceError> {
    if value.trim().is_empty()
        || value.chars().count() > MAX_ICON_LENGTH
        || value.chars().any(char::is_control)
    {
        return Err(ProductSurfaceError::internal());
    }
    Ok(())
}

fn validate_sources(sources: &[String]) -> Result<(), ProductSurfaceError> {
    if sources.len() < MIN_SOURCES || sources.len() > MAX_SOURCES {
        return Err(ProductSurfaceError::internal());
    }
    let mut unique = std::collections::HashSet::with_capacity(sources.len());
    for source in sources {
        if source.trim().is_empty()
            || source.chars().count() > MAX_SOURCE_LENGTH
            || source.chars().any(char::is_control)
        {
            return Err(ProductSurfaceError::internal());
        }
        if !unique.insert(source) {
            return Err(ProductSurfaceError::internal());
        }
    }
    Ok(())
}

fn parse_suggestion_id(value: &str) -> Result<SuggestionId, ProductSurfaceError> {
    let id = Uuid::parse_str(value).map_err(|_| {
        ProductSurfaceError::validation("suggestion_id", ProductSurfaceValidationCode::InvalidId)
    })?;
    SuggestionId::new(id.to_string()).map_err(ProductSurfaceError::internal_from)
}

fn start_response(
    suggestion_id: &SuggestionId,
    binding: &SuggestionBinding,
) -> RebornSuggestionStartResponse {
    RebornSuggestionStartResponse {
        suggestion_id: suggestion_id.as_str().to_string(),
        thread_id: binding.thread_id.to_string(),
        run_id: binding.run_id.to_string(),
    }
}

fn suggestion_start_reservation(
    caller: &ProductSurfaceCaller,
    suggestion_id: &SuggestionId,
) -> SuggestionStartReservation {
    // The operation identity follows the per-user suggestion document. Agent
    // and project are immutable target context, not idempotency dimensions:
    // changing either must be rejected by the durable reservation rather than
    // creating a second thread or turn.
    let action_uuid = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("{}:{}:{}", caller.tenant_id, caller.user_id, suggestion_id).as_bytes(),
    );
    SuggestionStartReservation {
        thread_action_id: format!("suggestion-thread-{action_uuid}"),
        turn_action_id: format!("suggestion-turn-{action_uuid}"),
        agent_id: caller.agent_id.clone(),
        project_id: caller.project_id.clone(),
    }
}

async fn record_generation_failure(
    store: &dyn crate::suggestions_store::SuggestionsStore,
    scope: &ResourceScope,
    generation_id: &GenerationId,
    lease_owner: &str,
    error: &UnboundTurnError,
) {
    tracing::debug!(
        %generation_id,
        operation = "record_generation_failure",
        error = %error,
        "suggestion generation submission failed before run binding"
    );
    if let Err(store_error) = store
        .fail_generation(
            scope,
            generation_id,
            lease_owner,
            SAFE_PRE_SUBMIT_FAILURE_REASON.to_string(),
            Utc::now(),
        )
        .await
    {
        tracing::warn!(
            %generation_id,
            error_code = suggestions_store_error_code(&store_error),
            operation = "record_generation_failure",
            "failed to persist suggestion generation failure"
        );
    }
}

async fn record_invalid_generation_output(
    store: &dyn crate::suggestions_store::SuggestionsStore,
    scope: &ResourceScope,
    generation_id: &GenerationId,
    lease_owner: &str,
    reason: &str,
) {
    if let Err(store_error) = store
        .fail_generation(
            scope,
            generation_id,
            lease_owner,
            reason.to_string(),
            Utc::now(),
        )
        .await
    {
        tracing::warn!(
            %generation_id,
            error_code = suggestions_store_error_code(&store_error),
            operation = "record_invalid_generation_output",
            "failed to persist invalid suggestion output"
        );
    }
}

fn suggestions_store_error_code(error: &SuggestionsStoreError) -> &'static str {
    match error {
        SuggestionsStoreError::Serialization { .. } => "serialization",
        SuggestionsStoreError::Filesystem { .. } => "filesystem",
        SuggestionsStoreError::InvalidId { .. } => "invalid_id",
        SuggestionsStoreError::GenerationNotCurrent { .. } => "generation_not_current",
        SuggestionsStoreError::GenerationInProgress { .. } => "generation_in_progress",
        SuggestionsStoreError::SuggestionNotFound { .. } => "suggestion_not_found",
        SuggestionsStoreError::SuggestionDismissed { .. } => "suggestion_dismissed",
        SuggestionsStoreError::StartReservationConflict { .. } => "start_reservation_conflict",
    }
}

fn map_store_error(error: SuggestionsStoreError) -> ProductSurfaceError {
    match error {
        SuggestionsStoreError::SuggestionNotFound { .. }
        | SuggestionsStoreError::SuggestionDismissed { .. } => ProductSurfaceError::not_found(),
        SuggestionsStoreError::GenerationNotCurrent { .. } => {
            ProductSurfaceError::service_unavailable(true)
        }
        SuggestionsStoreError::GenerationInProgress { .. } => {
            ProductSurfaceError::from_status_kind(
                ProductSurfaceErrorCode::Conflict,
                ProductSurfaceErrorKind::Conflict,
                409,
                true,
            )
        }
        SuggestionsStoreError::StartReservationConflict { .. } => {
            ProductSurfaceError::from_status_kind(
                ProductSurfaceErrorCode::Conflict,
                ProductSurfaceErrorKind::Conflict,
                409,
                true,
            )
        }
        SuggestionsStoreError::Filesystem { operation, .. } => {
            tracing::debug!(
                store_operation = operation,
                error_code = "filesystem",
                "suggestions store unavailable"
            );
            ProductSurfaceError::service_unavailable(true)
        }
        SuggestionsStoreError::Serialization { .. } => {
            tracing::error!(
                operation = "suggestions_store",
                error_code = "serialization",
                "suggestions store serialization failed"
            );
            ProductSurfaceError::internal()
        }
        SuggestionsStoreError::InvalidId { .. } => ProductSurfaceError::internal(),
    }
}

fn finish_start_binding(
    result: Result<SuggestionBinding, SuggestionsStoreError>,
    accepted_binding: SuggestionBinding,
) -> Result<SuggestionBinding, ProductSurfaceError> {
    match result {
        Ok(binding) => Ok(binding),
        Err(SuggestionsStoreError::SuggestionNotFound { .. }) => {
            // A replacement generation clears the current suggestion
            // projection in one CAS. If it wins after this start's
            // reservation, the canonical thread/run side effects still
            // exist and their idempotency keys make `accepted_binding` the
            // stable result of the already-accepted start. Do not turn that
            // replacement-generation race into a 500 (or orphan the run).
            Ok(accepted_binding)
        }
        Err(error) => Err(map_store_error(error)),
    }
}

fn suggestions_response_from_state(state: &GenerationState) -> RebornSuggestionsResponse {
    let generation_id = match state {
        GenerationState::Never => None,
        GenerationState::Generating { generation_id, .. }
        | GenerationState::Pending { generation_id, .. }
        | GenerationState::Ready { generation_id, .. }
        | GenerationState::Failed { generation_id, .. } => Some(generation_id.as_str().to_string()),
    };
    RebornSuggestionsResponse {
        status: generation_status(state),
        generation_id,
        retry_after_seconds: match state {
            GenerationState::Generating { .. } | GenerationState::Pending { .. } => {
                Some(GENERATION_RETRY_AFTER_SECONDS)
            }
            GenerationState::Never
            | GenerationState::Ready { .. }
            | GenerationState::Failed { .. } => None,
        },
        suggestions: Vec::new(),
    }
}

fn empty_suggestions_response() -> RebornSuggestionsResponse {
    RebornSuggestionsResponse {
        status: RebornSuggestionGenerationStatus::Empty,
        generation_id: None,
        retry_after_seconds: None,
        suggestions: Vec::new(),
    }
}

fn generation_status(state: &GenerationState) -> RebornSuggestionGenerationStatus {
    match state {
        GenerationState::Never => RebornSuggestionGenerationStatus::Empty,
        GenerationState::Generating { .. } | GenerationState::Pending { .. } => {
            RebornSuggestionGenerationStatus::Generating
        }
        GenerationState::Ready { .. } => RebornSuggestionGenerationStatus::Ready,
        GenerationState::Failed { .. } => RebornSuggestionGenerationStatus::Failed,
    }
}

fn validate_client_action_id(value: &str) -> Result<(), ProductSurfaceError> {
    const MAX_BYTES: usize =
        ironclaw_product_contracts::inbound_requests::SUGGESTIONS_CLIENT_ACTION_ID_MAX_BYTES;
    if value.is_empty()
        || value.len() > MAX_BYTES
        || value.trim() != value
        || value
            .chars()
            .any(|character| character == '\0' || character.is_control())
    {
        return Err(ProductSurfaceError::validation(
            "client_action_id",
            ProductSurfaceValidationCode::InvalidId,
        ));
    }
    Ok(())
}

fn map_unbound_error(error: UnboundTurnError) -> ProductSurfaceError {
    match error {
        UnboundTurnError::InvalidRequest { .. } => ProductSurfaceError::internal(),
        UnboundTurnError::Unavailable => ProductSurfaceError::service_unavailable(true),
        UnboundTurnError::RunFailed { .. } | UnboundTurnError::RunCancelled => {
            ProductSurfaceError::service_unavailable(true)
        }
        UnboundTurnError::Internal { .. } => ProductSurfaceError::internal(),
    }
}

#[cfg(test)]
#[path = "suggestions_tests.rs"]
mod tests;
