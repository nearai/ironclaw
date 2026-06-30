//! Test-only helpers for the Reborn integration-test framework and budget E2E tests.
//!
//! Gated behind the `test-support` feature so production builds never pay the cost
//! of the mock gateway / introspection accessors. The module covers three areas:
//!
//! 1. **Budget / mock-gateway helpers** — [`BudgetTestGateway`], [`FailingTestGateway`],
//!    [`ScriptedReply`] — scripted model responses with configurable token counts for
//!    `RebornRuntimeInput::with_model_gateway_override` tests.
//! 2. **OAuth / product-auth test bundles** — [`ScriptedOAuthTokenEgress`],
//!    [`OAuthProductAuthTestBundle`], `build_oauth_product_auth_for_test`,
//!    `build_google_oauth_product_auth_for_test` — real store / real client / scripted
//!    HTTP egress for OAuth connect, refresh, and error-path tests.
//! 3. **Reborn integration-test framework accessors** — `build_local_dev_approval_gate_evidence_for_test`,
//!    `build_default_local_dev_database_roots_for_test`, `mount_local_dev_database_roots_for_test`,
//!    `build_local_dev_secret_store_for_test` — mirror the production local-dev boot
//!    sequence so the integration-test harness (`tests/support/reborn/`) drives the
//!    real local-dev composition paths without duplicating the wiring logic.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::ThreadId;
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_turns::{
    TurnRunId, TurnStatus,
    run_profile::{LoopCapabilityPort, LoopModelUsage},
};

/// Filename of the local-dev libSQL database within the per-user root
/// directory. Value is derived from the production factory constant so
/// there is one owner of the string; tests access it through this
/// test-support surface.
pub const LOCAL_DEV_DB_FILENAME: &str = crate::factory::LOCAL_DEV_DB_FILENAME;

use crate::runtime::{AssistantReply, ConversationId};

/// Build a terminal/no-text assistant reply for CLI and product-surface tests.
///
/// Kept behind `test-support` so downstream crates can exercise presentation
/// paths without depending directly on lower-level turn/thread crates.
pub fn assistant_reply_without_text_for_test(
    status: TurnStatus,
    failure_category: Option<&str>,
) -> AssistantReply {
    AssistantReply {
        conversation: ConversationId(
            ThreadId::new("test-assistant-reply").expect("static test thread id"), // safety: static test helper id is a valid thread id literal.
        ),
        run_id: TurnRunId::new(),
        status,
        failure_category: failure_category.map(str::to_owned),
        text: None,
    }
}

/// One scripted reply from the mock LLM.
///
/// `usage` is forwarded into [`HostManagedModelResponse::usage`] so the
/// budget accountant reconciles against real provider numbers, not the
/// reservation estimate.
#[derive(Debug, Clone)]
pub struct ScriptedReply {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl ScriptedReply {
    pub fn new(text: impl Into<String>, input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            text: text.into(),
            input_tokens,
            output_tokens,
        }
    }

    fn into_response(self) -> HostManagedModelResponse {
        HostManagedModelResponse::assistant_reply(self.text).with_usage(LoopModelUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }
}

/// Mock [`HostManagedModelGateway`] that returns scripted assistant
/// replies with configurable token usage.
///
/// Replies are consumed in FIFO order. When the script runs out the
/// gateway falls back to a sentinel reply with zero tokens — tests that
/// drive multiple turns should pre-load the matching number of
/// [`ScriptedReply`] entries.
///
/// Every `stream_model` call is recorded so tests can assert the call
/// count after the run completes.
#[derive(Debug, Default)]
pub struct BudgetTestGateway {
    replies: Mutex<Vec<ScriptedReply>>,
    fallback: Option<ScriptedReply>,
    calls: Mutex<Vec<HostManagedModelRequest>>,
}

impl BudgetTestGateway {
    pub fn new() -> Self {
        Self::default()
    }

