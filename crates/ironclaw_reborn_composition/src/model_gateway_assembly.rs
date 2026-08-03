use std::sync::Arc;

use ironclaw_extension_host::skill_learning::SkillLearnedNotifier;
use ironclaw_host_api::ids::UserId;
use ironclaw_product::projection::LiveProjectionPublisher;
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::runtime::RebornRuntimeError;

/// [`SkillLearnedNotifier`] over the runtime's live projection publisher —
/// emits a `SkillActivation` projection item rendered as a chat bubble.
///
/// The adapter lives here, not beside the port. `LiveProjectionPublisher` is
/// one of `ironclaw_product`'s *concrete* types, and naming it was the whole
/// of `ironclaw_extension_host::skill_learning`'s `ironclaw_product`
/// dependency — the last one that file carried, and the one that would have
/// blocked the crate's `products` → `loops` re-layer. The port's own doc always
/// said "composition implements it over the projection publisher"; this makes
/// that true (PROPOSAL §6.8.2 shed list, CHECKLIST WS2 strays row).
///
/// It sits in this module because composition's other skill-learning assembly
/// piece — `build_skill_learning_provider` below — already does. It is
/// deliberately *not* a crate-root `skill_learning` module: the whole module is
/// what #6616/#6691 moved out of composition, and
/// `reborn_composition_boundaries.rs` holds it out.
pub(crate) struct LiveSkillLearnedNotifier {
    publisher: Arc<LiveProjectionPublisher>,
}

impl LiveSkillLearnedNotifier {
    pub(crate) fn new(publisher: Arc<LiveProjectionPublisher>) -> Self {
        Self { publisher }
    }
}

impl SkillLearnedNotifier for LiveSkillLearnedNotifier {
    fn notify(
        &self,
        owner: &UserId,
        scope: &TurnScope,
        run_id: TurnRunId,
        skill_name: &str,
        feedback: &str,
    ) {
        self.publisher
            .publish_skill_learned(Some(owner), scope, run_id, skill_name, feedback);
    }
}

pub(crate) async fn build_production_model_gateway(
    provider_factory: Option<ironclaw_operator::RebornProviderFactory>,
) -> Result<
    (
        Arc<dyn ironclaw_loop_host::HostManagedModelGateway>,
        Option<ironclaw_loop_host::StaticModelCostTable>,
        Option<RebornLlmReloadParts>,
    ),
    RebornRuntimeError,
> {
    let LlmGatewayBundle {
        gateway, reload, ..
    } = build_placeholder_llm_gateway(provider_factory).await?;
    Ok((gateway, None, Some(reload)))
}

pub(crate) async fn build_skill_learning_provider(
    config: &ironclaw_llm::LlmConfig,
) -> Option<(Arc<dyn ironclaw_llm::LlmProvider>, String)> {
    let model = std::env::var("IRONCLAW_SKILL_LEARNING_MODEL")
        .ok()
        .filter(|model| !model.trim().is_empty())?;
    if !matches!(config.backend.as_str(), "nearai" | "near_ai" | "near") {
        tracing::debug!(
            backend = %config.backend,
            "skill-learning: learning model is only wired for the nearai backend; skill learning disabled"
        );
        return None;
    }
    let mut nearai = config.nearai.clone();
    nearai.model = model.clone();
    let session = ironclaw_llm::create_session_manager(config.session.clone()).await;
    match ironclaw_llm::create_llm_provider_with_config(
        &nearai,
        session,
        config.request_timeout_secs,
    ) {
        Ok(provider) => Some((provider, model)),
        Err(error) => {
            tracing::debug!(%error, "skill-learning: could not build the learning provider; skill learning disabled");
            None
        }
    }
}

pub(crate) struct LlmGatewayBundle {
    pub(crate) gateway: Arc<dyn ironclaw_loop_host::HostManagedModelGateway>,
    pub(crate) reload: RebornLlmReloadParts,
}

pub(crate) struct RebornLlmReloadParts {
    pub(crate) reload_handle: Arc<ironclaw_llm::LlmReloadHandle>,
    pub(crate) session: Arc<ironclaw_llm::SessionManager>,
    pub(crate) nearai_login_states:
        Arc<ironclaw_operator::llm_admin::llm_config_service::NearAiLoginStateStore>,
}

async fn build_placeholder_llm_gateway(
    provider_factory: Option<ironclaw_operator::RebornProviderFactory>,
) -> Result<LlmGatewayBundle, RebornRuntimeError> {
    let session =
        ironclaw_llm::create_session_manager(ironclaw_llm::SessionConfig::default()).await;
    let raw: Arc<dyn ironclaw_llm::LlmProvider> = Arc::new(PlaceholderLlmProvider);
    wrap_swappable_gateway(raw, session, provider_factory)
}

/// Apply instrumentation outside the swappable provider so it survives reloads.
pub(crate) fn wrap_swappable_gateway(
    raw: Arc<dyn ironclaw_llm::LlmProvider>,
    session: Arc<ironclaw_llm::SessionManager>,
    provider_factory: Option<ironclaw_operator::RebornProviderFactory>,
) -> Result<LlmGatewayBundle, RebornRuntimeError> {
    use ironclaw_llm::{LlmProvider, LlmReloadHandle, SwappableLlmProvider};
    use ironclaw_loop_contracts::ModelProfileId;
    use ironclaw_runner::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};

    let swappable = Arc::new(SwappableLlmProvider::new(raw));
    let reload_handle = Arc::new(LlmReloadHandle::new(Arc::clone(&swappable), None));
    let swappable_provider: Arc<dyn LlmProvider> = swappable;
    let provider: Arc<dyn LlmProvider> = match provider_factory {
        Some(factory) => factory(Arc::clone(&swappable_provider)),
        None => swappable_provider,
    };

    let model_profile_id = ModelProfileId::new("interactive_model").map_err(|reason| {
        RebornRuntimeError::LlmProvider(format!("invalid interactive model profile id: {reason}"))
    })?;
    let policy = LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, None);
    let gateway = LlmProviderModelGateway::new(provider, policy);
    Ok(LlmGatewayBundle {
        gateway: Arc::new(gateway),
        reload: RebornLlmReloadParts {
            reload_handle,
            session,
            nearai_login_states: Arc::new(
                ironclaw_operator::llm_admin::llm_config_service::NearAiLoginStateStore::new(),
            ),
        },
    })
}

