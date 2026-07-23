use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_auth::{AuthProductError, AuthProviderId, OAuthAuthorizationUrl};
use ironclaw_host_api::{AgentId, ProjectId, UserId};
use ironclaw_product::AuthPromptChallengeKind;
use ironclaw_turns::{TurnRunId, TurnScope};

use ironclaw_product::{AuthChallengeProvider, AuthChallengeView};

use super::{AGENT, AUTH_GATE, PROJECT, TENANT, USER};

type AuthChallengeCall = (
    TurnScope,
    UserId,
    TurnRunId,
    String,
    Vec<ironclaw_host_api::RuntimeCredentialAuthRequirement>,
);

#[derive(Debug, Default)]
pub(super) struct FakeAuthChallengeProvider {
    calls: Mutex<Vec<AuthChallengeCall>>,
}

impl FakeAuthChallengeProvider {
    pub(super) fn assert_single_call(&self) {
        let calls = self
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let [(scope, owner_user_id, run_id, gate_ref, credential_requirements)] = calls.as_slice()
        else {
            panic!(
                "expected one auth challenge provider call, got {}",
                calls.len()
            );
        };
        assert_eq!(scope.tenant_id.as_str(), TENANT); // safety: test-only fake provider assertion.
        assert_eq!(scope.agent_id.as_ref().map(AgentId::as_str), Some(AGENT)); // safety: test-only fake provider assertion.
        let project_id = scope.project_id.as_ref().map(ProjectId::as_str);
        assert_eq!(project_id, Some(PROJECT)); // safety: test-only fake provider assertion.
        let explicit_owner_user_id = scope.explicit_owner_user_id().map(UserId::as_str);
        assert_eq!(explicit_owner_user_id, Some(USER)); // safety: test-only fake provider assertion.
        assert_eq!(owner_user_id.as_str(), USER); // safety: test-only fake provider assertion.
        assert!(!run_id.to_string().is_empty()); // safety: test-only fake provider assertion.
        assert_eq!(gate_ref, AUTH_GATE); // safety: test-only fake provider assertion.
        assert!(credential_requirements.is_empty()); // safety: test-only fake provider assertion.
    }
}

#[async_trait]
impl AuthChallengeProvider for FakeAuthChallengeProvider {
    async fn challenge_for_gate(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
        credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((
                scope.clone(),
                owner_user_id.clone(),
                run_id,
                gate_ref.to_string(),
                credential_requirements.to_vec(),
            ));
        // Keyed by the gate only: DM runs own their gates as the bound user,
        // admitted shared channels as their managed/configured subject — the
        // engine serves the OAuth challenge either way (the channel-side
        // suppression of the personal setup link is the behavior under test).
        if gate_ref != AUTH_GATE {
            return Ok(None);
        }
        Ok(Some(AuthChallengeView {
            kind: AuthPromptChallengeKind::OAuthUrl,
            provider: AuthProviderId::new("provider".to_string())
                .expect("static provider id should be valid"), // safety: static test provider id is valid.
            account_label: None,
            authorization_url: Some(
                OAuthAuthorizationUrl::new("https://provider.example/oauth".to_string())
                    .expect("static OAuth URL should be valid"), // safety: static test URL is valid.
            ),
            expires_at: None,
            pairing: None,
        }))
    }
}
