# Reborn Product Auth Contract

- **Status:** contract and composition seam
- **Issue:** #3289 / #3810 / #3811 / #3812 / #3881 / #3882 / #3883 / #3884
- **Crate:** `crates/ironclaw_auth`
- **Composition:** `ironclaw_reborn_composition::RebornProductAuthServices`

---

## Purpose

Product-facing auth is the user/operator workflow for setting up, recovering,
selecting, refreshing, and cleaning up credentials for integrations,
providers, extensions, MCP servers, WASM tools/channels, and future identity
login flows.

This slice is contract-first. `ironclaw_auth` defines Reborn-native vocabulary,
traits, validation helpers, and fake services. `ironclaw_reborn_composition`
owns the production filesystem-backed adapter and factory wiring. #3811 adds a
Reborn composition seam, #3812 adds callback completion handling, #3881 mounts
the first Reborn-native OAuth start/callback HTTP routes through
`ironclaw_reborn_composition`, #3882 adds the composition-facing manual-token
secure-submit entrypoint, #3883 adds recovery/selection facade coverage, and
#3884 adds refresh/cleanup lifecycle contracts. It does not migrate production
extension setup routes, CLI/setup flows, a production refresh scheduler/HTTP
provider implementation, or runtime credential injection.

Behavior may remain compatible with legacy UX. Code paths must not mingle V1
components with Reborn components: V1 route handlers, pending maps, extension
manager authority, and V1 secret stores are inventory evidence only.

---

## Ownership Boundaries

| Boundary | Owns | Must not own |
| --- | --- | --- |
| `AuthFlowManager` | scoped flow records, callback consumption, terminal state | provider HTTP, extension activation, turn replay |
| `AuthInteractionService` | redacted auth-required projections and secure manual-token submit | secret persistence internals or model-visible token transport |
| `CredentialSetupService` | create/update account records from OAuth/manual setup results | durable encryption or runtime injection |
| `CredentialAccountService` | account metadata, ownership, grants, status, redacted projections | raw access/refresh token material |
| `AuthProviderClient` | one-shot OAuth provider exchange/refresh vocabulary over host egress | product workflow or route state |
| `SecretCleanupService` | ownership-aware uninstall/deactivate cleanup reports | deleting reusable/shared accounts by default |

Low-level encrypted storage, leases, host-mediated HTTP credential injection,
approval interaction UI, and no-exposure enforcement remain owned by their
respective Reborn substrate contracts.

### Credential ownership granularity (#4935)

Credential accounts are owned at **durable owner granularity** —
`tenant_id` / `user_id` / `agent_id` / `project_id`. `thread_id`,
`mission_id`, and `invocation_id` are **transient invocation provenance**, not
ownership: a credential a user authorizes in one chat thread stays resolvable
from every other thread/mission/invocation of the same owner. `session_id` and
the `surface` are **path-segmenting** — both are part of the durable storage
key (`/secrets/.../product-auth/{surface}/accounts/{id}.json`), so they are
matched *exactly* for bind/update writes (a reconnect binds only within its own
session and surface), while runtime resolution reads enumerate across the
owner's sessions and surfaces.

Concretely:

- Owner matching uses `CredentialAccountOwnerScope` (tenant/user/agent/project
  hard-required; mission/thread wildcarded when absent). The shared primitives
  are `ResourceScope::without_thread_and_mission` (a *neutral* scope-narrowing
  helper in `ironclaw_host_api`, with no credential semantics) and
  `AuthProductScope::credential_owner` / `to_credential_owner` (the
  credential-ownership contract, owned by `ironclaw_auth`). Resolvers and route
  construction must route through these rather than re-deriving the field strip
  inline.
