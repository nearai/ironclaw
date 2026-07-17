use async_trait::async_trait;
use ironclaw_auth::{AuthProductError, AuthProviderId, OAuthAuthorizationUrl};
use ironclaw_host_api::{TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{AuthPromptChallengeKind, AuthPromptView};
use ironclaw_product_workflow::{
    AuthChallengeProvider, AuthChallengeView, approval_prompt_lookup, enrich_auth_prompt_view,
};
use ironclaw_turns::{GateRef, TurnRunId, TurnScope};

#[derive(Debug)]
struct OAuthChallenge;

#[async_trait]
impl AuthChallengeProvider for OAuthChallenge {
    async fn challenge_for_gate(
        &self,
        _scope: &TurnScope,
        _owner_user_id: &UserId,
        _run_id: TurnRunId,
        _gate_ref: &str,
        _credential_requirements: &[ironclaw_host_api::RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        Ok(Some(AuthChallengeView {
            kind: AuthPromptChallengeKind::OAuthUrl,
            provider: AuthProviderId::new("github").expect("provider"),
            account_label: None,
            authorization_url: Some(
                OAuthAuthorizationUrl::new("https://github.com/login/oauth/authorize")
                    .expect("authorization URL"),
            ),
            expires_at: None,
        }))
    }
}

fn turn_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-prompt").expect("tenant"),
        None,
        None,
        ThreadId::new("thread-prompt").expect("thread"),
    )
}

#[tokio::test]
async fn auth_prompt_enrichment_accepts_the_owned_view_without_a_crossing_request_dto() {
    let run_id = TurnRunId::new();
    let view = AuthPromptView {
        turn_run_id: run_id,
        auth_request_ref: "auth:github".to_string(),
        invocation_id: None,
        headline: "Authentication required".to_string(),
        body: "Connect GitHub".to_string(),
        challenge_kind: None,
        provider: None,
        account_label: None,
        authorization_url: None,
        expires_at: None,
        connection: None,
    };

    let enriched = enrich_auth_prompt_view(
        view,
        &UserId::new("owner-prompt").expect("owner"),
        &turn_scope(),
        &[],
        Some(&OAuthChallenge),
    )
    .await
    .expect("prompt enrichment succeeds");

    assert_eq!(
        enriched.challenge_kind,
        Some(AuthPromptChallengeKind::OAuthUrl)
    );
    assert_eq!(enriched.provider.as_deref(), Some("github"));
    assert_eq!(
        enriched.authorization_url.as_deref(),
        Some("https://github.com/login/oauth/authorize")
    );
}

#[tokio::test]
async fn approval_prompt_lookup_without_a_store_is_empty() {
    let lookup = approval_prompt_lookup(
        None,
        &GateRef::new("approval:missing").expect("gate ref"),
        &UserId::new("owner-prompt").expect("owner"),
        &turn_scope(),
    )
    .await;

    assert!(lookup.context.is_none());
    assert!(lookup.invocation_id.is_none());
}
