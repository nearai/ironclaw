//! E-TRIGGERED-SUBMIT enabler seam: submit a turn through the REAL
//! `TrustedTriggerFireSubmitter` so it carries a genuine
//! `TurnOriginKind::ScheduledTrigger` origin, end to end.
//!
//! [`RebornIntegrationHarness::submit_triggered_turn`] builds a synthetic
//! `TriggerFire` + `TriggerMaterializedPrompt::for_fire` (test ctor) and hands
//! them to the production `trusted_trigger_fire_submitter` — it does NOT fake
//! or re-implement origin tagging itself. That submitter runs over a fresh,
//! per-call `InMemoryConversationServices` dedicated to the trigger path (the
//! same conversation-services type production's own local-dev build wires for
//! this exact purpose), while the harness's REAL shared `coordinator` is
//! passed through unchanged, so the submitted run lands in the same turn
//! store/scheduler as every other harness turn.
//!
//! Two variants:
//! - [`RebornIntegrationHarness::submit_triggered_turn`] — submit only; the
//!   run then fails benignly on the scope-miss sentinel (asserts submit-time
//!   state, e.g. the persisted origin).
//! - [`RebornIntegrationHarness::submit_triggered_turn_scripted`] — mirrors
//!   the production materializer (binding resolve + thread-recorded prompt +
//!   real content ref) and registers a scripted gateway for the fire's exact
//!   scope, so the triggered run can be driven to completion or through
//!   mid-fire gates (scope-aware `wait`/`approve`/`deny` helpers below).

// Shared integration-test support: not every binary that mounts the
// `reborn_support` tree consumes this module — its symbols read as dead there
// under the all-features `-D warnings` lane. Module-level allow matches
// `builder.rs`/`assertions.rs`/`session_thread.rs`.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ConversationBindingService, ConversationRouteKind,
    ExternalActorRef, ExternalConversationRef, ExternalEventId, InMemoryConversationServices,
    ResolveConversationRequest, trusted_trigger_fire_submitter,
};
use ironclaw_llm::testing::provider_chain_over;
use ironclaw_llm::{LlmProvider, SessionConfig, create_session_manager};
use ironclaw_loop_support::HostManagedModelGateway;
use ironclaw_product_workflow::automation_trigger_thread_metadata_json;
use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, SessionThreadService,
    ThreadScope,
};
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerFire, TriggerFireIdentity, TriggerId,
    TriggerInboundContentRef, TriggerMaterializedPrompt, TriggerTrustedInboundBinding,
    TrustedTriggerFireSubmitOutcome, TrustedTriggerSubmitRequest,
};
use ironclaw_turns::run_profile::ModelProfileId;
use ironclaw_turns::{
    GateRef, GateResumeDisposition, GetRunStateRequest, IdempotencyKey, ReplyTargetBindingRef,
    ResumeTurnPrecondition, ResumeTurnRequest, SourceBindingRef, TurnActor, TurnRunId,
    TurnRunState, TurnScope, TurnStateStore, TurnStatus,
};

use super::builder::{INTERACTIVE_MODEL_PROFILE, RebornIntegrationHarness};
use super::reply::RebornScriptedReply;
use super::scripted_provider::{SCRIPTED_MODEL_NAME, scripted_trace_llm};
use crate::support::trace_llm::TraceLlm;

// `builder.rs`'s `HarnessResult` is module-private; every sibling file that
// needs the alias (`assertions.rs`, `harness.rs`, `harness_mcp.rs`) declares
// its own identical copy rather than reaching across the module boundary.
type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Far-future, deterministic fire slot — no wall-clock flake, matches the
/// style already used in `tests/reborn_group_triggers/scenario_verbs_lifecycle.rs`.
fn triggered_fire_slot() -> chrono::DateTime<chrono::Utc> {
    Utc.with_ymd_and_hms(2999, 1, 1, 0, 0, 0).unwrap()
}

