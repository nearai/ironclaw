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
//!
//! The abandoned/denied/expired/replayed POPUP journeys over the same seam
//! live next door in `oauth_popup_journeys.rs`.

#[path = "common.rs"]
mod common;

use std::sync::Arc;

use chrono::{Duration, Utc};
use common::{authorized_callback_request, hex64, new_flow_request, test_scope};
use ironclaw_auth::AuthProviderClient;
use ironclaw_auth::{
    AuthErrorCode, AuthFlowId, AuthProductScope, AuthProviderId, AuthSurface,
    AuthorizationCodeHash, CredentialAccountLabel, CredentialAccountListRequest,
    CredentialAccountLookupRequest, CredentialAccountStatus, CredentialOwnership,
    NewCredentialAccount, OAuthAuthorizationCode, OAuthProviderCallbackRequest,
    OAuthProviderExchangeContext, OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret,
    PrepareOAuthFlowRequest, ProviderScope,
};
use ironclaw_composition::test_support::{
    ScriptedOAuthTokenEgress, build_oauth_product_auth_for_test,
};
use ironclaw_extension_registry::{
    ExtensionInstallation, ExtensionInstallationId, ExtensionInstallationStore,
    ExtensionInstallationStorePort, ExtensionManifestRecord, ExtensionManifestRef, ManifestSource,
};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{
    ids::{ExtensionId, SecretHandle, UserId},
    path::VirtualPath,
    resource::ResourceScope,
};

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
}

/// Cross-implementation conformance: the durable `FilesystemAuthProductServices`
/// must satisfy the same observable OAuth-callback state machine
/// (`ironclaw_auth::test_support::conformance`) as the in-memory fake most consumer tests
/// run against; the fake's invocation lives in
/// `crates/domains/ironclaw_auth/tests/auth_product_contract/oauth_flow_contract.rs`.
/// The suite drives `AuthFlowManager` directly with pre-exchanged outcomes,
/// so no token-exchange egress is involved — the exchange leg is covered by
/// the surrounding tests in this file.
#[tokio::test]
async fn durable_flow_manager_satisfies_shared_oauth_flow_conformance() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();
    ironclaw_auth::test_support::conformance::assert_auth_flow_callback_conformance(
        bundle.services.flow_manager().as_ref(),
        &scope,
        &provider,
    )
    .await;
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

// ---------------------------------------------------------------------------
// #7069 — one vendor authorization covers every installed extension that
// shares the vendor account.
// ---------------------------------------------------------------------------

