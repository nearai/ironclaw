//! Product-neutral rendering support for blocked-auth prompts.
//!
//! One owner for the blocked-auth prompt vocabulary: the challenge view, the
//! challenge/cancel ports composition implements, and the prompt-view
//! constructor both the delivery path and the projection layer render
//! through. Composition consumes these — it must not re-declare them.

use crate::{
    AuthPromptChallengeKind, AuthPromptView, ConnectionPromptContext, ProductAdapterError,
    RedactedString,
};
use async_trait::async_trait;
use ironclaw_auth::{
    AuthProductError, AuthProviderId, CredentialAccountLabel, OAuthAuthorizationUrl,
};
use ironclaw_extension_contracts::package_lifecycle::ChannelConnectionRequirement;
use ironclaw_host_api::product_adapter::PairingPromptView;
use ironclaw_host_api::{
    capability::RuntimeCredentialAccountSetup,
    decision::RuntimeCredentialAuthRequirement,
    ids::{InvocationId, UserId},
};
use ironclaw_turns::{TurnRunId, TurnScope};

/// Map a manifest display string onto the projection's optional field: a blank
/// value means the affordance does not exist, which is `None` on the wire. The
/// projection validator rejects `Some("")`.
fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// A host-issued pairing challenge: the minted proof code plus the manifest
/// connection recipe it belongs to. Carrying both is what lets a product
/// surface render the pairing panel instead of a generic "unsupported
/// challenge" fallback.
#[derive(Debug, Clone)]
pub struct PairingAuthChallengeView {
    /// The host-issued proof code, already rendered. Deliberately primitives
    /// rather than `ChannelPairingIssue`: that type lives in
    /// `ironclaw_extension_host`, which depends on this crate, so importing it
    /// would close a dependency cycle.
    pub code: String,
    pub deep_link: Option<String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
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
            // These are `Option` because the field may be genuinely ABSENT, and
            // the projection validator rejects a present-but-empty display
            // string. A manifest legitimately ships empty values for a recipe
            // that has no such affordance -- a `web_generated_code`
            // pairing has no text input, so `input_placeholder = ""` -- so an
            // empty string must map to `None`, not `Some("")`. Emitting
            // `Some("")` fails `ConnectionPromptContext::validate` and takes the
            // whole chat stream down with "The chat stream failed: Validation".
            view.connection = Some(ConnectionPromptContext {
                channel: connection.channel.clone(),
                strategy: Some(connection.strategy.as_str().to_string()),
                instructions: non_empty(&connection.instructions),
                input_placeholder: non_empty(&connection.input_placeholder),
                submit_label: non_empty(&connection.submit_label),
                error_message: non_empty(&connection.error_message),
            });
            // `PairingPromptView` validates with the NON-optional
            // `validate_bounded_text`, so a blank value here is fatal rather
            // than merely absent -- and `deep_link` is the one field that can
            // legitimately arrive blank (a manifest may omit
            // `deep_link_template`, or render it to nothing). Guard it the same
            // way; the required strings are manifest-validated non-empty at
            // parse time (`ChannelConnectionDescriptor::validate`).
            view.pairing = Some(PairingPromptView {
                channel: connection.channel,
                display_name: connection.display_name,
                instructions: connection.instructions,
                code: pairing.code,
                deep_link: pairing.deep_link.as_deref().and_then(non_empty),
                expires_at: pairing.expires_at,
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

/// Narrow read-only interface used by product surfaces to enrich
/// `AuthPromptView` with challenge metadata. Implemented by the composition's
/// product-auth services when a flow record source is wired in.
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
                ProductAdapterError::SurfaceTransient {
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
        // A pairing setup IS a serviceable challenge: the product surface
        // renders the pairing panel. Leaving the kind unset drops the caller
        // onto the "unsupported challenge" fallback card.
        RuntimeCredentialAccountSetup::Pairing => {
            view.challenge_kind = Some(AuthPromptChallengeKind::Pairing);
        }
    }
    view.provider = Some(provider);
    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_extension_contracts::package_lifecycle::ChannelConnectStrategy;
    use ironclaw_host_api::{capability::RuntimeCredentialAccountSetup, ids::VendorId};

    fn requirement(setup: RuntimeCredentialAccountSetup) -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: VendorId::new("acme").expect("vendor"),
            setup,
            requester_extension: ironclaw_host_api::ids::ExtensionId::new("acme")
                .expect("extension id"),
            provider_scopes: Vec::new(),
        }
    }

