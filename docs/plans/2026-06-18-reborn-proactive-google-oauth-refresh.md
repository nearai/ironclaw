# Reborn — Proactive Google OAuth Token Refresh (issue #5071)

**Status:** Implemented
**Date:** 2026-06-18
**Issue:** nearai/ironclaw#5071 — "[Reborn] Proactively refresh Google OAuth tokens before expiry"
**Labels:** bug, risk: high, scope: worker, scope: secrets, reborn, OAuth

## Design update (as implemented — supersedes §A4 / §B where they differ)

After code review the cross-process guard was simplified from a **per-account blocking
advisory lock on the inline + worker paths** to a **leader-election lock used by the worker
only**:

- The keepalive worker, per tick, calls `CredentialRefreshLeaderLock::run_as_leader` which
  acquires ONE deployment-wide `pg_try_advisory_lock` on a fixed key (`KEEPALIVE_LOCK_KEY`).
  Non-leaders skip the tick (`LeaderOutcome::NotLeader`); the leader holds exactly one pooled
  connection for its sequential sweep, then releases. libsql / no-pool path = trivial leader.
  Lives in `product_auth_refresh_lock.rs` (gated `any(libsql, postgres)`).
- The **inline** dispatch path no longer takes any cross-process lock — it reverts to the
  existing in-process `refresh_locks` guard on `ProviderBackedCredentialAccountService`, plus
  the margin-skip (read stored `expires_at`, refresh only within margin). No DB connection is
  held on the hot path. Trade-off: a rare concurrent inline refresh of the *same* account
  across processes isn't serialized, but it's margin-gated + worker-warmed and any resulting
  `invalid_grant` is classified → reauth, self-healing next tick.
- **D2 (production activation):** `CredentialRefreshSettings::default()` stays disabled; the
  CLI (`ironclaw_reborn_cli/src/runtime/mod.rs`) enables it for the `Serve` caller via
  `credential_refresh_settings`, with `IRONCLAW_CREDENTIAL_REFRESH_ENABLED` (1/true/0/false)
  as an operator kill-switch — mirrors the trigger-poller pattern.
- Review simplifications also applied: `RebornProductAuthServices::secret_store` is now
  non-optional (removes a silent margin-skip no-op); the inline refresh margin is a config
  field (`CredentialRefreshSettings::access_refresh_margin`, default 5 min).

Known: the pre-existing `runtime::tests::local_dev_runtime_*` suite is flaky under high test
parallelism (one case already fails on `main`); the suite passes serially. Not introduced by
this change.

## Problem

Google OAuth **access** tokens expire after ~1h. Google OAuth **refresh** tokens, for an
app in **testing** publishing status, die after **7 days of inactivity**. Reborn must keep
Google-connected users working without manual reconnects, and should only surface reauth
when the refresh token is genuinely missing/revoked/invalid.

Current state on `main`:

- Refresh is purely **inline / on-dispatch** (#4113 gsuite 401→refresh→retry; #5053
  runtime-staging refresh). No proactive/background refresh exists in Reborn **or** legacy
  `src/` — both refresh lazily. (`hermes-agent`, an external reference, is also lazy-only.)
- **Access-token expiry is not persisted in a queryable form.** `OAuthTokenResponse.expires_in_seconds`
  is parsed then dropped in `store_token_pair` (`oauth_provider_client.rs`). Because expiry
  is unknown, #5053 refreshes **unconditionally on every staging** — it hammers Google's
  token endpoint on every dispatch even when the access token is still valid.
- Per-account concurrency guard is **in-process only** (`refresh_locks` on
  `ProviderBackedCredentialAccountService`, `credential.rs:666`). Reborn production runs
  **multi-process against one Postgres DB**, so two processes can POST the same refresh
  token concurrently; if Google rotates it, the loser gets `invalid_grant` on a healthy
  account → false `needs_reauth`.

## Design principles

