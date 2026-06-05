pub use chrono::{Duration, Utc};
pub use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowKind, AuthFlowManager,
    AuthFlowStatus, AuthGateRef, AuthInteractionService, AuthProductError, AuthProductScope,
    AuthProviderClient, AuthProviderId, AuthSessionId, AuthSurface, AuthorizationCodeHash,
    CredentialAccount, CredentialAccountChoiceRequest, CredentialAccountId, CredentialAccountLabel,
    CredentialAccountListRequest, CredentialAccountLookupRequest, CredentialAccountMutation,
    CredentialAccountProjection, CredentialAccountSelectionRequest, CredentialAccountService,
    CredentialAccountStatus, CredentialAccountUpdate, CredentialAccountUpdateBinding,
    CredentialOwnership, CredentialRecoveryKind, CredentialRecoveryProjection,
    CredentialRecoveryReason, CredentialRecoveryRequest, CredentialRefreshRequest,
    CredentialSelectionInput, CredentialSetupService, InMemoryAuthProductServices,
    LifecyclePackageRef, ManualTokenSetupRequest, NewAuthFlow, NewCredentialAccount,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthCallbackInput,
    OAuthProviderCallbackRequest, OAuthProviderExchange, OAuthProviderExchangeContext,
    OAuthProviderRefreshRequest, OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret,
    ProviderCallbackOutcome, ProviderScope, SecretCleanupAction, SecretCleanupQuarantineReason,
    SecretCleanupRequest, SecretCleanupService, SecretSubmitRequest, SecretSubmitResult,
    TurnRunRef,
};
pub use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope, SecretHandle, UserId};
pub use secrecy::SecretString;

pub fn scope(user: &str) -> AuthProductScope {
    AuthProductScope::new(
        ResourceScope::local_default(UserId::new(user).expect("valid user"), InvocationId::new())
            .expect("valid scope"),
        AuthSurface::Web,
    )
    .with_session_id(AuthSessionId::new(format!("session-{user}")).expect("valid session"))
}

pub fn provider() -> AuthProviderId {
    AuthProviderId::new("github").expect("valid provider")
}

pub fn label(value: &str) -> CredentialAccountLabel {
    CredentialAccountLabel::new(value).expect("valid label")
}

pub fn state_hash(value: &str) -> OpaqueStateHash {
    OpaqueStateHash::new(fake_digest(value)).expect("valid state hash")
}

pub fn pkce_hash(value: &str) -> PkceVerifierHash {
    PkceVerifierHash::new(fake_digest(value)).expect("valid pkce hash")
}

pub fn code_hash(value: &str) -> AuthorizationCodeHash {
    AuthorizationCodeHash::new(fake_digest(value)).expect("valid code hash")
}

pub fn fake_digest(value: &str) -> String {
    format!(
        "{:064x}",
        value.bytes().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        })
    )
}

pub fn authorization_url(value: &str) -> OAuthAuthorizationUrl {
    OAuthAuthorizationUrl::new(value).expect("valid authorization url")
}

pub fn provider_scope(value: &str) -> ProviderScope {
    ProviderScope::new(value).expect("valid provider scope")
}

pub fn provider_scopes(values: &[&str]) -> Vec<ProviderScope> {
    values.iter().map(|value| provider_scope(value)).collect()
}

pub fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_string())
}

pub fn account_request(
    owner: AuthProductScope,
    label_value: &str,
    status: CredentialAccountStatus,
) -> NewCredentialAccount {
    NewCredentialAccount {
        scope: owner,
        provider: provider(),
        label: label(label_value),
        status,
        ownership: CredentialOwnership::UserReusable,
        owner_extension: None,
        granted_extensions: Vec::new(),
        access_secret: None,
        refresh_secret: None,
        scopes: Vec::new(),
    }
}

pub fn account_ids(accounts: &[CredentialAccountProjection]) -> Vec<CredentialAccountId> {
    let mut ids = accounts
        .iter()
        .map(|account| account.id)
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

pub fn update_binding(account: &CredentialAccount) -> CredentialAccountUpdateBinding {
    CredentialAccountUpdateBinding {
        account_id: account.id,
        ownership: account.ownership,
        owner_extension: account.owner_extension.clone(),
        granted_extensions: account.granted_extensions.clone(),
    }
}

pub async fn oauth_flow(
    services: &InMemoryAuthProductServices,
    owner: AuthProductScope,
) -> ironclaw_auth::AuthFlowRecord {
    services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner,
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
            },
            update_binding: None,
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
        .expect("flow")
}

pub async fn oauth_update_flow(
    services: &InMemoryAuthProductServices,
    owner: AuthProductScope,
    account: &CredentialAccount,
) -> ironclaw_auth::AuthFlowRecord {
    try_oauth_update_flow(services, owner, account)
        .await
        .expect("update flow")
}

pub async fn try_oauth_update_flow(
    services: &InMemoryAuthProductServices,
    owner: AuthProductScope,
    account: &CredentialAccount,
) -> Result<ironclaw_auth::AuthFlowRecord, AuthProductError> {
    services
        .create_flow(NewAuthFlow {
            id: None,
            scope: owner,
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: authorization_url("https://provider.example/oauth"),
                expires_at: Utc::now() + Duration::minutes(5),
            },
            continuation: AuthContinuationRef::LifecycleActivation {
                package_ref: LifecyclePackageRef::new("github-extension").expect("valid package"),
            },
            update_binding: Some(update_binding(account)),
            opaque_state_hash: Some(state_hash("state-hash")),
            pkce_verifier_hash: Some(pkce_hash("pkce-hash")),
            expires_at: Utc::now() + Duration::minutes(5),
        })
        .await
}
