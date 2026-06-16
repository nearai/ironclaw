use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProviderId, CredentialAccountLabel, OAuthAuthorizationUrl,
};
use ironclaw_host_api::{RuntimeCredentialAccountSetup, UserId};
use ironclaw_product_adapters::{
    AuthPromptChallengeKind, AuthPromptView, ProductAdapterError, RedactedString,
};
use ironclaw_turns::{TurnRunId, TurnScope};

/// Redacted view of a pending auth challenge used for product auth prompt
/// enrichment. Contains only data safe to surface over product adapters.
/// No raw secrets, PKCE verifiers, state hashes, or tokens.
#[derive(Debug, Clone)]
pub struct AuthChallengeView {
    pub kind: AuthPromptChallengeKind,
    pub provider: AuthProviderId,
    pub account_label: Option<CredentialAccountLabel>,
    pub authorization_url: Option<OAuthAuthorizationUrl>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl AuthChallengeView {
    /// Apply the view's enrichment fields onto a partially-constructed
    /// `AuthPromptView`, removing the 5-field manual mapping at call sites.
    ///
    /// Caller constructs the 4 mandatory fields; this method fills the 5
    /// optional enrichment fields from `self`.
    pub(crate) fn enrich(self, mut view: AuthPromptView) -> AuthPromptView {
        view.challenge_kind = Some(self.kind);
        view.provider = Some(self.provider.as_str().to_string());
        view.account_label = self.account_label.map(|label| label.as_str().to_string());
        view.authorization_url = self.authorization_url.map(|url| url.as_str().to_string());
        view.expires_at = self.expires_at;
        view
    }
}

/// Narrow read-only interface used by product surfaces to enrich
/// `AuthPromptView` with challenge metadata. Implemented by
/// `RebornProductAuthServices` when a `flow_record_source` is wired in.
///
/// Implementations MUST verify caller user, run id, gate ref, and
/// tenant/agent/project/thread before returning a record.
#[async_trait]
pub trait AuthChallengeProvider: Send + Sync {
    /// Return the product-safe challenge view for the given gate ref and caller
    /// scope, or `None` if the auth flow cannot be found (already consumed, not
    /// yet created, wrong scope, or record source unavailable). Fallible
    /// challenge creation, such as DCR discovery/registration, must surface
    /// errors instead of silently degrading to a missing challenge.
    async fn challenge_for_gate(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
        credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, AuthProductError>;
}

pub(crate) async fn auth_prompt_view_for_blocked_auth(
    fallback_owner_user_id: &UserId,
    scope: &TurnScope,
    run_id: TurnRunId,
    gate_ref: &str,
    body: String,
    credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
    auth_challenges: Option<&dyn AuthChallengeProvider>,
) -> Result<AuthPromptView, ProductAdapterError> {
    // Explicit turn owners represent shared/team subjects; actor fallback keeps
    // the existing personal/WebUI behavior for legacy scopes.
    let owner_user_id = scope
        .explicit_owner_user_id()
        .unwrap_or(fallback_owner_user_id);
    let challenge = match auth_challenges {
        Some(provider) => provider
            .challenge_for_gate(
                scope,
                owner_user_id,
                run_id,
                gate_ref,
                credential_requirements,
            )
            .await
            .map_err(|error| {
                tracing::debug!(
                    %error,
                    %run_id,
                    "auth challenge lookup failed during auth prompt rendering"
                );
                ProductAdapterError::WorkflowTransient {
                    reason: RedactedString::new("auth challenge lookup failed"),
                }
            })?,
        None => None,
    };
    let base_view = AuthPromptView {
        turn_run_id: run_id,
        auth_request_ref: gate_ref.to_string(),
        headline: "Authentication required".to_string(),
        body,
        challenge_kind: None,
        provider: None,
        account_label: None,
        authorization_url: None,
        expires_at: None,
    };
    Ok(match challenge {
        Some(c) => c.enrich(base_view),
        None => auth_prompt_from_credential_requirement(base_view, credential_requirements),
    })
}

fn auth_prompt_from_credential_requirement(
    mut view: AuthPromptView,
    credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
) -> AuthPromptView {
    let [requirement] = credential_requirements else {
        return view;
    };
    let provider = requirement.provider.as_str().to_string();
    match &requirement.setup {
        RuntimeCredentialAccountSetup::ManualToken => {
            view.challenge_kind = Some(AuthPromptChallengeKind::ManualToken);
            view.account_label = Some(provider.clone());
        }
        RuntimeCredentialAccountSetup::OAuth { .. } => {
            view.challenge_kind = Some(AuthPromptChallengeKind::Other);
        }
    }
    view.provider = Some(provider);
    view
}