- OAuth/manual-token bind **and the subsequent bound-account apply/update**
  resolve through `ironclaw_auth::binding_scope_owns_account`, which clears
  `thread_id`/`mission_id` and ignores `invocation_id` (not part of the owner
  comparison) but requires exact `session_id` and `surface` equality. Using full
  scope equality here (the prior `scope_matches`) forked a duplicate
  `UserReusable` account on every reconnect and bound credentials to the thread
  they were authorized in. The apply step must use the owner-granularity check on
  *both* transports: the OAuth callback (`update_bound_oauth_account`) and the
  manual-token submit (`validate_bound_account_update_target`). Validating the
  setup binding at owner granularity but applying it with full `scope_matches`
  accepts a reconnect at setup and then rejects it at apply — re-forking the
  account on the manual-token path.
- Requester authorization — which extension may *use* a non-`UserReusable`
  account — is enforced separately by
  `CredentialAccount::is_authorized_for_requester` and the runtime visibility
  policy, never by the thread/mission a credential was authorized in.

`RebornProductAuthServices` is the single composition bundle for the product
auth ports above. WebUI/setup/extension surfaces should call this bundle once
routes are migrated instead of reconstructing auth-flow stores, credential
stores, provider clients, or cleanup services locally.

Host-owned OAuth callback routes should parse and validate their HTTP input,
derive hashes for opaque state/code/verifier values, then call
`RebornProductAuthServices` for flow preflight/callback handling. The handler
claims the flow through `AuthFlowManager`, performs provider exchange through
`AuthProviderClient`, completes the auth flow through `AuthFlowManager`, and
dispatches an `AuthContinuationEvent` to the injected continuation dispatcher.
If continuation dispatch fails, the handler returns a sanitized retryable error
instead of reporting callback success; retrying an already-completed callback
may re-dispatch the typed continuation without re-exchanging provider code until
that dispatch is durably marked. After the continuation marker is stored,
callback replay returns the completed flow without dispatching again.
Callback route code must not activate extensions, resume turns, replay prompts,
or dispatch runtime work directly.

The first WebUI-mounted OAuth start route accepts only a provider authorization
endpoint shape, not a precomposed browser-owned provider URL. Host composition
must create the flow first, then return a sanitized authorization URL carrying
flow/callback metadata; raw state and PKCE verifier material stay hashed or
process-local and must not be serialized. Callback query parsing is bounded and
malformed fields fail closed before provider exchange.
Provider-specific routes may own additional provider URL construction when they
need host-only client metadata; the Google setup route builds the Google
authorization URL from configured Reborn product-auth client metadata and keeps
the static redirect URI aligned with the provider exchange client.

`ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher` is the
product-workflow bridge for `AuthContinuationRef::TurnGateResume`. It converts
that specific typed auth continuation into a `TurnCoordinator::resume_turn` call
using the canonical turn scope, actor, run id, and gate ref carried by the auth
event. It does not define auth state, credential vocabulary, or generic
continuation dispatch. Setup-only, lifecycle-activation, and product-action
continuations remain explicit non-turn cases for their owning handlers and must
not be performed inline by the OAuth callback route.

`ironclaw_product_workflow::AuthInteractionService` owns the product/WebUI
blocked-auth interaction loop from #3094. It reads auth-required gates from
scoped blocked run-state plus auth-flow records, returns redacted
adapter/UI-safe DTOs, and routes credential/callback/cancel decisions back
through `AuthFlowManager` and `TurnCoordinator` with the `BlockedAuthGate`
precondition. It consumes the auth-flow boundary here for auth gates; it must
not create a second credential-account or OAuth-flow model.
Only non-terminal auth-flow states are listed as pending interactions. Terminal
states such as `failed`, `completed`, `expired`, and `canceled` must not be
rendered as actionable auth gates.

Legacy web/CLI/channel auth UX may remain behavior-compatible during
migration, but Reborn paths should enter through `ProductWorkflow` or the
WebUI-facing `RebornServicesApi` facade. They must not call V1 pending maps,
V1 OAuth routes, extension-manager authority, or route-local credential state.
Dedicated HTTP route mounting for manual-token and OAuth callback transports
remains a host-composition concern around `RebornProductAuthServices`.

---

## Source Of Truth

Reborn product auth records are scoped by `AuthProductScope` plus opaque ids:

```text
AuthProductScope + AuthFlowId -> AuthFlowRecord
AuthProductScope + AuthInteractionId -> secure manual-token interaction
AuthProductScope + CredentialAccountId -> CredentialAccount
```