    /// Single-reply convenience: every model call returns the same
    /// assistant text with the given token counts.
    pub fn with_constant(text: impl Into<String>, input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            replies: Mutex::new(Vec::new()),
            fallback: Some(ScriptedReply::new(text, input_tokens, output_tokens)),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Push one scripted reply. Replies are consumed in FIFO order.
    pub fn push(&self, reply: ScriptedReply) {
        self.replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(reply);
    }

    /// Number of `stream_model` calls observed so far.
    pub fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    fn next_reply(&self) -> ScriptedReply {
        let mut script = self
            .replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if script.is_empty() {
            return self
                .fallback
                .clone()
                .unwrap_or_else(|| ScriptedReply::new("budget test fallback reply", 0, 0));
        }
        script.remove(0)
    }
}

#[async_trait]
impl HostManagedModelGateway for BudgetTestGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        Ok(self.next_reply().into_response())
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        // The budget tests don't need capability dispatch — fall through
        // to the plain stream path. If a future test needs tool calls,
        // extend this with a separate scripted-tool-call queue.
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        Ok(self.next_reply().into_response())
    }
}

/// Mock gateway that always fails with the given error kind. Useful for
/// driving the cancellation / provider-error paths in budget tests
/// without depending on tokio cancel semantics.
#[derive(Debug)]
pub struct FailingTestGateway {
    pub kind: HostManagedModelErrorKind,
    pub summary: String,
}

impl FailingTestGateway {
    pub fn new(kind: HostManagedModelErrorKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
        }
    }
}

#[async_trait]
impl HostManagedModelGateway for FailingTestGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Err(HostManagedModelError::safe(self.kind, self.summary.clone()))
    }
}

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

/// Noop capability-obligation handler: permits every OAuth egress obligation.
#[derive(Debug)]
struct TestNoopObligationHandler;

#[async_trait]
impl ironclaw_capabilities::CapabilityObligationHandler for TestNoopObligationHandler {
    async fn satisfy(
        &self,
        _request: ironclaw_capabilities::CapabilityObligationRequest<'_>,
    ) -> Result<(), ironclaw_capabilities::CapabilityObligationError> {
        Ok(())
    }
}

/// Noop continuation dispatcher: swallows every auth-continuation event.
#[derive(Debug)]
struct TestNoopContinuationDispatcher;

