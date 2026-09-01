use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_auth::{AuthProductError, AuthProviderId, OAuthAuthorizationUrl};
use ironclaw_extension_contracts::auth_prompt::AuthPromptChallengeKind;
use ironclaw_host_api::ids::{AgentId, ProjectId, UserId};
use ironclaw_turns::{TurnRunId, TurnScope};

use ironclaw_auth::product_prompt::{AuthChallengeProvider, AuthChallengeView};

use super::{AGENT, AUTH_GATE, PROJECT, TENANT, USER};

type AuthChallengeCall = (
    TurnScope,
    UserId,
    TurnRunId,
    String,
    Vec<ironclaw_host_api::decision::RuntimeCredentialAuthRequirement>,
);

#[derive(Debug)]
pub(super) struct FakeAuthChallengeProvider {
    calls: Mutex<Vec<AuthChallengeCall>>,
    /// Which challenge the engine serves for [`AUTH_GATE`]. Defaults to
    /// `OAuthUrl` (the serviceable shape the setup-link tests drive);
    /// [`Self::device_link`] selects the shape that is never serviceable from
    /// a chat surface, so delivery takes the unavailable-message path.
    kind: AuthPromptChallengeKind,
}

impl Default for FakeAuthChallengeProvider {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            kind: AuthPromptChallengeKind::OAuthUrl,
        }
    }
}

impl FakeAuthChallengeProvider {
    /// A device-link challenge: no authorization URL, and
    /// `auth_prompt_is_serviceable` rejects it on every surface, so the run is
    /// auto-denied and the user gets the unavailable copy instead of a prompt.
    pub(super) fn device_link() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            kind: AuthPromptChallengeKind::DeviceLink,
        }
    }

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
        credential_requirements: &[ironclaw_host_api::decision::RuntimeCredentialAuthRequirement],
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
        let authorization_url = match self.kind {
            // A device link has no URL to follow: the exchange happens on the
            // vendor's own client, which is exactly why it is unserviceable
            // here.
            AuthPromptChallengeKind::DeviceLink => None,
            _ => Some(
                OAuthAuthorizationUrl::new("https://provider.example/oauth".to_string())
                    .expect("static OAuth URL should be valid"), // safety: static test URL is valid.
            ),
        };
        Ok(Some(AuthChallengeView {
            kind: self.kind,
            provider: AuthProviderId::new("provider".to_string())
                .expect("static provider id should be valid"), // safety: static test provider id is valid.
            account_label: None,
            authorization_url,
            expires_at: None,
            pairing: None,
            device_link: None,
        }))
    }
}
