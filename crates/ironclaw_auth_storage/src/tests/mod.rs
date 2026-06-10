use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    InvocationId, MountAlias, MountGrant, MountPermissions, SecretHandle, ThreadId, UserId,
    VirtualPath,
};
use ironclaw_secrets::{InMemorySecretStore, SecretStore};
use secrecy::SecretString;
use tokio::task::JoinSet;

use super::*;
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthFlowManager, AuthFlowOwnerScope,
    AuthFlowRecordSource, AuthFlowStatus, AuthGateRef, AuthInteractionId, AuthInteractionService,
    AuthProductError, AuthProductScope, AuthProviderId, AuthSessionId, AuthSurface,
    AuthorizationCodeHash, CredentialAccountChoiceRequest, CredentialAccountLabel,
    CredentialAccountListRequest, CredentialAccountLookupRequest, CredentialAccountRecordSource,
    CredentialAccountSelectionRequest, CredentialAccountService, CredentialAccountStatus,
    CredentialOwnership, ManualTokenCompletionInput, ManualTokenSetupRequest, NewAuthFlow,
    NewCredentialAccount, OAuthAuthorizationUrl, OAuthCallbackClaimRequest, OAuthCallbackInput,
    OAuthProviderExchange, OpaqueStateHash, PkceVerifierHash, ProviderScope, SecretSubmitRequest,
    TurnGateAuthFlowQuery, TurnRunRef,
};

fn test_scope() -> AuthProductScope {
    let resource =
        ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new()).unwrap();
    AuthProductScope::new(resource, AuthSurface::Web)
}

fn test_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = ironclaw_host_api::MountView::new(vec![MountGrant::new(
        MountAlias::new("/secrets").unwrap(),
        VirtualPath::new("/tenants/test/users/alice/secrets").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

fn test_service(
    filesystem: Arc<ScopedFilesystem<InMemoryBackend>>,
    secret_store: Arc<dyn SecretStore>,
) -> FilesystemAuthProductServices<InMemoryBackend> {
    FilesystemAuthProductServices::new(filesystem, secret_store)
}

fn google_provider() -> AuthProviderId {
    AuthProviderId::new("google").unwrap()
}

fn account_label() -> CredentialAccountLabel {
    CredentialAccountLabel::new("Alice Google").unwrap()
}

fn fake_digest(value: &str) -> String {
    format!(
        "{:064x}",
        value.bytes().fold(0_u64, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u64::from(byte))
        })
    )
}

fn state_hash(value: &str) -> OpaqueStateHash {
    OpaqueStateHash::new(fake_digest(value)).unwrap()
}

fn pkce_hash(value: &str) -> PkceVerifierHash {
    PkceVerifierHash::new(fake_digest(value)).unwrap()
}

fn code_hash(value: &str) -> AuthorizationCodeHash {
    AuthorizationCodeHash::new(fake_digest(value)).unwrap()
}

async fn create_manual_token_flow(
    service: &FilesystemAuthProductServices<InMemoryBackend>,
    scope: &AuthProductScope,
    expires_at: chrono::DateTime<Utc>,
) -> AuthInteractionId {
    let challenge = service
        .request_secret_input(ManualTokenSetupRequest {
            scope: scope.clone(),
            provider: google_provider(),
            label: account_label(),
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            expires_at,
        })
        .await
        .unwrap();
    let AuthChallenge::ManualTokenRequired {
        interaction_id,
        provider,
        label,
        expires_at: challenge_expires_at,
    } = challenge
    else {
        panic!("expected manual token challenge");
    };
    service
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider,
            challenge: AuthChallenge::ManualTokenRequired {
                interaction_id,
                provider: google_provider(),
                label,
                expires_at: challenge_expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: None,
            pkce_verifier_hash: None,
            expires_at,
        })
        .await
        .unwrap();
    interaction_id
}

mod accounts;
mod flows;
mod manual_token;
mod oauth;
mod provider_and_edges;