    fn base_view() -> AuthPromptView {
        AuthPromptView {
            turn_run_id: ironclaw_turns::TurnRunId::new(),
            auth_request_ref: "gate:auth:1".to_string(),
            invocation_id: None,
            headline: "Authentication required".to_string(),
            body: "body".to_string(),
            challenge_kind: None,
            provider: None,
            account_label: None,
            authorization_url: None,
            expires_at: None,
            connection: None,
            pairing: None,
        }
    }

    fn connection() -> ChannelConnectionRequirement {
        ChannelConnectionRequirement {
            channel: "acme".to_string(),
            display_name: "Acme Chat".to_string(),
            strategy: ChannelConnectStrategy::WebGeneratedCode,
            instructions: "Send this code to the bot.".to_string(),
            // Real `web_generated_code` recipes ship this BLANK — keep it so.
            input_placeholder: String::new(),
            submit_label: "Open pairing".to_string(),
            error_message: "Pairing failed.".to_string(),
        }
    }

    /// A pairing credential requirement MUST set `challenge_kind = Pairing`.
    /// #6616 replaced this arm with a no-op asserting pairing "is not an
    /// auth-prompt challenge", which dropped every caller onto the generic
    /// "not available in this view" card. Nothing pinned the restored arm, so
    /// reverting it left the whole suite green.
    #[test]
    fn pairing_credential_requirement_sets_the_pairing_challenge_kind() {
        let view = auth_prompt_from_credential_requirement(
            base_view(),
            &[requirement(RuntimeCredentialAccountSetup::Pairing)],
        );

        assert_eq!(
            view.challenge_kind,
            Some(AuthPromptChallengeKind::Pairing),
            "a pairing setup is a serviceable challenge, not an unknown one"
        );
        assert_eq!(view.provider.as_deref(), Some("acme"));
    }

    /// Positive control for `non_empty`. The blank-maps-to-None assertion alone
    /// is satisfied by `fn non_empty(_) -> None`, which would silently blank the
    /// entire pairing card. Populated manifest values must reach the wire.
    #[test]
    fn populated_connection_fields_reach_the_projection() {
        let challenge = AuthChallengeView {
            kind: AuthPromptChallengeKind::Pairing,
            provider: AuthProviderId::new("acme".to_string()).expect("provider"),
            account_label: None,
            authorization_url: None,
            expires_at: None,
            pairing: Some(PairingAuthChallengeView {
                code: "ABCD2345".to_string(),
                deep_link: Some("https://acme.test/pair?start=ABCD2345".to_string()),
                expires_at: chrono::Utc::now(),
                connection: connection(),
            }),
        };

        let view = challenge.enrich(base_view());
        let ctx = view.connection.as_ref().expect("connection context");

        assert_eq!(
            ctx.instructions.as_deref(),
            Some("Send this code to the bot."),
            "populated instructions must survive the non_empty mapping"
        );
        assert_eq!(ctx.submit_label.as_deref(), Some("Open pairing"));
        assert_eq!(ctx.error_message.as_deref(), Some("Pairing failed."));
        assert_eq!(ctx.strategy.as_deref(), Some("web_generated_code"));
        // ...and the one field real recipes ship blank stays absent.
        assert_eq!(
            ctx.input_placeholder, None,
            "a blank manifest placeholder must be None, never Some(\"\")"
        );

        let pairing = view.pairing.as_ref().expect("pairing view");
        assert_eq!(pairing.code, "ABCD2345");
        assert_eq!(
            pairing.deep_link.as_deref(),
            Some("https://acme.test/pair?start=ABCD2345")
        );
    }

    /// `PairingPromptView` validates with the NON-optional `validate_bounded_text`,
    /// so a blank `deep_link` is fatal rather than merely absent — the same class
    /// that took the chat stream down through `input_placeholder`.
    #[test]
    fn blank_pairing_deep_link_projects_as_absent() {
        let challenge = AuthChallengeView {
            kind: AuthPromptChallengeKind::Pairing,
            provider: AuthProviderId::new("acme".to_string()).expect("provider"),
            account_label: None,
            authorization_url: None,
            expires_at: None,
            pairing: Some(PairingAuthChallengeView {
                code: "ABCD2345".to_string(),
                deep_link: Some("   ".to_string()),
                expires_at: chrono::Utc::now(),
                connection: connection(),
            }),
        };

        let view = challenge.enrich(base_view());

        assert_eq!(
            view.pairing.as_ref().expect("pairing view").deep_link,
            None,
            "a blank deep link must be None; Some(\"\") fails PairingPromptView::validate"
        );
    }
}
