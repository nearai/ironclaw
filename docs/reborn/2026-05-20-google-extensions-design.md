# Reborn Google Suite Extensions Design

**Date:** 2026-05-20
**Status:** Spec — awaiting plan
**Tracking issue:** [nearai/ironclaw#3829](https://github.com/nearai/ironclaw/issues/3829)
**Branch:** `reborn-integration`

## Goal

Implement Google Calendar and Gmail as native Reborn extension-v2 capability packages. The agent model can invoke them; mutations are approval-gated; missing or scope-mismatched OAuth surfaces typed auth-required state. Build a provider-agnostic OAuth substrate so additional providers (Notion, Slack, Composio, etc.) drop in without per-provider crate sprawl.

Per the issue, v1 WASM Google tools are reference only — no shimming or compatibility-wrapping.

## Non-goals

- Generalized "always allow" persistent approval policy (deferred — uses one-shot leases per invocation).
- CLI driver text fallback for `BlockedAuth` / `BlockedApproval` (deferred — webui_v2 only).
- Per-user dynamic OAuth redirect URIs.
- Google Drive, Docs, Sheets capabilities (substrate prepared, not implemented).
- Live broker deployment or broker code (broker assumed to exist already).

## Architecture overview

Two new crates plus integrations into existing Reborn composition + UI layers.

```
crates/
├── ironclaw_oauth/                  # NEW substrate (provider-agnostic OAuth)
│   ├── provider.rs                  # OAuthProvider trait + ProviderRegistry
│   ├── state.rs                     # PKCE/CSRF/redirect_uri store (5-min TTL)
│   ├── flow.rs                      # OAuthRuntime with ProviderMode { Brokered, Direct }
│   ├── callback.rs                  # axum Router → /auth/callback/{provider}
│   ├── storage.rs                   # TokenPersister over SecretStore
│   ├── refresh.rs                   # RefreshScheduler (per-credential mutex)
│   └── resume.rs                    # OAuthResumeNotifier (signal BlockedAuth turns)
└── ironclaw_native_extensions/      # NEW top-level (all native extension-v2 packages)
    ├── lib.rs                       # register_all() → RegistrationOutput
    └── google/                      # first provider subdir
        ├── oauth_provider.rs        # impl OAuthProvider for Google
        ├── client.rs                # shared Google HTTP client (refresh-aware)
        ├── credential.rs            # google_oauth_token resolver + scope mismatch + refcount
        ├── network.rs               # NetworkPolicy factory (Google API hosts)
        ├── calendar/                # 9 capability handlers + manifest
        └── gmail/                   # 6 capability handlers + manifest
```

### Dispatch path

```
agent model
  → AgentLoop emits CapabilityCalls
    → CapabilityHost::invoke_json
      → authorization + approval check (mutations → BlockedApproval if no lease)
      → obligation prepare:
          • ApplyNetworkPolicy (Google API hosts)
          • InjectSecretOnce (google_oauth_token resolved + refreshed if needed)
          • AuditBefore
        if credential missing/scope-mismatch → BlockedAuth (typed payload)
      → RuntimeDispatcher → FirstPartyRuntimeAdapter → handler
      → obligation complete: AuditAfter, RedactOutput, EnforceOutputLimit
```

### OAuth path

```
user clicks "Install Google Calendar"
  → extension records RequiresCredential("google_oauth_token", scopes=[…])
  → first invocation triggers BlockedAuth (no credential)
  → webui_v2 renders auth prompt with oauth_url from ironclaw_oauth
  → user authorizes in browser → Google redirects to /auth/callback/google
  → ironclaw_oauth: validate state, exchange code via broker or direct → write encrypted token to ironclaw_secrets
  → OAuthResumeNotifier → run_state resumes parked turn
  → second dispatch attempt: InjectSecretOnce succeeds → handler runs
```

## What we have vs what's new

### Existing substrate (reused, not built)

| Piece | Where | Status |
|---|---|---|
| FirstParty native capability runtime | `crates/ironclaw_host_runtime/src/first_party.rs:109,119` | Wired |
| Extension-v2 manifest schema | `crates/ironclaw_extensions/src/v2.rs:50` | Merged via #3787–#3795 |
| Capability dispatch + obligations | `crates/ironclaw_capabilities/src/host.rs:135` | InjectSecretOnce, ApplyNetworkPolicy, AuditBefore/After, RedactOutput |
| Scoped secrets + one-shot lease | `crates/ironclaw_secrets/src/lib.rs:42,123` | AES-256-GCM, in-mem + filesystem |
| Network policy + hardened egress | `crates/ironclaw_network/src/lib.rs:20` | DNS/private-IP checks, leak scanning |
| Approval substrate | `crates/ironclaw_approvals/src/lib.rs:17` | Lease-backed, resume_json (BlockedApproval) |
| Run state | `crates/ironclaw_run_state/src/lib.rs:34` | `BlockedAuth` enum variant exists |
| v1 Google tools (reference only) | `tools-src/google-calendar/`, `tools-src/gmail/` | Logic source for handlers |

### Legacy OAuth contract (reused, mirrored in Reborn)

| Component | Legacy location | Notes |
|---|---|---|
| Broker URL env | `IRONCLAW_OAUTH_EXCHANGE_URL` | If set → brokered mode |
| Broker auth env | `IRONCLAW_OAUTH_PROXY_AUTH_TOKEN` (fallback `GATEWAY_AUTH_TOKEN`) | Bearer to broker |
| Broker endpoints | `POST /oauth/exchange`, `POST /oauth/refresh` | Defined by hosted broker; reuse exactly |
| SSRF guard | `src/auth/mod.rs:580` | HTTPS-only, private-IP block |
| State+PKCE store | `src/channels/web/oauth/state_store.rs:32` | 5-min TTL, S256 challenge |
| Token exchange | `src/auth/oauth.rs:305,1098` (`exchange_oauth_code` + `exchange_via_proxy`) | Pattern reused |
| Token storage row layout | `src/auth/oauth.rs:445` | `{name}` / `{name}_refresh_token` / `{name}_scopes` / `{name}_expiry` |
| Hosted secret suppression | `src/auth/providers.rs:69` (`hosted_proxy_client_secret`) | Local CLIENT_SECRET suppressed in broker mode |

### New work

1. `ironclaw_oauth` crate (substrate)
2. `ironclaw_native_extensions` crate (Google package container)
3. `BlockedAuth` resume path in `ironclaw_capabilities::host::resume_json`
4. `OAuthResumeNotifier` ↔ `run_state` query + broadcast for parked turns
5. webui_v2 descriptor + SSE event + frontend prompts for `BlockedAuth` / `BlockedApproval`
6. Architecture boundary test updates for both new crates
7. Deterministic fake Google API + fake OAuth broker fixtures
8. Replay snap fixtures for read + approved-write paths

## `ironclaw_oauth` crate

### Cargo.toml

```toml
async-trait = "0.1"
axum = "0.7"
chrono = { version = "0.4", features = ["serde"] }
ironclaw_host_api = { path = "../ironclaw_host_api" }
ironclaw_network  = { path = "../ironclaw_network" }
ironclaw_secrets  = { path = "../ironclaw_secrets" }
ironclaw_run_state= { path = "../ironclaw_run_state" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
base64 = "0.22"
rand = "0.8"
secrecy = "0.10"
thiserror = "2"
tokio = { version = "1", features = ["sync", "rt"] }
tracing = "0.1"
url = "2"
uuid = { version = "1", features = ["v4"] }

[features]
default = []
test-fixtures = []  # gates allow_loopback_broker_for_tests builder method
```

### Core types

```rust
// provider.rs
#[async_trait]
pub trait OAuthProvider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn auth_url(&self) -> &str;
    fn token_url(&self) -> &str;
    fn credential_name(&self) -> &str;
    fn public_client_id(&self) -> &str;          // safe to send to broker + embed in authorize URL
    fn direct_client_secret(&self) -> Option<&SecretString>;  // None in broker mode
    fn build_authorize_url(&self, state: &str, code_challenge: &str, scopes: &[String], redirect_uri: &str) -> String;
    fn parse_token_response(&self, body: &serde_json::Value) -> Result<TokenSet, OAuthError>;
    fn detect_scope_mismatch(&self, stored: &[String], required: &[String]) -> Vec<String>;
}

// flow.rs
pub enum ProviderMode {
    Brokered { broker_url: Url, broker_auth: SecretString },
    Direct,  // each provider supplies its own CLIENT_SECRET via direct_client_secret()
}

pub struct OAuthRuntime {
    mode: ProviderMode,
    state: Arc<OAuthStateStore>,
    providers: Arc<ProviderRegistry>,
    egress: Arc<HardenedHttpEgressClient>,
    secrets: Arc<dyn SecretStore>,
    resume: Arc<OAuthResumeNotifier>,
}

impl OAuthRuntime {
    pub fn from_env(/* deps */) -> Result<Self, OAuthError>;
    pub async fn start(&self, provider_id: &str, scopes: Vec<String>, scope: ResourceScope) -> Result<StartedFlow, OAuthError>;
    pub async fn exchange(&self, provider_id: &str, code: String, state: String) -> Result<(), OAuthError>;
    pub async fn refresh_if_needed(&self, provider_id: &str, scope: &ResourceScope) -> Result<(), OAuthError>;
}

pub struct OAuthRuntimeBuilder {
    ssrf_policy: SsrfPolicy,
    /* … */
}

impl OAuthRuntimeBuilder {
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn allow_loopback_broker_for_tests(mut self) -> Self { … }
}
```

### Env config

| Var | Required | Purpose |
|---|---|---|
| `IRONCLAW_OAUTH_EXCHANGE_URL` | optional | Broker base URL. If set → brokered mode. |
| `IRONCLAW_OAUTH_PROXY_AUTH_TOKEN` | optional | Broker bearer. Falls back to `GATEWAY_AUTH_TOKEN`. |
| `OAUTH_BASE_URL` | optional | Local callback redirect_uri base. Falls back to webui_v2 listen URL. |

No `OAUTH_ENABLED` master kill-switch (per-extension install/uninstall is the real gate).
No `IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK` env var (compile-gated builder method only).

### Broker call shape (reused from legacy)

```
POST {broker}/oauth/exchange
Authorization: Bearer {gateway_token}
Body: { code, redirect_uri, token_url, client_id, access_token_field }
       — no client_secret
Response: { access_token, refresh_token, expires_in, scope }
```

```
POST {broker}/oauth/refresh
Authorization: Bearer {gateway_token}
Body: { refresh_token, token_url, client_id, provider }
Response: { access_token, expires_in, scope }
```

### Token storage row layout (per credential_name, mirrors legacy)

| Row | Content |
|---|---|
| `{credential_name}` | access_token (encrypted) |
| `{credential_name}_refresh_token` | refresh_token (encrypted) |
| `{credential_name}_scopes` | JSON array of granted scopes |
| `{credential_name}_expiry` | Unix timestamp |
| `{credential_name}_refs` | JSON array of extension_ids referencing this credential |

### BlockedAuth payload

```rust
pub struct AuthRequiredPayload {
    pub provider_id: String,           // "google"
    pub credential_name: String,       // "google_oauth_token"
    pub missing_scopes: Vec<String>,
    pub oauth_url: String,             // pre-built consent URL with delta scopes
    pub flow_id: Uuid,
    pub extension_id: ExtensionId,
}
```

Stored at `RunStatus::BlockedAuth(AuthRequiredPayload)`. `resume_json` accepts `OAuthCallbackOutcome { flow_id, success }` to resume.

### Boundary updates

`crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`:
- `ironclaw_oauth` allowed deps: `host_api`, `network`, `secrets`, `run_state`
- `ironclaw_oauth` MUST NOT be depended on by: `host_api`, `secrets`, `network`, `run_state`, `host_runtime`, `extensions`, `capabilities`

## `ironclaw_native_extensions` crate

### Cargo.toml

```toml
[features]
default = ["google"]
google = []

[dependencies]
async-trait = "0.1"
ironclaw_capabilities = { path = "../ironclaw_capabilities" }
ironclaw_extensions   = { path = "../ironclaw_extensions" }
ironclaw_host_api     = { path = "../ironclaw_host_api" }
ironclaw_host_runtime = { path = "../ironclaw_host_runtime" }
ironclaw_network      = { path = "../ironclaw_network" }
ironclaw_oauth        = { path = "../ironclaw_oauth" }
ironclaw_secrets      = { path = "../ironclaw_secrets" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
secrecy = "0.10"
thiserror = "2"
tokio = { version = "1", features = ["sync"] }
tracing = "0.1"
url = "2"
chrono = { version = "0.4", features = ["serde"] }
```

### Registration entry point

```rust
pub struct RegistrationOutput {
    pub packages: Vec<ExtensionPackage>,
    pub handlers: Vec<(CapabilityId, Arc<dyn FirstPartyCapabilityHandler>)>,
    pub oauth_providers: Vec<Arc<dyn OAuthProvider>>,
    pub network_policies: Vec<NetworkPolicy>,
}

pub fn register_all(env: &EnvConfig, secrets: Arc<dyn SecretStore>) -> Result<RegistrationOutput, NativeExtensionError> {
    let mut out = RegistrationOutput::default();
    #[cfg(feature = "google")]
    google::register(env, secrets, &mut out)?;
    Ok(out)
}
```

Composition (`ironclaw_reborn_composition/src/factory.rs`) consumes `RegistrationOutput` and merges into the appropriate registries.

### Provider split policy

`crates/ironclaw_native_extensions/CLAUDE.md`:

```markdown
## Provider split policy

Each provider lives in a self-contained subdir under `src/`. Subdirs MUST NOT import from siblings.

Split a provider into its own sibling crate (`ironclaw_native_extensions_<provider>`) when ANY of:
- Provider needs heavy external deps not shared by others (>5 MB additional binary size or >5s compile-time impact)
- Provider count exceeds ~10
- Provider has independent release cadence (e.g. vendored SDK with own versioning)

Split is mechanical: copy subdir → new crate → update workspace + reborn_dependency_boundaries.rs → adjust register fn name.
```

### Boundary updates

`reborn_dependency_boundaries.rs`:
- `ironclaw_native_extensions` allowed deps: `capabilities`, `extensions`, `host_api`, `host_runtime`, `network`, `oauth`, `secrets`
- `ironclaw_native_extensions` MUST NOT be depended on by: any other Reborn crate (top-level consumer only, used from composition)
- Internal rule: `src/google/` MUST NOT import from `src/notion/`, etc. (sibling provider isolation)

## Google provider

### `google/oauth_provider.rs`

```rust
const BAKED_IN_GOOGLE_DESKTOP_CLIENT_ID: &str = /* well-known Google Desktop OAuth client_id */;

pub struct GoogleProvider {
    public_client_id: String,
    direct_client_secret: Option<SecretString>,
    allowed_hd: Option<String>,
}

impl GoogleProvider {
    pub fn from_env(broker_active: bool) -> Result<Option<Arc<Self>>, OAuthError> {
        let public_client_id = std::env::var("GOOGLE_CLIENT_ID").ok().filter(|s| !s.is_empty())
            .or_else(|| broker_active.then(|| BAKED_IN_GOOGLE_DESKTOP_CLIENT_ID.to_string()));
        let public_client_id = match public_client_id {
            Some(id) => id,
            None => return Ok(None),  // direct mode + no GOOGLE_CLIENT_ID → provider disabled
        };
        let direct_client_secret = if broker_active {
            None
        } else {
            Some(SecretString::from(std::env::var("GOOGLE_CLIENT_SECRET")
                .map_err(|_| OAuthError::IncompleteConfig { provider: "google", reason: "direct mode requires GOOGLE_CLIENT_SECRET" })?))
        };
        let allowed_hd = std::env::var("GOOGLE_ALLOWED_HD").ok().filter(|s| !s.is_empty());
        Ok(Some(Arc::new(Self { public_client_id, direct_client_secret, allowed_hd })))
    }
}

#[async_trait]
impl OAuthProvider for GoogleProvider {
    fn provider_id(&self) -> &str { "google" }
    fn auth_url(&self) -> &str { "https://accounts.google.com/o/oauth2/v2/auth" }
    fn token_url(&self) -> &str { "https://oauth2.googleapis.com/token" }
    fn credential_name(&self) -> &str { "google_oauth_token" }
    fn public_client_id(&self) -> &str { &self.public_client_id }
    fn direct_client_secret(&self) -> Option<&SecretString> { self.direct_client_secret.as_ref() }

    fn build_authorize_url(&self, state: &str, code_challenge: &str, scopes: &[String], redirect_uri: &str) -> String {
        let mut url = Url::parse(self.auth_url()).expect("static URL");
        url.query_pairs_mut()
            .append_pair("client_id", &self.public_client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &scopes.join(" "))
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("include_granted_scopes", "true");  // incremental OAuth
        if let Some(hd) = &self.allowed_hd {
            url.query_pairs_mut().append_pair("hd", hd);
        }
        url.to_string()
    }

    fn parse_token_response(&self, body: &serde_json::Value) -> Result<TokenSet, OAuthError> { … }
    fn detect_scope_mismatch(&self, stored: &[String], required: &[String]) -> Vec<String> {
        required.iter().filter(|s| !stored.contains(s)).cloned().collect()
    }
}
```

### Google API env config

| Var | Required | Behavior if missing |
|---|---|---|
| `GOOGLE_CLIENT_ID` | direct mode: required; broker mode: optional (baked-in default) | Provider not registered in direct mode |
| `GOOGLE_CLIENT_SECRET` | direct mode: required; broker mode: ignored | Direct mode init fails |
| `GOOGLE_ALLOWED_HD` | optional | No Workspace domain restriction |

### Scope catalog

```rust
pub mod scopes {
    pub const CALENDAR_READONLY: &str = "https://www.googleapis.com/auth/calendar.readonly";
    pub const CALENDAR_EVENTS: &str   = "https://www.googleapis.com/auth/calendar.events";
    pub const GMAIL_READONLY: &str    = "https://www.googleapis.com/auth/gmail.readonly";
    pub const GMAIL_SEND: &str        = "https://www.googleapis.com/auth/gmail.send";
    pub const GMAIL_MODIFY: &str      = "https://www.googleapis.com/auth/gmail.modify";
}
```

### Refresh flow

Triggered from `inject_secrets()` obligation when:
- Token expiry < now + 60s buffer, OR
- 401 response from Google API during dispatch

`RefreshScheduler::refresh(provider, scope)`:
1. Read `{credential_name}_refresh_token` via one-shot lease.
2. Call `OAuthRuntime::refresh` — dispatches by mode (broker `/oauth/refresh` vs direct provider token_url).
3. Write new access_token + expiry; preserve refresh_token if response omits it.
4. Return fresh token for current invocation.
5. On `invalid_grant` → surface `BlockedAuth` to re-prompt full flow.

Concurrency: per-credential `Mutex` ensures only one in-flight refresh per credential.

## Capability handlers

### Common shape

```rust
pub struct ListEventsHandler { client: Arc<GoogleHttpClient> }

#[async_trait]
impl FirstPartyCapabilityHandler for ListEventsHandler {
    async fn dispatch(&self, req: FirstPartyCapabilityRequest) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let input: ListEventsInput = serde_json::from_value(req.input)
            .map_err(|e| FirstPartyCapabilityError::input_invalid(e.to_string()))?;
        let token = req.injected_secrets.get("google_oauth_token")
            .ok_or(FirstPartyCapabilityError::missing_secret("google_oauth_token"))?;
        let response = self.client
            .get(&format!("/calendar/v3/calendars/{}/events", input.calendar_id))
            .bearer(token)
            .query(&input.to_query_params())
            .send().await
            .map_err(map_google_error)?;
        let body: GoogleEventsResponse = response.json().await?;
        let output = ListEventsOutput::from(body);
        Ok(FirstPartyCapabilityResult {
            output: serde_json::to_value(output)?,
            resource_usage: ResourceUsage { wall_clock_ms: started.elapsed().as_millis() as u64, output_bytes: … },
        })
    }
}
```

### Calendar capabilities (9)

| Capability ID | Effects | Scopes | Approval |
|---|---|---|---|
| `google-calendar.list_calendars` | Read | `calendar.readonly` | — |
| `google-calendar.list_events` | Read | `calendar.readonly` | — |
| `google-calendar.get_event` | Read | `calendar.readonly` | — |
| `google-calendar.create_event` | Write | `calendar.events` | RequiresApproval |
| `google-calendar.update_event` | Write | `calendar.events` | RequiresApproval |
| `google-calendar.delete_event` | Write | `calendar.events` | RequiresApproval |
| `google-calendar.find_free_slots` | Read | `calendar.readonly` | — |
| `google-calendar.add_attendees` | Write | `calendar.events` | RequiresApproval |
| `google-calendar.set_reminder` | Write | `calendar.events` | RequiresApproval |

### Gmail capabilities (6)

| Capability ID | Effects | Scopes | Approval |
|---|---|---|---|
| `gmail.list_messages` | Read | `gmail.readonly` | — |
| `gmail.get_message` | Read | `gmail.readonly` | — |
| `gmail.send_message` | Network egress (send) | `gmail.send` | RequiresApproval |
| `gmail.create_draft` | Write | `gmail.modify` | RequiresApproval |
| `gmail.reply_to_message` | Network egress (send) | `gmail.send`, `gmail.modify` | RequiresApproval |
| `gmail.trash_message` | Write | `gmail.modify` | RequiresApproval |

### Network policy

```rust
NetworkPolicy {
    allowed_targets: vec![
        NetworkTarget::exact("www.googleapis.com"),
        NetworkTarget::exact("gmail.googleapis.com"),
        NetworkTarget::exact("oauth2.googleapis.com"),
    ],
    allowed_methods: vec![Method::GET, Method::POST, Method::PUT, Method::PATCH, Method::DELETE],
    max_response_bytes: 16 * 1024 * 1024,
    max_request_bytes: 10 * 1024 * 1024,
    redirect_policy: RedirectPolicy::DenyExceptSameOrigin,
}
```

Plus broker host if brokered mode active.

### Error mapping (Google API → Reborn)

- HTTP 401 + `error: invalid_token` → refresh → if refresh fails → `BlockedAuth`
- HTTP 403 + `error: insufficient_scope` → `BlockedAuth` with missing scopes parsed from response
- HTTP 429 + `Retry-After` → `RuntimeDispatchError::RateLimited { retry_after }`
- HTTP 5xx → `RuntimeDispatchError::ProviderUnavailable` (retryable)
- HTTP 4xx other → `FirstPartyCapabilityError::invalid_input(message)`
- Network/timeout → `RuntimeDispatchError::Transport`

### Output redaction

`google/client.rs` strips internal IDs (`internalDate`, `historyId`), raw `payload.headers`, and large base64 attachments (returned as `attachment_ref` for separate fetch). Token never appears in output. Backed by `RedactOutput` obligation.

### Shared credential lifecycle

`google/credential.rs::register_extension_use(extension_id)` increments `{credential_name}_refs` array at extension install. Uninstall decrements; deletes credential row only when refs empty. Satisfies #3829 shared cleanup requirement.

## Approval + auth flow

### `BlockedAuth` resume path (NEW)

Current `capabilities/src/host.rs:439` `resume_json` handles only `BlockedApproval`. Extend:

```rust
pub async fn resume_json(&self, request: ResumeRequest) -> Result<ResumeResult, HostError> {
    let current = self.run_state.get(&request.invocation_id)?;
    match current.status {
        RunStatus::BlockedApproval => self.resume_after_approval(request, current).await,
        RunStatus::BlockedAuth => self.resume_after_auth(request, current).await,  // NEW
        other => Err(HostError::CannotResume { status: other }),
    }
}

async fn resume_after_auth(&self, request: ResumeRequest, current: RunRecord) -> Result<ResumeResult, HostError> {
    let payload: AuthRequiredPayload = serde_json::from_value(current.blocked_payload)?;
    let outcome: OAuthCallbackOutcome = serde_json::from_value(request.payload)?;
    if outcome.flow_id != payload.flow_id { return Err(HostError::FlowMismatch); }
    if !outcome.success { return Err(HostError::AuthDenied); }
    self.invoke_json_inner(current.original_request).await
}
```

### `OAuthResumeNotifier`

```rust
impl OAuthResumeNotifier {
    pub async fn notify(&self, credential_name: &str, scope: &ResourceScope) {
        let parked = self.run_state.query_blocked_auth(credential_name, scope).await;
        for record in parked {
            self.resume_channel.send(ResumeSignal {
                invocation_id: record.invocation_id,
                outcome: OAuthCallbackOutcome { flow_id: record.flow_id, success: true },
            }).await;
        }
    }
}
```

Loop/turn coordinator consumes `ResumeSignal` and calls `CapabilityHost::resume_json`.

### UI surface (webui_v2)

```rust
pub enum BlockedDescriptor {
    AuthRequired {
        provider_id: String,
        extension_id: ExtensionId,
        missing_scopes: Vec<String>,
        oauth_url: String,
        flow_id: Uuid,
    },
    ApprovalRequired {
        capability_id: CapabilityId,
        extension_id: ExtensionId,
        input_preview: serde_json::Value,
        effects: Vec<EffectKind>,
        approval_request_id: Uuid,
    },
}
```

SSE event `agent.turn.blocked` carries `BlockedDescriptor`. Frontend renders:
- AuthRequired → "Connect Google Calendar" button → `window.open(oauth_url)`
- ApprovalRequired → modal: capability name, redacted input preview, Approve/Deny

Files: `crates/ironclaw_gateway/static/js/core/oauth-prompt.js` + `approval-prompt.js`, styles in `crates/ironclaw_gateway/static/styles/surfaces/`.

## Testing

### Test pyramid

| Tier | What | Where |
|---|---|---|
| Unit | Handler input/output/error mapping | per-handler `#[cfg(test)]` |
| Crate integration | `CapabilityHost::invoke_json` → fake Google API → audit/event assertions | `crates/ironclaw_native_extensions/tests/` |
| OAuth flow | Brokered + direct modes, fake broker, fake Google token endpoint | `crates/ironclaw_oauth/tests/` |
| BlockedAuth resume | NEW resume path: capability → BlockedAuth → OAuth complete → resume | `crates/ironclaw_capabilities/tests/resume_blocked_auth.rs` |
| Approval gate | Mutation → BlockedApproval → approve → lease consumed → dispatch | `crates/ironclaw_native_extensions/tests/google_calendar_approval.rs` |
| Reborn E2E | Full stack with fake broker + fake Google + Playwright | `tests/e2e/google_native_extension.py` |

### Required scenarios (matches #3829 exit criteria)

- Calendar read path
- Calendar mutation approval path
- Gmail read path
- Gmail send approval path
- Scope mismatch → BlockedAuth
- Missing credential → BlockedAuth
- Refresh on expiry → success
- Refresh failure (`invalid_grant`) → BlockedAuth
- Shared credential cleanup (uninstall Calendar keeps Gmail)
- No token/scope leak in audit/events/errors
- Approval-service unavailable → fail closed
- Brokered exchange flow (legacy contract)
- Direct exchange flow

### Live test gating

Optional, behind `LIVE_GOOGLE_TESTS=1` + `LIVE_GOOGLE_REFRESH_TOKEN=…`. Default-skipped. Documented in `tests/support/LIVE_TESTING.md`.

### Fixtures

```
crates/ironclaw_native_extensions/tests/fixtures/google_api/
├── calendar/{list_calendars,list_events,…}.{GET,POST}.json
├── gmail/{list_messages,send_message,…}.{GET,POST}.json
└── oauth/{authorize.html, token.POST.json, refresh.POST.json}

crates/ironclaw_oauth/tests/fixtures/fake_broker.rs   # axum server bound to 127.0.0.1:0
```

### Replay snaps

`tests/fixtures/llm_traces/google_calendar_read/` + `tests/fixtures/llm_traces/google_gmail_send_approved/`. Verified via `scripts/replay-snap.sh test google_calendar_read`.

## Build sequence

8 phases, one PR per phase, sequential against `reborn-integration`.

### Phase 1 — `ironclaw_oauth` substrate

New crate. No consumers yet. All tests pass.

- Crate skeleton + workspace registration + Cargo.toml
- `OAuthProvider` trait + `ProviderRegistry`
- `OAuthStateStore` (5-min TTL)
- `TokenSet` + `TokenPersister` (SecretStore facade, legacy row layout)
- `OAuthRuntime` + `ProviderMode { Brokered, Direct }` (env detection + SSRF guard + builder)
- `OAuthFlow::start/exchange/refresh` dispatched by mode (broker contract reuse)
- `RefreshScheduler` (per-credential mutex)
- `OAuthResumeNotifier`
- `callback::router(flow)` axum Router export
- Tests: brokered + direct flow, SSRF reject, refresh, fake broker
- Update `reborn_dependency_boundaries.rs`

### Phase 2 — `BlockedAuth` resume path

`resume_json` extension. Integration test green.

- `AuthRequiredPayload` shape in `ironclaw_run_state`
- `capabilities/src/host.rs::resume_after_auth`
- `OAuthResumeNotifier` ↔ `run_state` query + broadcast
- Integration test: capability → BlockedAuth → mock OAuth complete → resume → succeeds
- Loop_support update if needed (`capability_port.rs:1696`)

### Phase 3 — `ironclaw_native_extensions` scaffold + Google provider registration

New crate. Google provider registered. No capabilities yet.

- Crate skeleton + workspace + Cargo.toml (features `["google"]`)
- `src/lib.rs::register_all` returning `RegistrationOutput`
- `google/oauth_provider.rs::GoogleProvider`
- `google/credential.rs` (resolver, scope-mismatch, refcount)
- `google/network.rs::google_api_network_policy()`
- `google/client.rs::GoogleHttpClient` (refresh-aware, error mapper)
- Tests: provider registration in both modes
- `CLAUDE.md` split policy
- Boundary test update

### Phase 4 — Composition wiring

`ironclaw_reborn_composition::factory` wires both crates.

- Call `OAuthRuntime::from_env`, mount `callback::router` on webui_v2
- Call `ironclaw_native_extensions::register_all` → merge into `ExtensionRegistry` + `FirstPartyCapabilityRegistry` + `NetworkPolicyEnforcer` + `ProviderRegistry`
- Production composition integration test

### Phase 5 — Google Calendar package

9 calendar capabilities. All tests green. Replay snap recorded.

- `google/calendar/manifest.rs` (HostBundled package, 9 descriptors)
- 3 read handlers: `list_calendars`, `list_events`, `get_event`
- Read-path integration test
- `find_free_slots`
- 5 write handlers: `create_event`, `update_event`, `delete_event`, `add_attendees`, `set_reminder` (all `RequiresApproval`)
- Approval-gate tests: blocks → approve → succeeds; approval-unreachable → fail closed
- Scope-mismatch + missing-credential tests
- Redaction test
- Shared-credential lifecycle test
- Replay snap `tests/fixtures/llm_traces/google_calendar_read/`

### Phase 6 — Gmail package

6 gmail capabilities. All tests green. Replay snap recorded.

- `google/gmail/manifest.rs` (6 descriptors)
- Read handlers: `list_messages`, `get_message`
- Write handlers: `send_message`, `create_draft`, `reply_to_message`, `trash_message` (all `RequiresApproval`)
- Tests: read, approval, scope mismatch, refresh, redaction
- Replay snap `tests/fixtures/llm_traces/google_gmail_send_approved/`

### Phase 7 — webui_v2 UI + frontend prompts

User-facing OAuth + approval prompts.

- `webui_v2::descriptors::BlockedDescriptor` enum
- SSE event `agent.turn.blocked` emission from loop integration
- Frontend `oauth-prompt.js` + `approval-prompt.js`
- CSS in `crates/ironclaw_gateway/static/styles/`
- E2E test (Playwright): install Calendar → trigger event create → OAuth prompt → consent → approval prompt → approve → success

### Phase 8 — Live test harness (optional)

- `tests/live/google_calendar_live.rs`, `tests/live/google_gmail_live.rs`
- Update `tests/support/LIVE_TESTING.md`
- CI lane skipped by default

## Open questions

- Confirm broker URL value to use in dev/staging (production broker URL is operational config).
- Confirm `BAKED_IN_GOOGLE_DESKTOP_CLIENT_ID` value — should match what legacy ships, or get a new Reborn-specific public client_id.
- Whether SSE event name `agent.turn.blocked` collides with any existing event name in webui_v2; verify before Phase 7.

## Exit criteria

Maps directly to #3829 exit criteria:

- Full Google Calendar capability set in catalog ✓ (Phase 5)
- Full Gmail capability set in catalog ✓ (Phase 6)
- AgentLoop invokes via `HostRuntime` + native extension-v2 path ✓ (Phases 5+6)
- Fake Google OAuth/API tests pass for read + mutation paths ✓ (Phase 5+6 tests)
- Mutations require approval, fail closed if approval service unavailable ✓ (Phase 5+6 tests)
- Scope mismatch or missing credential → typed auth-required state ✓ (Phase 2 + 5+6 tests)
- Shared credential cleanup semantics preserved ✓ (Phase 3 refcount + Phase 5 test)
- No token, host path, or tool input leaks in user-visible/persisted surfaces ✓ (Phase 5+6 redaction tests)
