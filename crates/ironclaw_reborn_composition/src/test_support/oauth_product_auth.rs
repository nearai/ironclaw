//! OAuth / product-auth test bundles (Slices 7-8).
//!
//! [`ScriptedOAuthTokenEgress`], [`OAuthProductAuthTestBundle`],
//! `build_oauth_product_auth_for_test`, `build_google_oauth_product_auth_for_test`
//! — real store / real client / scripted HTTP egress for OAuth connect,
//! refresh, and error-path tests.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

// ─── Slice 7: OAuth connect-flow test support ─────────────────────────────────
//
// Constructs a real `FilesystemAuthProductServices<InMemoryBackend>` + a real
// `HostOAuthProviderClient` wired to a scripted token-exchange egress. The
// resulting `RebornProductAuthServices` bundle exercises the full OAuth
// claim→exchange→complete→credential-account path with no network.

/// Scripted [`ironclaw_host_api::RuntimeHttpEgress`] for OAuth token-exchange
/// tests.
///
/// Returns a configurable HTTP status and JSON body on every call, records
/// every request so the test can assert the exchange happened, and ignores the
/// URL so the `HostOAuthProviderClient`'s HTTPS guard can use a fake-but-valid
/// URL.
///
/// The default `(status, body)` is used for every call unless a per-call
/// override has been queued with [`push_response`].
///
/// **Sequential-use assumption.** Callers drive this egress from a single test
/// thread. The internal `Mutex` guards against accidental concurrent access but
/// is not intended to support concurrent callers — FIFO ordering of
/// `push_response` / `captured_count` / `captured_grant_types` is meaningful
/// only under sequential use.
///
/// [`push_response`]: ScriptedOAuthTokenEgress::push_response
pub struct ScriptedOAuthTokenEgress {
    /// HTTP status code returned by the default response.  Success constructors
    /// set this to `200`; `with_error_response` sets it to the supplied code.
    status: u16,
    body: Vec<u8>,
    captured: Arc<Mutex<Vec<ironclaw_host_api::RuntimeHttpEgressRequest>>>,
    /// Pre-scripted sequential response overrides consumed FIFO on each
    /// `execute()` call.  While the queue is non-empty the front entry is
    /// popped and used instead of `(status, body)`.  Use `push_response` to
    /// stage per-call overrides after construction.
    response_queue: ScriptedResponseQueue,
}

/// FIFO queue of per-call `(status, body)` overrides for [`ScriptedOAuthTokenEgress`].
type ScriptedResponseQueue = Arc<Mutex<std::collections::VecDeque<(u16, Vec<u8>)>>>;

