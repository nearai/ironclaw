//! Reborn integration-test framework — slice 7: OAuth connect-flow.
//!
//! Drives a real OAuth connect flow through the Reborn product-auth boundary:
//! `create_flow` → `handle_oauth_callback` → assert `CredentialAccount`
//! persisted and readable.  The token-exchange HTTP is captured by a
//! `ScriptedOAuthTokenEgress` (no real network); all other stores (flow +
//! account persistence) are real `FilesystemAuthProductServices<InMemoryBackend>`.
//!
//! This proves design-spec §3.8 coverage: real stores, mock only the OAuth HTTP
//! seam at the `RuntimeHttpEgress` boundary.

use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowKind, AuthProductScope,
    AuthProviderId, AuthSurface, AuthorizationCodeHash, CredentialAccountLabel,
    CredentialAccountListRequest, CredentialAccountLookupRequest, NewAuthFlow,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderScope,
};
use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
use ironclaw_reborn_composition::{
    RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    test_support::build_oauth_product_auth_for_test,
};
use secrecy::SecretString;

/// Build a 64-character hex string from a repeated byte value.
fn hex64(fill: u8) -> String {
    format!("{fill:02x}").repeat(32)
}

fn test_scope() -> AuthProductScope {
    let resource =
        ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new())
            .expect("local_default scope must build");
    AuthProductScope::new(resource, AuthSurface::Callback)
}

/// Core slice-7 scenario: a real OAuth connect flow produces a persisted
/// `CredentialAccount` that reads back correctly, and exactly one
/// token-exchange HTTP call was made to the scripted egress.
#[tokio::test]
async fn oauth_connect_flow_persists_credential_account() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();

    // Stable hash values shared across flow creation and callback claim.
    let state_hash = OpaqueStateHash::new(hex64(0xaa)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0xbb)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0xcc)).unwrap();
    let expires_at = Utc::now() + Duration::minutes(5);

    let flow = bundle
        .services
        .flow_manager()
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider.clone(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://accounts.example.com/o/oauth2/auth",
                )
                .unwrap(),
                expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash.clone()),
            pkce_verifier_hash: Some(pkce_hash.clone()),
            expires_at,
        })
        .await
        .expect("create_flow must succeed");

    // Drives claim → token exchange → complete. The scripted egress returns a
    // fixed access-token JSON body; no real network call is made.
    let response = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope.clone(),
            flow_id: flow.id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: provider.clone(),
                    account_label: CredentialAccountLabel::new("Test Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "raw-auth-code-value".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "raw-pkce-verifier-value".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier_hash: pkce_hash,
                    scopes: vec![ProviderScope::new("test.readonly").unwrap()],
                },
            },
        })
        .await
        .expect("handle_oauth_callback must succeed");

    let account_id = response
        .credential_account_id
        .expect("completed callback must carry a credential_account_id");

    let account = bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(scope, account_id))
        .await
        .expect("get_account must not error")
        .expect("credential account must be persisted after a successful OAuth callback");

    assert_eq!(
        account.id, account_id,
        "account id matches the callback response"
    );
    assert_eq!(
        account.provider, provider,
        "account provider matches the flow provider"
    );

    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "exactly one token-exchange HTTP call must be captured by the scripted egress"
    );

    // Must use authorization_code, not the refresh grant — proves the right
    // OAuth flow crossed the egress.
    let grant_types = bundle.egress.captured_grant_types();
    assert_eq!(
        grant_types.first().map(String::as_str),
        Some("authorization_code"),
        "connect-flow token exchange must use the authorization_code grant; grant_types: {grant_types:?}"
    );

    // Recipe-engine pins (names only — values carry secrets): the exchange
    // body is host-constructed per the vendor recipe, carrying the PKCE
    // verifier, the deployment client id, and the static vendor redirect.
    let param_names = bundle.egress.captured_form_param_names();
    let exchange_params = param_names.first().expect("one captured exchange");
    for expected in [
        "grant_type",
        "code",
        "code_verifier",
        "client_id",
        "redirect_uri",
    ] {
        assert!(
            exchange_params.iter().any(|name| name == expected),
            "engine exchange body must carry `{expected}`; got {exchange_params:?}"
        );
    }
}

