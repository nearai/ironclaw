# Reborn Product Auth Contract

**Status:** first-slice contract draft  
**Issue:** #3289 / #3810  
**Crate:** `crates/ironclaw_auth`

---

## Purpose

Product-facing auth is the user/operator workflow for setting up, recovering,
selecting, refreshing, and cleaning up credentials for integrations,
providers, extensions, MCP servers, WASM tools/channels, and future identity
login flows.

This slice is contract-first. It defines Reborn-native vocabulary and fake
services, but does not migrate production OAuth routes, extension setup routes,
CLI/setup flows, durable secret storage, or runtime credential injection.

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

- Durable records may store state/verifier/code hashes, ids, handles, and
  redacted metadata only.
- Raw OAuth state, authorization code, PKCE verifier, access token, refresh
  token, and provider response bodies must not be serialized or projected.
- Public callbacks exchange raw code/verifier through non-serializable one-shot
  provider inputs before completing the flow.
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
scopes[]
```

Statuses are `configured`, `missing`, `expired`, `refresh_failed`, `revoked`,
and `pending_setup`.

Ownership classes are `extension_owned`, `user_reusable`,
`shared_admin_managed`, and `system`.

Rules:

- `extension_owned` accounts require `owner_extension`.
- Model/tool requests may express provider/capability intent, but cannot invent
  or bind arbitrary account ids.
- If policy cannot choose a unique configured account, return
  `account_selection_required` instead of guessing.
- Admin/shared credentials must be explicit accounts/grants, not implicit
  `default` fallback authority.

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
  scopes
}
```

The request is intentionally not serializable. The exchange result is safe to
store because it contains only hashes, handles, ids, scopes, and redacted
metadata.

Future production implementations must route provider HTTP through Reborn
network/egress policy and return sanitized errors.

---

## Cleanup

Cleanup is ownership-aware:

| Event | Extension-owned account | User reusable/shared account |
| --- | --- | --- |
| `deactivate` | retain account metadata, remove active visibility/grants | remove extension grant/visibility only |
| `uninstall` | revoke/delete/tombstone owned account and grants | keep account, remove extension grant/visibility |

Reports contain account ids and redacted diagnostics only, never secret handles
or backend detail strings.

---

## V1 Behavior Inventory

| Product behavior | V1 evidence path | Reborn owner | First-slice status |
| --- | --- | --- | --- |
| Extension/provider OAuth start | `src/extensions/manager.rs`, `src/auth/mod.rs` | `AuthFlowManager`, `CredentialSetupService`, `AuthProviderClient` | Contracted; production migration deferred |
| Hosted OAuth callback | `src/channels/web/features/oauth/mod.rs` | `AuthFlowManager`, continuation sink | Contracted; Reborn-native callback route deferred |
| Local OAuth callback | `src/extensions/manager.rs`, `src/auth/oauth.rs` | `AuthFlowManager`, `AuthProviderClient` | Inventory only |
| Manual token entry from chat | `src/agent/agent_loop.rs`, `src/agent/thread_ops.rs` | `AuthInteractionService` secure submit | Contracted; route migration deferred |
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
- secure manual-token submit, cross-scope denial, empty input, expiry, and debug
  redaction;
- missing, refresh-failed, single-account, and multi-account selection states;
- extension-owned owner validation and deactivate/uninstall cleanup behavior;
- serde validation for newtypes and snake_case wire enums;
- serialization checks proving raw code/verifier/token material is absent.