impl ScriptedOAuthTokenEgress {
    fn build(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            captured: Arc::new(Mutex::new(Vec::new())),
            response_queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
        }
    }

    /// Build a scripted egress that returns `200` with a minimal
    /// `{access_token, token_type, expires_in}` body.
    pub fn with_access_token(access_token: &str) -> Self {
        let body = serde_json::json!({
            "access_token": access_token,
            "token_type": "Bearer",
            "expires_in": 3600
        })
        .to_string()
        .into_bytes();
        Self::build(200, body)
    }

    /// Build a scripted egress that returns `200` with
    /// `{access_token, refresh_token, token_type, expires_in}`.
    ///
    /// Use this for Google OAuth tests where the initial token exchange must
    /// store a refresh secret handle so the keepalive worker can later load and
    /// use it.
    pub fn with_access_and_refresh_token(access_token: &str, refresh_token: &str) -> Self {
        let body = serde_json::json!({
            "access_token": access_token,
            "refresh_token": refresh_token,
            "token_type": "Bearer",
            "expires_in": 3600
        })
        .to_string()
        .into_bytes();
        Self::build(200, body)
    }

    /// Build a scripted egress that returns `200` with an arbitrary JSON
    /// body — for token responses carrying vendor identity claims that the
    /// recipe's `identity` pointers extract.
    pub fn with_json_body(body: &serde_json::Value) -> Self {
        Self::build(200, body.to_string().into_bytes())
    }

    /// Build a scripted egress that returns `status` with a minimal
    /// `{"error":"<error_code>"}` body — for example, `(400, "invalid_grant")`
    /// to simulate an OAuth provider permanently revoking a refresh token.
    ///
    /// Every call returns this error response until a per-call override is
    /// pushed via [`push_response`].  To interleave a success response followed
    /// by an error (e.g. a valid connect exchange then a rejected sweep), use a
    /// success constructor and queue the error with `push_response`:
    ///
    /// ```ignore
    /// let bundle = build_google_oauth_product_auth_for_test(); // default: 200
    /// connect_google_account(&bundle, &scope, 0xcc).await;     // call 1 → 200
    /// bundle.egress.push_response(400, b"{\"error\":\"invalid_grant\"}".to_vec());
    /// bundle.sweep_for_refresh(candidates, settings, now).await; // call 2 → 400
    /// ```
    ///
    /// [`push_response`]: ScriptedOAuthTokenEgress::push_response
    pub fn with_error_response(status: u16, error_code: &str) -> Self {
        let body = serde_json::json!({"error": error_code})
            .to_string()
            .into_bytes();
        Self::build(status, body)
    }

    /// Stage a one-shot response override to be consumed on the next
    /// `execute()` call before the default `(status, body)` is used.
    ///
    /// Overrides are consumed in FIFO order; each `push_response` call adds
    /// one entry that covers exactly one future `execute()` call.
    pub fn push_response(&self, status: u16, body: Vec<u8>) {
        self.response_queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back((status, body));
    }

    /// Number of token-exchange HTTP calls captured so far.
    pub fn captured_count(&self) -> usize {
        self.captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// The form parameter NAMES of every captured token-exchange request, in
    /// order. Names only — values may carry authorization codes, PKCE
    /// verifiers, or client secrets and are never exposed. Tests use this to
    /// pin which protocol parameters crossed the egress.
    pub fn captured_form_param_names(&self) -> Vec<Vec<String>> {
        self.captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|request| {
                url::form_urlencoded::parse(&request.body)
                    .map(|(name, _)| name.into_owned())
                    .collect()
            })
            .collect()
    }

    /// The OAuth `grant_type` of every captured token-exchange request, in order.
    ///
    /// Deliberately returns ONLY the non-secret `grant_type` discriminator —
    /// NOT the raw request body, which carries the authorization code / refresh
    /// token / client credentials. Tests use this to distinguish the
    /// `authorization_code` connect exchange from the `refresh_token` exchange
    /// without exposing secrets in assertion output.
    pub fn captured_grant_types(&self) -> Vec<String> {
        self.captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|request| parse_grant_type(&request.body))
            .collect()
    }
}

/// Extract the `grant_type` value from an `application/x-www-form-urlencoded`
/// token-exchange body. Returns `"<unknown>"` if the field is absent or the
/// body is not valid UTF-8.
///
/// OAuth `grant_type` values (`authorization_code`, `refresh_token`) are ASCII
/// alphanumeric + underscore and are never percent-encoded; no decoder is
/// needed for these specific values.
///
/// # Security
///
/// This helper is intentionally narrow: it returns ONLY the grant_type string
/// and never echoes authorization codes, refresh tokens, client secrets, or
/// any other field from the body. Call sites must not widen this to expose
/// additional body fields.
fn parse_grant_type(body: &[u8]) -> String {
    let text = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return "<unknown>".to_string(),
    };
    for pair in text.split('&') {
        if let Some(value) = pair.strip_prefix("grant_type=") {
            // grant_type values are ASCII alphanumeric + underscore; they are
            // not percent-encoded and contain no secret material.
            return value.to_string();
        }
    }
    "<unknown>".to_string()
}

impl std::fmt::Debug for ScriptedOAuthTokenEgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedOAuthTokenEgress")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .finish()
    }
}

#[async_trait]
impl ironclaw_host_api::RuntimeHttpEgress for ScriptedOAuthTokenEgress {
    async fn execute(
        &self,
        request: ironclaw_host_api::RuntimeHttpEgressRequest,
    ) -> Result<
        ironclaw_host_api::RuntimeHttpEgressResponse,
        ironclaw_host_api::RuntimeHttpEgressError,
    > {
        let request_bytes = request.body.len() as u64;
        self.captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        // Pop a per-call override if one was staged; fall back to the default
        // (status, body) set by the constructor.
        let (status, body) = {
            let mut queue = self
                .response_queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            queue
                .pop_front()
                .unwrap_or_else(|| (self.status, self.body.clone()))
        };
        let response_bytes = body.len() as u64;
        Ok(ironclaw_host_api::RuntimeHttpEgressResponse {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body,
            saved_body: None,
            request_bytes,
            response_bytes,
            redaction_applied: false,
        })
    }
}