/// AUTH-4 at the integration tier: a callback requesting scopes beyond the
/// vendor recipe's ceiling is rejected by the engine BEFORE any vendor call —
/// no token-exchange egress, no persisted credential account.
#[tokio::test]
async fn oauth_connect_rejects_scopes_beyond_the_recipe_ceiling() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();
    let state_hash = OpaqueStateHash::new(hex64(0x21)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0x22)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0x23)).unwrap();
    let expires_at = Utc::now() + Duration::minutes(5);

    let flow = bundle
        .services
        .flow_manager()
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider.clone(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://accounts.example.com/o/oauth2/auth",
                )
                .unwrap(),
                expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash.clone()),
            pkce_verifier_hash: Some(pkce_hash.clone()),
            expires_at,
        })
        .await
        .expect("create_flow must succeed");

    let error = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope.clone(),
            flow_id: flow.id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: provider.clone(),
                    account_label: CredentialAccountLabel::new("Ceiling Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "widened-auth-code".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "widened-pkce-verifier".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier_hash: pkce_hash,
                    // The bundle recipe's ceiling is ["test.readonly"].
                    scopes: vec![ProviderScope::new("test.admin").unwrap()],
                },
            },
        })
        .await
        .expect_err("scopes beyond the recipe ceiling must be rejected");

    assert_eq!(error.code, AuthErrorCode::InvalidRequest);
    assert_eq!(
        bundle.egress.captured_count(),
        0,
        "widening must be rejected before the vendor call"
    );
    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(scope, provider))
        .await
        .expect("list_accounts must not error");
    assert!(
        page.accounts.is_empty(),
        "no credential account must be created for a rejected exchange"
    );
}

/// The same connect flow persisted through the libSQL-backed durable
/// flow/account store — the auth engine's second persistence backend
/// (checklist AUTH-15; the primary suite runs on the in-memory backend).
#[cfg(feature = "libsql")]
#[tokio::test]
async fn oauth_connect_flow_persists_credential_account_on_libsql() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bundle =
        ironclaw_reborn_composition::test_support::build_oauth_product_auth_for_test_on_libsql(
            &dir.path().join("oauth-connect.db"),
        )
        .await;
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();
    let state_hash = OpaqueStateHash::new(hex64(0x31)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0x32)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0x33)).unwrap();
    let expires_at = Utc::now() + Duration::minutes(5);

    let flow = bundle
        .services
        .flow_manager()
        .create_flow(NewAuthFlow {
            id: None,
            scope: scope.clone(),
            kind: AuthFlowKind::IntegrationCredential,
            provider: provider.clone(),
            challenge: AuthChallenge::OAuthUrl {
                authorization_url: OAuthAuthorizationUrl::new(
                    "https://accounts.example.com/o/oauth2/auth",
                )
                .unwrap(),
                expires_at,
            },
            continuation: AuthContinuationRef::SetupOnly,
            update_binding: None,
            opaque_state_hash: Some(state_hash.clone()),
            pkce_verifier_hash: Some(pkce_hash.clone()),
            expires_at,
        })
        .await
        .expect("create_flow must succeed on libsql");

    let response = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope.clone(),
            flow_id: flow.id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: provider.clone(),
                    account_label: CredentialAccountLabel::new("LibSql Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "libsql-auth-code".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "libsql-pkce-verifier".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier_hash: pkce_hash,
                    scopes: vec![ProviderScope::new("test.readonly").unwrap()],
                },
            },
        })
        .await
        .expect("handle_oauth_callback must succeed on libsql");

    let account_id = response
        .credential_account_id
        .expect("completed callback must carry a credential_account_id");
    let account = bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(scope, account_id))
        .await
        .expect("get_account must not error")
        .expect("credential account must persist on the libsql backend");
    assert_eq!(account.provider, provider);
    assert_eq!(
        bundle
            .egress
            .captured_grant_types()
            .first()
            .map(String::as_str),
        Some("authorization_code")
    );
}