In-memory maps are allowed only as fakes, tests, or non-authoritative
accelerators. Production product authority must be durable Reborn state, not
V1 pending maps.

## Durable Production Slice (#4175)

Production product-auth records use the `ironclaw_auth` contract types and are
stored by the `ironclaw_reborn_composition` filesystem adapter under the normal
Reborn scoped filesystem substrate, rooted at the caller's
`/secrets/product-auth/{surface}` tree. The production factory constructs
`FilesystemAuthProductServices` over the same libSQL/PostgreSQL-backed
`ScopedFilesystem` and `SecretStore` used by the rest of Reborn; callers no
longer need to inject `InMemoryAuthProductServices` or an external product-auth
facade for production.

The storage-home decision is deliberately **not** to make
`ironclaw_secrets::CredentialAccountStore` own product-auth UX records. Runtime
credential broker accounts and product-auth account records have different
semantics. Product-auth durable records store provider id, label, ownership,
owner extension, grants, status, provider scopes, and access/refresh secret
handles directly in filesystem JSON records; raw manual-token values and
provider token values are stored only through `SecretStore` and referenced by
handles.

Indexing is path-first for this slice:

```text
/secrets/.../product-auth/{surface}/flows/{flow_id}.json
/secrets/.../product-auth/{surface}/interactions/{interaction_id}.json
/secrets/.../product-auth/{surface}/accounts/{account_id}.json
```

Account lookup by provider/scope/extension/status is implemented by scoped
account listing plus in-memory filtering over redacted metadata. This keeps raw
secret values, host paths, and backend details out of query indexes while the
record volume is still small. If product-auth account volume grows, add
filesystem record indexes for provider/status/owner/grants without indexing
secret handles or raw token material.

OAuth callback de-duplication is flow-id based. Callback-created accounts use
a deterministic account id derived from the flow id; bound reauthorization
updates the pre-authorized account id captured in the flow. Completed flows
return their stored credential account id on callback retry/replay, so provider
code is not re-exchanged after a completed durable claim.

Google OAuth production config is resolved by host composition before provider
client construction. Injected `OAuthClientConfig` is the canonical production
source for client id, optional client secret, and redirect URI in this slice.
Legacy Google tool environment variables remain bootstrap compatibility for
older v1/v2 tool paths only until a Reborn settings-config resolver explicitly
adopts and documents them. Production startup rejects malformed OAuth config at
construction time through the typed `OAuthClientConfig` validators.

The HA-safe PKCE decision for this slice is documented sticky callback routing:
the mounted WebUI OAuth route keeps raw PKCE verifiers in process-local bounded
state while durable flow records store only hashes. Single-instance or sticky
callback deployments are supported. Multi-replica/restart-surviving deployments
must add a host-owned encrypted verifier store before claiming HA safety.

---

## Auth Flows

`AuthFlowRecord` tracks integration credential setup and future identity-login
flows:

```text
AuthFlowKind::{integration_credential, identity_login}
AuthFlowStatus::{pending, awaiting_user, callback_received, completing,
                 completed, failed, expired, canceled}
AuthChallenge::{oauth_url, manual_token_required, account_selection_required,
                setup_required, reauthorize_required}
AuthContinuationRef::{setup_only, lifecycle_activation, turn_gate_resume,
                     product_action_resume}
```

Rules:

- OAuth redirect challenges carry a validated HTTPS `OAuthAuthorizationUrl`.
- Browser-facing OAuth start routes must derive scope from authenticated
  host context and trusted installation defaults. They must not accept
  caller-supplied `AuthFlowKind` or `AuthContinuationRef`; route-specific
  code chooses the allowed flow kind and continuation.
- Durable records may store state/verifier/code hashes, ids, handles, and
  redacted metadata only.
- Raw OAuth state, authorization code, PKCE verifier, access token, refresh
  token, and provider response bodies must not be serialized or projected.
