//! E-TRIGGERED-SUBMIT seam: submits turns through the REAL
//! `trusted_trigger_fire_submitter` (not a fake) over the harness's shared
//! `coordinator`, so runs carry a genuine `TurnOriginKind::ScheduledTrigger`
//! origin and land in the same turn store/scheduler as any other turn.
//!
//! Two variants: [`IronClawIntegrationHarness::submit_triggered_turn`]
//! (submit-only; asserts submit-time state) and
//! [`IronClawIntegrationHarness::submit_triggered_turn_scripted`] (full
//! production materialization + scripted gateway, drivable to completion or
//! a mid-fire gate).

// Shared integration-test support: not every binary that mounts the
// `ironclaw_support` tree consumes this module — its symbols read as dead there
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
use ironclaw_loop_host::HostManagedModelGateway;
use ironclaw_runner::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
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

use super::builder::{INTERACTIVE_MODEL_PROFILE, IronClawIntegrationHarness};
use super::reply::IronClawScriptedReply;
use super::scripted_provider::{SCRIPTED_MODEL_NAME, scripted_trace_llm};
use crate::support::trace_llm::TraceLlm;

// `builder.rs`'s `HarnessResult` is module-private, so each sibling file
// declares its own identical copy rather than reaching across the boundary.
type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Far-future, deterministic fire slot — avoids wall-clock flake.
fn triggered_fire_slot() -> chrono::DateTime<chrono::Utc> {
    Utc.with_ymd_and_hms(2999, 1, 1, 0, 0, 0).unwrap()
}

/// Result of a successful `submit_triggered_turn` call.
pub(crate) struct TriggeredSubmission {
    pub(crate) run_id: TurnRunId,
    /// The trigger's OWN resolved scope. The trusted-submit contract forbids
    /// re-deriving binding keys from a `TriggerFire`, so callers must read
    /// this back rather than reconstruct it.
    pub(crate) turn_scope: TurnScope,
}

impl IronClawIntegrationHarness {
    /// Synthetic `TriggerFire` shared by both triggered-submit seams, handed
    /// to the production submitter.
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
            delivery_target: None,
        }
    }

    /// Fresh `InMemoryConversationServices` with the trigger's actor
    /// pre-paired (mirrors `TriggerTrustedInboundBinding::for_fire`;
    /// production's `resolve_actor` hard-fails `BindingRequired` without it).
    /// Uses `try_pair_external_actor` so a pairing failure surfaces here, not
    /// as an indirect binding-resolution error downstream.
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

    /// Submits through the REAL `TrustedTriggerFireSubmitter` for a genuine
    /// `TurnOriginKind::ScheduledTrigger` origin (E-TRIGGERED-SUBMIT). No
    /// scripted gateway is registered, so the run fails benignly on
    /// scope-miss after submit-time state (e.g. persisted origin) is
    /// recorded — this seam asserts only that state; driving to model
    /// completion is C-TRIGGERED-DELIVERY.
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

    /// [`submit_triggered_turn`](Self::submit_triggered_turn) with model
    /// calls scripted and the prompt recorded into the harness's REAL thread
    /// service, so the run can be driven to completion or a mid-fire gate
    /// instead of failing on scope-miss.
    ///
    /// Materializes via `materialize_trigger_prompt_for_test`, which calls
    /// the SAME production code as the trigger poller's own materializer —
    /// trusted-trigger materialization is an ownership boundary (see
    /// `AGENTS.md`), so this must not hand-mirror it field-by-field.
    ///
    /// After materialization, registers the scripted gateway for the exact
    /// resolved scope (real-chain construction: `scripted_trace_llm` →
    /// `provider_chain_over` → `LlmProviderModelGateway`, preserving the
    /// one-fake-at-the-vendor-SDK-seam invariant) before submitting, so the
    /// run executes under the pre-registered scope with no race.
    pub(crate) async fn submit_triggered_turn_scripted(
        &self,
        prompt: &str,
        replies: impl IntoIterator<Item = IronClawScriptedReply>,
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
            ironclaw_composition::test_support::materialize_trigger_prompt_for_test(
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

    /// Scope-parameterized twin of `builder.rs::wait_for_status` (which polls
    /// `self.turn_scope`), for runs in another scope (e.g. triggered runs).
    /// Same poll loop, deadline, and fail-fast-on-wrong-terminal semantics.
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

    /// `builder.rs::approve_gate` for a non-thread scope (triggered runs):
    /// resolve the persisted approval to an issued lease, then resume with
    /// `BlockedApprovalGate` precondition in the given scope.
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

    /// `builder.rs::deny_gate` for a non-thread scope (triggered runs):
    /// resolve the persisted request to `Denied`, then resume with
    /// `GateResumeDisposition::Denied` so the executor surfaces a
    /// non-retryable authorization failure.
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

    /// Scope-parameterized twin of `builder.rs::resume_run` (resumes in
    /// `self.turn_scope`). The approving actor is the harness actor, matching
    /// the trigger creator (`binding.actor_user_id`), as in production.
    ///
    /// `pub(crate)` so deny-edge scenarios can drive a resume with a
    /// deliberately WRONG scope without hand-rebuilding `ResumeTurnRequest` —
    /// a coordinator-level rejection propagates out for the caller to pin.
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