/// Build a recipe-driven [`ironclaw_auth::AuthEngine`] over the scripted
/// egress for one synthetic test vendor.
fn engine_provider_client_for_test(
    vendor: &str,
    scopes: &[&str],
    token_endpoint: &str,
    egress: Arc<ScriptedOAuthTokenEgress>,
    secret_store: Arc<dyn ironclaw_secrets::SecretStorePort>,
) -> Arc<ironclaw_auth::AuthEngine> {
    let recipe: ironclaw_host_api::VendorAuthRecipe = serde_json::from_value(serde_json::json!({
        "method": "oauth2_code",
        "display_name": format!("{vendor} account"),
        "authorization_endpoint": "https://oauth.test.example.com/authorize",
        "token_endpoint": token_endpoint,
        "scopes": scopes,
        "client_credentials": { "client_id_handle": format!("{vendor}_oauth_client_id") },
        "token_response": {
            "access_token": "/access_token",
            "refresh_token": "/refresh_token",
            "expires_in": "/expires_in",
            "scope": { "path": "/scope", "missing": "fallback_to_requested" }
        },
        // Test vendors declare a 7-day idle lifetime so sweep tests exercise
        // the engine keepalive path (accounts become due at half-life).
        "refresh": { "keepalive_idle_seconds": 604_800 },
    }))
    .expect("test vendor recipe parses");
    Arc::new(ironclaw_auth::AuthEngine::new(
        ironclaw_auth::AuthEngineDeps {
            recipes: Arc::new(ironclaw_auth::StaticAuthRecipeResolver::new(vec![
                ironclaw_auth::ResolvedVendorAuthRecipe {
                    vendor: vendor.to_string(),
                    recipe,
                    token_exchange_resource: None,
                },
            ])),
            client_credentials: Arc::new(TestStaticClientCredentials),
            egress: egress as Arc<dyn ironclaw_host_api::RuntimeHttpEgress>,
            secret_store,
            callback_base: ironclaw_auth::EngineCallbackBase::new(
                "https://localhost/api/reborn/product-auth/oauth",
            )
            .expect("test callback base"),
            dcr_client_name: "Ironclaw test".to_string(),
        },
    ))
}

#[derive(Debug)]
struct TestStaticClientCredentials;

#[async_trait]
impl ironclaw_auth::EngineClientCredentialsSource for TestStaticClientCredentials {
    async fn resolve(
        &self,
        _vendor: &str,
        _credentials: &ironclaw_host_api::RecipeClientCredentials,
    ) -> Result<ironclaw_auth::EngineOAuthClientMaterial, ironclaw_auth::AuthProductError> {
        Ok(ironclaw_auth::EngineOAuthClientMaterial {
            client_id: ironclaw_auth::OAuthClientId::new("test-client-id")?,
            client_secret: None,
        })
    }
}

/// Noop continuation dispatcher: swallows every auth-continuation event.
#[derive(Debug)]
struct TestNoopContinuationDispatcher;

