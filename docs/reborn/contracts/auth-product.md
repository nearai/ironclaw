# Reborn Product Auth Contract

**Status:** First-slice contract draft  
**Issue:** #3289 / #3810  
**Crate:** `crates/ironclaw_auth`  
**Depends on:** `docs/reborn/contracts/host-api.md`, `docs/reborn/contracts/secrets.md`, `docs/reborn/contracts/network.md`, `docs/reborn/contracts/extensions.md`, `docs/reborn/contracts/migration-compatibility.md`

---

## 1. Purpose

Product-facing authentication is the user/operator workflow that lets IronClaw set up, recover, select, and clean up credentials for integrations, providers, extensions, MCP servers, WASM tools/channels, and future identity-login flows.

This contract defines the Reborn product auth boundary. It preserves existing V1 UX where safe, but moves product authority into typed services:

```text
AuthInteractionService
  -> AuthFlowManager
  -> CredentialSetupService
  -> CredentialAccountService
  -> AuthProviderClient / OAuthHttpEgress
  -> SecretBroker / SecretRepository
  -> ProductWorkflow continuation sink
  -> SecretCleanupService
```

This slice is contract-first. It does not migrate production routes yet.

---

## 2. Ownership boundaries

| Boundary | Owns | Must not own |
| --- | --- | --- |
| `AuthFlowManager` | durable scoped auth-flow records, callback consumption, flow status | raw provider HTTP, extension activation, message replay |
| `AuthInteractionService` | redacted auth-required projections, secure manual-token input interactions, cancel/retry UX | secret persistence internals, model-visible token transport |
| `CredentialSetupService` | creates/updates credential accounts from OAuth/manual setup results | durable encryption, runtime injection, extension lifecycle mutation |
| `CredentialAccountService` | typed account metadata, ownership, grants, redacted account projections | raw access/refresh token material |
| `AuthProviderClient` / `OAuthHttpEgress` | OAuth discovery/token exchange/refresh through host-managed egress and sanitized errors | product workflow or extension lifecycle state |
| `ProductWorkflow` continuation sink | setup-only/lifecycle/turn/product-action continuation routing | raw OAuth callback parsing, raw prompt replay |
| `SecretCleanupService` | ownership-aware uninstall/deactivate cleanup plans/results | deleting user-reusable/shared accounts by default |

Low-level encrypted secret storage, one-shot leases, brokered HTTP injection, approval authorization, and no-exposure enforcement remain owned by their Reborn substrate contracts.

---

## 3. Source of truth

V1 has multiple pending authorities, many in memory:

- `PendingOAuthRegistry`
- web login `OAuthStateStore`
- extension `pending_auth`
- engine `PendingGateStore`
- MCP OAuth helpers

Reborn product auth uses durable scoped records instead:

```text
AuthProductScope + AuthFlowId -> AuthFlowRecord
AuthProductScope + AuthInteractionId -> secure input interaction
AuthProductScope + CredentialAccountId -> CredentialAccount
```

In-memory maps are allowed only as fakes, caches, or non-authoritative accelerators.

---

## 4. Auth flows

`AuthFlowRecord` is the source of truth for browser/OAuth-style product flows.

Minimum fields:

```text
AuthFlowId
AuthProductScope
AuthFlowKind::{IntegrationCredential, IdentityLogin /* future */}
AuthFlowStatus::{Pending, AwaitingUser, CallbackReceived, Completing, Completed, Failed, Expired, Canceled}
AuthProviderId
AuthChallenge
AuthContinuationRef
CredentialAccountId?          # when a credential account is created/updated
opaque_state_hash?            # never raw state
pkce_verifier_hash?           # never raw verifier
AuthErrorCode?
created_at / updated_at / expires_at
```

Rules:

- OAuth state and PKCE verifier values must not be adapter-visible.
- Public callbacks validate and atomically consume flow records.
- Unknown, stale, malformed, provider-denied, and cross-scope callbacks produce stable sanitized errors.
- Callback completion emits typed continuation refs; callback routes must not directly activate extensions, resume turns, or replay messages.
- Identity login can share the substrate later, but the first implementation slice targets integration credential setup.