/// Result of a successful `submit_triggered_turn` call.
pub(crate) struct TriggeredSubmission {
    pub(crate) run_id: TurnRunId,
    /// The trigger's OWN resolved scope, as returned by the real submitter.
    /// The trusted-submit contract forbids re-deriving binding keys from a
    /// `TriggerFire` (see `ironclaw_triggers::trusted_submit` docs), so callers
    /// must read this back rather than reconstruct it.
    pub(crate) turn_scope: TurnScope,
}

impl RebornIntegrationHarness {
    /// Synthetic `TriggerFire` for this harness's binding — the shared fire
    /// construction both triggered-submit seams hand to the production
    /// submitter.
    fn triggered_fire(&self, prompt: &str, fire_slot: chrono::DateTime<Utc>) -> TriggerFire {
        TriggerFire {
            identity: TriggerFireIdentity::new(
                self.binding.tenant_id.clone(),
                TriggerId::new(),
                fire_slot,
            ),
            creator_user_id: self.binding.actor_user_id.clone(),
            agent_id: self.binding.agent_id.clone(),
            project_id: self.binding.project_id.clone(),
            prompt: prompt.to_string(),
        }
    }

    /// Fresh `InMemoryConversationServices` for the trigger path with the
    /// trigger's canonical external actor pre-paired — mirrors
    /// `TriggerTrustedInboundBinding::for_fire`'s own derivation exactly and
    /// mirrors production's pre-seed requirement (`resolve_actor` hard-fails
    /// `BindingRequired` without it, on both trusted and untrusted resolve
    /// paths). Uses `try_pair_external_actor` (not the infallible
    /// `pair_external_actor` wrapper) so a pairing failure surfaces here, at
    /// the seam boundary, instead of resurfacing later as an indirect
    /// binding-resolution error.
    async fn trigger_conversations_with_paired_actor(
        &self,
    ) -> HarnessResult<InMemoryConversationServices> {
        let conversations = InMemoryConversationServices::default();
        conversations
            .try_pair_external_actor(
                self.binding.tenant_id.clone(),
                AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND)?,
                AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)?,
                ExternalActorRef::new(
                    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE,
                    self.binding.actor_user_id.as_str(),
                )?,
                self.binding.actor_user_id.clone(),
            )
            .await?;
        Ok(conversations)
    }

    /// Submit a turn through the REAL `TrustedTriggerFireSubmitter` so it carries
    /// a genuine `TurnOriginKind::ScheduledTrigger` origin, end to end (E-TRIGGERED-SUBMIT).
    ///
    /// Builds a synthetic `TriggerFire` + `TriggerMaterializedPrompt::for_fire` (test
    /// ctor) and hands them to the production `trusted_trigger_fire_submitter`, over a
    /// fresh, per-call `InMemoryConversationServices` dedicated to the trigger path —
    /// the same conversation-services type production's own local-dev build wires for
    /// this exact purpose (`ironclaw_reborn_composition::runtime.rs`,
    /// `build_trigger_poller_services_from_conversation_services`), NOT the harness's
    /// unrelated direct-chat product-workflow binding service (a different axis, even
    /// in real production). The harness's REAL shared `coordinator` is passed through
    /// unchanged, so the submitted run lands in the same turn store/scheduler as every
    /// other harness turn.
    ///
    /// The submitted run then executes autonomously on the background scheduler; no
    /// scripted model gateway is registered for the trigger's own resolved scope, so it
    /// fails benignly on a scope-miss (`ScopeRegistryGateway`'s `ConfigurationError`
    /// sentinel). `product_context` (carrying the origin) is persisted synchronously at
    /// submit time, before that later failure — this seam and its driving test assert
    /// only on that submit-time state, not on anything after. Driving a triggered run
    /// to model completion, or asserting behavior across that later failure, is
    /// C-TRIGGERED-DELIVERY, not this seam.
    pub(crate) async fn submit_triggered_turn(
        &self,
        prompt: &str,
    ) -> HarnessResult<TriggeredSubmission> {
        let fire_slot = triggered_fire_slot();
        let fire = self.triggered_fire(prompt, fire_slot);
        let content_ref = TriggerInboundContentRef::new(format!(
            "content:triggered-submit:{}",
            fire.identity.external_event_id().as_str()
        ))?;
        let materialized_prompt = TriggerMaterializedPrompt::for_fire(&fire, content_ref);
        let request =
            TrustedTriggerSubmitRequest::new_for_test(fire, materialized_prompt, fire_slot);

        let conversations = self.trigger_conversations_with_paired_actor().await?;

        let submitter = trusted_trigger_fire_submitter(
            conversations.clone(),
            conversations,
            Arc::clone(&self.coordinator),
        );
        match submitter.submit_trusted_trigger_fire(request).await? {
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id, turn_scope, ..
            } => Ok(TriggeredSubmission { run_id, turn_scope }),
            TrustedTriggerFireSubmitOutcome::Replayed { .. } => {
                Err("first triggered submit unexpectedly replayed".into())
            }
        }
    }

    /// [`submit_triggered_turn`](Self::submit_triggered_turn), with the
    /// triggered run's model calls scripted and the trigger prompt recorded
    /// into the harness's REAL thread service — so the run can be driven past
    /// submission (to completion, or to a mid-fire gate) instead of failing
    /// benignly on the scope-miss sentinel.
    ///
    /// Mirrors the production trigger poller's materializer
    /// (`ConversationContentRefMaterializer::materialize_prompt`,
    /// `crates/ironclaw_reborn_composition/src/trigger_poller_trusted_submit.rs`
    /// — `pub(crate)` there, hence mirrored, not called), which the plain
    /// `submit_triggered_turn`'s `TriggerMaterializedPrompt::for_fire` shortcut
    /// skips:
    /// 1. resolve-or-create the trigger's conversation binding (same fresh
    ///    `InMemoryConversationServices`, same canonical
    ///    `TriggerTrustedInboundBinding::for_fire` derivation) — this mints the
    ///    fire's `TurnScope` BEFORE the submit;
    /// 2. record the prompt as a real inbound thread message
    ///    (`ensure_thread` + `accept_inbound_message`, production's
    ///    `record_trigger_prompt` shape) so the loop host's thread-context
    ///    read finds the trigger thread — without this the run fails
    ///    `driver_unavailable` on `unknown thread`;
    /// 3. materialize the prompt with the REAL `thread-message:<id>` content
    ///    ref (production's shape), not the synthetic `for_fire` ref;
    /// 4. register the scripted gateway for the EXACT resolved scope on the
    ///    group's `ScopeRegistryGateway` (the same real-chain construction
    ///    thread gateways use: `scripted_trace_llm` → `provider_chain_over` →
    ///    `LlmProviderModelGateway`, preserving the
    ///    one-fake-at-the-vendor-SDK-seam invariant), then submit through the
    ///    production submitter over the SAME conversation services — its own
    ///    resolve reuses the just-created binding, so the run executes under
    ///    the pre-registered scope with no race.
    ///
    /// Two production materializer steps are intentionally NOT mirrored:
    /// `authorize_trigger_fire` and `validate_trusted_trigger_prompt` — both
    /// are poller-side pre-flight already pinned by
    /// `trigger_poller_trusted_submit.rs`'s own crate-tier tests, and neither
    /// affects the submit→run wire this seam exists to drive.
    pub(crate) async fn submit_triggered_turn_scripted(
        &self,
        prompt: &str,
        replies: impl IntoIterator<Item = RebornScriptedReply>,
    ) -> HarnessResult<TriggeredSubmission> {
        let fire_slot = triggered_fire_slot();
        let fire = self.triggered_fire(prompt, fire_slot);

        let conversations = self.trigger_conversations_with_paired_actor().await?;

        // Production materializer step 1: resolve the binding (mints the scope).
        let trusted_inbound_binding = TriggerTrustedInboundBinding::for_fire(&fire);
        let resolve_request = ResolveConversationRequest {
            tenant_id: self.binding.tenant_id.clone(),
            adapter_kind: AdapterKind::new(trusted_inbound_binding.adapter_kind())?,
            adapter_installation_id: AdapterInstallationId::new(
                trusted_inbound_binding.adapter_installation_id(),
            )?,
            external_actor_ref: ExternalActorRef::new(
                trusted_inbound_binding.external_actor_namespace(),
                trusted_inbound_binding.external_actor_id(),
            )?,
            external_conversation_ref: ExternalConversationRef::new(
                None,
                trusted_inbound_binding.external_conversation_id(),
                Some(trusted_inbound_binding.route_thread_id()),
                None,
            )?,
            external_event_id: ExternalEventId::new(trusted_inbound_binding.external_event_id())?,
            route_kind: ConversationRouteKind::Direct,
            requested_agent_id: None,
            requested_project_id: None,
        };
        let resolution = conversations
            .resolve_or_create_binding_with_trusted_scope(
                resolve_request,
                fire.agent_id.clone(),
                fire.project_id.clone(),
                Some(fire.creator_user_id.clone()),
            )
            .await?;

        // Production materializer step 2 (`record_trigger_prompt`): ensure the
        // trigger thread + record the prompt as a real inbound thread message
        // in the harness's REAL thread service.
        let thread_service = self.thread_harness.service_instance()?;
        let agent_id = resolution
            .turn_scope
            .agent_id
            .clone()
            .ok_or("triggered binding resolution missing agent id")?;
        let thread_scope = ThreadScope {
            tenant_id: resolution.turn_scope.tenant_id.clone(),
            agent_id,
            project_id: resolution.turn_scope.project_id.clone(),
            owner_user_id: Some(resolution.actor.user_id.clone()),
            mission_id: None,
        };
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: thread_scope.clone(),
                thread_id: Some(resolution.turn_scope.thread_id.clone()),
                created_by_actor_id: resolution.actor.user_id.as_str().to_string(),
                title: None,
                metadata_json: Some(automation_trigger_thread_metadata_json(
                    fire.identity.trigger_id(),
                )),
            })
            .await?;
        let accepted = thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: thread_scope,
                thread_id: resolution.turn_scope.thread_id.clone(),
                actor_id: resolution.actor.user_id.as_str().to_string(),
                source_binding_id: Some(resolution.source_binding_ref.as_str().to_string()),
                reply_target_binding_id: Some(
                    resolution.reply_target_binding_ref.as_str().to_string(),
                ),
                external_event_id: Some(format!(
                    "trigger:{}",
                    trusted_inbound_binding.external_event_id()
                )),
                content: MessageContent::text(prompt.to_string()),
            })
            .await?;

        // Production materializer step 3: the REAL content-ref shape.
        let content_ref =
            TriggerInboundContentRef::new(format!("thread-message:{}", accepted.message_id))?;
        let materialized_prompt =
            TriggerMaterializedPrompt::new(content_ref, trusted_inbound_binding);

        // Step 4: scripted gateway for the EXACT resolved scope, registered
        // before the submit so the scheduler can never race an unrouted scope.
        let scripted_llm: Arc<TraceLlm> = Arc::new(scripted_trace_llm(replies));
        let raw: Arc<dyn LlmProvider> = scripted_llm;
        // Distinct session path so the triggered gateway never clobbers the
        // harness thread's own LLM session cache under the same turn_root.
        let session = create_session_manager(SessionConfig {
            session_path: self
                ._shared
                .turn_root
                .path()
                .join(format!("{}.triggered.session.json", self.conversation_id)),
            ..SessionConfig::default()
        })
        .await;
        let llm_config = ironclaw_llm::testing::nearai_test_config(SCRIPTED_MODEL_NAME);
        let provider = provider_chain_over(raw, &llm_config, session).await?;
        let model_profile_id = ModelProfileId::new(INTERACTIVE_MODEL_PROFILE)
            .map_err(|reason| format!("invalid model profile id: {reason}"))?;
        let policy = LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, None);
        let gateway: Arc<dyn HostManagedModelGateway> =
            Arc::new(LlmProviderModelGateway::new(provider, policy));
        self._shared
            .scope_gateway
            .register(resolution.turn_scope.clone(), gateway);

        let request =
            TrustedTriggerSubmitRequest::new_for_test(fire, materialized_prompt, fire_slot);
        let submitter = trusted_trigger_fire_submitter(
            conversations.clone(),
            conversations,
            Arc::clone(&self.coordinator),
        );
        match submitter.submit_trusted_trigger_fire(request).await? {
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id, turn_scope, ..
            } => {
                if turn_scope != resolution.turn_scope {
                    return Err(format!(
                        "triggered submit resolved a different scope than the pre-submit \
                         materializer resolve (gateway registered for the wrong scope): \
                         pre-submit={:?} submit={turn_scope:?}",
                        resolution.turn_scope
                    )
                    .into());
                }
                Ok(TriggeredSubmission { run_id, turn_scope })
            }
            TrustedTriggerFireSubmitOutcome::Replayed { .. } => {
                Err("first triggered submit unexpectedly replayed".into())
            }
        }
    }

    /// `wait_for_status` for a run living in a scope OTHER than this harness
    /// thread's own (`builder.rs::wait_for_status` polls `self.turn_scope`) —
    /// today only triggered runs, whose scope is minted at submit time and
    /// returned on [`TriggeredSubmission`]. Same poll loop, deadline, and
    /// fail-fast-on-wrong-terminal semantics.
    pub(crate) async fn wait_for_status_in_scope(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        expected: TurnStatus,
    ) -> HarnessResult<TurnRunState> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let state = self
                .turn_store
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await?;
            if state.status == expected {
                return Ok(state);
            }
            if state.status.is_terminal() {
                return Err(format!(
                    "expected {expected:?} but run reached terminal status {:?}; failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for {expected:?}; last status={:?} failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// `approve_gate` for a run in a non-thread scope (triggered runs). Same
    /// two steps as `builder.rs::approve_gate` — resolve the persisted approval
    /// request to an issued lease, then resume with the production
    /// `BlockedApprovalGate` precondition — but resuming in the given scope.
    pub(crate) async fn approve_gate_in_scope(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &GateRef,
    ) -> HarnessResult<()> {
        self.capability_recorder
            .approve_local_dev_gate(gate_ref)
            .await?;
        self.resume_run_in_scope(
            scope,
            run_id,
            gate_ref.clone(),
            None,
            ResumeTurnPrecondition::BlockedApprovalGate,
        )
        .await
    }

    /// `deny_gate` for a run in a non-thread scope (triggered runs). Mirrors
    /// `builder.rs::deny_gate`: resolve the persisted request to `Denied`, then
    /// resume with `GateResumeDisposition::Denied` so the executor surfaces a
    /// non-retryable authorization failure to the model.
    pub(crate) async fn deny_gate_in_scope(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: &GateRef,
    ) -> HarnessResult<()> {
        self.capability_recorder
            .deny_local_dev_gate(gate_ref)
            .await?;
        self.resume_run_in_scope(
            scope,
            run_id,
            gate_ref.clone(),
            Some(GateResumeDisposition::Denied),
            ResumeTurnPrecondition::BlockedApprovalGate,
        )
        .await
    }

    /// Scope-parameterised twin of `builder.rs::resume_run` (which resumes in
    /// `self.turn_scope`). The actor stays the harness actor: a trigger fire's
    /// creator IS `binding.actor_user_id` (`submit_triggered_turn` builds the
    /// fire from it), so the approving user and the trigger creator coincide,
    /// as in production's approval-resolution path.
    async fn resume_run_in_scope(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: GateRef,
        resume_disposition: Option<GateResumeDisposition>,
        precondition: ResumeTurnPrecondition,
    ) -> HarnessResult<()> {
        let response = self
            .coordinator
            .resume_turn(ResumeTurnRequest {
                scope: scope.clone(),
                actor: TurnActor::new(self.binding.actor_user_id.clone()),
                run_id,
                gate_resolution_ref: gate_ref,
                precondition,
                source_binding_ref: SourceBindingRef::new("src:resume")?,
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:resume")?,
                idempotency_key: IdempotencyKey::new(format!("resume-{run_id}"))?,
                resume_disposition,
            })
            .await?;
        if response.status != TurnStatus::Queued {
            return Err(format!("expected resumed run to queue, got {:?}", response.status).into());
        }
        Ok(())
    }
}
