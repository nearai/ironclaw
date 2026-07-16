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

// Mounted for the Postgres arm only: the harness owns the real-Postgres
// testcontainer provisioner (`start_postgres_testcontainer` + `postgres_pool`),
// reused here rather than duplicated (correction A: prefer the harness's
// real-Postgres lane).
#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use chrono::{DateTime, Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowKind, AuthFlowStatus,
    AuthProductScope, AuthProviderId, AuthSurface, AuthorizationCodeHash, CredentialAccountLabel,
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

/// A `NewAuthFlow` for the connect-flow tests; only identity hashes and the
/// expiry vary between scenarios.
fn new_flow_request(
    scope: &AuthProductScope,
    provider: &AuthProviderId,
    state_hash: &OpaqueStateHash,
    pkce_hash: &PkceVerifierHash,
    expires_at: DateTime<Utc>,
) -> NewAuthFlow {
    NewAuthFlow {
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
    }
}

/// An `Authorized` provider-callback request matching `new_flow_request`'s
/// hashes — the "user completed the provider consent page" leg.
fn authorized_callback_request(
    scope: &AuthProductScope,
    flow_id: AuthFlowId,
    provider: &AuthProviderId,
    state_hash: &OpaqueStateHash,
    pkce_hash: &PkceVerifierHash,
    code_hash: &AuthorizationCodeHash,
    label: &str,
) -> RebornOAuthCallbackRequest {
    RebornOAuthCallbackRequest {
        scope: scope.clone(),
        flow_id,
        opaque_state_hash: state_hash.clone(),
        outcome: RebornOAuthCallbackOutcome::Authorized {
            provider_request: OAuthProviderCallbackRequest {
                provider: provider.clone(),
                account_label: CredentialAccountLabel::new(label).unwrap(),
                authorization_code: OAuthAuthorizationCode::new(SecretString::from(format!(
                    "auth-code-{label}"
                )))
                .unwrap(),
                authorization_code_hash: code_hash.clone(),
                pkce_verifier: PkceVerifierSecret::new(SecretString::from(format!(
                    "pkce-verifier-{label}"
                )))
                .unwrap(),
                pkce_verifier_hash: pkce_hash.clone(),
                scopes: vec![ProviderScope::new("test.readonly").unwrap()],
            },
        },
    }
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

    let flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &state_hash,
            &pkce_hash,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("create_flow must succeed");

    // Drives claim → token exchange → complete. The scripted egress returns a
    // fixed access-token JSON body; no real network call is made.
    let response = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            flow.id,
            &provider,
            &state_hash,
            &pkce_hash,
            &code_hash,
            "Test Account",
        ))
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

/// The same connect flow persisted through a real Postgres-backed durable
/// flow/account store — the auth engine's both-DB persistence leg on real
/// PostgreSQL (checklist AUTH-15; REL-3: a Postgres skip is a failure). Reuses
/// the harness's testcontainer provisioner; the OAuth product-auth bundle is
/// built outside the harness storage composite (so it can't reuse
/// `StorageMode::Postgres`) and takes the pool directly.
#[cfg(feature = "postgres")]
#[tokio::test]
async fn oauth_connect_flow_persists_credential_account_on_postgres() {
    let (_container, database_url) = reborn_support::builder::start_postgres_testcontainer()
        .await
        .expect("postgres testcontainer must start (REL-3: a skip is a failure)");
    let pool =
        reborn_support::builder::postgres_pool(&database_url).expect("postgres pool must build");
    let root = std::sync::Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(pool));
    root.run_migrations()
        .await
        .expect("postgres filesystem migrations");
    let bundle =
        ironclaw_reborn_composition::test_support::build_oauth_product_auth_for_test_on_root(root)
            .await;
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();
    let state_hash = OpaqueStateHash::new(hex64(0x51)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0x52)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0x53)).unwrap();
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
        .expect("create_flow must succeed on postgres");

    let response = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope.clone(),
            flow_id: flow.id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: provider.clone(),
                    account_label: CredentialAccountLabel::new("Postgres Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "postgres-auth-code".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "postgres-pkce-verifier".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier_hash: pkce_hash,
                    scopes: vec![ProviderScope::new("test.readonly").unwrap()],
                },
            },
        })
        .await
        .expect("handle_oauth_callback must succeed on postgres");

    let account_id = response
        .credential_account_id
        .expect("completed callback must carry a credential_account_id");
    let account = bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(scope, account_id))
        .await
        .expect("get_account must not error")
        .expect("credential account must persist on the postgres backend");
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
    let state_hash = OpaqueStateHash::new(hex64(0xdd)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0xee)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0xff)).unwrap();

    let error = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            AuthFlowId::new(), // no flow was created for this id
            &AuthProviderId::new("test-oauth-provider").unwrap(),
            &state_hash,
            &pkce_hash,
            &code_hash,
            "Guard Account",
        ))
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
            scope,
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
                ironclaw_extensions::InstallationOwner::Tenant,
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