- The first mounted WebUI OAuth route keeps the raw PKCE verifier in a bounded,
  expiring process-local cache keyed by `AuthFlowId`, while the durable flow
  record stores only the verifier hash. This is a single-instance first-slice
  constraint: multi-replica or restart-surviving deployments must introduce a
  host-owned encrypted verifier store or equivalent sticky callback mechanism
  before treating the route as HA-safe.
- Public callbacks must validate and claim the scoped flow/state/provider/PKCE
  hash before exchanging raw code/verifier through non-serializable one-shot
  provider inputs and completing the flow.
- Callback completion emits typed continuations; callback routes must not
  directly activate extensions, resume turns, replay messages, or dispatch work.
- Terminal flows cannot be completed or canceled again.

---

## Credential Accounts

Product surfaces refer to `CredentialAccountId` and redacted account metadata,
not loose secret names.

Credential accounts carry:

```text
provider
label
status
ownership
owner_extension?
granted_extensions[]
access_secret?
refresh_secret?
ProviderScope[]
```

Statuses are `configured`, `inactive`, `missing`, `expired`, `refresh_failed`,
`revoked`, and `pending_setup`.

Ownership classes are `extension_owned`, `user_reusable`,
`shared_admin_managed`, and `system`.

Rules:

- `extension_owned` accounts require `owner_extension`.
- Model/tool requests may express provider/capability intent, but cannot invent
  or bind arbitrary account ids.
- Recovery projections return stable UI-safe states:
  `configured`, `setup_required`, `reauthorize_required`, and
  `account_selection_required`.
- Account-selection challenges and recovery projections carry redacted account
  projections, not loose account-id lists.
- Recovery reasons are stable categories only: missing accounts, pending setup,
  expired credentials, refresh failures, revoked credentials, inactive accounts,
  and ambiguous account choices. Empty authorized choices use the same public
  missing-account reason whether no accounts exist or only unauthorized accounts
  exist, so recovery projections do not reveal hidden account existence.
  Backend errors, provider response bodies, host paths, state tokens, secret
  names, leases, and raw tokens must not appear in recovery projections.
- If policy cannot choose a unique configured account, return
  `account_selection_required` instead of guessing.
- Explicit account choice must go through `select_configured_account`, which
  revalidates scope, provider, configured status, ownership, and requester
  grants before returning a redacted projection. A raw `CredentialAccountId` is
  never authority by itself.
- Account lookup and listing requests carry requester extension identity and
  apply the same ownership/grant filter before returning records or redacted
  projections.
- Admin/shared credentials must be explicit accounts/grants, not implicit
  `default` fallback authority.
- Account updates must name the target `CredentialAccountId` and preserve the
  existing ownership/grant authority. Matching label/provider/scope is not
  enough to replace handles or ownership.
- OAuth callback account updates must be bound to a pre-authorized
  `CredentialAccountUpdateBinding` on the flow before provider exchange
  completion.
- Account listing uses explicit limit/cursor pagination and returns only
  authorized redacted projections.
- Credential refresh must go through `CredentialAccountService::refresh_account`
  or `RebornProductAuthServices::refresh_credential_account`. Refresh requests
  revalidate scope, provider, account status, ownership, and requester grants;
  a refreshable account id is never authority by itself.
- Refresh is allowed only for recoverable/configured accounts that still carry
  refresh authority. Revoked, inactive, pending-setup, missing, cross-scope, or
  unauthorized accounts fail closed even if a stale refresh handle still exists.
- Refresh results project only redacted account metadata and stable recovery
  state. They must not expose raw provider error text, response bodies, host
  paths, access-token handles, refresh-token handles, or secret values.

### Runtime Credential Consumers

Product-auth accounts are the account-selection and recovery authority for
runtime credential consumers; runtime lanes still consume only host-staged
secret material. Composition resolves this boundary as follows:

- GSuite first-party capabilities continue to choose a Google account
  dynamically through `CredentialAccountService` because the selected access
  secret is account-specific. Reborn composition stages that selected access
  secret through the host-runtime `InjectSecretOnce` obligation handler before
  first-party HTTP egress consumes the `StagedObligation`. GSuite should not
  publish static `runtime_credentials` declarations until a manifest credential
  handle can name a stable product-auth account binding without bypassing
  account selection, refresh, or recovery.