Right-sized, no new DB table, no new substrate crate. We deliberately diverge from the
issue's literal "DB lease table" + "expiry-scan table" + "persisted retry metadata" wording
where the repo already offers a cheaper equivalent. Deviations are called out in
[§ Deviations from issue text](#deviations-from-issue-text).

Division of labor by TTL:

- **Access token (1h)** → kept fresh by **inline refresh at dispatch**, made *conditional*
  on stored expiry. This is the primary, hot-path mechanism (unchanged ownership, already
  exists; we only make it conditional + safe).
- **Refresh token (7-day idle death)** → kept warm by a **low-frequency background worker**
  (daily-ish) that only touches idle accounts. Cheap; never on the hot path.

Everything routes through the frozen `auth-product.md` contract:
`RebornProductAuthServices::refresh_credential_account`. No store/provider reconstruction at
any call site.

## Verified facts (from code, 2026-06-18)

- `StoredSecret` (`crates/ironclaw_secrets/src/filesystem_store.rs:84`) already has
  `expires_at: Option<Timestamp>`, already (de)serialized, already enforced in `lease_once`
  (`filesystem_store.rs:388` → `SecretStoreError::SecretExpired`). `put()` hardcodes
  `expires_at: None` (`filesystem_store.rs:347`). Path-addressed JSON, no schema version →
  populating the field is backward-compatible.
- `SecretStore::put` signature: `put(scope, handle, material: SecretMaterial)` returning
  `SecretMetadata { scope, handle }` (`crates/ironclaw_secrets/src/lib.rs:67,985`).
  `SecretMaterial = secrecy::SecretString`. `metadata(scope, handle)` returns the same
  `SecretMetadata` (no expiry exposed today).
- `CredentialAccount` (`crates/ironclaw_auth/src/credential.rs:42`) has `updated_at`
  (bumps on refresh), `status: CredentialAccountStatus`
  (Configured/Inactive/Missing/Expired/RefreshFailed/Revoked/PendingSetup), `provider`,
  `refresh_secret: Option<SecretHandle>`. No access-token-expiry field (intentional — token
  + expiry live in the secret store).
- `OAuthTokenResponse.expires_in_seconds: Option<u64>` exists (`crates/ironclaw_auth/src/oauth.rs:429`),
  flows into `store_token_pair` (`oauth_provider_client.rs:329-365`) and is dropped.
- `ironclaw_auth` is a **pure-trait crate, zero DB deps** — advisory lock cannot live there;
  must live in `ironclaw_reborn_composition`.
- Advisory-lock pattern: `pg_try_advisory_lock` / `pg_try_advisory_xact_lock` +
  `advisory_lock_key()` helpers (`crates/ironclaw_hooks_postgres/src/backend.rs:587,688-715`),
  no table. Postgres `deadpool_postgres::Pool` is available at `build_postgres_production`
  (`crates/ironclaw_reborn_composition/src/factory.rs:3370-3401`) but is consumed into
  `PostgresRootFilesystem` + `PostgresTriggerRepository` and not retained afterward.
- Worker spawn template: `trigger_poller.rs` (`spawn_trigger_poller` →
  `run_trigger_poller` loop with `CancellationToken` + `sleep_or_cancel`), spawned in
  `build_reborn_runtime` (`runtime.rs:2910-2964`); settings carried on `RebornRuntimeInput`.

## Phase A — inline refresh: persist expiry, make conditional, classify failures, guard concurrency

### A1. Persist access-token expiry (no table)

- Add `expires_at: Option<Timestamp>` parameter to `SecretStore::put`
  (`crates/ironclaw_secrets/src/lib.rs`).
- **Enumerate every `SecretStore` implementor and update each** (signature change ripples —
  WS1 must land first and compile clean before WS2/WS3 branch): `FilesystemSecretStore::put`
  (`filesystem_store.rs:347` — replace the hardcoded `None`), the in-memory
  `InMemorySecretsStore`, the legacy `ScopedSecretsStoreAdapter` (`lib.rs` ~1169 — pass
  `None`), and any test-support fakes in downstream crates. Run `cargo test -p ironclaw_secrets`
  and `cargo test -p ironclaw_architecture` immediately after the signature change to catch
  missed impls.
- In `store_token_pair` (`oauth_provider_client.rs:329`), convert
  `tokens.expires_in_seconds` → `Some(Utc::now() + Duration::seconds(n))` (saturating cast,
  guard `i64::MAX`) and pass to `put` for the **access** secret. Refresh secret keeps
  `expires_at: None` (refresh-token idle expiry is server-side, not a stored timestamp).
- **Write order for crash safety:** persist the rotated **refresh** secret *first*, then the
  **access** secret carrying the new `expires_at`. A crash between the two writes then leaves
  the *old* (expired/soon-expired) access secret in place → next dispatch refreshes again
  (safe), never a fresh `expires_at` paired with a stale refresh token. Preserve the existing
  cleanup-on-failure (`cleanup_written_access`).
- This alone fixes the every-staging hammer reactively: an expired access token now fails
  `lease_once` with `SecretExpired`, which the inline path already treats as "refresh".

### A2. Margin-aware conditional refresh

The issue wants refresh *before* hard expiry (margin `now + 5–10 min`). `lease_once` only
trips on hard expiry. Make the margin readable:

- Add `expires_at: Option<Timestamp>` to `SecretMetadata` (`lib.rs:67`), returned by
  `put`/`metadata`. `FilesystemSecretStore` already has `StoredLease.secret_expires_at`, so
  the read path is plumbing, not new state. **Blast radius — this is NOT a pure additive
  change:** `SecretMetadata` is built by struct-literal at construction sites outside
  `ironclaw_secrets`. Update each atomically with the field addition:
  `crates/ironclaw_host_runtime/src/egress/credential.rs:625,633` and the legacy adapter
  literal in `ironclaw_secrets/src/lib.rs:1169`, plus the in-crate sites. (Considered
  `#[non_exhaustive]` to localize future additions — rejected because the external literal
  sites must be updated now regardless, and `#[non_exhaustive]` would forbid the very
  literal construction `host_runtime` relies on.)
- In the runtime-staging path (`product_auth_runtime_credentials.rs`,
  `refresh_configured_runtime_account` / `resolve_access_secret`): before calling refresh,
  read access-secret `metadata().expires_at`. If `expires_at - margin > now`, **skip refresh**
  and reuse the staged secret. If absent (legacy records, or record removed by
  `cleanup_written_access`) → refresh (preserves current behavior, fail-safe).
- **Skip-invariant (must hold in the A2 implementation):** the margin-skip fires only when
  `expires_at` is present. Because A1 writes the access secret (with expiry) *last* and
  cleanup deletes it on partial-write failure, a present `expires_at` always implies a
  completed token-pair write — so a skip can never reuse an access token paired with a stale
  refresh token. Do not cache `expires_at` anywhere; always re-read from the store so a
  cleaned/rotated record is observed.
- Margin is config (default 5 min) and is an **inline-path concern only**; the worker does
  not re-read `expires_at` (see B3).

### A3. Failure classification (steal hermes-agent's pattern)

`HostOAuthProviderClient::refresh_token` already distinguishes 4xx (`RefreshFailed`) vs 5xx
(`BackendUnavailable`). Tighten:

- Parse the token-endpoint error body for `"invalid_grant"` (revoked/expired refresh token)
  → drive account status to `Revoked` (or `RefreshFailed`) so
  `recovery_kind_and_reason_for_status` projects `ReauthorizeRequired` → caller gets
  `AuthRequired`, not a generic tool failure.
- Other 4xx → `RefreshFailed`. 5xx / network → `BackendUnavailable` (transient): **do not**
  mutate status; inline retries next dispatch, worker retries next tick.
- Missing refresh token (`refresh_secret.is_none()`) already → `RefreshFailed`; confirm it
  surfaces `ReauthorizeRequired` and never re-attempts an impossible refresh. (#5054 adds the
  offline-consent reconnect guidance — keep compatible, don't duplicate.)
- **Redaction:** error-body parsing must extract only the `error` code; never log/Debug/serde
  the raw body, token, or PKCE material. Reuse existing redaction (`RedactedString`, custom
  Debug, trace header scrub).

### A4. Cross-process per-account concurrency guard (no table)

- Clone the Postgres pool before it is moved (`factory.rs:3370-3401`:
  `let pool_for_refresh_lock = pool.clone();` before `PostgresTriggerRepository::new(pool)`).
- New composition-owned wrapper (e.g. `crates/ironclaw_reborn_composition/src/product_auth_refresh_lock.rs`)
  implementing the runtime credential-refresh port, wrapping
  `ProviderBackedCredentialAccountService`. On `refresh_credential_account(account_id)`:
  - Postgres: open a connection, `SELECT pg_try_advisory_lock($1, $2)` with key derived from
    `account_id`. If **not** acquired → another process owns the refresh; skip (return current
    account / treat as no-op success). Release on drop (session-scoped; returns to pool).
    Wrap at the `with_provider_client` install site (`auth.rs:663-669`).
  - libsql / non-postgres: identity pass-through (local file = single-writer; remote libsql
    has no advisory primitive).
- **Key-derivation reuse (avoid a divergent second copy):** `advisory_lock_key_from_bytes`
  (`crates/ironclaw_hooks_postgres/src/backend.rs:701`) is currently a private `fn`. If
  composition already depends on `ironclaw_hooks_postgres`, promote it (or a
  `pub fn advisory_lock_key_for_id(&[u8]) -> (i32, i32)`) to `pub` and import it. If adding
  that dep would cross a boundary, define a single small helper in composition with a comment
  pointing at the canonical one. Note: the credential-refresh advisory namespace is **disjoint**
  from the hooks-eviction namespace (different resources), so the two derivations need not
  agree on a key for the same resource — but a single helper avoids accidental scheme drift.
- **Two-layer concurrency ownership (intentional, not duplicate truth):** the cross-process
  per-account serialization contract is owned solely by this wrapper (Postgres advisory lock).
  The pre-existing in-process `refresh_locks` on `ProviderBackedCredentialAccountService`
  (`credential.rs:666`) is retained as a strictly *intra-process* stampede guard (prevents two
  threads in one process both reaching the token endpoint) — a local optimization layered
  under, not a competing source of truth for, the cross-process lock. Document this split at
  both sites so the layering is visible. Do not remove `refresh_locks` (it is the only guard
  on the libsql path).
- The wrapper is used by **both** the inline path and the worker, so all refreshes for one
  account serialize across processes.

## Phase B — background keepalive worker

### B1. Cross-owner account enumeration (resolve first)

The worker must enumerate Google accounts **across all owners** on the deployment. The
existing `CredentialAccountListRequest` (`crates/ironclaw_auth/src/credential.rs:267`) is
**per-user** (keyed by `AuthProductScope` = tenant+user) — there is no cross-owner sweep
today, so this gap must be closed before B2.

**Chosen path (decided, not deferred):** add a deployment-scoped enumeration method on the
**composition-owned** `FilesystemAuthProductServices` (`product_auth_durable.rs`), e.g.
`list_refresh_candidates() -> Vec<CredentialAccountId/scope>`, implemented as a listing over
the underlying `RootFilesystem` account records (dual-backend for free — `RootFilesystem`
already abstracts libsql/postgres; no new SQL, no new table). **Do not** add a cross-owner
list to the `ironclaw_auth` `CredentialAccountService` trait — that crate is pure-trait and
must stay DB-agnostic; a deployment-wide scan is a composition/durable-store concern. The
worker then calls the per-account refresh through the A4-locked port for each candidate.
Restrict the enumeration to refresh-relevant fields (id, provider, status, has-refresh,
`updated_at`) — never project secret handles or material.

### B2. Worker module

- New `crates/ironclaw_reborn_composition/src/credential_refresh_worker.rs` mirroring
  `trigger_poller.rs` exactly:
  - `spawn_credential_refresh_worker(...) -> CredentialRefreshWorkerRuntimeHandle` +
    `run_credential_refresh_worker` loop with `CancellationToken` + `sleep_or_cancel`.
  - Define `pub(crate) CredentialRefreshWorkerRuntimeHandle { cancel, handle }` with
    `shutdown(timeout)` / `join_with_timeout` mirroring
    `TriggerPollerRuntimeHandle` (`trigger_poller.rs:30-64`), and a
    `CREDENTIAL_REFRESH_WORKER_SHUTDOWN_TIMEOUT` constant (match the 5s sibling).
- Spawn in `build_reborn_runtime` (`runtime.rs`, alongside the trigger-poller spawn ~2910),
  gated on `settings.enabled`. Hold `Option<CredentialRefreshWorkerRuntimeHandle>` on
  `RebornRuntime` and call `.shutdown(CREDENTIAL_REFRESH_WORKER_SHUTDOWN_TIMEOUT)` in
  `RebornRuntime::stop`, parallel to the trigger poller (`runtime.rs:1674-1676`).
- Settings struct named **`CredentialRefreshSettings`** (match the
  `TriggerPollerSettings`/`TurnRunnerSettings` convention — no "Worker" in the settings type),
  carried on `RebornRuntimeInput` (`runtime_input.rs`) as field `credential_refresh`, wired
  from CLI `runtime/mod.rs`.

### B3. Tick logic

Each tick:
1. Enumerate candidates (B1) and filter to: Google provider + `status == Configured` +
   `refresh_secret.is_some()` + `updated_at` older than the **idle threshold** (default
   2 days — well under the 7-day refresh-token death window, with margin for downtime). The
   worker decides purely on `updated_at` + status; it does **not** read `expires_at` (that is
   the inline path's concern), so the inline `margin` config is not used here.
2. For each candidate, call the **advisory-locked** refresh port from A4
   (`RebornProductAuthServices::refresh_credential_account`). Multi-process safe by
   construction; a candidate already refreshed by another process this window has a fresh
   `updated_at` and is filtered out next tick.
3. Bounded per-tick count + structured `debug!` logging only (no `info!` — REPL/TUI rule).
   Transient failures: leave for next tick. `invalid_grant`/revoked: status already moved to
   reauth by A3; filtered out (not `Configured`) so we stop hammering an impossible refresh.

### B4. Config (safe defaults)

`CredentialRefreshSettings` in reborn config (`crates/ironclaw_reborn_config/` +
`RebornRuntimeInput`), shaped after `TriggerPollerSettings`:

- `enabled` — default **on** in production composition path.
- `interval` — default e.g. 6h.
- `idle_threshold` — default 2 days.
- `startup_jitter_max` / `tick_jitter_max` — mirror `TriggerPollerSettings`
  (`runtime_input.rs:237-238`); default `Duration::ZERO`. **Required** — without startup
  jitter every process in the multi-process deployment fires its first tick simultaneously,
  producing a thundering herd the advisory lock can serialize but not spread. Apply via
  `jitter_delay(...)` in the run loop like `trigger_poller.rs:162,180`.

The inline refresh `margin` (default 5 min) lives on the **inline** path (A2), not in this
worker settings struct — the worker selects on `idle_threshold`/`updated_at` only. Keep the
two values distinct to avoid a shared-constant ambiguity.

## Boundaries & ownership

- Worker + advisory-lock wrapper live in `ironclaw_reborn_composition` (facade). Nothing new
  in `ironclaw_auth` (pure-trait, no DB).
- No parallel auth/refresh/secret path: refresh goes through
  `RebornProductAuthServices::refresh_credential_account` per `auth-product.md` (frozen).
- Composition public API stays facade-shaped: do not leak the cloned pool or raw secret store
  through `lib.rs`/`input.rs`/`factory.rs` public surface (keep the pool clone private to the
  refresh-lock wrapper construction).
- Dual-backend parity: secret-store `expires_at`, account listing, and the advisory
  guard each have a defined behavior on both libsql and postgres.

## Tests (required validation)

- **Unit:** margin decision (skip vs refresh given stored expiry); `expires_in_seconds` →
  `expires_at` conversion + saturation; failure classification (`invalid_grant` →
  reauth vs 5xx → transient); missing-refresh-token → reauth, no repeated attempts.
- **Caller-level:** an expired-but-refreshable Google credential is refreshed before GSuite
  dispatch with no user interaction (drive the gsuite executor / runtime credential resolver,
  not just the helper — per CLAUDE.md "test through the caller").
- **Concurrency (postgres, `--features integration`):** two refreshers cannot both
  rotate/write the same account; advisory lock serializes them; loser skips.
- **Rotation:** a rotated refresh token atomically replaces the old handle (extend existing
  `refresh_account` rotation coverage).
- **Dual-backend:** candidate listing + expiry persistence work on both libsql and postgres.
- **Worker:** tick selects exactly Google + Configured + has-refresh + stale `updated_at`;
  excludes fresh / non-Configured / no-refresh-token accounts.
- **Security/redaction:** token-endpoint error bodies, public projections, and recovery
  messages leak no token/refresh-token/auth-code/PKCE/secret-handle material.
- **Inline fallback:** with the worker disabled, inline refresh still keeps dispatch working.
- `cargo test -p ironclaw_architecture` after the new module / facade wiring (boundary +
  facade-shape tests).

## Deviations from issue text (intentional, flag for sign-off)

1. **No dedicated DB lease/queue table.** Use `pg_try_advisory_lock` (postgres) + existing
   in-process `refresh_locks` (libsql). Satisfies "two processes cannot refresh the same
   account concurrently"; avoids a new table + dual-backend schema.
2. **No expiry-scan table / `expires_at` column on `CredentialAccount`.** Expiry persisted in
   the existing `StoredSecret.expires_at`; candidate selection uses the existing `updated_at`
   + status filter rather than an `expires_within(margin)` query.
3. **No persisted `next_retry` / `last_error_class` backoff metadata.** Transient failures
   retry next tick / next dispatch; status enum already encodes healthy vs reauth-required.
   *This is the one acceptance-criterion line we trim* — call it if persisted backoff is
   required.

## Sequencing for implementation

Parallelizable workstreams (after B1 list-method is confirmed):

- **WS1 (secrets):** A1 `put(expires_at)` trait + impls + `store_token_pair` wiring; A2
  `SecretMetadata.expires_at`. Foundational — others depend on the signature.
- **WS2 (auth/classification):** A3 invalid_grant classification + status/recovery mapping.
- **WS3 (composition concurrency):** A4 advisory-lock wrapper + pool clone.
- **WS4 (worker):** B1–B4 worker module, config, spawn wiring. Depends on WS3's locked port
  and WS1's signature.

WS1 lands first (signature change ripples). WS2/WS3 parallel. WS4 last.
