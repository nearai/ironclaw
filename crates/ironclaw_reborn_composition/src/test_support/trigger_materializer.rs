//! Trusted-trigger prompt materialization test support (E-TRIGGERED-SUBMIT
//! seam).
//!
//! Single production-owned entry point for driving a triggered run's
//! materialization step in an integration-test harness — see
//! [`materialize_trigger_prompt_for_test`]. This helper routes tests through
//! `ConversationContentRefMaterializer::materialize_prompt` so they avoid
//! duplicating trusted-trigger materialization, the PR #5584 drift trap around
//! trusted-trigger binding and thread recording.

/// Materialize a `TriggerFire`'s prompt through the REAL trusted-trigger
/// pipeline (`ConversationContentRefMaterializer::materialize_prompt` —
/// authorize, validate, resolve-or-create binding, record the prompt as a
/// real inbound thread message, build the real `thread-message:<id>` content
/// ref), and additionally return the resolved `TurnScope` the materializer
/// mints internally but the production `TriggerPromptMaterializer` trait does
/// not expose — the value an integration-test harness needs to register a
/// scripted model gateway for the trigger's exact scope before submitting.
///
/// `binding_service`/`thread_service` are the caller's own (real, in-process)
/// conversation/thread services — this does not fake or re-implement the
/// materialization logic itself, only supplies the collaborators production
/// wires from its own `RebornServices` assembly. Tests only.
#[cfg(feature = "test-support")]
pub async fn materialize_trigger_prompt_for_test(
    binding_service: ironclaw_conversations::InMemoryConversationServices,
    thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    default_agent_id: ironclaw_host_api::AgentId,
    fire: ironclaw_triggers::TriggerFire,
) -> Result<
    (
        ironclaw_triggers::TriggerMaterializedPrompt,
        ironclaw_turns::TurnScope,
    ),
    ironclaw_triggers::TriggerError,
> {
    crate::trigger_poller_trusted_submit::materialize_trigger_prompt_for_test(
        binding_service,
        thread_service,
        default_agent_id,
        fire,
    )
    .await
}
