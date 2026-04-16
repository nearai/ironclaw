use crate::channels::web::types::{AppEvent, ChannelOnboardingState, OnboardingStateDto};
use crate::extensions::ConfigureResult;

pub(crate) enum ConfigureFlowOutcome {
    Ready,
    AuthRequired,
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
    } else if result.auth_url.is_some()
        || matches!(
            result.onboarding_state,
            Some(ChannelOnboardingState::AuthRequired)
        )
    {
        ConfigureFlowOutcome::AuthRequired
    } else {
        ConfigureFlowOutcome::RetryAuth
    }
}

pub(crate) fn event_from_configure_result(
    extension_name: String,
    result: &ConfigureResult,
    thread_id: Option<String>,
) -> AppEvent {
    let outcome = classify_configure_result(result);
    let state = match &outcome {
        ConfigureFlowOutcome::PairingRequired { .. } => OnboardingStateDto::PairingRequired,
        ConfigureFlowOutcome::Ready => OnboardingStateDto::Ready,
        ConfigureFlowOutcome::AuthRequired => OnboardingStateDto::AuthRequired,
        ConfigureFlowOutcome::RetryAuth => OnboardingStateDto::Failed,
    };

    let instructions = match outcome {
        ConfigureFlowOutcome::AuthRequired => Some(result.message.clone()),
        _ => None,
    };

    AppEvent::OnboardingState {
        extension_name,
        state,
        request_id: None,
        message: Some(result.message.clone()),
        instructions,
        auth_url: result.auth_url.clone(),
        setup_url: None,
        onboarding: result
            .onboarding
            .as_ref()
            .and_then(|o| serde_json::to_value(o).ok()),
        thread_id,
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigureFlowOutcome, classify_configure_result, event_from_configure_result};
    use crate::channels::web::types::ChannelOnboardingState;
    use crate::extensions::ConfigureResult;

    #[test]
    fn classify_configure_result_treats_oauth_continuation_as_auth_required() {
        let result = ConfigureResult {
            activated: false,
            message: "Complete authentication to continue.".to_string(),
            auth_url: Some("https://example.test/oauth".to_string()),
            pairing_required: false,
            onboarding_state: Some(ChannelOnboardingState::AuthRequired),
            onboarding: None,
        };

        assert!(matches!(
            classify_configure_result(&result),
            ConfigureFlowOutcome::AuthRequired
        ));
    }

    #[test]
    fn event_from_configure_result_emits_auth_required_for_oauth_continuation() {
        let result = ConfigureResult {
            activated: false,
            message: "Complete authentication to continue.".to_string(),
            auth_url: Some("https://example.test/oauth".to_string()),
            pairing_required: false,
            onboarding_state: Some(ChannelOnboardingState::AuthRequired),
            onboarding: None,
        };

        let event = event_from_configure_result("notion".to_string(), &result, Some("t1".into()));
        match event {
            crate::channels::web::types::AppEvent::OnboardingState {
                state,
                auth_url,
                instructions,
                ..
            } => {
                assert_eq!(
                    state,
                    crate::channels::web::types::OnboardingStateDto::AuthRequired
                );
                assert_eq!(auth_url.as_deref(), Some("https://example.test/oauth"));
                assert_eq!(
                    instructions.as_deref(),
                    Some("Complete authentication to continue.")
                );
            }
            other => panic!("expected onboarding state event, got {other:?}"),
        }
    }
}
