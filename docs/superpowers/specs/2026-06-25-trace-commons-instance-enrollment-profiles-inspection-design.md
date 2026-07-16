# Trace Commons: instance-wide enrollment, user profiles, and trace inspection

Status: proposed (2026-06-25)
Repos: `ironclaw` (client) + `trace-commons-server` (server)
Server reference branch: `contributor-account-slice1`

## Problem

Trace Commons is gaining three capabilities that IronClaw must integrate:

1. **Instance-wide enrollment** — an IronClaw deployment enrolls *once* and all
   of its users contribute under that enrollment, instead of every user (scope)
   redeeming an invite individually.
2. **User profiles / contributor accounts** — a human can hold a Trace Commons
   contributor account (public handle/bio, credit balance, browser session) and
   manage it from a web session minted by IronClaw.
3. **Trace inspection** — a user can read their *submitted* traces back from the
   server (list / detail / scrubbed content), complementing IronClaw's existing
   *local* held-trace review.

The existing per-scope **user-invite** onboarding must keep working unchanged and
must coexist with the new instance-wide model on the same instance.

## What already exists (reuse — do not rebuild)

### IronClaw side
- **Onboarding wire contract already matches the server.**
  `crates/ironclaw_reborn_traces/src/onboarding/protocol.rs` defines
  `ONBOARD_REQUEST_SCHEMA_VERSION = "trace_commons.onboard_request.v1"` and the
  response constant, identical to the server, with matching fields
  (`invite_code`, `device_public_key`, `client_info`).
- **Per-scope onboarding flow** (`onboarding/mod.rs`): parses the invite (trust
  root), generates a device keypair (`device_key.rs`), POSTs via an injectable
  `OnboardingHttpSink`, anchors the issuer origin, and writes a
  `StandingTraceContributionPolicy` to
  `trace_contribution_dir_for_scope(Some(scope))/policy.json`.
- **Pseudonymization helpers** (`contribution.rs`):
  `local_pseudonymous_contributor_id(scope)` and
  `local_pseudonymous_tenant_scope_ref(scope)` turn a `"{tenant}:{user}"` scope
  string into a stable opaque hash (`tenant_sha256:…`). This is the per-user
  subject mechanism — it already exists.
- **Trace contribution pipeline**: capture → redact → queue → hold → submit,
  plus credits, held-trace review, and `ContributionHttpSink` for host-routed
  egress. Web handlers in `src/channels/web/handlers/traces.rs`.
- **First-party Trace Commons tools** (`crates/ironclaw_host_runtime/src/first_party_tools/trace_commons.rs`):
  `onboard`, `status`, `credits`, `profile_token`, `profile_set` — model-visible
  capabilities routed through host egress + `ToolDispatcher`.

### Admin / multi-tenant primitives
- `UserRole::{Owner, Admin, Regular}` with centralized `is_admin()`
  (`src/ownership/mod.rs`).
- `AdminScope` (`src/tenant.rs`) — constructable only when `identity.is_admin()`;
  the fail-closed typed gate for admin operations (currently user management).
- `DeploymentMode::HostedMultiTenant`, `Config::is_multi_tenant_deployment()`,
  `TenantScope` compile-time isolation, per-tenant rate limiting.
- Secrets module (AES-256-GCM, OS keychain) for credential storage.

### Server side (trace-commons-server)
- Device-key onboarding (`crates/trace-commons-protocol/src/onboarding.rs`,
  migrations `V28__device_keys.sql`, `V29__onboarding_invites.sql`).
- Upload-claim issuer (`src/trace_upload_claim_issuer.rs`) — mints bearer claims;
  for device keys the JWT subject is currently fixed to `device_key_id`.
- Trace submission `POST /v1/traces`; stores `auth_principal_ref` from the
  bearer principal.
- Community profile (`PUT/DELETE /v1/community/profile`), credits
  (`GET /v1/contributors/me/credit`, `…/credit-events`,
  `POST /v1/contributors/me/submission-status`), public
  `GET /v1/community/leaderboard|contributors/{handle}|analytics/summary`.
- Admin device-key + tenant-access-grant management
  (`/v1/admin/device-keys`, `/v1/admin/tenant-access-grants`).