---

## 5. Credential accounts

V1 product paths primarily refer to loose secret names such as `github_token` or `google_oauth_token`.

Reborn product paths refer to `CredentialAccountId` and redacted metadata:

```text
CredentialAccount
  id
  scope
  provider
  label
  status
  ownership
  owner_extension?
  granted_extensions[]
  access_secret: SecretHandle?
  refresh_secret: SecretHandle?
  scopes[]
```

Statuses:

```text
Configured
Missing
Expired
RefreshFailed
Revoked
PendingSetup
```

Ownership classes:

```text
ExtensionOwned
UserReusable
SharedAdminManaged
System
```

Rules:

- Product projections never expose raw secret material.
- Adapter-safe projections should avoid exposing backend secret handle names unless explicitly required by a trusted backend adapter.
- Model/tool requests may express provider/capability intent, but cannot invent or bind arbitrary `CredentialAccountId`s.
- If policy cannot choose a unique authorized account, return `account_selection_required` instead of guessing.
- Admin/shared credentials must be explicit accounts/grants, not implicit `default` secret fallback.

---

## 6. Manual token setup

Chat may initiate setup, but token values must be collected through secure secret input interactions:

```text
AuthInteractionService::request_secret_input(...)
  -> AuthChallenge::ManualTokenRequired { interaction_id, ... }

secure submit endpoint / hidden CLI input / web secret field
  -> AuthInteractionService::submit_secret(...)
  -> SecretSubmitResult { account_id, status, continuation }
```

Rules:

- Raw token values must not enter model transcript, tool arguments, durable chat history, projections, or debug output.
- Model-visible output may say only redacted states such as `credential_submitted`, `auth_failed`, or `auth_required`.
- Empty/stale/cross-scope submissions fail closed with stable errors.

---

## 7. OAuth provider client and egress

Product routes and auth-flow services must not instantiate raw HTTP clients for OAuth exchange/refresh/discovery.

Future production code should use:

```text
AuthProviderClient
  -> OAuthHttpEgress
  -> ironclaw_network / host-managed egress policy
```

Rules:

- Enforce HTTPS/redirect/body-limit/proxy/SSRF behavior in one narrow auth egress boundary.
- Provider/backend errors are sanitized before product surfaces see them.
- Token refresh occurs in credential broker/session/pre-injection paths before approved use; product flows render `expired`, `refresh_failed`, or `reauthorize_required` states.

---

## 8. Continuations

Auth completion uses typed refs instead of raw message replay:

```text
AuthContinuationRef::SetupOnly
AuthContinuationRef::LifecycleActivation { package_ref }
AuthContinuationRef::TurnGateResume { turn_run_ref, gate_ref }
AuthContinuationRef::ProductActionResume { action_ref }
```

Rules:

- Continuation records must not store raw prompt/message content.
- ProductWorkflow handles continuation after auth completes.
- Callback routes only complete auth and enqueue/emit typed continuations.

---

## 9. Cleanup

Cleanup is ownership-aware:

| Event | Extension-owned account | User reusable account | Shared/admin account |
| --- | --- | --- | --- |
| Deactivate | revoke sessions/visibility, keep account metadata as appropriate | revoke extension binding/session only | revoke visibility/session only |
| Uninstall | revoke/delete/tombstone account and grants | keep account, remove extension grants/bindings | keep account, remove visibility/binding |

Rules:

- Cleanup is idempotent.
- Partial failures produce redacted quarantine diagnostics.
- Raw secret values and backend details do not appear in cleanup reports.

---

## 10. V1 behavior inventory