/// A credential account is SHARED by every installed extension of a vendor, so
/// the authorization a gate starts for ONE of them must ask for the whole
/// shared-vendor ceiling. Otherwise each sibling Google service raises its own
/// consent and keeps returning `auth_required` until separately authorized
/// (#7069).
///
/// Drives the production chain end to end: the real bundled gmail +
/// google-drive manifests -> the real `InstalledManifestAuthRecipeResolver`
/// (which unions their scope ceilings) -> a real `AuthEngine` connect flow ->
/// the real token exchange -> a persisted account -> the real dispatch-gate
/// predicate that produces the user-visible `auth_required`.
#[tokio::test]
async fn one_google_authorization_satisfies_every_installed_google_extension() {
    let gmail = bundled_manifest_record("gmail");
    let drive = bundled_manifest_record("google-drive");

    // The production resolver, over exactly the two installed manifests.
    let resolver = Arc::new(
        ironclaw_extension_host::InstalledManifestAuthRecipeResolver::new(
            installed_store(&["gmail", "google-drive"]).await,
        ),
    );

    let gmail_requirements = runtime_credential_requirements(&gmail);
    let drive_requirements = runtime_credential_requirements(&drive);
    let gmail_scopes = provider_scopes(&gmail_requirements);
    let drive_scopes = provider_scopes(&drive_requirements);
    assert!(
        !gmail_scopes.is_empty() && !drive_scopes.is_empty(),
        "both bundled google manifests must declare runtime credential scopes"
    );

    // A gmail-scoped connect flow, exactly as the auth gate starts one.
    // Google echoes the cumulative grant on every exchange
    // (`include_granted_scopes=true`, declared by every google manifest).
    let granted = gmail_scopes
        .iter()
        .chain(drive_scopes.iter())
        .map(|scope| scope.as_str().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let egress = Arc::new(ScriptedOAuthTokenEgress::with_json_body(
        &serde_json::json!({
            "access_token": "itest-google-access-token",
            "refresh_token": "itest-google-refresh-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": granted,
        }),
    ));
    let engine = engine_over_resolver(resolver, Arc::clone(&egress));
    let scope = test_scope();
    let gmail_extension = ExtensionId::new("gmail").expect("gmail extension id");
    let prepared = engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "google".to_string(),
            requester_extension: Some(gmail_extension.clone()),
            scope: scope.clone(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("google").unwrap(),
            requested_scopes: gmail_scopes.clone(),
        })
        .await
        .expect("gmail connect flow prepares over the installed google recipe");

    let authorize_url = url::Url::parse(prepared.authorization_url.as_str()).unwrap();
    let consented: Vec<String> = authorize_url
        .query_pairs()
        .find(|(key, _)| key == "scope")
        .map(|(_, value)| value.split(' ').map(str::to_string).collect())
        .expect("authorize URL carries a scope param");
    for scope_needed in gmail_scopes.iter().chain(drive_scopes.iter()) {
        assert!(
            consented
                .iter()
                .any(|granted| granted == scope_needed.as_str()),
            "one consent must cover every installed google extension; {} missing from {consented:?}",
            scope_needed.as_str()
        );
    }

    // The vendor grants what was asked for; the real exchange clamps it to the
    // recipe ceiling and that is what the account stores.
    let exchange = engine
        .exchange_callback_for_requester(
            Some(gmail_extension),
            OAuthProviderExchangeContext {
                scope: scope.clone(),
                flow_id: AuthFlowId::new(),
            },
            google_callback_request(consented.clone()),
        )
        .await
        .expect("token exchange succeeds against the scripted vendor");

    // Persist it through a real account store, then ask the real dispatch-gate
    // predicate whether google-drive still needs authorization.
    // `RebornProductAuthServices::from_shared` is the constructor that wires
    // the credential-account RECORD SOURCE the runtime selector reads.
    let services = ironclaw_auth::RebornProductAuthServices::from_shared(
        Arc::new(ironclaw_auth::InMemoryAuthProductServices::new()),
        Arc::new(NoopContinuationDispatcher),
    );
    let access_secret = SecretHandle::new("google_oauth_access_token").unwrap();
    services
        .credential_account_service()
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            // What the OAuth callback creates for every account it mints.
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(access_secret),
            refresh_secret: None,
            scopes: exchange.scopes.clone(),
        })
        .await
        .expect("credential account persists");

    let missing = ironclaw_auth::product_auth::credentials::runtime_credentials::missing_runtime_credential_auth_requirements(
        services.runtime_credential_account_selection_service().as_ref(),
        &scope.resource,
        drive_requirements,
    )
    .await
    .expect("credential readiness reads the account store");

    assert!(
        missing.is_empty(),
        "after ONE google authorization started by gmail, google-drive must not \
         report a missing credential — that is the `auth_required` users see; \
         missing: {missing:?}"
    );
}