- **In progress (Slice 1, this branch):** contributor accounts —
  `trace_accounts`, `trace_account_principals`, `trace_login_links`,
  `trace_sessions` (planned `V30`), the `account_session.rs` module, and the
  `/v1/account/*` endpoints (login-links, traces list/detail/content, logout,
  revoke-all). Account is keyed `(tenant_id, principal_ref)`, `UNIQUE` per
  principal, "one principal per account" in Slice 1.

## Core decision: per-user identity under one instance device key

The server resolves a contributor account and stamps `auth_principal_ref`
**from the bearer principal**, which for a device key is fixed to the
`device_key_id`. Therefore one shared instance device key would collapse all
users into one account/principal. Per-user separation under a shared key
requires a small, additive server change (authorized by the maintainer).

The change runs in the **safe direction**: one instance device key *fanning out*
to many per-user accounts, all confined to the instance's own tenant by the
server's existing RLS. A compromised instance can only mis-attribute *within its
own tenant* — it can never reach another tenant. This is the trust boundary
IronClaw already owns (it authenticates its own users via `UserRole`/`TenantScope`).
This is distinct from — and far weaker than — the Slice 3 threat (linking many
principals *into* one account), which remains gated behind a strong authenticator.

## Two coexisting models, one resolver

Both onboarding models must function simultaneously on one instance. They differ
only in what a single new **trace-credential resolver** returns for a given
`(tenant, user)` / scope:

| Model | Device key source | `auth_principal_ref` | Subject sent | Account |
|-------|-------------------|----------------------|--------------|---------|
| **User invite** (existing) | the user's own onboarded key (per-scope `policy.json`) | bare `device:{tenant}:{key}` | No | 1:1, exactly as today |
| **Instance-wide** (new) | shared instance key (instance-level policy) | `instance:{tenant}:{key}:user:{subject}` | Yes (pseudonymized) | per-user under instance |

`subject = local_pseudonymous_contributor_id("{tenant}:{user_id}")` — opaque,
never the raw user id.

Everything downstream — claim minting, submission, login-links, trace inspection
— flows through the same code; only the resolver output differs.

**Precedence:** a user's own (personal-invite) enrollment wins over the instance
enrollment when both exist. A self-hoster who never sets up an instance
enrollment keeps working exactly as today; a managed-instance user who redeems a
personal invite can "bring their own" Trace Commons identity.

## Server changes (additive, backward-compatible)

All three make `subject` optional → absent reproduces today's behavior exactly.

1. **`TraceUploadClaimRequest`** (`src/trace_upload_claim_issuer.rs`): add
   `subject: Option<String>` (`#[serde(default)]`).
2. **`issue_claim_for_device_key`**: when `subject` is present, validate/normalize
   it (length, charset; it is already a `tenant_sha256:…`-style opaque token) and
   set both the JWT `sub` and the derived
   `auth_principal_ref = principal_storage_ref("instance:{tenant}:{device_key_id}:user:{subject}")`.
   When absent, keep the current `device_key_id` behavior.
3. **`POST /v1/account/login-links`** + `create_or_reuse_account`: accept the same
   optional `subject` so the resolved account principal matches the submitting
   principal. Request body gains an optional `subject` field; account resolution
   keys on the derived principal_ref, not the bare device principal.

No new tables. `trace_accounts` / `trace_account_principals` already key on
`(tenant_id, principal_ref)`; we only change what `principal_ref` is. Subject
namespacing (`instance:{tenant}:{device_key_id}:user:{subject}`) guarantees
subjects cannot collide across instances/tenants.

Server-side validation requirements:
- A device-key bearer may only request a `subject` claim for its own
  tenant/device namespace (enforced by the derivation, not by trusting a
  client-supplied prefix).
- Subject format validated against an explicit pattern; reject malformed.
- Audit (hash/label only) records subject-scoped mints distinctly from
  device-only mints.

## IronClaw changes

### 1. Trace-credential resolver (new seam)
A single function (in `ironclaw_reborn_traces`, host-side) that, given the
authenticated user/scope, returns `{ device_key_id, tenant, ingest_url,
issuer_url, subject: Option<String> }` by:
1. checking for a per-scope (personal-invite) `StandingTraceContributionPolicy`
   → if present, use it with `subject = None` (current behavior);
