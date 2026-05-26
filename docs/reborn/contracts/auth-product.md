# Reborn Product Auth Contract

- **Status:** contract and composition seam
- **Issue:** #3289 / #3810 / #3811 / #3812 / #3883 / #3884
- **Crate:** `crates/ironclaw_auth`
- **Composition:** `ironclaw_reborn_composition::RebornProductAuthServices`

---

## Purpose

Product-facing auth is the user/operator workflow for setting up, recovering,
selecting, refreshing, and cleaning up credentials for integrations,
providers, extensions, MCP servers, WASM tools/channels, and future identity
login flows.

This slice is contract-first. It defines Reborn-native vocabulary and fake
services, #3811 adds a Reborn composition seam, #3812 adds callback completion
handling for host-mounted Reborn OAuth callback routes, and #3884 adds the
product-auth token refresh and cleanup lifecycle contract. It does not migrate
production extension setup routes, CLI/setup flows, durable secret storage,
runtime HTTP injection, or runtime credential injection.

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
| `CredentialAccountService` | account metadata, ownership, grants, status, redacted projections, refresh lifecycle reports | raw access/refresh token material or raw provider diagnostics |
| `AuthProviderClient` | one-shot OAuth provider exchange/refresh vocabulary over host egress | product workflow or route state |
| `SecretCleanupService` | ownership-aware uninstall/deactivate cleanup and quarantine reports | deleting reusable/shared accounts by default |

Low-level encrypted storage, leases, host-mediated HTTP credential injection,
approval interaction UI, and no-exposure enforcement remain owned by their
respective Reborn substrate contracts.

`RebornProductAuthServices` is the single composition bundle for the product
auth ports above. WebUI/setup/extension surfaces should call this bundle once
routes are migrated instead of reconstructing auth-flow stores, credential
stores, provider clients, or cleanup services locally.
Refresh and lifecycle cleanup callers should enter through
`RebornProductAuthServices::refresh_credential_account` and
`RebornProductAuthServices::cleanup_credentials_for_lifecycle`, which delegate
to injected Reborn-native product-auth ports. Production profiles must inject
real durable/provider implementations; local-dev may use the in-memory fake.

Host-owned OAuth callback routes should parse and validate their HTTP input,
derive hashes for opaque state/code/verifier values, then call
`RebornProductAuthServices::handle_oauth_callback`. The handler claims the flow
through `AuthFlowManager`, performs provider exchange through
`AuthProviderClient`, completes the auth flow through `AuthFlowManager`, and
dispatches an `AuthContinuationEvent` to the injected continuation dispatcher.
If continuation dispatch fails, the handler returns a sanitized retryable error
instead of reporting callback success; retrying an already-completed callback
may re-dispatch the typed continuation without re-exchanging provider code.
Callback route code must not activate extensions, resume turns, replay prompts,
or dispatch runtime work directly.

`ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher` is the
product-workflow bridge for `AuthContinuationRef::TurnGateResume`. It converts
that specific typed auth continuation into a `TurnCoordinator::resume_turn` call
using the canonical turn scope, actor, run id, and gate ref carried by the auth
event. It does not define auth state, credential vocabulary, or generic
continuation dispatch. Setup-only, lifecycle-activation, and product-action
continuations remain explicit non-turn cases for their owning handlers and must
not be performed inline by the OAuth callback route.

The blocked-run interaction loop is separate: #3094 owns listing/rendering
approval/auth gates from blocked run-state and routing user decisions back into
the trusted resume path. That issue should consume the auth-flow boundary here
for auth gates; it must not create a second credential-account or OAuth-flow
model.

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
- Durable records may store state/verifier/code hashes, ids, handles, and
  redacted metadata only.
- Raw OAuth state, authorization code, PKCE verifier, access token, refresh
  token, and provider response bodies must not be serialized or projected.
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
  ambiguous account choices, and missing requester grants. Backend errors,
  provider response bodies, host paths, state tokens, secret names, leases, and
  raw tokens must not appear in recovery projections.
- If policy cannot choose a unique configured account, return
  `account_selection_required` instead of guessing.
- Explicit account choice must go through `select_configured_account`, which
  revalidates scope, provider, configured status, ownership, and requester
  grants before returning a redacted projection. A raw `CredentialAccountId` is
  never authority by itself.
- Admin/shared credentials must be explicit accounts/grants, not implicit
  `default` fallback authority.
- Account updates must name the target `CredentialAccountId` and preserve the
  existing ownership/grant authority. Matching label/provider/scope is not
  enough to replace handles or ownership.
- OAuth callback account updates must be bound to a pre-authorized
  `CredentialAccountUpdateBinding` on the flow before provider exchange
  completion.
- Account listing uses explicit limit/cursor pagination and returns redacted
  projections only.

---

## Manual Token Setup

Manual token values are submitted through secure secret interactions:

```text
request_secret_input -> AuthChallenge::manual_token_required
submit_manual_token  -> SecretSubmitResult { account_id, status, continuation }
```

Rules:

- Raw token values must not enter model transcript, tool arguments, durable
  chat history, projections, debug output, or errors.
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
```

The request is intentionally not serializable. The exchange result is safe to
store because it contains only hashes, handles, ids, scopes, and redacted
metadata.

Future production implementations must route provider HTTP through Reborn
network/egress policy and return sanitized errors.

The composition root may expose an in-memory product-auth bundle only for
local-dev/testing. Production profiles must receive durable Reborn-native auth
services explicitly; they must not fall back to V1 pending maps, V1 route
state, or V1 secret stores as product authority.

---

## Token Refresh

Token refresh is requested through `CredentialAccountService::refresh_account`
and provider work is delegated to `AuthProviderClient::refresh_token`:

```text
CredentialRefreshRequest {
  AuthProductScope,
  provider,
  account_id,
  requester_extension?
}