/// Registration is tenant-wide; installation is per user. The scope ceiling
/// must follow the INSTALL, so one user's consent screen never carries scopes
/// for an extension a DIFFERENT user installed (#7078).
#[tokio::test]
async fn one_users_consent_never_carries_another_users_installed_scopes() {
    let gmail_scopes = provider_scopes(&runtime_credential_requirements(&bundled_manifest_record(
        "gmail",
    )));
    let drive_scopes = provider_scopes(&runtime_credential_requirements(&bundled_manifest_record(
        "google-drive",
    )));

    // alice installed gmail; bob installed google-drive. Both manifests are
    // registered tenant-wide, as registration always is.
    let store = installed_store_for_users(&[("gmail", "alice"), ("google-drive", "bob")]).await;
    let resolver =
        Arc::new(ironclaw_extension_host::InstalledManifestAuthRecipeResolver::new(store));
    let engine = engine_over_resolver(
        resolver,
        Arc::new(ScriptedOAuthTokenEgress::with_access_token("itest-token")),
    );

    let alice = AuthProductScope::new(
        ResourceScope::local_default(
            UserId::new("alice").expect("alice"),
            ironclaw_host_api::ids::InvocationId::new(),
        )
        .expect("alice scope"),
        AuthSurface::Callback,
    );
    let prepared = engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "google".to_string(),
            requester_extension: Some(ExtensionId::new("gmail").expect("gmail extension id")),
            scope: alice,
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("google").unwrap(),
            requested_scopes: gmail_scopes.clone(),
        })
        .await
        .expect("alice's gmail connect flow prepares");

    let consented = url::Url::parse(prepared.authorization_url.as_str())
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "scope")
        .map(|(_, value)| value.to_string())
        .expect("authorize URL carries a scope param");
    // Token, not substring, membership: these scopes are genuine prefixes of
    // one another (e.g. `.../auth/drive` inside `.../auth/drive.readonly`),
    // so a raw `.contains` on the joined string can match the wrong scope.
    let tokens: Vec<&str> = consented.split(' ').collect();

    for own in &gmail_scopes {
        assert!(
            tokens.iter().any(|token| *token == own.as_str()),
            "alice must still consent to the extension she installed; {} missing from {consented}",
            own.as_str()
        );
    }
    for other in &drive_scopes {
        assert!(
            !tokens.iter().any(|token| *token == other.as_str()),
            "google-drive is BOB's install — its scopes must never reach alice's \
             consent screen; {} appeared in {consented}",
            other.as_str()
        );
    }
}

/// A sibling extension uninstalled while the user is on the vendor's consent
/// screen must not fail the requesting extension's own callback.
///
/// An extension-scoped flow persists the shared-vendor ceiling as it stood at
/// prepare time, and the callback re-resolves the recipe from the live
/// installation store. Rejecting scopes that the shrunken ceiling no longer
/// declares would fail gmail's authorization because google-drive happened to
/// be removed mid-consent — lifecycle cleanup deliberately does not cancel
/// shared-provider flows. The exchange clamps to the CURRENT ceiling instead,
/// so the grant is never wider than what is authorized now.
#[tokio::test]
async fn sibling_uninstall_during_consent_does_not_fail_the_requesters_callback() {
    let gmail_scopes = provider_scopes(&runtime_credential_requirements(&bundled_manifest_record(
        "gmail",
    )));
    let drive_scopes = provider_scopes(&runtime_credential_requirements(&bundled_manifest_record(
        "google-drive",
    )));

    // Prepare while BOTH are installed: the consent covers the shared ceiling.
    let store = installed_store(&["gmail", "google-drive"]).await;
    let resolver =
        Arc::new(ironclaw_extension_host::InstalledManifestAuthRecipeResolver::new(store.clone()));
    let granted = gmail_scopes
        .iter()
        .chain(drive_scopes.iter())
        .map(|scope| scope.as_str().to_string())
        .collect::<Vec<_>>();
    let egress = Arc::new(ScriptedOAuthTokenEgress::with_json_body(
        &serde_json::json!({
            "access_token": "itest-google-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": granted.join(" "),
        }),
    ));
    let engine = engine_over_resolver(resolver, egress);
    let scope = test_scope();
    let gmail_extension = ExtensionId::new("gmail").expect("gmail extension id");
    let prepared = engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "google".to_string(),
            requester_extension: Some(gmail_extension.clone()),
            scope: scope.clone(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("google").unwrap(),
            requested_scopes: gmail_scopes.clone(),
        })
        .await
        .expect("gmail connect flow prepares over both installed manifests");
    let consented = url::Url::parse(prepared.authorization_url.as_str())
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "scope")
        .map(|(_, value)| value.split(' ').map(str::to_string).collect::<Vec<_>>())
        .expect("authorize URL carries a scope param");

    // google-drive is uninstalled while the consent screen is open.
    store
        .delete_installation(
            &ExtensionInstallationId::new("itest-google-drive".to_string())
                .expect("installation id"),
        )
        .await
        .expect("remove google-drive installation");
    store
        .delete_manifest(&ExtensionId::new("google-drive").expect("drive extension id"))
        .await
        .expect("uninstall google-drive");

    // The callback still completes, carrying the prepare-time scope set.
    let exchange = engine
        .exchange_callback_for_requester(
            Some(gmail_extension),
            OAuthProviderExchangeContext {
                scope: scope.clone(),
                flow_id: AuthFlowId::new(),
            },
            google_callback_request(consented),
        )
        .await
        .expect("an unrelated sibling uninstall must not fail this flow's callback");

    for scope_needed in &gmail_scopes {
        assert!(
            exchange.scopes.contains(scope_needed),
            "the requester's own scopes must survive the clamp; {} missing from {:?}",
            scope_needed.as_str(),
            exchange.scopes
        );
    }
    for removed in &drive_scopes {
        assert!(
            !exchange.scopes.contains(removed),
            "a scope whose extension is gone must be clamped out of the stored grant; \
             {} survived in {:?}",
            removed.as_str(),
            exchange.scopes
        );
    }
}