#[async_trait]
impl crate::product_auth::api::auth::RebornAuthContinuationDispatcher
    for TestNoopContinuationDispatcher
{
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

/// Bundle returned by [`build_oauth_product_auth_for_test`].
///
/// The `services` arc exposes `flow_manager()` and `credential_account_service()`
/// for creating flows and reading back persisted accounts.  The `egress` arc
/// lets the test assert how many token-exchange calls were made.
pub struct OAuthProductAuthTestBundle {
    /// Fully-wired product-auth services (real stores, scripted token egress).
    pub services: Arc<crate::RebornProductAuthServices>,
    /// Scripted egress — inspect after `handle_oauth_callback` to verify
    /// the token-exchange HTTP call happened.
    pub egress: Arc<ScriptedOAuthTokenEgress>,
    /// The engine's recipe data (synthetic test vendors declare a 7-day
    /// keepalive lifetime); `sweep_for_refresh` resolves idle thresholds
    /// through this, exactly like the production sweep.
    pub keepalive_recipes: Arc<dyn ironclaw_auth::AuthRecipeResolver>,
}

/// Shared infrastructure preamble for OAuth product-auth test bundles.
///
/// Shared in-memory product-auth infra (named to keep the helper's return type
/// out of clippy's `type_complexity` lint — a 3-tuple of nested `Arc`s trips it).
struct OAuthProductAuthInfra {
    secret_store: Arc<dyn ironclaw_secrets::SecretStorePort>,
    durable: Arc<
        crate::product_auth::durable::FilesystemAuthProductServices<
            ironclaw_filesystem::InMemoryBackend,
        >,
    >,
}

/// Builds the fixed-view in-memory secrets filesystem, the secret store, and
/// the durable `FilesystemAuthProductServices`. The two callers
/// (`build_oauth_product_auth_for_test` and
/// `build_google_oauth_product_auth_for_test`) differ only in the egress
/// constructor (the scripted token-response body) and the optional
/// `.with_provider_client()` call.
fn build_oauth_product_auth_infra() -> OAuthProductAuthInfra {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
    use ironclaw_secrets::SecretStore;

    // Fixed-view scoped filesystem: the product-auth durable layer writes
    // flow/account JSON under /secrets/agents/…/product-auth/… so we only
    // need the /secrets mount to be writable.
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/secrets").unwrap(),
        VirtualPath::new("/tenants/test-tenant/users/test-user/secrets").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    let backend = Arc::new(InMemoryBackend::new());
    let scoped_fs: Arc<ScopedFilesystem<InMemoryBackend>> =
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts));
    let secret_store: Arc<dyn ironclaw_secrets::SecretStorePort> =
        Arc::new(SecretStore::ephemeral());
    // Real durable product-auth services over the in-memory scoped filesystem.
    let durable = Arc::new(
        crate::product_auth::durable::FilesystemAuthProductServices::new(
            Arc::clone(&scoped_fs),
            Arc::clone(&secret_store),
        ),
    );
    // `scoped_fs` is intentionally not returned: `durable` holds its own
    // `Arc` clone above, and no caller needs the filesystem handle directly.
    OAuthProductAuthInfra {
        secret_store,
        durable,
    }
}

/// Construct a self-contained [`OAuthProductAuthTestBundle`] for OAuth
/// connect-flow tests.
///
/// Uses:
/// - `InMemoryBackend` with a fixed `MountView` scoped to
///   `/tenants/test-tenant/users/test-user/secrets` (no `libsql`/`postgres`
///   feature dependency).
/// - `SecretStore::ephemeral()` for access/refresh token handles.
/// - `ScriptedOAuthTokenEgress` intercepting the provider token endpoint.
/// - Real `FilesystemAuthProductServices<InMemoryBackend>` for flow + account
///   persistence — zero mocks on the storage layer.
/// - Noop continuation dispatcher and noop obligation handler.
///
/// Calling this multiple times produces independent, isolated bundles.
pub fn build_oauth_product_auth_for_test() -> OAuthProductAuthTestBundle {
    let OAuthProductAuthInfra {
        secret_store,
        durable,
    } = build_oauth_product_auth_infra();

    // Scripted egress: returns a valid access-token JSON body, records calls.
    let egress = Arc::new(ScriptedOAuthTokenEgress::with_access_token(
        "test-access-token-abc123",
    ));

    // The recipe-driven auth engine wired to the scripted egress.
    // The token endpoint must be HTTPS to pass the engine's endpoint guard;
    // ScriptedOAuthTokenEgress ignores the actual URL.
    let engine = engine_provider_client_for_test(
        "test-oauth-provider",
        &["test.readonly"],
        "https://oauth.test.example.com/token",
        Arc::clone(&egress),
        Arc::clone(&secret_store),
    );

    let services = Arc::new(crate::RebornProductAuthServices::new(
        durable.clone() as Arc<dyn ironclaw_auth::AuthFlowManager>,
        durable.clone() as Arc<dyn ironclaw_auth::AuthInteractionService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialSetupService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialAccountService>,
        Arc::clone(&engine) as Arc<dyn ironclaw_auth::AuthProviderClient>,
        durable as Arc<dyn ironclaw_auth::SecretCleanupService>,
        Arc::new(TestNoopContinuationDispatcher),
    ));

    let keepalive_recipes = Arc::clone(engine.recipes());
    OAuthProductAuthTestBundle {
        services,
        egress,
        keepalive_recipes,
    }
}