/// Guard test: attempting an OAuth callback for a non-existent flow must fail
/// with `UnknownOrExpiredFlow`.  No credential account must be created, and no
/// token-exchange call should be made.
///
/// Both guarantees are verified: `captured_count()` asserts no token-exchange
/// HTTP call was made; `list_accounts` asserts no credential account was
/// persisted to the durable store.
#[tokio::test]
async fn oauth_callback_without_prior_flow_fails() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    // Clone scope before it is moved into the callback request so we can use
    // it for the list_accounts assertion after the error is returned.
    let scope_for_assert = scope.clone();
    let state_hash = OpaqueStateHash::new(hex64(0xdd)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0xee)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0xff)).unwrap();

    let error = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope,
            flow_id: AuthFlowId::new(), // no flow was created for this id
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: AuthProviderId::new("test-oauth-provider").unwrap(),
                    account_label: CredentialAccountLabel::new("Guard Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "guard-auth-code".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "guard-pkce-verifier".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier_hash: pkce_hash,
                    scopes: vec![],
                },
            },
        })
        .await
        .expect_err("callback with no prior flow must return an error");

    assert_eq!(
        error.code,
        AuthErrorCode::UnknownOrExpiredFlow,
        "missing flow must surface as UnknownOrExpiredFlow"
    );

    // The claim step fails before any token-exchange — egress must be clean.
    assert_eq!(
        bundle.egress.captured_count(),
        0,
        "no token-exchange call should be made when the flow is missing"
    );

    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(
            scope_for_assert,
            AuthProviderId::new("test-oauth-provider").unwrap(),
        ))
        .await
        .expect("list_accounts must not error after a failed callback");
    assert!(
        page.accounts.is_empty(),
        "no credential account must be created when the flow is missing; got {} accounts",
        page.accounts.len()
    );
}

