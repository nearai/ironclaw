use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProviderId, CredentialAccountLabel, OAuthAuthorizationUrl,
};
use ironclaw_host_api::{
    InvocationId, RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, UserId,
};
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
        // OAuth relay / stored-secret challenges carry no channel-connection
        // context; that rides only on the credential-requirement fallback path.
        view.connection = None;
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

/// Cancels the durable `AuthFlow` record behind a blocked-auth turn gate.
///
/// When a Slack run blocked on interactive auth is auto-denied (a non-OAuth
/// challenge the Slack surface can't satisfy), the delivery path cancels the run
/// directly via `TurnCoordinator` rather than through the canonical
/// `AuthInteractionService` deny path (which *resumes* the run with a denied
/// disposition instead of cancelling it). Without this port the underlying
/// `AuthFlow` record lingers non-terminal (`Pending`/`AwaitingUser`) until it
/// expires — see issue #4952. Implemented by `RebornProductAuthServices` when a
/// `flow_record_source` is wired in; a no-op when it isn't.
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

pub(crate) struct BlockedAuthPromptRequest<'a> {
    pub(crate) fallback_owner_user_id: &'a UserId,
    pub(crate) scope: &'a TurnScope,
    pub(crate) run_id: TurnRunId,
    pub(crate) gate_ref: &'a str,
    pub(crate) invocation_id: Option<InvocationId>,
    pub(crate) body: String,
    pub(crate) credential_requirements: &'a [RuntimeCredentialAuthRequirement],
    pub(crate) auth_challenges: Option<&'a dyn AuthChallengeProvider>,
}

pub(crate) async fn auth_prompt_view_for_blocked_auth(
    request: BlockedAuthPromptRequest<'_>,
) -> Result<AuthPromptView, ProductAdapterError> {
    let BlockedAuthPromptRequest {
        fallback_owner_user_id,
        scope,
        run_id,
        gate_ref,
        invocation_id,
        body,
        credential_requirements,
        auth_challenges,
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
        RuntimeCredentialAccountSetup::ChannelPairing { channel } => {
            // A pairing code is pasted like any other secret, so it reuses the
            // `manual_token` (ManualToken) modality; what differs is the resolve
            // route, which rides in `connection` context. Render copy comes from
            // the single backend-authored connect builder (the same one the
            // connectable descriptor uses), so any inbound channel gets correct
            // instructions — including Slack's "/pair" recovery — with no
            // per-channel frontend code.
            let requirement =
                crate::extension_lifecycle::channel_connection_requirement(channel, channel);
            let strategy = serde_json::to_value(requirement.strategy)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned));
            view.challenge_kind = Some(AuthPromptChallengeKind::ManualToken);
            view.headline = format!("Connect {}", capitalize_channel(channel));
            view.body = requirement.instructions.clone();
            view.account_label = None;
            view.connection = Some(ironclaw_product_adapters::ConnectionPromptContext {
                channel: requirement.channel,
                strategy,
                instructions: Some(requirement.instructions),
                input_placeholder: Some(requirement.input_placeholder),
                submit_label: Some(requirement.submit_label),
                error_message: Some(requirement.error_message),
            });
        }
    }
    view.provider = Some(provider);
    view
}

fn capitalize_channel(channel: &str) -> String {
    let mut chars = channel.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => channel.to_string(),
    }
}