/// T4 of the #6105 lifecycle transitions (issues #2858/#2534/#6043 shape): a
/// callback that lands AFTER the flow lapsed (the user abandoned or lost the
/// provider tab) must be rejected terminally — and a RETRIED connect with a
/// fresh flow must then succeed cleanly. The expired first attempt must leave
/// no half-connected state behind: its record reads back terminal `Expired`,
/// no token exchange crossed the egress for it, and after the retry exactly
/// one credential account exists.
#[tokio::test]
async fn expired_flow_callback_rejected_then_fresh_flow_retry_succeeds() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();

    let state_hash = OpaqueStateHash::new(hex64(0x11)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0x22)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0x33)).unwrap();

    // The flow lapsed before the callback arrived.
    let expired_flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &state_hash,
            &pkce_hash,
            Utc::now() - Duration::seconds(10),
        ))
        .await
        .expect("create_flow must accept a flow that will read back as lapsed");

    let error = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            expired_flow.id,
            &provider,
            &state_hash,
            &pkce_hash,
            &code_hash,
            "Late Callback",
        ))
        .await
        .expect_err("a callback for a lapsed flow must be rejected");
    assert_eq!(
        error.code,
        AuthErrorCode::UnknownOrExpiredFlow,
        "lapsed-flow callback must surface as UnknownOrExpiredFlow"
    );

    // Durable evidence of a clean rejection: the record is terminal Expired
    // (not half-claimed), and the token exchange never ran.
    let record = bundle
        .services
        .flow_manager()
        .get_flow(&scope, expired_flow.id)
        .await
        .expect("get_flow must not error")
        .expect("the expired flow record must remain readable");
    assert_eq!(
        record.status,
        AuthFlowStatus::Expired,
        "the lapsed flow must be marked terminal Expired, not left pending"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        0,
        "no token exchange may run for a lapsed-flow callback"
    );

    // Retry: a fresh flow (new state/PKCE — a new grant) for the same
    // provider and scope. An expired predecessor must not fence it out.
    let retry_state = OpaqueStateHash::new(hex64(0x44)).unwrap();
    let retry_pkce = PkceVerifierHash::new(hex64(0x55)).unwrap();
    let retry_code = AuthorizationCodeHash::new(hex64(0x66)).unwrap();
    let retry_flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &retry_state,
            &retry_pkce,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("a retried create_flow must succeed after an expired predecessor");
    let response = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            retry_flow.id,
            &provider,
            &retry_state,
            &retry_pkce,
            &retry_code,
            "Retry Account",
        ))
        .await
        .expect("the retried callback must succeed; an expired flow must not wedge the retry");
    let account_id = response
        .credential_account_id
        .expect("the retried callback must mint a credential account");

    // Exactly ONE account and ONE token exchange: the failed first attempt
    // contributed nothing.
    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(
            scope.clone(),
            provider.clone(),
        ))
        .await
        .expect("list_accounts must not error after the retry");
    assert_eq!(
        page.accounts.len(),
        1,
        "only the retry's account may exist; a lapsed flow must not leave a half-connected account"
    );
    assert_eq!(
        page.accounts[0].id, account_id,
        "the surviving account is the retry's account"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "exactly the retry's token exchange may cross the egress"
    );
}