/// Construct the same engine-backed bundle as [`build_oauth_product_auth_for_test`]
/// with the durable flow/account store persisted on a real libSQL-backed root
/// filesystem instead of the in-memory backend — the second persistence leg
/// for the auth engine (checklist AUTH-15).
pub async fn build_oauth_product_auth_for_test_on_libsql(
    db_path: &std::path::Path,
) -> OAuthProductAuthTestBundle {
    use ironclaw_filesystem::{LibSqlRootFilesystem, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
    use ironclaw_secrets::SecretStore;

    let db = Arc::new(
        libsql::Builder::new_local(db_path.display().to_string())
            .build()
            .await
            .expect("build libsql database for oauth bundle"),
    );
    let root = Arc::new(LibSqlRootFilesystem::new(db));
    root.run_migrations()
        .await
        .expect("libsql filesystem migrations");
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/secrets").expect("mount alias"),
        VirtualPath::new("/tenants/test-tenant/users/test-user/secrets").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped_fs: Arc<ScopedFilesystem<LibSqlRootFilesystem>> =
        Arc::new(ScopedFilesystem::with_fixed_view(root, mounts));
    let secret_store: Arc<dyn ironclaw_secrets::SecretStorePort> =
        Arc::new(SecretStore::ephemeral());
    let durable = Arc::new(
        crate::product_auth::durable::FilesystemAuthProductServices::new(
            Arc::clone(&scoped_fs),
            Arc::clone(&secret_store),
        ),
    );
    let egress = Arc::new(ScriptedOAuthTokenEgress::with_access_token(
        "test-access-token-abc123",
    ));
    let engine = engine_provider_client_for_test(
        "test-oauth-provider",
        &["test.readonly"],
        "https://oauth.test.example.com/token",
        Arc::clone(&egress),
        Arc::clone(&secret_store),
    );
    let services = Arc::new(crate::RebornProductAuthServices::new(
        durable.clone() as Arc<dyn ironclaw_auth::AuthFlowManager>,
        durable.clone() as Arc<dyn ironclaw_auth::AuthInteractionService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialSetupService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialAccountService>,
        Arc::clone(&engine) as Arc<dyn ironclaw_auth::AuthProviderClient>,
        durable as Arc<dyn ironclaw_auth::SecretCleanupService>,
        Arc::new(TestNoopContinuationDispatcher),
    ));
    let keepalive_recipes = Arc::clone(engine.recipes());
    OAuthProductAuthTestBundle {
        services,
        egress,
        keepalive_recipes,
    }
}

/// The same engine-backed bundle as [`build_oauth_product_auth_for_test`] with
/// the durable flow/account store persisted on a caller-supplied real root
/// filesystem — the both-DB persistence leg for the auth engine (checklist
/// AUTH-15; REL-3: a Postgres skip is a failure). Generic over the backend so
/// composition test-support does not need a concrete-backend feature enabled
/// through the `ironclaw` dependency: the caller (which already links
/// `ironclaw_filesystem/postgres`) builds and migrates the root filesystem and
/// passes it here. There is no root-generic bundle builder otherwise (the
/// shared `build_oauth_product_auth_infra` is `InMemoryBackend`-hardcoded), and
/// the OAuth product-auth bundle is built outside the harness's storage
/// composite, so the harness's `StorageMode::Postgres` cannot construct it
/// (correction A: this is the sanctioned thin composition-tier addition).
pub async fn build_oauth_product_auth_for_test_on_root<F>(
    root: Arc<F>,
) -> OAuthProductAuthTestBundle
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    use ironclaw_filesystem::ScopedFilesystem;
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
    use ironclaw_secrets::SecretStore;

    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/secrets").expect("mount alias"),
        VirtualPath::new("/tenants/test-tenant/users/test-user/secrets").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped_fs: Arc<ScopedFilesystem<F>> =
        Arc::new(ScopedFilesystem::with_fixed_view(root, mounts));
    let secret_store: Arc<dyn ironclaw_secrets::SecretStorePort> =
        Arc::new(SecretStore::ephemeral());
    let durable = Arc::new(
        crate::product_auth::durable::FilesystemAuthProductServices::new(
            Arc::clone(&scoped_fs),
            Arc::clone(&secret_store),
        ),
    );
    let egress = Arc::new(ScriptedOAuthTokenEgress::with_access_token(
        "test-access-token-abc123",
    ));
    let engine = engine_provider_client_for_test(
        "test-oauth-provider",
        &["test.readonly"],
        "https://oauth.test.example.com/token",
        Arc::clone(&egress),
        Arc::clone(&secret_store),
    );
    let services = Arc::new(crate::RebornProductAuthServices::new(
        durable.clone() as Arc<dyn ironclaw_auth::AuthFlowManager>,
        durable.clone() as Arc<dyn ironclaw_auth::AuthInteractionService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialSetupService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialAccountService>,
        Arc::clone(&engine) as Arc<dyn ironclaw_auth::AuthProviderClient>,
        durable as Arc<dyn ironclaw_auth::SecretCleanupService>,
        Arc::new(TestNoopContinuationDispatcher),
    ));
    let keepalive_recipes = Arc::clone(engine.recipes());
    OAuthProductAuthTestBundle {
        services,
        egress,
        keepalive_recipes,
    }
}

