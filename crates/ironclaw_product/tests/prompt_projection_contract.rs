use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_auth::{AuthProductError, AuthProviderId, OAuthAuthorizationUrl};
use ironclaw_host_api::{
    ExtensionId, RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, TenantId,
    ThreadId, UserId, VendorId,
};
use ironclaw_product::AuthPromptChallengeKind;
use ironclaw_product::{
    AuthChallengeProvider, AuthChallengeView, BlockedAuthPromptRequest, approval_prompt_lookup,
    auth_prompt_view_for_blocked_auth,
};
use ironclaw_turns::{GateRef, TurnRunId, TurnScope};

#[derive(Debug)]
struct OAuthChallenge {
    captured: Mutex<Option<CapturedChallengeArguments>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedChallengeArguments {
    scope: TurnScope,
    owner_user_id: UserId,
    run_id: TurnRunId,
    gate_ref: String,
    credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
}

#[async_trait]
impl AuthChallengeProvider for OAuthChallenge {
    async fn challenge_for_gate(
        &self,
        scope: &TurnScope,
        owner_user_id: &UserId,
        run_id: TurnRunId,
        gate_ref: &str,
        credential_requirements: &[RuntimeCredentialAuthRequirement],
    ) -> Result<Option<AuthChallengeView>, AuthProductError> {
        let mut captured = self.captured.lock().expect("capture lock");
        *captured = Some(CapturedChallengeArguments {
            scope: scope.clone(),
            owner_user_id: owner_user_id.clone(),
            run_id,
            gate_ref: gate_ref.to_string(),
            credential_requirements: credential_requirements.to_vec(),
        });
        Ok(Some(AuthChallengeView {
            kind: AuthPromptChallengeKind::OAuthUrl,
            provider: AuthProviderId::new("github").expect("provider"),
            account_label: None,
            authorization_url: Some(
                OAuthAuthorizationUrl::new("https://github.com/login/oauth/authorize")
                    .expect("authorization URL"),
            ),
            expires_at: None,
            pairing: None,
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
    let scope = turn_scope();
    let owner_user_id = UserId::new("owner-prompt").expect("owner");
    let credential_requirements = vec![RuntimeCredentialAuthRequirement {
        provider: VendorId::new("github").expect("provider"),
        setup: RuntimeCredentialAccountSetup::OAuth {
            scopes: vec!["repo:read".to_string()],
        },
        requester_extension: ExtensionId::new("github").expect("extension"),
        provider_scopes: vec!["repo:read".to_string()],
    }];
    let challenge = OAuthChallenge {
        captured: Mutex::new(None),
    };
    let enriched = auth_prompt_view_for_blocked_auth(
        BlockedAuthPromptRequest {
            fallback_owner_user_id: &owner_user_id,
            scope: &scope,
            run_id,
            gate_ref: "auth:github",
            invocation_id: None,
            body: "Connect GitHub".to_string(),
            credential_requirements: &credential_requirements,
        },
        Some(&challenge),
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
    assert_eq!(
        challenge.captured.lock().expect("capture lock").clone(),
        Some(CapturedChallengeArguments {
            scope,
            owner_user_id,
            run_id,
            gate_ref: "auth:github".to_string(),
            credential_requirements,
        })
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
    .await
    .expect("an absent store is a supported empty projection");

    assert!(lookup.context.is_none());
    assert!(lookup.invocation_id.is_none());
}