/// The other half of the #7069 contract, and the control that proves the
/// assertions above discriminate:
///
/// - consent is BOUNDED by what is installed — a google extension the user has
///   not installed must not have its scopes added to the consent screen;
/// - the dispatch-gate predicate still REPORTS a missing credential when the
///   account genuinely lacks a sibling's scopes. Without this, the
///   "nothing is missing" assertion above could pass vacuously.
#[tokio::test]
async fn google_consent_is_bounded_by_installed_extensions() {
    let gmail = bundled_manifest_record("gmail");
    let drive = bundled_manifest_record("google-drive");
    let gmail_scopes = provider_scopes(&runtime_credential_requirements(&gmail));
    let drive_requirements = runtime_credential_requirements(&drive);
    let drive_scopes = provider_scopes(&drive_requirements);

    // Only gmail is installed: the ceiling is gmail's alone.
    let resolver = Arc::new(
        ironclaw_extension_host::InstalledManifestAuthRecipeResolver::new(
            installed_store(&["gmail"]).await,
        ),
    );
    let egress = Arc::new(ScriptedOAuthTokenEgress::with_access_token(
        "itest-google-access-token",
    ));
    let engine = engine_over_resolver(resolver, egress);
    let scope = test_scope();
    let prepared = engine
        .prepare_oauth_flow(PrepareOAuthFlowRequest {
            vendor: "google".to_string(),
            requester_extension: Some(ExtensionId::new("gmail").expect("gmail extension id")),
            scope: scope.clone(),
            flow_id: AuthFlowId::new(),
            account_label: CredentialAccountLabel::new("google").unwrap(),
            requested_scopes: gmail_scopes.clone(),
        })
        .await
        .expect("gmail connect flow prepares");
    let authorize_url = url::Url::parse(prepared.authorization_url.as_str()).unwrap();
    let consented = authorize_url
        .query_pairs()
        .find(|(key, _)| key == "scope")
        .map(|(_, value)| value.to_string())
        .expect("authorize URL carries a scope param");
    // Token, not substring, membership — see the note in
    // `one_users_consent_never_carries_another_users_installed_scopes`.
    let tokens: Vec<&str> = consented.split(' ').collect();
    for uninstalled in &drive_scopes {
        assert!(
            !tokens.iter().any(|token| *token == uninstalled.as_str()),
            "an extension the user never installed must not widen the consent \
             screen; {} appeared in {consented}",
            uninstalled.as_str()
        );
    }

    // Control: an account carrying only gmail's scopes must still leave
    // google-drive reported as needing authorization.
    let services = ironclaw_auth::RebornProductAuthServices::from_shared(
        Arc::new(ironclaw_auth::InMemoryAuthProductServices::new()),
        Arc::new(NoopContinuationDispatcher),
    );
    services
        .credential_account_service()
        .create_account(NewCredentialAccount {
            scope: scope.clone(),
            provider: AuthProviderId::new("google").unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership: CredentialOwnership::UserReusable,
            owner_extension: None,
            granted_extensions: Vec::new(),
            access_secret: Some(SecretHandle::new("google_oauth_access_token").unwrap()),
            refresh_secret: None,
            scopes: gmail_scopes,
        })
        .await
        .expect("credential account persists");

    let missing = ironclaw_auth::product_auth::credentials::runtime_credentials::missing_runtime_credential_auth_requirements(
        services.runtime_credential_account_selection_service().as_ref(),
        &scope.resource,
        drive_requirements,
    )
    .await
    .expect("credential readiness reads the account store");

    assert!(
        !missing.is_empty(),
        "a gmail-only account must NOT satisfy google-drive — otherwise the          one-authorization assertion in the test above proves nothing"
    );
}