- GitHub first-party WASM starts as a manual-token product-auth provider. The
  manifest handle `github_token` is the runtime credential declaration; host
  composition maps that handle to an authorized configured GitHub account and
  stages that account's access secret for the declared `api.github.com`
  audience only.
- GSuite first-party handlers receive an explicit credential stager backed by
  host-runtime product-auth ports. Composition does not synthesize an
  `ExecutionContext`; host runtime owns the scoped one-shot secret handoff into
  `RuntimeCredentialSource::StagedObligation`.
- MCP HTTP/SSE auth is modeled as server-scoped product-auth account
  selection. A host-owned MCP server entry supplies the provider/server auth
  requirement; composition selects the account before building a runtime egress
  plan. MCP protocol code must not choose accounts.
- When multiple authorized accounts match a GitHub extension or MCP server,
  account selection returns `account_selection_required`. Consumers must not
  pick the first account as a fallback.
- Missing, expired, revoked, refresh-failed, inactive, unauthorized, or
  ambiguous credentials project typed auth recovery (`setup_required`,
  `reauthorize_required`, or `account_selection_required`) so product workflow
  can surface `AuthRequired` and resume the blocked turn. Non-recoverable
  backend failures return stable fail-closed errors.
- Refresh-on-use is provider-specific behind
  `CredentialAccountService::refresh_account` and provider clients. Runtime
  egress and MCP/WASM/first-party consumers must not implement generic token
  refresh or inspect provider token material.
- Header and query-param credential targets are the preferred first production
  targets. Path-placeholder targets are supported by host egress for
  compatibility, but consumers should use them only when an upstream protocol
  explicitly requires URL path placement.

---

## Manual Token Setup

Manual token values are submitted through secure secret interactions:

```text
request_secret_input -> AuthChallenge::manual_token_required
submit_manual_token  -> SecretSubmitResult { account_id, status, continuation }
```

Host-owned routes should call
`RebornProductAuthServices::request_manual_token_setup` to mint the typed
challenge and `RebornProductAuthServices::submit_manual_token` with a
non-serializable `RebornManualTokenSubmitRequest` after reading the dedicated
secret-submit body. Routes must not pass manual-token material through chat
commands, model-visible messages, product projections, route DTOs, or logs.
The setup request is also not a route DTO: host routes must construct its
`AuthProductScope` from authenticated caller/session context, and may attach a
pre-authorized `CredentialAccountUpdateBinding` when the secret submit is
intended to update an existing scoped account.

Rules:

- Raw token values must not enter model transcript, tool arguments, durable
  chat history, projections, debug output, or errors.
- Manual-token account updates must be bound to a pre-authorized account update
  binding before the challenge is minted.
- Cross-scope submit attempts must not consume another user's pending
  interaction.
- Empty, expired, malformed, or cross-scope submissions fail closed with stable
  errors.

---

## OAuth Provider Exchange

Provider exchange uses `AuthProviderClient` and one-shot request types:

```text
OAuthProviderCallbackRequest {
  raw authorization_code,
  authorization_code_hash,
  raw pkce_verifier,
  pkce_verifier_hash,
  provider,
  account_label,
  ProviderScope[]
}

OAuthProviderRefreshRequest {
  provider,
  account_id,
  refresh_secret,
  ProviderScope[]
}
```

The request types are intentionally not serializable. Exchange/refresh results
are safe to store because they contain only hashes, handles, ids, scopes, and
redacted metadata.

Production implementations must route provider HTTP through Reborn
network/egress policy, use bounded retry/backoff and per-account/provider rate
limits, and return sanitized errors. Refresh implementations must never log raw
refresh tokens, access tokens, provider response bodies, authorization codes,
PKCE verifiers, or backend secret handles; provider diagnostics must be mapped
to stable categories before they reach route responses, projections, traces, or
audit logs.