#[derive(Debug)]
struct PlaceholderLlmProvider;

#[async_trait::async_trait]
impl ironclaw_llm::LlmProvider for PlaceholderLlmProvider {
    fn model_name(&self) -> &str {
        "unconfigured"
    }

    fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
    }

    async fn complete(
        &self,
        _request: ironclaw_llm::CompletionRequest,
    ) -> Result<ironclaw_llm::CompletionResponse, ironclaw_llm::LlmError> {
        Err(placeholder_unconfigured_error())
    }

    async fn complete_with_tools(
        &self,
        _request: ironclaw_llm::ToolCompletionRequest,
    ) -> Result<ironclaw_llm::ToolCompletionResponse, ironclaw_llm::LlmError> {
        Err(placeholder_unconfigured_error())
    }
}

fn placeholder_unconfigured_error() -> ironclaw_llm::LlmError {
    ironclaw_llm::LlmError::RequestFailed {
        provider: ironclaw_llm::UNCONFIGURED_PROVIDER_ID.to_string(),
        reason: "no LLM provider is configured yet; choose one in Settings → Inference".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironclaw_events::{DurableEventLog, InMemoryDurableEventLog};
    use ironclaw_host_api::ids::{AgentId, TenantId, ThreadId};
    use ironclaw_product::projection::build_reborn_projection_services;
    use ironclaw_product::{ProductOutboundPayload, ProductProjectionItem};
    use ironclaw_product_contracts::projection::ProjectionSubscriptionRequest;
    use ironclaw_turns::{ReplyTargetBindingRef, TurnActor};

    const SKILL_NAME: &str = "csv-column-sum";
    const FEEDBACK: &str = "picked this up summing a report column";

    /// The adapter is four lines of forwarding, which is exactly the shape that
    /// fails silently: a swapped `skill_name`/`feedback` pair still compiles
    /// (both are `&str`), and dropping the `Some(owner)` wrapper still compiles
    /// (the publisher takes `Option<&UserId>`) while quietly re-keying every
    /// learned-skill bubble onto the runtime operator's stream instead of the
    /// user's. Neither is reachable from `skill_learning.rs`'s `StubNotifier`
    /// tests, which stop at the port.
    ///
    /// So this drives the production trait object over the *real*
    /// `LiveProjectionPublisher` — no double anywhere — with the runtime actor
    /// deliberately different from the run owner, and reads the result back off
    /// the product event stream the WebUI actually drains.
    #[tokio::test]
    async fn live_notifier_forwards_each_argument_and_keys_the_bubble_to_the_run_owner() {
        let runtime_actor = UserId::new("skill-learned-runtime-actor").expect("valid user");
        let run_owner = UserId::new("skill-learned-run-owner").expect("valid user");
        let scope = TurnScope::new(
            TenantId::new("skill-learned-tenant").expect("valid tenant"),
            Some(AgentId::new("skill-learned-agent").expect("valid agent")),
            None,
            ThreadId::new("skill-learned-thread").expect("valid thread"),
        );

        let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
        let services = build_reborn_projection_services(
            event_log,
            ReplyTargetBindingRef::new("skill-learned-reply").expect("valid reply ref"),
        );
        let notifier: Arc<dyn SkillLearnedNotifier> = Arc::new(LiveSkillLearnedNotifier::new(
            services.live_projection_publisher(runtime_actor.clone()),
        ));

        notifier.notify(&run_owner, &scope, TurnRunId::new(), SKILL_NAME, FEEDBACK);

        let owner_items = skill_activations_for(&services, &run_owner, &scope).await;
        assert_eq!(
            owner_items,
            vec![(vec![SKILL_NAME.to_string()], vec![FEEDBACK.to_string()])],
            "the run owner must see exactly one learned-skill bubble carrying the name in \
             `skill_names` and the feedback in `feedback` — a swap or a dropped argument \
             shows up here"
        );

        let runtime_actor_items = skill_activations_for(&services, &runtime_actor, &scope).await;
        assert!(
            runtime_actor_items.is_empty(),
            "the bubble must be keyed to the run owner the notifier was handed, not to the \
             runtime actor the publisher was built with; found {runtime_actor_items:?}"
        );
    }

    async fn skill_activations_for(
        services: &ironclaw_product::projection::RebornProjectionServices,
        actor: &UserId,
        scope: &TurnScope,
    ) -> Vec<(Vec<String>, Vec<String>)> {
        services
            .product_event_stream()
            .drain(ProjectionSubscriptionRequest {
                actor: TurnActor::new(actor.clone()),
                scope: scope.clone(),
                after_cursor: None,
            })
            .await
            .expect("projection drain succeeds")
            .iter()
            .filter_map(|event| match event.payload() {
                ProductOutboundPayload::ProjectionUpdate { state } => Some(state.items.clone()),
                _ => None,
            })
            .flatten()
            .filter_map(|item| match item {
                ProductProjectionItem::SkillActivation {
                    skill_names,
                    feedback,
                    ..
                } => Some((skill_names, feedback)),
                _ => None,
            })
            .collect()
    }
}