/// A per-user installation: the membership rows that record WHO installed an
/// extension, as opposed to the tenant-wide registration of its manifest.
async fn installed_store_for_users(packages: &[(&str, &str)]) -> Arc<ExtensionInstallationStore> {
    let store = ExtensionInstallationStore::load_at(
        Arc::new(InMemoryBackend::new()),
        VirtualPath::new("/system/extensions/.installations/oauth-connect-multiuser")
            .expect("valid installation root"),
        ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog"),
        ironclaw_extension_registry::default_host_api_contract_registry()
            .expect("host API contracts"),
    )
    .await
    .expect("filesystem installation store");
    for (package, owner) in packages {
        let record = bundled_manifest_record(package);
        let extension_id = record.extension_id().clone();
        store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new(format!("itest-{package}"))
                        .expect("installation id"),
                    extension_id.clone(),
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    Utc::now(),
                    ironclaw_extension_registry::InstallationOwner::Users {
                        user_ids: [UserId::new(*owner).expect("owner user id")]
                            .into_iter()
                            .collect(),
                    },
                )
                .expect("installation"),
            )
            .await
            .expect("persist install");
    }
    Arc::new(store)
}

/// The real durable installation store, holding exactly `packages` — the
/// inventory `InstalledManifestAuthRecipeResolver` unions vendor recipes over.
/// Same construction as `oauth_popup_journeys.rs`.
async fn installed_store(packages: &[&str]) -> Arc<ExtensionInstallationStore> {
    let store = ExtensionInstallationStore::load_at(
        Arc::new(InMemoryBackend::new()),
        VirtualPath::new("/system/extensions/.installations/oauth-connect")
            .expect("valid installation root"),
        ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog"),
        ironclaw_extension_registry::default_host_api_contract_registry()
            .expect("host API contracts"),
    )
    .await
    .expect("filesystem installation store");
    for package in packages {
        let record = bundled_manifest_record(package);
        let extension_id = record.extension_id().clone();
        store
            .upsert_manifest_and_installation(
                record,
                ExtensionInstallation::new(
                    ExtensionInstallationId::new(format!("itest-{package}"))
                        .expect("installation id"),
                    extension_id.clone(),
                    ExtensionManifestRef::new(extension_id, None),
                    Vec::new(),
                    Utc::now(),
                    ironclaw_extension_registry::InstallationOwner::Tenant,
                )
                .expect("installation"),
            )
            .await
            .expect("persist install");
    }
    Arc::new(store)
}