#[async_trait]
impl crate::auth::RebornAuthContinuationDispatcher for TestNoopContinuationDispatcher {
    async fn dispatch_auth_continuation(
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
}

/// Shared infrastructure preamble for OAuth product-auth test bundles.
///
/// Shared in-memory product-auth infra (named to keep the helper's return type
/// out of clippy's `type_complexity` lint — a 3-tuple of nested `Arc`s trips it).
struct OAuthProductAuthInfra {
    secret_store: Arc<dyn ironclaw_secrets::SecretStore>,
    durable: Arc<
        crate::product_auth_durable::FilesystemAuthProductServices<
            ironclaw_filesystem::InMemoryBackend,
        >,
    >,
}

/// Builds the fixed-view in-memory secrets filesystem, the secret store, and
/// the durable `FilesystemAuthProductServices`. The two callers
/// (`build_oauth_product_auth_for_test` and
/// `build_google_oauth_product_auth_for_test`) differ only in the egress
/// constructor, the `HostOAuthProviderSpec` fields, and the optional
/// `.with_provider_client()` call.
fn build_oauth_product_auth_infra() -> OAuthProductAuthInfra {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
    use ironclaw_secrets::InMemorySecretStore;

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
    let secret_store: Arc<dyn ironclaw_secrets::SecretStore> = Arc::new(InMemorySecretStore::new());
    // Real durable product-auth services over the in-memory scoped filesystem.
    let durable = Arc::new(
        crate::product_auth_durable::FilesystemAuthProductServices::new(
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
/// - `InMemorySecretStore` for access/refresh token handles.
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

    // Real OAuth provider client wired to the scripted egress.
    // token_endpoint must be HTTPS to pass HostOAuthProviderClient's guard;
    // ScriptedOAuthTokenEgress ignores the actual URL.
    let spec = crate::oauth_provider_client::HostOAuthProviderSpec {
        provider_id: "test-oauth-provider",
        capability_id: "builtin.oauth.test",
        token_endpoint: "https://oauth.test.example.com/token",
        secret_handle_prefix: "test-oauth",
        resource: None,
        exchange_scope_policy:
            crate::oauth_provider_client::ExchangeScopePolicy::FallbackToRequested,
    };
    let provider_client = crate::oauth_provider_client::HostOAuthProviderClient::new(
        spec,
        Arc::clone(&egress) as Arc<dyn ironclaw_host_api::RuntimeHttpEgress>,
        Arc::clone(&secret_store),
        Arc::new(TestNoopObligationHandler),
        ironclaw_auth::OAuthClientId::new("test-client-id").unwrap(),
        ironclaw_auth::OAuthRedirectUri::new("https://localhost/oauth/callback").unwrap(),
    )
    .expect("test OAuth provider client must build");

    let services = Arc::new(crate::RebornProductAuthServices::new(
        durable.clone() as Arc<dyn ironclaw_auth::AuthFlowManager>,
        durable.clone() as Arc<dyn ironclaw_auth::AuthInteractionService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialSetupService>,
        durable.clone() as Arc<dyn ironclaw_auth::CredentialAccountService>,
        Arc::new(provider_client) as Arc<dyn ironclaw_auth::AuthProviderClient>,
        durable as Arc<dyn ironclaw_auth::SecretCleanupService>,
        Arc::new(TestNoopContinuationDispatcher),
    ));

    OAuthProductAuthTestBundle { services, egress }
}

// ─── Slice 8: OAuth credential-refresh sweep test support ────────────────────
//
// `FixedCandidateSource` and `OAuthProductAuthTestBundle::sweep_for_refresh`
// together let a test drive `credential_refresh_worker::sweep_once` with:
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

/// Fixed candidate source for credential-refresh sweep tests (slice 8).
///
/// Returns a caller-supplied list of accounts from `list_refresh_candidates`,
/// bypassing the `FilesystemAuthProductServices` filesystem walk. This lets a
/// test inject a real `CredentialAccount` (read back after an OAuth connect
/// flow) directly into `sweep_once` without needing the full tenant-path
/// enumeration to work in an in-memory backend.
///
/// Gated on `any(feature = "libsql", feature = "postgres")` because
/// `credential_refresh_worker` is only compiled under those features.
// TODO(follow-up): add a LibSql-backed sweep test that drives the real
// `FilesystemCredentialRefreshCandidateSource` enumeration. `FixedCandidateSource`
// bypasses the tenant-path filesystem walk because this bundle's fixed view
// mounts only `/secrets` (no tenant tree to enumerate). The refresh path itself
// (`sweep_once` -> `refresh_account` -> provider client -> egress -> status
// write-back) is already covered here at full fidelity; only candidate
// enumeration is stubbed.
#[cfg(any(feature = "libsql", feature = "postgres"))]
struct FixedCandidateSource {
    candidates: Vec<ironclaw_auth::CredentialAccount>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
#[async_trait]
impl crate::credential_refresh_worker::CredentialRefreshCandidateSource for FixedCandidateSource {
    async fn list_refresh_candidates(&self) -> Vec<ironclaw_auth::CredentialAccount> {
        self.candidates.clone()
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl OAuthProductAuthTestBundle {
    /// Run one credential-refresh sweep tick with a fixed account list and a
    /// frozen clock.
    ///
    /// This exercises the production `sweep_once` path — `select_idle_candidates`
    /// (idle-threshold + cap), `CredentialRefreshRequest` construction,
    /// `RebornProductAuthServices::refresh_credential_account` →
    /// `ProviderBackedCredentialAccountService::refresh_account` →
    /// `HostOAuthProviderClient::refresh_token` → scripted HTTP egress — without
    /// needing a real filesystem walk or a Postgres leader lock.
    ///
    /// # Arguments
    ///
    /// * `candidates` — `CredentialAccount` records to feed into the sweep.
    ///   Obtain these by calling `services.credential_account_service().get_account()`
    ///   after a successful OAuth connect flow so the handles are real.
    /// * `settings` — pass `CredentialRefreshSettings::enabled()` to enable the
    ///   sweep with the default 2-day idle threshold and cap of 5.
    /// * `now` — frozen instant. Pass `Utc::now() + Duration::days(3)` to make a
    ///   just-created account appear idle; pass `Utc::now()` (or any time within
    ///   the threshold) to verify no refresh is triggered.
    pub async fn sweep_for_refresh(
        &self,
        candidates: Vec<ironclaw_auth::CredentialAccount>,
        settings: crate::runtime_input::CredentialRefreshSettings,
        now: chrono::DateTime<chrono::Utc>,
    ) {
        use crate::credential_refresh_worker::{CredentialRefreshWorkerDeps, sweep_once};
        use tokio_util::sync::CancellationToken;

        let candidate_source = std::sync::Arc::new(FixedCandidateSource { candidates });

        // Build an always-leader lock: no Postgres pool needed for tests.
        #[cfg(not(feature = "postgres"))]
        let leader_lock = std::sync::Arc::new(
            crate::product_auth_refresh_lock::CredentialRefreshLeaderLock::always_leader(),
        );
        #[cfg(feature = "postgres")]
        let leader_lock = std::sync::Arc::new(
            crate::product_auth_refresh_lock::CredentialRefreshLeaderLock::new(None),
        );

        let deps = CredentialRefreshWorkerDeps {
            candidate_source,
            refresh_port: std::sync::Arc::clone(&self.services),
            leader_lock,
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
/// Gated on `any(feature = "libsql", feature = "postgres")` because
/// `sweep_for_refresh` (the primary consumer) requires `credential_refresh_worker`,
/// which is compiled only under those features.
///
/// Calling this multiple times produces independent, isolated bundles.
#[cfg(any(feature = "libsql", feature = "postgres"))]
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

    // Google OAuth spec: provider_id must be "google" for
    // HostOAuthProviderClient::refresh_token to accept the request.
    let spec = crate::oauth_provider_client::HostOAuthProviderSpec {
        provider_id: "google",
        capability_id: "builtin.oauth.google",
        token_endpoint: "https://oauth2.googleapis.com/token",
        secret_handle_prefix: "google",
        resource: None,
        exchange_scope_policy:
            crate::oauth_provider_client::ExchangeScopePolicy::FallbackToRequested,
    };
    let provider_client: Arc<dyn ironclaw_auth::AuthProviderClient> = Arc::new(
        crate::oauth_provider_client::HostOAuthProviderClient::new(
            spec,
            Arc::clone(&egress) as Arc<dyn ironclaw_host_api::RuntimeHttpEgress>,
            Arc::clone(&secret_store),
            Arc::new(TestNoopObligationHandler),
            ironclaw_auth::OAuthClientId::new("test-client-id").unwrap(),
            ironclaw_auth::OAuthRedirectUri::new("https://localhost/oauth/callback").unwrap(),
        )
        .expect("google test OAuth provider client must build"),
    );

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

    OAuthProductAuthTestBundle { services, egress }
}

/// Test-only accessor mirroring the full local-dev database-roots boot path
/// (`build_local_dev_root_filesystem` → `build_default_local_dev_database_roots`).
///
/// Constructs the durable database backend and mounts it across the
/// control-plane roots (`/tenants`, `/memory`, `/events`) of `composite`,
/// selecting the backend by compile-time feature:
/// - With `libsql`: opens `root/reborn-local-dev.db`, runs migrations, mounts.
/// - Without a durable backend feature: mounts an in-memory backend.
///
/// Called by the Reborn integration-test framework's `StorageMode::LibSql`
/// builder arm (`tests/support/reborn/builder.rs:build_storage_composite`) so
/// the 4-step libSQL setup sequence lives once (production call site:
/// `build_local_dev_root_filesystem` → `build_default_local_dev_database_roots`).
/// For tests only — gated behind `test-support`, ships zero bytes in production.
#[cfg(feature = "test-support")]
pub async fn build_default_local_dev_database_roots_for_test(
    root: &std::path::Path,
    composite: &mut ironclaw_filesystem::CompositeRootFilesystem,
) -> Result<(), crate::RebornBuildError> {
    crate::factory::mount_default_local_dev_database_roots(root, composite).await
}

/// Test-only accessor mirroring the production local-dev boot path
/// (`build_local_dev_root_filesystem` → `mount_local_dev_database_roots`).
///
/// Mounts `database` across the control-plane roots (`/tenants`, `/memory`,
/// `/events`) of `root` exactly as the libSQL local-dev boot path does, so
/// downstream integration tests (the Reborn integration-test framework in
/// `tests/support/reborn/`) construct one real `LibSqlRootFilesystem` over a
/// composite without a second copy of the mount wiring (design spec §3.2).
/// For tests only — gated behind `test-support`, so it ships zero bytes in
/// production binaries.
#[cfg(feature = "test-support")]
pub fn mount_local_dev_database_roots_for_test<F>(
    root: &mut ironclaw_filesystem::CompositeRootFilesystem,
    database: std::sync::Arc<F>,
) -> Result<(), crate::RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    crate::factory::mount_local_dev_database_roots(root, database)
}

/// Test-only entry point for building a local-dev
/// [`ironclaw_secrets::FilesystemSecretStore`] without going through the full
/// Reborn runtime assembly.
///
/// Mirrors the production wiring in `build_local_runtime` where
/// `build_local_dev_secret_store` is called with the scoped filesystem and a
/// master key resolved from the environment or the root directory's cached key
/// file. Tests that need a real `FilesystemSecretStore` — for example, to
/// verify `put` + `lease_once` + `consume` round-trips against an in-process
/// backend — can call this instead of wiring a full runtime.
///
/// The master key is resolved exactly as production does: from the
/// `SECRETS_MASTER_KEY` env var when set, otherwise from (or generating to)
/// the `.reborn-local-dev-secrets-master-key` file under `root`. Using the
/// same `root` across two calls therefore yields the same key, so a second
/// `FilesystemSecretStore` over the same scoped filesystem can consume a
/// secret written by the first. For tests only — zero bytes shipped in
/// production builds.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub fn build_local_dev_secret_store_for_test<F>(
    root: &std::path::Path,
    scoped: std::sync::Arc<ironclaw_filesystem::ScopedFilesystem<F>>,
) -> Result<std::sync::Arc<ironclaw_secrets::FilesystemSecretStore<F>>, crate::RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
{
    crate::factory::build_local_dev_secret_store(root, scoped, None)
}

/// Mirrors the production approval-gate evidence wiring done by
/// `build_local_runtime` (runtime.rs ~line 2799) — returns the REAL
/// `LocalDevApprovalGateEvidence` so the gate-evidence lookup logic
/// (the `gate:approval-` prefix parse + `ApprovalStatus::Pending` check)
/// never drifts from production. Tests only.
///
/// Wired by the Reborn integration-test framework's `assemble_thread_runtime`
/// so a `BlockedApproval` run is verified against the persisted `Pending`
/// approval request at loop exit and genuinely pauses — mirrors the production
/// `runtime.rs` path with the real type, never a hand-mirrored copy.
#[cfg(feature = "test-support")]
pub fn build_local_dev_approval_gate_evidence_for_test(
    approval_requests: std::sync::Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
) -> std::sync::Arc<dyn ironclaw_reborn::loop_exit_applier::ApprovalGateEvidenceStore> {
    crate::runtime::build_local_dev_approval_gate_evidence_for_test(approval_requests)
}
