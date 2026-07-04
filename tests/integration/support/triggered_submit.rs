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
//! - [`RebornIntegrationHarness::submit_triggered_turn_scripted`] — materializes
//!   through the REAL production trusted-trigger pipeline
//!   (`ironclaw_reborn_composition::test_support::materialize_trigger_prompt_for_test`:
//!   authorize, validate, resolve binding, thread-recorded prompt, real
//!   content ref) and registers a scripted gateway for the fire's exact
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
    AdapterInstallationId, AdapterKind, ExternalActorRef, InMemoryConversationServices,
    trusted_trigger_fire_submitter,
};
use ironclaw_llm::testing::provider_chain_over;
use ironclaw_llm::{LlmProvider, SessionConfig, create_session_manager};
use ironclaw_loop_support::HostManagedModelGateway;
use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerFire, TriggerFireIdentity, TriggerId,
    TriggerInboundContentRef, TriggerMaterializedPrompt, TrustedTriggerFireSubmitOutcome,
    TrustedTriggerSubmitRequest,
};
use ironclaw_turns::run_profile::ModelProfileId;
use ironclaw_turns::{
    GateRef, GateResumeDisposition, GetRunStateRequest, ResumeTurnPrecondition, TurnRunId,
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
    /// See the module docs for how the fire/prompt/conversation-services are built.
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
    /// Materializes through the REAL production trusted-trigger pipeline via
    /// `ironclaw_reborn_composition::test_support::materialize_trigger_prompt_for_test`
    /// (authorize, validate, resolve-or-create binding, record the prompt as
    /// a real inbound thread message, build the real `thread-message:<id>`
    /// content ref, AND return the resolved `TurnScope`) rather than
    /// hand-mirroring `trigger_resolve_request` + `record_trigger_prompt` +
    /// the content-ref shape field-by-field — flagged as a drift trap on PR
    /// #5584 (trusted-trigger materialization is an ownership boundary,
    /// `AGENTS.md:61`) and extracted as the agreed fast-follow. This
    /// mirrors the production trigger poller's OWN materializer
    /// (`ConversationContentRefMaterializer::materialize_prompt`) exactly,
    /// since it now calls the SAME code, not a copy of it — which the plain
    /// `submit_triggered_turn`'s `TriggerMaterializedPrompt::for_fire`
    /// shortcut still skips.
    ///
    /// After materialization: register the scripted gateway for the EXACT
    /// resolved scope on the group's `ScopeRegistryGateway` (the same
    /// real-chain construction thread gateways use: `scripted_trace_llm` →
    /// `provider_chain_over` → `LlmProviderModelGateway`, preserving the
    /// one-fake-at-the-vendor-SDK-seam invariant), then submit through the
    /// production submitter over the SAME conversation services — its own
    /// resolve reuses the just-created binding, so the run executes under the
    /// pre-registered scope with no race.
    pub(crate) async fn submit_triggered_turn_scripted(
        &self,
        prompt: &str,
        replies: impl IntoIterator<Item = RebornScriptedReply>,
    ) -> HarnessResult<TriggeredSubmission> {
        let fire_slot = triggered_fire_slot();
        let fire = self.triggered_fire(prompt, fire_slot);

        let conversations = self.trigger_conversations_with_paired_actor().await?;
        // Unsize-coerced to `Arc<dyn SessionThreadService>` at the call below
        // (the parameter type), so no trait import is needed here.
        let thread_service = Arc::new(self.thread_harness.service_instance()?);
        let default_agent_id = self
            .binding
            .agent_id
            .clone()
            .ok_or("triggered submit requires a harness binding with an agent id")?;

        let (materialized_prompt, turn_scope) =
            ironclaw_reborn_composition::test_support::materialize_trigger_prompt_for_test(
                conversations.clone(),
                thread_service,
                default_agent_id,
                fire.clone(),
            )
            .await?;

        // Scripted gateway for the EXACT resolved scope, registered before
        // the submit so the scheduler can never race an unrouted scope.
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
            .register(turn_scope.clone(), gateway);

        let request =
            TrustedTriggerSubmitRequest::new_for_test(fire, materialized_prompt, fire_slot);
        let submitter = trusted_trigger_fire_submitter(
            conversations.clone(),
            conversations,
            Arc::clone(&self.coordinator),
        );
        match submitter.submit_trusted_trigger_fire(request).await? {
            TrustedTriggerFireSubmitOutcome::Accepted {
                run_id,
                turn_scope: submitted_scope,
                ..
            } => {
                if submitted_scope != turn_scope {
                    return Err(format!(
                        "triggered submit resolved a different scope than the pre-submit \
                         materializer resolve (gateway registered for the wrong scope): \
                         pre-submit={turn_scope:?} submit={submitted_scope:?}",
                    )
                    .into());
                }
                Ok(TriggeredSubmission {
                    run_id,
                    turn_scope: submitted_scope,
                })
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
    ///
    /// `pub(crate)` (not just a private tail of `approve_gate_in_scope`/
    /// `deny_gate_in_scope`) so deny-edge scenarios can drive a resume with a
    /// deliberately WRONG scope without rebuilding the `ResumeTurnRequest` by
    /// hand — a coordinator-level rejection (`TurnError`) propagates out
    /// unchanged for the caller to pin.
    pub(crate) async fn resume_run_in_scope(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        gate_ref: GateRef,
        resume_disposition: Option<GateResumeDisposition>,
        precondition: ResumeTurnPrecondition,
    ) -> HarnessResult<()> {
        self.resume_run_in_scope_impl(
            scope.clone(),
            run_id,
            gate_ref,
            resume_disposition,
            precondition,
        )
        .await
    }
}