// ─── Slice 8: OAuth credential-refresh sweep test support ────────────────────
//
// `FixedCandidateSource` and `OAuthProductAuthTestBundle::sweep_for_refresh`
// together let a test drive the engine-owned `keepalive::sweep_once` with:
//   • a pre-seeded list of accounts (bypasses the filesystem walk)
//   • a frozen `now` instant (controls the idle-cutoff comparison)
//   • the real `ProviderBackedCredentialAccountService` refresh path
//   • the scripted `ScriptedOAuthTokenEgress` for HTTP assertion
//
// `build_google_oauth_product_auth_for_test` wires the same fixture chain as
// `build_oauth_product_auth_for_test` but for `provider_id = "google"`, includes
// a `refresh_token` in the scripted response so the exchange stores a refresh
// secret handle, and calls `.with_provider_client()` so `refresh_account` routes
// through `ProviderBackedCredentialAccountService` instead of returning
// `BackendUnavailable`.

/// Fixed candidate source for credential-keepalive sweep tests (slice 8).
///
/// Returns a caller-supplied list of accounts from
/// `list_keepalive_candidates`, bypassing the `FilesystemAuthProductServices`
/// filesystem walk. This lets a test inject a real `CredentialAccount` (read
/// back after an OAuth connect flow) directly into the engine's `sweep_once`
/// without needing the full tenant-path enumeration to work in an in-memory
/// backend.
///
// TODO(follow-up): add a LibSql-backed sweep test that drives the real
// durable candidate enumeration. `FixedCandidateSource` bypasses the
// tenant-path filesystem walk because this bundle's fixed view mounts only
// `/secrets` (no tenant tree to enumerate). The refresh path itself
// (`sweep_once` -> `refresh_account` -> provider client -> egress -> status
// write-back) is already covered here at full fidelity; only candidate
// enumeration is stubbed.
struct FixedCandidateSource {
    candidates: Vec<ironclaw_auth::CredentialAccount>,
}

#[async_trait]
impl ironclaw_auth::KeepaliveCandidateSource for FixedCandidateSource {
    async fn list_keepalive_candidates(&self) -> Vec<ironclaw_auth::CredentialAccount> {
        self.candidates.clone()
    }
}