/// T4 of the #6105 lifecycle transitions, replay arm (issue #2858 shape): a
/// callback REPLAYED after the flow already completed (browser back-button /
/// duplicated redirect) is IDEMPOTENT on the durable services: it returns the
/// original completed outcome — same credential account — without re-running
/// the token exchange or minting a second account, and a subsequent fresh
/// connect (reconnect epoch) still succeeds.
///
/// NOT a fake-vs-durable divergence (both impls agree at every
/// `AuthFlowManager` method — see `ironclaw_auth::conformance`): the
/// replay-safety split is between SEAMS. `claim_oauth_callback` is
/// idempotent on terminal flows in both impls, and `handle_oauth_callback`
/// (this wrapper, the hosted unauthenticated callback route's entry)
/// short-circuits at that claim — while trait-level
/// `complete_oauth_callback` stays fail-closed (`FlowAlreadyTerminal`) in
/// both. This test pins the wrapper's replay-idempotent shape.
#[tokio::test]
async fn replayed_callback_is_idempotent_then_fresh_flow_reconnects() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();

    let state_hash = OpaqueStateHash::new(hex64(0x71)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0x72)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0x73)).unwrap();

    let flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &state_hash,
            &pkce_hash,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("create_flow must succeed");
    let first = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            flow.id,
            &provider,
            &state_hash,
            &pkce_hash,
            &code_hash,
            "First Grant",
        ))
        .await
        .expect("the first callback must complete the flow");
    let first_account = first
        .credential_account_id
        .expect("the first callback must mint a credential account");
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "the first callback runs exactly one token exchange"
    );

    // Replay the identical callback (browser back-button / duplicated
    // redirect). It must return the ORIGINAL completed outcome — not error,
    // not a half-processed or duplicated grant.
    let replay = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            flow.id,
            &provider,
            &state_hash,
            &pkce_hash,
            &code_hash,
            "First Grant",
        ))
        .await
        .expect("a replayed callback for a completed flow must be idempotent, not an error");
    assert_eq!(
        replay.credential_account_id,
        Some(first_account),
        "the replayed callback must return the original grant's credential account"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "a replayed callback must not re-run the token exchange"
    );
    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(
            scope.clone(),
            provider.clone(),
        ))
        .await
        .expect("list_accounts must not error after the replay");
    assert_eq!(
        page.accounts.len(),
        1,
        "a replayed callback must not mint a second credential account"
    );

    // Reconnect: a fresh flow must still succeed after the idempotent replay
    // (which returned the ORIGINAL completed outcome — no rejection occurred).
    let reconnect_state = OpaqueStateHash::new(hex64(0x81)).unwrap();
    let reconnect_pkce = PkceVerifierHash::new(hex64(0x82)).unwrap();
    let reconnect_code = AuthorizationCodeHash::new(hex64(0x83)).unwrap();
    let reconnect_flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &reconnect_state,
            &reconnect_pkce,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("a reconnect create_flow must succeed after a replayed predecessor");
    let reconnect = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            reconnect_flow.id,
            &provider,
            &reconnect_state,
            &reconnect_pkce,
            &reconnect_code,
            "Reconnect Grant",
        ))
        .await
        .expect("the reconnect callback must succeed; a replayed flow must not wedge reconnects");
    let reconnect_account = reconnect
        .credential_account_id
        .expect("the reconnect callback must mint a credential account");
    assert_ne!(
        reconnect_account, first_account,
        "a fresh reconnect flow must mint a DISTINCT credential account, not reuse the original"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        2,
        "the reconnect must run its own token exchange (first callback's plus one)"
    );
    let listed = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(scope, provider))
        .await
        .expect("list_accounts must not error after the reconnect");
    assert_eq!(
        listed.accounts.len(),
        2,
        "exactly the original and the reconnect accounts must exist"
    );
    for expected in [&first_account, &reconnect_account] {
        assert!(
            listed
                .accounts
                .iter()
                .any(|account| &account.id == expected),
            "account {expected:?} must be listed after the reconnect"
        );
    }
}
