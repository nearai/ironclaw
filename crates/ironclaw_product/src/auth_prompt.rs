//! Product-neutral rendering support for blocked-auth prompts.
//!
//! One owner for the blocked-auth prompt vocabulary: the challenge view, the
//! challenge/cancel ports composition implements, and the prompt-view
//! constructor both the delivery path and the projection layer render
//! through. Composition consumes these — it must not re-declare them.

use crate::{
    AuthPromptChallengeKind, AuthPromptView, ConnectionPromptContext, PairingPromptView,
    ProductAdapterError, RedactedString,
};
use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProviderId, CredentialAccountLabel, OAuthAuthorizationUrl,
};
use ironclaw_host_api::{
    InvocationId, RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, UserId,
};
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::{ChannelConnectionRequirement, ChannelPairingIssue};

#[derive(Debug, Clone)]
pub struct PairingAuthChallengeView {
    pub issue: ChannelPairingIssue,
    pub connection: ChannelConnectionRequirement,
}

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
    pub pairing: Option<PairingAuthChallengeView>,
}

impl AuthChallengeView {
    /// Apply the view's enrichment fields onto a partially-constructed
    /// `AuthPromptView`, removing the 5-field manual mapping at call sites.
    ///
    /// Caller constructs the 4 mandatory fields; this method fills the 5
    /// optional enrichment fields from `self`.
    fn enrich(self, mut view: AuthPromptView) -> AuthPromptView {
        view.challenge_kind = Some(self.kind);
        view.provider = Some(self.provider.as_str().to_string());
        view.account_label = self.account_label.map(|label| label.as_str().to_string());
        view.authorization_url = self.authorization_url.map(|url| url.as_str().to_string());
        view.expires_at = self.expires_at;
        if let Some(pairing) = self.pairing {
            let connection = pairing.connection;
            view.connection = Some(ConnectionPromptContext {
                channel: connection.channel.clone(),
                strategy: Some(connection.strategy.as_str().to_string()),
                instructions: Some(connection.instructions.clone()),
                input_placeholder: Some(connection.input_placeholder),
                submit_label: Some(connection.submit_label),
                error_message: Some(connection.error_message),
            });
            view.pairing = Some(PairingPromptView {
                channel: connection.channel,
                display_name: connection.display_name,
                instructions: connection.instructions,
                code: pairing.issue.code.as_str().to_string(),
                deep_link: pairing.issue.deep_link,
                expires_at: pairing.issue.expires_at,
            });
        } else {
            // OAuth relay and stored-secret challenges carry no channel
            // connection context.
            view.connection = None;
            view.pairing = None;
        }
        view
    }
}

/// Narrow challenge-materialization interface used by product surfaces to
/// enrich `AuthPromptView`. Implemented by composition over product-auth and
/// host-issued pairing services. Materialization may durably create a bounded
/// challenge, but replay must reuse a still-live challenge rather than rotate
/// it.
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
        credential_requirements: &[RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, AuthProductError>;
}

/// Cancels the durable `AuthFlow` record behind a blocked-auth turn gate.
///
/// When a channel run blocked on interactive auth is auto-denied (a non-OAuth
/// challenge the channel surface can't satisfy), the delivery path cancels the
/// run directly via `TurnCoordinator` rather than through the canonical
/// `AuthInteractionService` deny path (which *resumes* the run with a denied
/// disposition instead of cancelling it). Without this port the underlying
/// `AuthFlow` record lingers non-terminal (`Pending`/`AwaitingUser`) until it
/// expires — see issue #4952. Implemented by the composition's product-auth
/// services when a flow record source is wired in; a no-op when it isn't.
///
/// Implementations MUST scope the lookup by caller user, run id, gate ref, and
/// tenant/agent/project/thread, and MUST treat an already-terminal (or absent)
/// flow as a graceful no-op so the OAuth-callback race — where the flow completes
/// just before auto-deny — does not surface an error.
#[async_trait]
pub trait BlockedAuthFlowCanceller: Send + Sync {
    /// Cancel the non-terminal auth flow backing `(scope, run_id, gate_ref)`.
    /// Returns `Ok(())` when the flow was cancelled, was already terminal, or
    /// could not be found (nothing to cancel).
    async fn cancel_blocked_auth_flow(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
    ) -> Result<(), AuthProductError>;
}

/// Inputs for resolving a blocked-auth run's prompt view. One request shape
/// for every renderer (delivery path, projection layer); the challenge
/// provider is a separate argument, not request data.
pub struct BlockedAuthPromptRequest<'a> {
    pub fallback_owner_user_id: &'a UserId,
    pub scope: &'a TurnScope,
    pub run_id: TurnRunId,
    pub gate_ref: &'a str,
    /// Invocation the blocked capability ran under, when the renderer has it
    /// (the projection layer does; the delivery path renders without one).
    pub invocation_id: Option<InvocationId>,
    pub body: String,
    pub credential_requirements: &'a [RuntimeCredentialAuthRequirement],
}

/// Build the full blocked-auth prompt view: challenge enrichment when the
/// provider can resolve the durable flow, credential-requirement fallback
/// otherwise.
pub async fn auth_prompt_view_for_blocked_auth(
    request: BlockedAuthPromptRequest<'_>,
    auth_challenges: Option<&dyn AuthChallengeProvider>,
) -> Result<AuthPromptView, ProductAdapterError> {
    let BlockedAuthPromptRequest {
        fallback_owner_user_id,
        scope,
        run_id,
        gate_ref,
        invocation_id,
        body,
        credential_requirements,
    } = request;
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
        invocation_id,
        headline: "Authentication required".to_string(),
        body,
        challenge_kind: None,
        provider: None,
        account_label: None,
        authorization_url: None,
        expires_at: None,
        connection: None,
        pairing: None,
    };
    Ok(match challenge {
        Some(c) => c.enrich(base_view),
        None => auth_prompt_from_credential_requirement(base_view, credential_requirements),
    })
}

fn auth_prompt_from_credential_requirement(
    mut view: AuthPromptView,
    credential_requirements: &[RuntimeCredentialAuthRequirement],
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
            view.challenge_kind = Some(AuthPromptChallengeKind::OAuthUrl);
        }
        // A retired setup kind (legacy persisted record) has no serviceable
        // challenge; keep the generic requirement-derived prompt.
        RuntimeCredentialAccountSetup::Retired => {}
        RuntimeCredentialAccountSetup::Pairing => {
            view.challenge_kind = Some(AuthPromptChallengeKind::Pairing);
        }
    }
    view.provider = Some(provider);
    view
}