impl OAuthProductAuthTestBundle {
    /// Run one credential-keepalive sweep tick with a fixed account list and
    /// a frozen clock.
    ///
    /// This exercises the engine-owned `sweep_once` path — recipe-threshold
    /// gating (the test vendor declares a 7-day keepalive lifetime; accounts
    /// become due at half-life), due selection + cap,
    /// `CredentialRefreshRequest` construction,
    /// `RebornProductAuthServices::refresh_credential_account` →
    /// `ProviderBackedCredentialAccountService::refresh_account` → engine
    /// `refresh_token` → scripted HTTP egress — without needing a real
    /// filesystem walk or a Postgres leader lock.
    ///
    /// # Arguments
    ///
    /// * `candidates` — `CredentialAccount` records to feed into the sweep.
    ///   Obtain these by calling `services.credential_account_service().get_account()`
    ///   after a successful OAuth connect flow so the handles are real.
    /// * `settings` — pass `KeepaliveSweepSettings::enabled()` (cap of 5).
    /// * `now` — frozen instant. Pass `Utc::now() + Duration::days(4)` to put
    ///   a just-created account past the test vendor's 3.5-day half-life;
    ///   pass `Utc::now()` (or anything under the half-life) to verify no
    ///   refresh is triggered.
    pub async fn sweep_for_refresh(
        &self,
        candidates: Vec<ironclaw_auth::CredentialAccount>,
        settings: ironclaw_auth::KeepaliveSweepSettings,
        now: chrono::DateTime<chrono::Utc>,
    ) {
        use ironclaw_auth::keepalive::sweep_once;
        use tokio_util::sync::CancellationToken;

        let deps = ironclaw_auth::KeepaliveSweepDeps {
            candidates: std::sync::Arc::new(FixedCandidateSource { candidates }),
            recipes: std::sync::Arc::clone(&self.keepalive_recipes),
            refresh: std::sync::Arc::clone(&self.services) as _,
            leader_lock: std::sync::Arc::new(ironclaw_auth::AlwaysLeaderKeepaliveLock),
        };
        let cancel = CancellationToken::new();
        sweep_once(&deps, &settings, &cancel, now).await;
    }
}

/// Construct a `OAuthProductAuthTestBundle` wired for the Google OAuth provider.
///
/// Unlike `build_oauth_product_auth_for_test`, this variant:
/// - Uses `provider_id = "google"` (required by
///   `HostOAuthProviderClient::refresh_token`, which rejects provider mismatches).
/// - Includes `refresh_token` in the scripted egress response so the initial
///   token exchange stores a refresh secret handle (required for the keepalive
///   refresh sweep to call the token endpoint).
/// - Calls `.with_provider_client()` on the constructed `RebornProductAuthServices`
///   so `refresh_credential_account` routes through
///   `ProviderBackedCredentialAccountService` rather than returning
///   `BackendUnavailable`.
///
/// Calling this multiple times produces independent, isolated bundles.
pub fn build_google_oauth_product_auth_for_test() -> OAuthProductAuthTestBundle {
    let OAuthProductAuthInfra {
        secret_store,
        durable,
    } = build_oauth_product_auth_infra();

    // Include a refresh_token in the scripted response so the token exchange
    // stores a refresh secret handle (needed for the keepalive sweep to call
    // the token endpoint on the next egress request).
    let egress = Arc::new(ScriptedOAuthTokenEgress::with_access_and_refresh_token(
        "test-google-access-token",
        "test-google-refresh-token",
    ));

    // The vendor id must be "google" so the engine resolves the same recipe
    // the refresh path requests; the recipe is synthetic test data.
    let engine = engine_provider_client_for_test(
        "google",
        &["email"],
        "https://oauth2.googleapis.com/token",
        Arc::clone(&egress),
        Arc::clone(&secret_store),
    );
    let keepalive_recipes = Arc::clone(engine.recipes());
    let provider_client: Arc<dyn ironclaw_auth::AuthProviderClient> = engine;

    // Build services then wrap credential_account_service with
    // ProviderBackedCredentialAccountService via with_provider_client() so
    // refresh_credential_account routes through the real refresh path instead
    // of returning BackendUnavailable.
    let services = Arc::new(
        crate::RebornProductAuthServices::new(
            durable.clone() as Arc<dyn ironclaw_auth::AuthFlowManager>,
            durable.clone() as Arc<dyn ironclaw_auth::AuthInteractionService>,
            durable.clone() as Arc<dyn ironclaw_auth::CredentialSetupService>,
            durable.clone() as Arc<dyn ironclaw_auth::CredentialAccountService>,
            Arc::clone(&provider_client),
            durable as Arc<dyn ironclaw_auth::SecretCleanupService>,
            Arc::new(TestNoopContinuationDispatcher),
        )
        .with_provider_client(provider_client),
    );

    OAuthProductAuthTestBundle {
        services,
        egress,
        keepalive_recipes,
    }
}

// ─── Generic channel-identity binding test support (extension-runtime P6) ────