/// Parse a real bundled package manifest the way the installation store does.
fn bundled_manifest_record(package: &str) -> ExtensionManifestRecord {
    let host_ports =
        ironclaw_host_api::host_port::default_host_port_catalog().expect("host port catalog loads");
    let contracts = ironclaw_extension_host::product_extension_host_api_contract_registry()
        .expect("host api contracts load");
    let path = format!(
        "{}/crates/extensions/packages/{package}/manifest.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    let toml_text =
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    ExtensionManifestRecord::from_toml(
        toml_text,
        ManifestSource::HostBundled,
        &host_ports,
        None,
        &contracts,
        None,
    )
    .unwrap_or_else(|error| panic!("{package} manifest parses: {error}"))
}

/// The runtime credential requirements production derives from an installed
/// manifest (`ExtensionLifecycleManager::removed_extension_providers_from_manifest`
/// takes the same route).
fn runtime_credential_requirements(
    record: &ExtensionManifestRecord,
) -> Vec<ironclaw_host_api::decision::RuntimeCredentialAuthRequirement> {
    let manifest = record
        .manifest()
        .clone()
        .try_into()
        .expect("bundled manifest converts to the resolved model");
    ironclaw_extension_host::manifest_runtime_credential_auth_requirements(&manifest)
}

fn provider_scopes(
    requirements: &[ironclaw_host_api::decision::RuntimeCredentialAuthRequirement],
) -> Vec<ProviderScope> {
    let mut scopes: Vec<ProviderScope> = Vec::new();
    for requirement in requirements {
        for scope in &requirement.provider_scopes {
            let scope = ProviderScope::new(scope.clone()).expect("manifest scope is valid");
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
    }
    scopes
}

/// A real `AuthEngine` whose recipes come from the installed-manifest
/// resolver — the production wiring in
/// `ironclaw_composition::factory::auth_engine_assembly`.
fn engine_over_resolver(
    recipes: Arc<dyn ironclaw_auth::AuthRecipeResolver>,
    egress: Arc<ScriptedOAuthTokenEgress>,
) -> ironclaw_auth::AuthEngine {
    ironclaw_auth::AuthEngine::new(ironclaw_auth::AuthEngineDeps {
        recipes,
        client_credentials: Arc::new(GoogleTestClientCredentials),
        egress,
        secret_store: Arc::new(ironclaw_secrets::SecretStore::ephemeral()),
        callback_base: ironclaw_auth::EngineCallbackBase::new(
            "https://host.example/api/reborn/product-auth/oauth",
        )
        .expect("callback base"),
        dcr_client_name: "IronClaw integration test".to_string(),
    })
}

#[derive(Debug)]
struct GoogleTestClientCredentials;

#[async_trait::async_trait]
impl ironclaw_auth::EngineClientCredentialsSource for GoogleTestClientCredentials {
    async fn resolve(
        &self,
        _vendor: &str,
        _credentials: &ironclaw_extension_contracts::recipe::RecipeClientCredentials,
    ) -> Result<ironclaw_auth::EngineOAuthClientMaterial, ironclaw_auth::AuthProductError> {
        Ok(ironclaw_auth::EngineOAuthClientMaterial {
            client_id: ironclaw_auth::OAuthClientId::new("itest-google-client-id")?,
            client_secret: Some(secrecy::SecretString::from(
                "itest-google-client-secret".to_string(),
            )),
        })
    }
}

fn google_callback_request(granted: Vec<String>) -> OAuthProviderCallbackRequest {
    OAuthProviderCallbackRequest {
        provider: AuthProviderId::new("google").unwrap(),
        account_label: CredentialAccountLabel::new("google").unwrap(),
        authorization_code: OAuthAuthorizationCode::new(secrecy::SecretString::from(
            "itest-google-auth-code".to_string(),
        ))
        .unwrap(),
        authorization_code_hash: AuthorizationCodeHash::new(hex64(0x11)).unwrap(),
        pkce_verifier: PkceVerifierSecret::new(secrecy::SecretString::from(
            "itest-google-pkce-verifier".to_string(),
        ))
        .unwrap(),
        pkce_verifier_hash: PkceVerifierHash::new(hex64(0x22)).unwrap(),
        scopes: granted
            .into_iter()
            .map(|scope| ProviderScope::new(scope).expect("granted scope is valid"))
            .collect(),
    }
}

struct NoopContinuationDispatcher;

#[async_trait::async_trait]
impl ironclaw_auth::RebornAuthContinuationDispatcher for NoopContinuationDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        _event: ironclaw_auth::AuthContinuationEvent,
    ) -> Result<(), ironclaw_auth::AuthProductError> {
        Ok(())
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: ironclaw_auth::AuthContinuationEvent,
    ) -> Result<(), ironclaw_auth::AuthProductError> {
        Ok(())
    }
}
