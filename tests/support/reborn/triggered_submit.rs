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

// Shared integration-test support: not every binary that mounts the
// `reborn_support` tree consumes this module — its symbols read as dead there
// under the all-features `-D warnings` lane. Module-level allow matches
// `builder.rs`/`assertions.rs`/`session_thread.rs`.
#![allow(dead_code)]

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ExternalActorRef, InMemoryConversationServices,
    trusted_trigger_fire_submitter,
};
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerFire, TriggerFireIdentity, TriggerId,
    TriggerInboundContentRef, TriggerMaterializedPrompt, TrustedTriggerFireSubmitOutcome,
    TrustedTriggerSubmitRequest,
};
use ironclaw_turns::{TurnRunId, TurnScope};

use super::builder::RebornIntegrationHarness;

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
        let tenant_id = self.binding.tenant_id.clone();
        let creator_user_id = self.binding.actor_user_id.clone();
        let fire_slot = triggered_fire_slot();
        let fire = TriggerFire {
            identity: TriggerFireIdentity::new(tenant_id.clone(), TriggerId::new(), fire_slot),
            creator_user_id: creator_user_id.clone(),
            agent_id: self.binding.agent_id.clone(),
            project_id: self.binding.project_id.clone(),
            prompt: prompt.to_string(),
        };
        let content_ref = TriggerInboundContentRef::new(format!(
            "content:triggered-submit:{}",
            fire.identity.external_event_id().as_str()
        ))?;
        let materialized_prompt = TriggerMaterializedPrompt::for_fire(&fire, content_ref);
        let request =
            TrustedTriggerSubmitRequest::new_for_test(fire, materialized_prompt, fire_slot);

        // Pre-pair the trigger's canonical external actor — mirrors
        // `TriggerTrustedInboundBinding::for_fire`'s own derivation exactly and mirrors
        // production's pre-seed requirement (`resolve_actor` hard-fails
        // `BindingRequired` without it, on both trusted and untrusted resolve paths).
        // Uses `try_pair_external_actor` (not the infallible `pair_external_actor`
        // wrapper) so a pairing failure surfaces here, at the seam boundary, instead
        // of resurfacing later as an indirect binding-resolution error.
        let conversations = InMemoryConversationServices::default();
        conversations
            .try_pair_external_actor(
                tenant_id,
                AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND)?,
                AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)?,
                ExternalActorRef::new(
                    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE,
                    creator_user_id.as_str(),
                )?,
                creator_user_id,
            )
            .await?;

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
}