2. else falling back to the instance-level policy with
   `subject = Some(local_pseudonymous_contributor_id("{tenant}:{user}"))`.

All existing call sites that mint claims / submit / build login-links route
through this resolver instead of reading a per-scope policy directly.

### 2. Instance-wide enrollment (admin-gated, additive)
- New `AdminScope` method to perform + persist instance enrollment, reusing the
  existing `onboarding/` flow (`onboard_at_dir_with_sink`) but writing to an
  **instance-level** policy location (not `trace_contribution_dir_for_scope`).
- Device private key stored via the secrets module; tenant/ingest/issuer policy
  in an instance-level `StandingTraceContributionPolicy`.
- Exposed through `ToolDispatcher::dispatch` (admin-gated) and a web settings
  action. Non-admin users never onboard; they inherit via the resolver.
- The existing per-scope `trace_commons.onboard` (personal invite) is unchanged.

### 3. Per-user subject plumbing
- Thread the resolver's `subject` into upload-claim requests
  (`fetch_trace_upload_claim_from_issuer` / `ContributionHttpSink`) and
  submission so per-user credits/attribution work under the instance key.
- `subject = None` path is byte-for-byte the current request.

### 4. User profiles — login-link capability
- New first-party capability `trace_commons.account_login_link` (model-visible,
  consent-gated like `profile_token`) → `POST /v1/account/login-links` with the
  resolver's subject → returns the one-time browser URL, surfaced to the user via
  their channel. This is how a user obtains a Trace Commons web session to manage
  their profile.
- Optional read-through of public `/v1/community/*` for profile/leaderboard
  display.

### 5. Trace inspection
- New capabilities + `/api/webchat/v2/traces/...` handlers wrapping
  `GET /v1/account/traces`, `/{submission_id}`, `/{submission_id}/content`
  (scrubbed), authenticated with the instance device bearer + per-user subject.
- Surface the user's *submitted* traces (status, credit, scrubbed content)
  alongside the existing *local* held-trace review and credit card.
- All mutations/reads via `ToolDispatcher::dispatch`; dual-backend persistence
  for any local state (Postgres + libSQL).

## Extension/auth invariants
- `credential_name` (backend secret identity) vs `extension_name` (user-facing)
  must not be conflated; instance enrollment credentials live under the secrets
  module keyed by a stable `credential_name`, with `extension_name = trace_commons`
  for any setup/configure UI routing.
- New onboarding/auth code reuses the shared resolver/controller path — no
  channel-specific or frontend-only fallbacks.

## Sequencing (each slice independently shippable, TDD)

0. **Server:** optional `subject` through claim issuance + login-link + account
   resolution (additive; existing behavior unchanged when subject absent).
1. **IronClaw:** trace-credential resolver + admin-gated instance enrollment
   (instance-level policy; per-scope flow untouched).
2. **IronClaw:** per-user subject plumbing through claim + submission → per-user
   credits/attribution under the instance key.
3. **IronClaw:** `account_login_link` capability → user profiles / web session.
4. **IronClaw:** trace-inspection capabilities + web UI (list/detail/scrubbed
   content) + optional public community read.

## Testing
- **Resolver:** unit tests for both branches + precedence (personal invite wins).
- **Server (subject path):** absent-subject request is byte-identical to today;
  present-subject yields a distinct `auth_principal_ref` and distinct account;
  two subjects under one device key resolve to two accounts; cross-tenant subject
  collision impossible.
- **Through the caller (per CLAUDE.md "Test Through the Caller"):** drive the
  submission/login-link/inspection *handlers* (not just the resolver helper) at
  the integration tier — claim mint, submission attribution, login-link account
  resolution, and inspection ownership filtering all exercised end-to-end.
- **Coexistence:** on one instance, a personal-invite user and an
  instance-inherited user both submit and inspect without cross-contamination.
- **Admin gate:** instance enrollment rejected for non-admin identities
  (`AdminScope::new` returns `None`).

## Out of scope (YAGNI)
- Multi-device / NEAR / passkey account linking (server Slice 3).
- Reviewer-role endpoints (`/v1/traces`, `/v1/review/*`) — operator-facing, not
  an IronClaw client concern.
- Migrating the legacy per-scope flow away; it remains the user-invite model.
