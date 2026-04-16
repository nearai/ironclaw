use crate::channels::web::types::{AppEvent, ChannelOnboardingState, OnboardingStateDto};
use crate::extensions::ConfigureResult;

pub(crate) enum ConfigureFlowOutcome {
    Ready,
    PairingRequired {
        instructions: Option<String>,
        onboarding: Option<serde_json::Value>,
    },
    RetryAuth,
}

pub(crate) fn classify_configure_result(result: &ConfigureResult) -> ConfigureFlowOutcome {
    if result.pairing_required
        || matches!(
            result.onboarding_state,
            Some(ChannelOnboardingState::PairingRequired)
        )
    {
        return ConfigureFlowOutcome::PairingRequired {
            instructions: result
                .onboarding
                .as_ref()
                .and_then(|o| o.pairing_instructions.clone()),
            onboarding: result
                .onboarding
                .as_ref()
                .and_then(|o| serde_json::to_value(o).ok()),
        };
    }

    if result.activated {
        ConfigureFlowOutcome::Ready
    } else {
        ConfigureFlowOutcome::RetryAuth
    }
}

pub(crate) fn event_from_configure_result(
    extension_name: String,
    result: &ConfigureResult,
    thread_id: Option<String>,
) -> AppEvent {
    let state = match classify_configure_result(result) {
        ConfigureFlowOutcome::PairingRequired { .. } => OnboardingStateDto::PairingRequired,
        ConfigureFlowOutcome::Ready => OnboardingStateDto::Ready,
        ConfigureFlowOutcome::RetryAuth => OnboardingStateDto::Failed,
    };

    AppEvent::OnboardingState {
        extension_name,
        state,
        request_id: None,
        message: Some(result.message.clone()),
        instructions: None,
        auth_url: result.auth_url.clone(),
        setup_url: None,
        onboarding: result
            .onboarding
            .as_ref()
            .and_then(|o| serde_json::to_value(o).ok()),
        thread_id,
    }
}