| Product behavior | V1 path | V1 source of truth | Reborn owner | First-slice status |
| --- | --- | --- | --- | --- |
| Extension/provider OAuth start | `src/extensions/manager.rs::{auth_wasm_tool, auth_mcp_server, start_secret_oauth_if_supported}` + `src/auth/mod.rs::build_pending_oauth_launch` | in-memory pending flow + settings/auth descriptors + loose secret name | `AuthFlowManager` + `CredentialSetupService` + `AuthProviderClient` | Contracted; production migration deferred |
| Hosted OAuth callback | `src/channels/web/features/oauth/mod.rs::oauth_callback_handler` | `PendingOAuthRegistry` keyed by state/flow id | `AuthFlowManager::complete_oauth_callback` + continuation sink | Contracted; route migration deferred |
| Local TCP OAuth callback | `src/extensions/manager.rs` spawned task + `src/auth/oauth.rs::wait_for_callback` | task handle + `pending_auth` map | `AuthFlowManager` + `AuthProviderClient` | Inventory only; migration deferred |
| Manual token entry from chat | `src/agent/agent_loop.rs` auth-mode intercept + `src/agent/thread_ops.rs::process_auth_token` | thread `pending_auth` | `AuthInteractionService` secure submit | Contracted; behavior migration deferred |
| Engine/gate auth credential submit | `src/bridge/router.rs::submit_pending_auth_credential` | `PendingGateStore` + extension/auth/secrets fallback chain | `AuthInteractionService` + `CredentialSetupService` + typed continuation | Contracted; production migration deferred |
| Extension/channel setup form token storage | `src/extensions/manager.rs::configure_token` and setup form handling | loose secret names under `SecretsStore` | `CredentialSetupService` + `CredentialAccountService` | Contracted; production migration deferred |
| MCP OAuth/DCR/discovery/refresh | `src/extensions/manager.rs::auth_mcp_server`, `src/tools/mcp/auth.rs`, OAuth helper code | MCP server config + secrets for token/client credentials | `AuthFlowManager` + `AuthProviderClient` + `CredentialAccountService` | Inventory only; migration deferred |
| HTTP credential injection | `src/tools/builtin/http.rs`, `src/tools/wasm/credential_injector.rs` | `SharedCredentialRegistry` + `SecretsStore` + optional refresh config | Secret broker/session + host-mediated runtime egress | Out of scope for #3289 first slice; composed by future migration |
| Token refresh before runtime use | `src/auth/mod.rs::resolve_secret_for_runtime` | loose secret + refresh token naming convention | broker/session/pre-injection behavior with status projection | Contracted status vocabulary; production migration deferred |
| Admin secret create/list/delete | `src/channels/web/handlers/secrets.rs` | `SecretsStore` records keyed by user/name | `CredentialAccountService` + secret repository/broker | Inventory only; migration deferred |
| Model-facing secret tools | `src/tools/builtin/secrets_tools.rs` | `SecretsStore` metadata/delete | redacted credential/account projections and authorized management actions | Inventory only; migration deferred |
| Setup wizard/provider credential entry | `src/setup/wizard.rs` | bootstrap config + settings + secrets | setup/admin surface over `CredentialSetupService` | Inventory only; migration deferred |
| Extension uninstall/deactivate cleanup | `src/extensions/manager.rs` cleanup helpers and registry removal | secret names, mapping registry, extension state | `SecretCleanupService` ownership-aware cleanup | Contracted with fake tests; production migration deferred |
| Auth-required UI projection | web SSE `OnboardingState`, bridge auth gates, chat statuses | per-surface event/session state | `AuthInteractionService` stable redacted projections | Contracted vocabulary; production migration deferred |

---

## 11. First-slice tests

`crates/ironclaw_auth` provides fake-service tests for:

- OAuth start/callback success and continuation enqueue;
- cross-scope callback denial;
- expired/stale callback;
- malformed callback;
- provider-denied callback;
- manual token secure submit without debug/projection leakage;
- missing/refresh-failed/account-selection credential states;
- typed continuations without raw replay content;
- ownership-aware idempotent cleanup.

These tests prove the contract shape only. Caller-level production-route tests are required when V1 routes are migrated.