/// Extension-runtime P6 S3: a CHANNEL extension's OAuth connect must bind
/// the proven vendor identity to the authenticated caller through the
/// GENERIC post-exchange hook (no vendor code in the path).
///
/// Real flow manager + durable account store + recipe engine with identity
/// pointers over a scripted token exchange; a real installed v3 channel
/// manifest in the durable installation store supplies the discovery and
/// the `[channel.config]` scoping values. Two phases, one flow each:
///
/// 1. Scoping mismatch — the workspace claim in the token body does not
///    match the configured scoping value: the callback FAILS, no
///    credential account is persisted, and no identity binding is written
///    (fail-closed §6.4).
/// 2. Match — the callback completes, the credential account persists,
///    and the identity-binding store holds exactly one binding keyed by
///    the installation-scoped provider user id.
#[tokio::test]
async fn oauth_connect_binds_channel_identity_through_the_generic_hook() {
    use std::sync::{Arc, Mutex};

    use ironclaw_extensions::{
        ExtensionActivationState, ExtensionInstallation, ExtensionInstallationId,
        ExtensionInstallationStore, ExtensionManifestRecord, ExtensionManifestRef,
        InMemoryExtensionInstallationStore, ManifestSource,
    };
    use ironclaw_host_api::ExtensionId;
    use ironclaw_reborn_composition::{
        ChannelIdentityBindingConfig, RebornUserIdentityBinding,
        RebornUserIdentityBindingDeleteStore, RebornUserIdentityBindingError,
        RebornUserIdentityBindingStore,
        test_support::{
            build_oauth_product_auth_with_identity_for_test,
            handle_oauth_callback_with_channel_identity_binding_for_test,
        },
    };

    const VENDOR: &str = "test-oauth-provider";
    const EXTENSION_ID: &str = "acmechat";
    const INSTALLATION_ID: &str = "acmechat-install-1";

    /// Minimal recording identity store: the durable production store is
    /// filesystem-root based and vendor-lane owned until the H.4 key
    /// migration; the binding CONTRACT (installation-scoped composite key,
    /// full-prefix rollback) is what this proof pins.
    #[derive(Default)]
    struct RecordingIdentityStore {
        bindings: Mutex<Vec<RebornUserIdentityBinding>>,
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityBindingStore for RecordingIdentityStore {
        async fn bind_user_identity(
            &self,
            binding: RebornUserIdentityBinding,
        ) -> Result<(), RebornUserIdentityBindingError> {
            self.bindings.lock().unwrap().push(binding);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl RebornUserIdentityBindingDeleteStore for RecordingIdentityStore {
        async fn delete_user_identity_bindings_for_user(
            &self,
            provider: &str,
            user_id: &ironclaw_host_api::UserId,
            provider_user_id_prefix: Option<&str>,
        ) -> Result<usize, RebornUserIdentityBindingError> {
            let mut bindings = self.bindings.lock().unwrap();
            let before = bindings.len();
            bindings.retain(|binding| {
                let prefix_matches = provider_user_id_prefix
                    .map(|prefix| binding.provider_user_id.as_str().starts_with(prefix))
                    .unwrap_or(true);
                !(binding.provider.as_str() == provider
                    && &binding.user_id == user_id
                    && prefix_matches)
            });
            Ok(before - bindings.len())
        }
    }

    // A real installed v3 channel manifest: channel surface + [auth.{vendor}]
    // + non-secret scoping fields under the claim-suffix convention.
    let manifest = format!(
        r#"
schema_version = "reborn.extension_manifest.v3"
id = "{EXTENSION_ID}"
name = "AcmeChat"
version = "0.1.0"
description = "generic channel identity binding integration fixture"
trust = "first_party_requested"

[runtime]
kind = "first_party"
service = "acmechat.extension/v1"

[[tools]]
id = "acmechat.read_messages"
description = "Read AcmeChat messages"
effects = ["network", "use_secret"]
default_permission = "ask"
visibility = "model"
input_schema_ref = "schemas/acmechat/read_messages.input.v1.json"

[[tools.credentials]]
handle = "acmechat_user_token"
vendor = "{VENDOR}"
scopes = ["test.readonly"]
audience = {{ scheme = "https", host = "api.acmechat.example" }}
injection = {{ type = "header", name = "authorization", prefix = "Bearer " }}

[channel]
id = "messages"
display_name = "AcmeChat messages"
inbound = true
outbound = true
conversation_model = "continuous"

[channel.ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[channel.ingress.verification]
kind = "shared_secret_header"
secret_handle = "acmechat_webhook_secret"
header = "X-AcmeChat-Secret"

[channel.config]
fields = [
  {{ handle = "acmechat_webhook_secret", label = "Webhook secret", secret = true }},
  {{ handle = "acmechat_team_id", label = "Workspace ID", secret = false }},
  {{ handle = "acmechat_app_id", label = "App ID", secret = false }},
]

[channel.presentation]
supports_markdown = false
supports_threads = false

[auth.{VENDOR}]
method = "oauth2_code"
display_name = "AcmeChat account"
authorization_endpoint = "https://oauth.test.example.com/authorize"
token_endpoint = "https://oauth.test.example.com/token"
scopes = ["test.readonly"]
client_credentials = {{ client_id_handle = "acmechat_oauth_client_id" }}

[auth.{VENDOR}.token_response]
access_token = "/access_token"

[auth.{VENDOR}.identity]
account_id = "/authed_user/id"
team_id = "/team/id"
app_id = "/app_id"
"#
    );
    let installation_store = Arc::new(InMemoryExtensionInstallationStore::default());
    let record = ExtensionManifestRecord::from_toml(
        &manifest,
        ManifestSource::HostBundled,
        &ironclaw_host_runtime::default_host_port_catalog().expect("catalog"),
        None,
        &ironclaw_host_runtime::default_host_api_contract_registry().expect("contracts"),
    )
    .expect("fixture manifest parses");
    let extension_id = ExtensionId::new(EXTENSION_ID).expect("extension id");
    installation_store
        .upsert_manifest_and_installation(
            record,
            ExtensionInstallation::new(
                ExtensionInstallationId::new(INSTALLATION_ID.to_string()).expect("installation id"),
                extension_id.clone(),
                ExtensionActivationState::Installed,
                ExtensionManifestRef::new(extension_id.clone(), None),
                Vec::new(),
                chrono::Utc::now(),
            )
            .expect("installation"),
        )
        .await
        .expect("persist install");
    // Operator-configured connection scoping values ([channel.config]).
    installation_store
        .set_channel_config(
            &extension_id,
            vec![
                ("acmechat_team_id".to_string(), "T-team".to_string()),
                ("acmechat_app_id".to_string(), "A-app".to_string()),
            ],
        )
        .await
        .expect("store scoping values");

    let identity_store = Arc::new(RecordingIdentityStore::default());
    let scope = test_scope();
    let binding_config = ChannelIdentityBindingConfig::for_test(
        scope.resource.tenant_id.clone(),
        Arc::clone(&installation_store) as Arc<dyn ExtensionInstallationStore>,
        identity_store.clone(),
        identity_store.clone(),
    );
    let provider = AuthProviderId::new(VENDOR).unwrap();

    let run_callback = |token_body: serde_json::Value, fill: u8| {
        let scope = scope.clone();
        let provider = provider.clone();
        let binding_config = binding_config.clone();
        async move {
            let bundle = build_oauth_product_auth_with_identity_for_test(VENDOR, &token_body);
            let state_hash = OpaqueStateHash::new(hex64(fill)).unwrap();
            let expires_at = Utc::now() + Duration::minutes(5);
            let flow = bundle
                .services
                .flow_manager()
                .create_flow(NewAuthFlow {
                    id: None,
                    scope: scope.clone(),
                    kind: AuthFlowKind::IntegrationCredential,
                    provider: provider.clone(),
                    challenge: AuthChallenge::OAuthUrl {
                        authorization_url: OAuthAuthorizationUrl::new(
                            "https://oauth.test.example.com/authorize",
                        )
                        .unwrap(),
                        expires_at,
                    },
                    continuation: AuthContinuationRef::SetupOnly,
                    update_binding: None,
                    opaque_state_hash: Some(state_hash.clone()),
                    pkce_verifier_hash: Some(PkceVerifierHash::new(hex64(fill)).unwrap()),
                    expires_at,
                })
                .await
                .expect("create_flow must succeed");
            let response = handle_oauth_callback_with_channel_identity_binding_for_test(
                &bundle.services,
                RebornOAuthCallbackRequest {
                    scope: scope.clone(),
                    flow_id: flow.id,
                    opaque_state_hash: state_hash,
                    outcome: RebornOAuthCallbackOutcome::Authorized {
                        provider_request: OAuthProviderCallbackRequest {
                            provider: provider.clone(),
                            account_label: CredentialAccountLabel::new("Channel Account").unwrap(),
                            authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                                "channel-auth-code".to_string(),
                            ))
                            .unwrap(),
                            authorization_code_hash: AuthorizationCodeHash::new(hex64(fill))
                                .unwrap(),
                            pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                                "channel-pkce-verifier".to_string(),
                            ))
                            .unwrap(),
                            pkce_verifier_hash: PkceVerifierHash::new(hex64(fill)).unwrap(),
                            scopes: vec![ProviderScope::new("test.readonly").unwrap()],
                        },
                    },
                },
                &binding_config,
            )
            .await;
            (bundle, response)
        }
    };

    // Phase 1: the proven workspace claim does not match the configured
    // scoping value — the generic hook must fail the callback closed.
    let (bundle, response) = run_callback(
        serde_json::json!({
            "access_token": "channel-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "authed_user": { "id": "U123" },
            "team": { "id": "T-other" },
            "app_id": "A-app",
        }),
        0x41,
    )
    .await;
    response.expect_err("a scoping mismatch must fail the OAuth callback");
    assert!(
        identity_store.bindings.lock().unwrap().is_empty(),
        "no identity binding may be written for a rejected callback"
    );
    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(
            scope.clone(),
            provider.clone(),
        ))
        .await
        .expect("list_accounts must not error");
    assert!(
        page.accounts.is_empty(),
        "no credential account may persist when the identity check rejects"
    );

    // Phase 2: matching claims — the callback completes, the account
    // persists, and the binding is keyed by the installation-scoped id.
    let (bundle, response) = run_callback(
        serde_json::json!({
            "access_token": "channel-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "authed_user": { "id": "U123" },
            "team": { "id": "T-team" },
            "app_id": "A-app",
        }),
        0x42,
    )
    .await;
    let response = response.expect("matching claims must complete the callback");
    let account_id = response
        .credential_account_id
        .expect("completed callback must carry a credential_account_id");
    bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .expect("get_account must not error")
        .expect("credential account must be persisted");
    let bindings = identity_store.bindings.lock().unwrap();
    assert_eq!(
        bindings.len(),
        1,
        "exactly one identity binding must be written through the generic hook"
    );
    assert_eq!(bindings[0].provider.as_str(), VENDOR);
    assert_eq!(
        bindings[0].provider_user_id.as_str(),
        format!("{INSTALLATION_ID}:U123"),
        "the binding must be keyed by the installation-scoped provider user id"
    );
    assert_eq!(
        bindings[0].user_id.as_str(),
        scope.resource.user_id.as_str(),
        "the binding must attach to the authenticated caller"
    );
}
