use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_product_workflow::{
    OutboundPreferencesProductFacade, RebornOutboundDeliveryTargetStatus, WebUiAuthenticatedCaller,
};
use ironclaw_turns::{
    run_profile::{
        CommunicationContextProvider, CommunicationRuntimeContext, ConnectedChannelsState,
        DeliveryTargetState, DeliveryTargetSummary,
    },
    scope::{TurnActor, TurnScope},
};
use tokio::time::timeout;

const OUTBOUND_PREFERENCES_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct RuntimeCommunicationContextProvider {
    outbound_preferences: Arc<dyn OutboundPreferencesProductFacade>,
}

impl RuntimeCommunicationContextProvider {
    pub(crate) fn new(outbound_preferences: Arc<dyn OutboundPreferencesProductFacade>) -> Self {
        Self {
            outbound_preferences,
        }
    }
}

#[async_trait]
impl CommunicationContextProvider for RuntimeCommunicationContextProvider {
    async fn communication_context(
        &self,
        scope: &TurnScope,
        actor: Option<&TurnActor>,
        delivery_tools_visible: bool,
    ) -> Option<CommunicationRuntimeContext> {
        let actor = actor?;
        let caller = WebUiAuthenticatedCaller::new(
            scope.tenant_id.clone(),
            actor.user_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
        );
        let delivery_target = match timeout(
            OUTBOUND_PREFERENCES_TIMEOUT,
            self.outbound_preferences.get_outbound_preferences(caller),
        )
        .await
        {
            Ok(Ok(response)) => match (
                response.final_reply_target,
                response.final_reply_target_status,
            ) {
                (Some(target), _) => DeliveryTargetState::Set(DeliveryTargetSummary {
                    display_name: target.display_name.as_str().to_string(),
                    channel: target.channel.as_str().to_string(),
                }),
                // A target is stored but the resolving registry in this
                // composition cannot produce its summary (e.g. no delivery
                // target providers wired). Never report "none set" here — a
                // preference exists and triggered delivery will use it.
                (None, RebornOutboundDeliveryTargetStatus::Unavailable) => {
                    DeliveryTargetState::SetUnresolved
                }
                (None, _) => DeliveryTargetState::NoneSet,
            },
            Ok(Err(_)) | Err(_) => DeliveryTargetState::Unknown,
        };

        Some(CommunicationRuntimeContext {
            connected_channels: ConnectedChannelsState::Unknown,
            delivery_target,
            delivery_tools_visible,
            run_origin: None,
        })
    }
}