Refresh writes must be stale-safe. A production account store should use an
optimistic version, token generation, refresh-handle equality check, or an
equivalent compare-and-swap guard so a late refresh response cannot overwrite
newer credentials. Similarly, a failed refresh should mark an account
`refresh_failed` only if the account is still in the same pre-refresh
generation that initiated the provider call.

The composition root may expose an in-memory product-auth bundle only for
local-dev/testing. Production profiles must receive durable Reborn-native auth
services explicitly; they must not fall back to V1 pending maps, V1 route
state, or V1 secret stores as product authority.

---

## Cleanup

Cleanup is ownership-aware:

| Event | Extension-owned account | User reusable/shared account |
| --- | --- | --- |
| `deactivate` | retain account metadata, remove active visibility/grants | remove extension grant/visibility only |
| `uninstall` | revoke/delete/tombstone owned account and grants | keep account, remove extension grant/visibility |

Reports contain account ids and stable quarantine categories only, never secret
handles or backend detail strings. If a revoke, grant removal, tombstone, or
backend cleanup step cannot be completed safely, the report must quarantine the
affected account id and leave account metadata/grants unchanged rather than
pretending cleanup succeeded.

---

## Product Facing HTTP Surfaces (#4201)

The Reborn composition mounts host-owned HTTP routes that enter
`RebornProductAuthServices` (see
`crates/ironclaw_reborn_composition/src/product_auth_serve/mod.rs`). All mutation
routes share the same `LocalGateway` + `BearerToken` + per-caller body and
rate-limit posture as the original `oauth/start` route and derive
`AuthProductScope` from the authenticated caller, never from caller-supplied
tenant/user fields.

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/api/reborn/product-auth/oauth/start` | Open an OAuth setup flow; returns redacted authorization URL + invocation scope. |
| `GET`  | `/api/reborn/product-auth/oauth/callback/{flow_id}` | Public OAuth callback; validates scope/state hash before any product effect. |
| `POST` | `/api/reborn/product-auth/oauth/google/start` | Open a Google product-auth setup flow from configured Reborn Google OAuth client metadata; returns a Google authorization URL with PKCE/offline consent and invocation scope. |
| `GET`  | `/api/reborn/product-auth/oauth/google/callback` | Public static Google OAuth callback; resolves flow/scope from auth-owned encoded state, validates the durable state hash/PKCE claim, and completes through `RebornProductAuthServices`. |
| `POST` | `/api/reborn/product-auth/manual-token/submit` | One-shot manual-token setup + secret-submit (legacy WebUI shape, compatibility). |
| `POST` | `/api/reborn/product-auth/manual-token/setup` | Mint a manual-token interaction challenge; returns `interaction_id` + `invocation_id`. |
| `POST` | `/api/reborn/product-auth/manual-token/secret-submit` | Submit the raw token for an existing `interaction_id`; model transcript, tool arguments, logs, and durable events only ever see the redacted `credential_submitted` / `auth_failed` projection. |
| `POST` | `/api/reborn/product-auth/accounts/list` | List redacted credential account projections for a provider. |
| `POST` | `/api/reborn/product-auth/accounts/select` | Select a single configured account by id; returns its redacted projection. |
| `POST` | `/api/reborn/product-auth/accounts/recovery` | Project the stable recovery state for a provider (configured / setup_required / reauthorize_required / account_selection_required). |
| `POST` | `/api/reborn/product-auth/accounts/refresh` | Refresh / reauthorize an account; returns `CredentialRefreshReport` + projected recovery state. |
| `POST` | `/api/reborn/product-auth/lifecycle/cleanup` | Apply ownership-aware deactivate/uninstall cleanup for an extension; returns a redacted `SecretCleanupReport`. |

Rules:

- `secret-submit` is the only product-facing entry point for raw manual-token
  material. The raw token never enters tool arguments, model transcript,
  durable chat history, projections, debug output, or errors; only redacted
  `credential_submitted` / `auth_failed` projections cross the boundary.
- Manual-token setup and secret-submit are linked by `interaction_id` plus an
  `invocation_id` round-tripped through the browser, matching the OAuth
  start/callback pattern.
- Google OAuth setup is configured in the Reborn host process from env-only
  values: `IRONCLAW_REBORN_GOOGLE_CLIENT_ID`,
  `IRONCLAW_REBORN_GOOGLE_OAUTH_REDIRECT_URI`, optional
  `IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET`, and optional
  `IRONCLAW_REBORN_GOOGLE_HOSTED_DOMAIN_HINT`. For bootstrap compatibility, Reborn
  also accepts `GOOGLE_CLIENT_ID`, `GOOGLE_OAUTH_REDIRECT_URI`,
  `GOOGLE_CLIENT_SECRET`, and `GOOGLE_ALLOWED_HD` as a hosted-domain hint when
  the redirect URI opt-in is present. The hint only adds Google's `hd=`
  authorization parameter; product-auth setup does not treat it as a server-side
  domain allowlist. The redirect URI must match the static Google callback route
  exposed by the WebUI listener.
- All routes project only adapter-safe DTOs (`CredentialAccountProjection`,
  `CredentialAccountListPage`, `CredentialRecoveryProjection`,
  `CredentialRefreshReport`, `SecretCleanupReport`). Raw secret handles,
  backend error strings, and host paths must not be projected.
- These routes are an explicit exemption from the "everything goes through
  tools" `ToolDispatcher::dispatch()` invariant. Product-auth HTTP is a
  host-owned auth/secret-ingress boundary: credential setup, secure
  secret-submit, recovery, refresh, and lifecycle cleanup are not in-turn
  tool calls and must not surface raw secrets through the model-visible tool
  dispatch path. Routes still derive scope from authenticated caller context
  and enter `RebornProductAuthServices`.

---

## V1 Behavior Inventory

| Product behavior | V1 evidence path | Reborn owner | First-slice status |
| --- | --- | --- | --- |
| Extension/provider OAuth start | `src/extensions/manager.rs`, `src/auth/mod.rs` | `AuthFlowManager`, `CredentialSetupService`, `AuthProviderClient` | Reborn-native route mounted in composition; production provider wiring deferred |
| Hosted OAuth callback | `src/channels/web/features/oauth/mod.rs` | `RebornProductAuthServices::handle_oauth_callback`, `ProductAuthTurnGateResumeDispatcher` for turn-gate resume continuations | Reborn-native route mounted in composition; production provider wiring deferred |
| Local OAuth callback | `src/extensions/manager.rs`, `src/auth/oauth.rs` | `AuthFlowManager`, `AuthProviderClient` | Inventory only |
| Manual token entry from chat | `src/agent/agent_loop.rs`, `src/agent/thread_ops.rs` | `AuthInteractionService` secure submit via `RebornProductAuthServices::{request_manual_token_setup,submit_manual_token}` | Reborn facade ready; route migration deferred |
| Engine/gate auth credential submit | `src/bridge/router.rs` | `AuthInteractionService`, `CredentialSetupService`, typed continuation | Contracted; migration deferred |
| Extension/channel setup token storage | `src/extensions/manager.rs` | `CredentialSetupService`, `CredentialAccountService` | Contracted; migration deferred |
| MCP OAuth/DCR/discovery/refresh | `src/tools/mcp/auth.rs` | `AuthFlowManager`, `AuthProviderClient`, `CredentialAccountService` | Inventory only |
| HTTP credential injection | `src/tools/builtin/http.rs`, `src/tools/wasm/credential_injector.rs` | Secret broker/session and host-mediated egress | Out of scope for first slice |
| Token refresh before use | `src/auth/mod.rs` | broker/session/pre-injection behavior with status projection | Status vocabulary contracted |
| Admin secrets UI | `src/channels/web/handlers/secrets.rs` | `CredentialAccountService` plus secret repository/broker | Inventory only |
| Model-facing secret tools | `src/tools/builtin/secrets_tools.rs` | redacted account projections and authorized management actions | Inventory only |
| Setup wizard credential entry | `src/setup/wizard.rs` | setup/admin surface over `CredentialSetupService` | Inventory only |
| Extension uninstall/deactivate cleanup | `src/extensions/manager.rs` | `SecretCleanupService` | Contracted with fake tests |

---

## First-Slice Tests

`ironclaw_auth` contract tests cover:

- OAuth start, provider exchange, callback success, callback replay, stale,
  canceled, malformed, denied, and cross-scope callback behavior;
- secure manual-token submit, bound account update, cross-scope denial, empty
  input, expiry, and debug redaction;
- composition-facade manual-token request/submit, stale/duplicate/malformed
  failures, bound account update, sanitized backend/canceled errors, and no
  raw-token exposure in debug output, serialized responses/errors, or account
  projections;
- missing, refresh-failed, single-account, and multi-account selection states;
- credential recovery states for configured, missing, pending setup, inactive,
  expired, refresh-failed, revoked, ambiguous, and hidden unauthorized accounts;
- explicit account-choice validation plus lookup, listing, extension-owned, and
  shared-admin grant filtering;
- refresh success/failure, terminal-status rejection, stale concurrent refresh
  guards, request redaction, and scope/provider/grant revalidation;
- extension-owned owner validation, deactivate/uninstall cleanup behavior, and
  cleanup quarantine reporting;
- serde validation for newtypes and snake_case wire enums;
- serialization checks proving raw code/verifier/token material is absent.

---

## AuthPromptView v2 enrichment (issue #4112)

`AuthPromptView` in `crates/ironclaw_product_adapters/src/outbound.rs` carries
five new optional fields added in #4112 for WebUI v2 OAuth/PAT rendering:

| Field | Type | Present when |
|---|---|---|
| `challenge_kind` | `"oauth_url" \| "manual_token" \| "other"` | Projection finds a matching auth-flow record, or a blocked turn carries exactly one runtime credential auth requirement |
| `provider` | `string` | Same as above |
| `account_label` | `string \| null` | `ManualToken` challenge only |
| `authorization_url` | `string \| null` | `OAuthUrl` challenge only |
| `expires_at` | RFC-3339 string or `null` | When the flow has a bounded TTL |

All fields are `#[serde(default, skip_serializing_if = "Option::is_none")]`.
**Existing serialised rows without these fields round-trip safely** — they
deserialise as `None` on both ends. V1 channels that persist or replay
`AuthPromptView` are unaffected.

