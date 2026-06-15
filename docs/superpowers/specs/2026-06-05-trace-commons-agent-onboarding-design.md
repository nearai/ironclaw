# Trace Commons Agent Onboarding — Design

Date: 2026-06-05
Status: Approved design, pre-implementation
Repos: this repo (client + agent tool) and TraceCommons/trace-commons-server (server)

## Problem

Onboarding onto Trace Commons today requires running the IronClaw reborn binary
with ~15 CLI flags (`ironclaw traces opt-in --endpoint ... --upload-token-issuer-url
... --upload-token-tenant-id ... --upload-token-invite-code ...`). The operator
must hand each contributor four artifacts over a secure channel: the invite code,
an operator-minted EdDSA workload JWT (~1h expiry), the tenant ID, and both
service URLs. There is no registration endpoint; the invite code is only an
allowlist gate checked at upload-claim issuance.

Goal: a user pastes a single invite link into IronClaw chat and the agent walks
them through consent and registers them with the server. No operator-minted
per-contributor tokens, no manual URL/tenant configuration.

## Decisions (settled during brainstorming)

| Decision | Choice |
|---|---|
| How invite resolves to config | Server-side resolution via a new endpoint |
| Long-term client credential | Per-tenant Ed25519 device keypair, registered at onboard |
| Protocol shape | Single atomic `POST /v1/onboard` (resolve + register in one call) |
| Agent surface | Reborn engine tool (`trace_commons_onboard`); CLI opt-in unchanged |
| Consent UX | Agent gathers two explicit consents; all other policy knobs default |
| Invite format | Full URL, e.g. `https://issuer.example.com/onboard#INV9K3RT5FBQ72JX` |
| Multitenancy | One keypair per (user scope, tenant); tenant binding lives server-side |

## 1. Protocol and server changes (trace-commons-server)

### 1.1 `POST /v1/onboard` (new, on the upload-claim issuer)

Request:

```json
{
  "schema_version": "trace_commons.onboard_request.v1",
  "invite_code": "INV9K3RT5FBQ72JX",
  "device_public_key": "<base64 Ed25519 public key>",
  "client_info": { "agent": "ironclaw", "version": "0.x.y" }
}
```

Response (200):

```json
{
  "schema_version": "trace_commons.onboard_response.v1",
  "tenant_id": "tenant-zaki-pilot",
  "ingest_url": "https://ingest.example.com",
  "issuer_url": "https://issuer.example.com",
  "audience": "trace-commons-ingest",
  "device_key_id": "sha256:<pubkey-hash>",
  "contributor_label": "optional operator note",
  "community_url": "https://tracecommons.ai",
  "profile_url": "https://tracecommons.ai/profile",
  "leaderboard_url": "https://tracecommons.ai/leaderboard"
}
```