OAuthProviderRefreshRequest {
  provider,
  account_id,
  refresh_secret handle,
  ProviderScope[]
}
```

Rules:

- Product/WebUI/extension callers must not perform raw provider HTTP, read
  refresh-token values, or infer refresh authority from an account id alone.
- Product callers enter through `RebornProductAuthServices`, not by
  reconstructing credential stores, provider clients, or refresh state locally.
- Refresh revalidates scope, provider, account status, ownership, and requester
  grants before provider exchange.
- One-shot provider refresh input is not serializable and redacts the refresh
  handle in debug output.
- Successful refresh stores returned secret handles and returns a
  `CredentialRefreshReport` with a redacted account projection plus stable
  recovery state.
- Refresh failures that mean the user/operator must act become
  `refresh_failed` account status and `reauthorize_required` recovery state.
  Reports and errors must not contain provider response bodies, backend details,
  secret handles, host paths, raw tokens, or leases.

---

## Cleanup

Cleanup is ownership-aware:

| Event | Extension-owned account | User reusable/shared account |
| --- | --- | --- |
| `deactivate` | retain account metadata, remove active visibility/grants | remove extension grant/visibility only |
| `uninstall` | revoke/delete/tombstone owned account and grants | keep account, remove extension grant/visibility |

Cleanup must be idempotent. Partial cleanup failures are reported as stable
quarantine categories, such as `revoke_failed`, `grant_revoke_failed`,
`tombstone_failed`, or `backend_unavailable`, and the affected account is left
unchanged for explicit follow-up. Reports contain account ids and stable
categories only, never secret handles or backend detail strings.
Product callers enter through `RebornProductAuthServices` so lifecycle
surfaces do not depend on V1 extension-manager cleanup or route-local secret
authority.

---

## V1 Behavior Inventory

| Product behavior | V1 evidence path | Reborn owner | First-slice status |
| --- | --- | --- | --- |
| Extension/provider OAuth start | `src/extensions/manager.rs`, `src/auth/mod.rs` | `AuthFlowManager`, `CredentialSetupService`, `AuthProviderClient` | Contracted; production migration deferred |
| Hosted OAuth callback | `src/channels/web/features/oauth/mod.rs` | `RebornProductAuthServices::handle_oauth_callback`, `ProductAuthTurnGateResumeDispatcher` for turn-gate resume continuations | Reborn handler seam ready; HTTP route mounting deferred |
| Local OAuth callback | `src/extensions/manager.rs`, `src/auth/oauth.rs` | `AuthFlowManager`, `AuthProviderClient` | Inventory only |
| Manual token entry from chat | `src/agent/agent_loop.rs`, `src/agent/thread_ops.rs` | `AuthInteractionService` secure submit | Contracted; route migration deferred |
| Engine/gate auth credential submit | `src/bridge/router.rs` | `AuthInteractionService`, `CredentialSetupService`, typed continuation | Contracted; migration deferred |
| Extension/channel setup token storage | `src/extensions/manager.rs` | `CredentialSetupService`, `CredentialAccountService` | Contracted; migration deferred |
| MCP OAuth/DCR/discovery/refresh | `src/tools/mcp/auth.rs` | `AuthFlowManager`, `AuthProviderClient`, `CredentialAccountService` | Inventory only |
| HTTP credential injection | `src/tools/builtin/http.rs`, `src/tools/wasm/credential_injector.rs` | Secret broker/session and host-mediated egress | Out of scope for first slice |
| Token refresh before use | `src/auth/mod.rs` | `CredentialAccountService::refresh_account`, `AuthProviderClient::refresh_token`, broker/session/pre-injection behavior with status projection | Product-auth refresh lifecycle contracted; production egress/storage deferred |
| Admin secrets UI | `src/channels/web/handlers/secrets.rs` | `CredentialAccountService` plus secret repository/broker | Inventory only |
| Model-facing secret tools | `src/tools/builtin/secrets_tools.rs` | redacted account projections and authorized management actions | Inventory only |
| Setup wizard credential entry | `src/setup/wizard.rs` | setup/admin surface over `CredentialSetupService` | Inventory only |
| Extension uninstall/deactivate cleanup | `src/extensions/manager.rs` | `SecretCleanupService` | Ownership-aware cleanup and quarantine reporting contracted; V1 manager cleanup not reused |

---

## First-Slice Tests

`ironclaw_auth` contract tests cover:

- OAuth start, provider exchange, callback success, callback replay, stale,
  canceled, malformed, denied, and cross-scope callback behavior;
- secure manual-token submit, cross-scope denial, empty input, expiry, and debug
  redaction;
- missing, refresh-failed, single-account, and multi-account selection states;
- credential recovery states for configured, missing, pending setup, inactive,
  expired, refresh-failed, revoked, ambiguous, and unauthorized accounts;
- explicit account-choice validation and shared-admin grant filtering;
- refresh success, recoverable refresh failure, redaction, and refresh
  scope/provider/grant revalidation through both auth-contract and composition
  facade entrypoints;
- extension-owned owner validation, deactivate/uninstall cleanup behavior,
  idempotent cleanup, and quarantine reporting through both auth-contract and
  composition facade entrypoints;
- serde validation for newtypes and snake_case wire enums;
- serialization checks proving raw code/verifier/token material is absent.