When no auth-flow record exists, WebUI v2 projection may still render a
manual-token prompt from a single host-runtime credential auth requirement. That
fallback exposes only the provider id and account label needed to route secure
manual-token submit; raw token values still cross only the dedicated
secret-submit route.

### Redaction invariant

`authorization_url` is the opaque IDP authorization URL already surfaced in
the legacy `AppEvent::OnboardingState.auth_url` field. It is safe to render
in the browser. **None of the following ever appear in this view:**
PKCE verifier, opaque state, client secret, auth code, access token, refresh
token, `interaction_id`.

### WebUI v2 consumer contract

`gates.js::gateFromEvent` reads these fields into the gate object:
- `challengeKind` ← `prompt.challenge_kind || "manual_token"` (fallback)
- `authorizationUrl` ← `prompt.authorization_url || null`
- `expiresAt` ← `prompt.expires_at || null`

`chat.js` dispatches by `challengeKind`:
- `"oauth_url"` → `AuthOauthCard` (new in #4112; opens IDP URL in a new tab)
- `"manual_token"` → `AuthTokenCard` (existing manual-token form; also used for
  legacy prompts that omit `challenge_kind`)
- `"other"` or any explicit unknown value → `AuthGenericCard`

### Wire-shape tests

`crates/ironclaw_reborn_composition/tests/webui_v2_product_auth_4201.rs`
covers:
- Serialisation of the new optional fields when present.
- Omission of all new fields when absent (backward-compat check).
- Round-trip deserialisation of legacy rows without any new fields.
- `challenge_for_gate` returns an `AuthChallengeView` for a seeded OAuth flow.
- `challenge_for_gate` returns `None` for mismatched owner/scope/run/gate refs
  or terminal flows.
- `as_auth_challenge_provider` returns `None` when no flow record source.