/// Build the same recipe-driven engine as [`build_oauth_product_auth_for_test`]
/// with an `[auth.*.identity]` extraction section (subject/team/app pointers
/// over the token response body).
fn engine_provider_client_with_identity_for_test(
    vendor: &str,
    scopes: &[&str],
    egress: Arc<ScriptedOAuthTokenEgress>,
    secret_store: Arc<dyn ironclaw_secrets::SecretStorePort>,
) -> Arc<ironclaw_auth::AuthEngine> {
    let recipe: ironclaw_host_api::VendorAuthRecipe = serde_json::from_value(serde_json::json!({
        "method": "oauth2_code",
        "display_name": format!("{vendor} account"),
        "authorization_endpoint": "https://oauth.test.example.com/authorize",
        "token_endpoint": "https://oauth.test.example.com/token",
        "scopes": scopes,
        "client_credentials": { "client_id_handle": format!("{vendor}_oauth_client_id") },
        "token_response": {
            "access_token": "/access_token",
            "scope": { "path": "/scope", "missing": "fallback_to_requested" }
        },
        "identity": {
            "account_id": "/authed_user/id",
            "team_id": "/team/id",
            "app_id": "/app_id"
        },
    }))
    .expect("identity test vendor recipe parses");
    Arc::new(ironclaw_auth::AuthEngine::new(
        ironclaw_auth::AuthEngineDeps {
            recipes: Arc::new(ironclaw_auth::StaticAuthRecipeResolver::new(vec![
                ironclaw_auth::ResolvedVendorAuthRecipe {
                    vendor: vendor.to_string(),
                    recipe,
                    token_exchange_resource: None,
                },
            ])),
            client_credentials: Arc::new(TestStaticClientCredentials),
            egress: egress as Arc<dyn ironclaw_host_api::RuntimeHttpEgress>,
            secret_store,
            callback_base: ironclaw_auth::EngineCallbackBase::new(
                "https://localhost/api/reborn/product-auth/oauth",
            )
            .expect("test callback base"),
            dcr_client_name: "Ironclaw test".to_string(),
        },
    ))
}

/// [`build_oauth_product_auth_for_test`] with a vendor recipe that declares
/// identity extraction and a scripted token body carrying the claims —
/// the fixture chain for tests that drive the generic post-OAuth channel
/// identity binding.
pub fn build_oauth_product_auth_with_identity_for_test(
    vendor: &str,
    token_body: &serde_json::Value,
) -> OAuthProductAuthTestBundle {
    let OAuthProductAuthInfra {
        secret_store,
        durable,
    } = build_oauth_product_auth_infra();
    let egress = Arc::new(ScriptedOAuthTokenEgress::with_json_body(token_body));
    let engine = engine_provider_client_with_identity_for_test(
        vendor,
        &["test.readonly"],
        Arc::clone(&egress),
        Arc::clone(&secret_store),
    );
    let services = Arc::new(crate::RebornProductAuthServices::new(
        durable.clone() as Arc<dyn ironclaw_auth::AuthFlowManager>,
        durable.clone() as Arc<dyn ironclaw_auth::AuthInteractionService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialSetupService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialAccountService>,
        Arc::clone(&engine) as Arc<dyn ironclaw_auth::AuthProviderClient>,
        durable as Arc<dyn ironclaw_auth::SecretCleanupService>,
        Arc::new(TestNoopContinuationDispatcher),
    ));
    let keepalive_recipes = Arc::clone(engine.recipes());
    OAuthProductAuthTestBundle {
        services,
        egress,
        keepalive_recipes,
    }
}

/// Drive `handle_oauth_callback` WITH the generic channel-identity binding
/// hook — the exact post-exchange seam the production product-auth route
/// installs (`WebuiServeConfig::with_channel_identity_binding`) — so
/// integration tests can prove a channel extension's OAuth connect writes
/// an identity binding through the generic hook.
pub async fn handle_oauth_callback_with_channel_identity_binding_for_test(
    services: &crate::RebornProductAuthServices,
    request: crate::RebornOAuthCallbackRequest,
    binding: &crate::ChannelIdentityBindingConfig,
) -> Result<crate::RebornOAuthCallbackResponse, crate::RebornOAuthCallbackError> {
    let provider = match &request.outcome {
        crate::RebornOAuthCallbackOutcome::Authorized { provider_request } => {
            provider_request.provider.as_str().to_string()
        }
        _ => String::new(),
    };
    let factory = crate::extension_host::channel_identity::channel_identity_binding_hook_factory(
        binding.clone(),
    );
    let check = factory(&provider, &request.scope);
    services
        .handle_oauth_callback_with_optional_provider_identity_check(request, check)
        .await
}