The last three fields are optional browser-surface navigation hints
(trace-commons-server#137): the agent surfaces them after successful
onboarding so the user can reach their profile/leaderboard without the client
hardcoding community deployment details. They are deployment config, not
credential material — they MUST NOT participate in issuer trust anchoring
(§2.1), and non-HTTPS values are dropped client-side rather than failing the
onboard.

Server behavior, in one transaction:

1. Hash the invite code (SHA-256, same scheme as the existing allowlist) and
   look up the allowlist entry. The entry's existing `tenant_id` field is the
   multitenancy anchor: the invite determines the tenant; the client never
   asserts one.
2. Consume one use atomically. Allowlist entries gain `max_uses` (default 1)
   and a consumption counter; consumption is an atomic compare-and-increment
   (`UPDATE ... SET consumed = consumed + 1 WHERE consumed < max_uses`-style,
   inside the same transaction as the registry insert), never read-then-write.
   Single-use is the default; operators may issue shared pilot codes with
   `max_uses > 1`.
3. Insert into a new device-key registry table:
   `(device_key_id PK, tenant_id, public_key, invite_subject_hash,
   client_info, created_at, revoked_at NULL)`.
   `device_key_id = sha256:<hex of public key bytes>`.
   `invite_subject_hash` is the SHA-256 hash of the invite code that produced
   the registration (the same `subject_hash` value stored in the allowlist
   entry) — audit linkage from key back to invite.
4. Return tenant-scoped config. Issuer config gains a per-tenant map of
   `{ingest_url, audience}` with instance-wide defaults as fallback;
   `issuer_url` is the issuer's own public origin.

Idempotency and conflicts:

- A repeat request with the same `(invite_subject_hash, device_public_key)`
  pair returns the original 200 response without consuming an additional use.
  The `device_key_id` primary key is the idempotency guard: the uniqueness
  violation on insert (in the same transaction as the consumption increment)
  triggers the idempotent-return path, so two concurrent first-time requests
  with the same pubkey cannot double-consume a `max_uses = 1` code. This
  makes client retries safe after a network failure between registration and
  local policy write.
- A `device_public_key` already registered with a **different**
  `invite_subject_hash` or a **different** `tenant_id` is rejected with the
  uniform `InviteNotValid` error (never returning the other registration's
  config — that would be a cross-tenant leak). The client generates per-tenant
  keys (§2.2) so this case is attacker-only, but the server enforces it
  regardless.

Wire types (request/response structs, error codes) live locally in
`crates/ironclaw_reborn_traces/src/onboarding/protocol.rs`. There is no shared
`trace-commons-protocol` crate; the issuer/server side mirrors these definitions
in its own repo rather than sharing a published crate.

### 1.2 Upload-claim issuance: device-key auth branch

`POST /v1/trace-upload-claim` gains a second verification branch alongside the
existing operator-workload-key path (which is kept for back-compat):

- If the presented workload JWT's `kid` matches a device-key registry row:
  - verify the EdDSA signature against that row's `public_key`;
  - require `revoked_at IS NULL`;
  - validate freshness and audience: `exp` and `iat` claims are required;
    reject if expired (with bounded clock skew, ±60s), if `exp - iat` exceeds
    a maximum lifetime (5 minutes), or if `aud` is absent or does not match
    the issuer's configured audience. A captured device-signed JWT is
    therefore replayable for at most its short lifetime, against this issuer
    only.
  - the minted upload claim's `tenant_id` comes from the registry row, never
    from the request. If the JWT or request carries a `tenant_id` that
    disagrees, reject. A key registered under tenant A structurally cannot
    mint claims for tenant B.
  - the `invite_code` claim is not required on this branch; the registered
    key is the post-invite credential.
- Otherwise fall through to the existing operator-workload-key verification
  (including the invite-code allowlist check), unchanged.

### 1.3 Operator tooling

`trace-commons-tenant` gains `device-keys list [--tenant <id>]` and
`device-keys revoke <device_key_id>` subcommands (per-device revocation = set
`revoked_at`). The allowlist runbook (`docs/operator/pilot-allowlist.md`) is
updated for `max_uses`/consumption semantics.

### 1.4 Abuse resistance

- `/v1/onboard` is unauthenticated by design: rate-limited per source IP;
  invite codes remain high-entropy and operator-issued — 16 characters from
  the uppercase A–Z0–9 alphabet (~82 bits), making distributed brute force
  economically pointless even without per-IP limits. A global onboard
  failure-rate circuit breaker (issuer-wide, trips to 503 on sustained
  invalid-invite volume) is explicitly deferred for the pilot; the residual
  risk is bounded by code entropy and uniform errors.
- Failure responses are uniform ("invite not valid") so callers cannot
  distinguish unknown vs consumed vs revoked codes. The
  consumed/unknown/revoked distinction is visible only via the operator admin
  surface.
- Typed error codes for the client: `InviteNotValid`, `InviteMalformed`,
  `DeviceKeyMalformed`, `OnboardRateLimited`. (The agent surfaces these as
  human messages; `InviteNotValid` deliberately covers all
  not-found/consumed/revoked cases.)

## 2. Client core (`crates/ironclaw_reborn_traces`)

New `onboarding` module.

### 2.1 Invite URL parsing

Canonical form: `https://<issuer-host>/onboard#<code>`. Also accepted:
`?code=<code>` query form and bare `code@host`. Rules:

- Origin extraction = scheme + host + port only; the URL's path and fragment
  are ignored except for code extraction (the canonical link's `/onboard`
  path is for a future human-facing web page; the client always POSTs to
  `<origin>/v1/onboard`).
- Origin must be HTTPS (allow `http://localhost`/loopback for tests/dev).
  The bare `code@host` form implies HTTPS and supports an optional
  `:port`.
- Code must be non-empty after trim; otherwise typed parse error.

**Trust anchoring.** The operator-handed invite link is the trust root. The
invite-derived origin — not the server response — is authoritative for the
issuer: the client requires the response `issuer_url` origin to equal the
invite origin and rejects the onboard otherwise, and it seeds
`upload_token_issuer_url` and `upload_token_issuer_allowed_hosts` from the
invite-derived origin. `ingest_url` is accepted from the response (the
operator's invite link transitively vouches for it) but must be HTTPS.

### 2.2 Per-tenant device keypair

- Generated with Ed25519 at onboard time, one per `(user scope, tenant_id)`.
- The tenant is only known *after* the onboard response, so the keypair is
  staged before the network call under an invite-keyed pending path:
  `~/.ironclaw/trace_contributions/users/<scope-hash>/device_keys/pending/<invite-hash>.json`
  (mode 0600). On a successful response it is atomically renamed to
  `device_keys/<tenant-hash>.json`. On retry the client loads the pending
  file for the same invite (or the tenant file if already promoted) and
  reuses that keypair — combined with server-side idempotency this makes the
  whole flow retry-safe with no double-consumed invites and no orphaned
  registrations.
- The key file contains the private key (base64), `device_key_id`,
  `tenant_id` (once known), and `created_at`. The private key never leaves
  the machine and is never echoed in tool output, logs, or the policy file.
- Keying by tenant means joining a second tenant creates a second keypair:
  no cross-tenant key reuse and no key-based linkage between tenants.
- Pending files for permanently failed onboards (e.g. `InviteNotValid`) are
  deleted on that terminal failure; the staged key was never registered, so
  this is hygiene, not security.

### 2.3 `onboard()` and policy changes

```rust
pub async fn onboard(
    scope: &str,
    invite_url: &str,
    consents: OnboardConsents, // include_message_text, include_tool_payloads
) -> Result<OnboardOutcome, OnboardError>
```

Sequence: parse URL → load-or-generate keypair (staged to the pending path
before the network call, §2.2) → `POST /v1/onboard` → verify response
`issuer_url` against the invite origin (§2.1) → promote keypair to its tenant
path → write scoped `StandingTraceContributionPolicy` populated from the
response (subject to §2.1 trust anchoring).

`StandingTraceContributionPolicy` gains:

- `device_key_id: Option<String>`
- `auth_mode: TraceUploadAuthMode` — `WorkloadTokenEnv` (default, existing
  behavior, serde-default for old policy files) or `DeviceKey`.

All other policy fields take pilot defaults: `default_scope =
debugging-evaluation`, `min_submission_score = 0.35`, PII review gating on,
no tool filter. The existing CLI `traces opt-in` path is unchanged and still
writes `WorkloadTokenEnv` policies.

### 2.4 Upload-claim refresh: self-signed workload JWTs

When `auth_mode = DeviceKey`, at each upload-claim refresh the client signs a
short-lived workload JWT locally with the device private key instead of
reading `IRONCLAW_TRACE_SUBMIT_TOKEN`/workload-token env:

- header: `alg: EdDSA`, `kid: <device_key_id>`
- claims: `tenant_id`, `aud` (issuer audience), `iat`, `exp = iat + 60s`
- no `invite_code` claim needed.

Field mapping: the onboard response's `audience` field populates the policy's
`upload_token_audience` and is emitted verbatim as the JWT `aud` claim.

The existing env-var path is untouched for `WorkloadTokenEnv` policies.

## 3. Agent tool (reborn engine)

### 3.1 `trace_commons_onboard` (single-shot tool)

Params: `invite_url: string`, `include_message_text: bool`,
`include_tool_payloads: bool`, `confirmed: bool`.

- Refuses `confirmed: false` with a message instructing the agent to obtain
  explicit user consent first.
- On success returns enrollment summary (tenant, endpoints, consents,
  device_key_id, and the optional community/profile/leaderboard URLs when the
  server provides them) — no key material.
- Maps typed errors to user-facing messages (`InviteNotValid` → "this invite
  link isn't valid — it may have been used already; ask the operator for a
  new one").

### 3.2 Conversation flow (prompt, not code)

The consent conversation lives in the agent, guided by the tool description
plus a short prompt file in `crates/ironclaw_engine/prompts/` (per the
prompts-in-files rule). When the user pastes an invite link the agent:

1. explains what Trace Commons contribution is (redacted traces, what is and
   isn't collected, credit model);
2. asks consent question 1: confirm opt-in;
3. asks consent question 2: include redacted message text and/or redacted
   tool payloads (yes/no each);
4. calls `trace_commons_onboard` with `confirmed: true`;
5. reports the result, points the user at their profile/leaderboard URLs when
   the server provided them, and explains how to opt out / adjust later.

### 3.3 `trace_commons_status` (companion tool)

Read-only: reports whether the current scope is enrolled, tenant, auth mode,
consents, queue depth, last submission. Opt-out remains on the existing CLI
(`ironclaw traces opt-out`) for now.

### 3.4 `trace_commons_credits` (companion tool) + console display

Read-only: the payoff side of opt-in. The agent can query credit state on
demand ("how are my trace credits doing?"), and the IronClaw web console shows
that credits are accruing and the current balance. Both read the local
`CreditSummary`/`TraceCreditReport` (already in `contribution.rs`) derived from
scoped submission records + server status sync — pending vs. final vs.
delayed-ledger framing preserved so the surface doesn't over-promise. The
authoritative ledger is server-side; the local view is labeled "as last
synced." See plan Task 11 for the build details and the stable tool-output
contract the designer's onboarding-flow UI builds on.

All tools route through `ToolDispatcher::dispatch()` like every other tool
(audit trail, safety pipeline).

## 4. Error handling summary

| Failure | Behavior |
|---|---|
| Malformed/non-HTTPS invite URL | Typed parse error before any network call |
| Invite not valid (unknown/consumed/revoked) | Uniform server 4xx → agent explains, suggests contacting operator |
| Network failure after registration, before policy write | Keypair persisted locally + idempotent endpoint → retry with same invite succeeds |
| Rate limited | Typed error, agent suggests retrying later |
| `confirmed: false` | Tool refuses; agent must gather consent |
| Tenant mismatch at claim time | Issuer rejects; tenant is server-anchored (registry row), cannot occur via this client |

## 5. Testing

Server (trace-commons-server repo):

- Allowlist consumption: single-use enforced, `max_uses` honored, idempotent
  repeat with same pubkey does not double-consume; concurrent onboard
  requests never exceed `max_uses` (atomic compare-and-increment race test).
- Device-key registry: registration, revocation, `kid` lookup; a pubkey
  already registered under tenant A is rejected when presented with a
  tenant-B invite (uniform `InviteNotValid`, no config leak).
- Claim issuance: device-key branch verifies signature; revoked key rejected;
  expired / over-max-lifetime / wrong-`aud` JWTs rejected; cross-tenant
  invariant — a tenant-A key cannot mint a tenant-B claim even when the
  request asserts tenant B.
- Uniform error body for not-found vs consumed vs revoked.

Client (this repo, TDD):

- Invite URL parsing (canonical, query, `code@host` incl. port, rejects
  non-loopback http/empty code; origin extraction discards path/fragment).
- Trust anchoring: response `issuer_url` mismatching the invite origin
  rejects the onboard; non-HTTPS `ingest_url` rejected.
- Keypair persistence: 0600 mode, pending-path staging and atomic promotion,
  per-tenant keying, reuse-on-retry from both pending and promoted paths.
- Policy write: `auth_mode`/`device_key_id` round-trip; old policy files
  deserialize as `WorkloadTokenEnv`.
- Self-signed workload JWT shape (`kid`, `tenant_id`, 60s expiry).
- Mock-issuer integration test of the full onboard → claim refresh → submit
  chain, driven through the tool dispatch path (test through the caller, not
  just the helper).
- Tool: consent refusal, error mapping, no key material in output.

## 6. Out of scope

- Web UI onboarding visuals (design work proceeding in parallel).
- Agent-driven opt-out / policy editing (CLI covers it).
- Removing the operator-workload-token path (kept for back-compat).
- Public tenant creation (tenants remain operator-provisioned).
- iOS/Android/desktop app flows.
