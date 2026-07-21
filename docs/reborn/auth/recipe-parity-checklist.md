# OAuth / Auth Recipe-Parity Checklist

> **Definition of done.** A box flips to `[x]` **only when a test proves it** (both
> DB backends, through the harness). A recipe is production-grade only when its own
> section **and** §S (shared engine + flow controls) are fully ticked. `⏸` = deferred
> (owner), excluded from the count. `⚪` = N/A.

## ⚠️ Fold note — read first (this copy is re-expressed onto the rollup)

This checklist was **authored against `origin/main` (`f5c649ba3`)** and has been
**folded onto the reconciled rollup branch (`nea25/unified-vs-main`)** by
re-expressing each change onto that branch's (different) auth structure. The
per-item line numbers and symbol names below are **main's**, retained for
provenance — they are NOT live pointers on this branch. The structural map:

1. **The shared OAuth engine moved.** On main the exchange lives in
   `crates/ironclaw_reborn_composition/src/product_auth/oauth/oauth_provider_client.rs`
   (`HostOAuthProviderSpec`, `scopes_for_exchange`, `ExchangeScopePolicy`). On the
   rollup it is the recipe-driven `AuthEngine` in **`crates/ironclaw_auth/src/engine/`**
   (`exchange.rs` + `mod.rs`), executing `ironclaw_host_api::OAuth2CodeRecipe` data.
   Scope extraction is `extract_token_response` and the exchange-scope policy enum is
   `MissingScopeBehavior::{Reject, FallbackToRequested}` (not `ExchangeScopePolicy`).
   "Recipe-only" still holds: a vendor difference is recipe data, never engine
   `if vendor == …`.
2. **The OAuth "recipe" is Rust/manifest data, not `[auth.*.token_response]` TOML on
   main** — but on this rollup the first-party recipes ARE bundled TOML under
   `crates/ironclaw_first_party_extensions/assets/<pkg>/manifest.toml` (`[auth.<vendor>]`
   with `[token_response]` / `[token_response.scope]`), resolved into `OAuth2CodeRecipe`.
   The engine suite (`crates/ironclaw_auth/tests/auth_engine_contract.rs`) runs against
   those real bundled manifests.
3. **The specificity scanner is present on this rollup** (unlike main):
   `crates/ironclaw_architecture/tests/reborn_extension_specificity.rs` /
   `reborn_retired_taxonomy.rs` exist here, so §0.5.1 is enforceable — but the folded
   changes are generic (no vendor literals in the engine), so nothing new trips it.

**What was folded here (A1, A2a, A3, A6, A14 — DONE):** see the corrected
§S entries below. Items outside {A1, A2a, A3, A6, A14} are carried over from main's
audit unchanged (their status on this rollup was not re-verified in this pass — treat
them as main-provenance, not this-branch claims).

---

## Scoreboard

| Recipe / area | Handshake | Token lifecycle | Flow lifecycle (A1-A3) | Engine hardening (A6-A16) | Overall |
|---|---|---|---|---|---|
| **Notion** 🟢 | ✅ | ✅ | via §S | via §S | ⏳ pending §S |
| **Google** 🟢 (gmail/drive/calendar/docs/sheets/slides) | ✅ | ✅ | via §S | via §S | ⏳ pending §S |
| **Slack** 🟢 | ✅ | ✅ | via §S | via §S | ⏳ pending §S |
| **GitHub** 🟢 (manual token) | ⚪ (no OAuth) | ✅ | via §S | via §S | ⏳ pending §S |
| **NEAR AI** 🟢 (manual token) | ⚪ (no OAuth) | ✅ | via §S | via §S | ⏳ pending §S |
| **Telegram / web-access / acme** ⚪ | ⚪ | ⚪ | ⚪ | ⚪ | ⚪ N/A |
| **§S shared engine + flow controls** | ✅ (main-provenance) | ✅ (main-provenance) | ✅ A1, A2a, A3 · A2b backlog | ✅ A6, A14 (rollup) · A13/A15 main-provenance (A7/A10/A12/A16 backlog; A9 moot) | mostly ✅ |

Legend: ✅ all boxes ticked · ⏳ in progress · ⚪ N/A · 🔴 known-broken · ⚠️ divergence on this branch.

---

## Notion 🟢 (re-verified on THIS branch — 2026-07-15 audit correction)

Recipe = the `[auth.notion]` bundled TOML (`assets/notion-mcp/manifest.toml`) →
`OAuth2CodeRecipe` executed by `ironclaw_auth`'s `AuthEngine` (DCR — no
`client_credentials` block). _Audit correction: the previous 🟢 here rested on
main's auto-parsing `Standard` token shape, which did **not** survive the
merge — the pointer-driven engine captures only declared pointers, and the
bundled recipe declared none for refresh/expiry, so every Notion connection
died ~1h after connect. Fixed by declaring the captures in the manifest;
provenance below is this branch._

- [x] **Handshake** — PKCE-S256 + host `state` + HTTPS-only token endpoint (inherited from §S H1-H5).
- [x] **Token lifecycle — captures `refresh_token` + `expires_in`** (A4).
  `[auth.notion.token_response]` declares `/refresh_token` + `/expires_in`;
  `[auth.notion.refresh] rotates_refresh_token = true`. **Proven:**
  `auth_engine_contract::notion_recipe_declares_refresh_and_expiry_capture`
  (pin on the real manifest) +
  `auth_engine_contract::dcr_vendor_registers_once_and_runs_standard_oauth_afterwards`
  (exchange captures and stores the rotating refresh token).
- [x] **Non-expiring guard** — absent/`0` `expires_in` → non-expiring, not already-expired
  (`engine/exchange.rs::store_token_pair` `.filter(|seconds| *seconds > 0)`).
- [ ] **A16 (now non-latent)** — with refresh live, a Notion DCR client expiry
  surfaces as `invalid_client` on refresh; the engine does not yet re-register.
  Tracked backlog, sequenced behind this fix.
- flow-lifecycle & engine-hardening: inherit §S.

## Google 🟢 (gmail, drive, calendar, docs, sheets, slides)

Recipe = `oauth/google_oauth/mod.rs` (`HostOAuthProviderSpec`, `Standard` shape). The canonical
correct refresh reference the brief cites.

- [x] **Handshake** — inherited §S H1-H5.
- [x] **Token lifecycle** — refresh rotation + expiry via shared `Standard` shape / `store_token_pair`.
  **Proven:** `product_auth_providers.rs`
  `composed_google_provider_refreshes_account_through_credential_service:522`;
  `auth_product_contract/refresh_contract.rs` (`credential_refresh_updates_account_through_provider_boundary:78`).
- [x] **`invalid_grant` → account `Revoked`** (§S T3). Proven:
  `refresh_contract.rs::credential_refresh_invalid_grant_marks_account_revoked:668`.
- flow-lifecycle & engine-hardening: inherit §S.

## Slack 🟢

Recipe = `slack/slack_personal_oauth.rs:96-106` (`TokenResponseShape::SlackAuthedUser`,
provider `slack_personal`). **PKCE↔secret decision (brief in-pass): RESOLVED — confidential app
*with* PKCE.** Live code already implements exactly this: PKCE `code_verifier` is unconditional
(`oauth_provider_client.rs:847`) and `client_secret` is appended when present (`:849-851`); the
secret is deliberately dropped from the front-channel authorization URL
(`slack_personal_oauth.rs:705`) and only used back-channel. No change required. _(On the rollup:
`[auth.slack]` recipe with `[token_response.scope]`; `code_verifier` unconditional in
`exchange.rs::execute_oauth_exchange`; `client_secret` back-channel via
`token_request_headers_and_body`.)_

- [x] **Handshake** — PKCE-S256 front-channel + `state`; confidential-client secret back-channel only.
  **Proven:** `oauth_provider_client/tests.rs::exchange_request_includes_client_secret_and_derived_network_policy_host:217`
  (secret in body; PKCE verifier always supplied).
- [x] **Token lifecycle — `ok:false` rejected** (A8 for Slack). `SlackAuthedUser` parser checks
  `if !parsed.ok { Err }` before trusting the payload (`oauth_provider_client.rs:740-748`).
  **Proven:** `oauth_provider_client/tests.rs::slack_token_response_rejects_ok_false:429`.
- [x] **Token lifecycle — user-token extraction + comma-scope normalization + rotation**
  (`:756-790`). Proven: `slack_authed_user_token_response_extracts_user_token_and_scopes:409`.
  _(On the rollup the comma normalization is `exchange.rs::parse_scope_list`, proven by
  `auth_engine_contract::pointer_extraction_reads_nested_fields_and_scope_fallback`.)_
- [x] **Non-expiring `expires_in:0`** (Slack apps w/o rotation) → non-expiring (`store_token_pair:409`).
- flow-lifecycle & engine-hardening: inherit §S.

## GitHub 🟢 (manual token / PAT)

The `github` tool credential is a **manual token**, not OAuth
(`assets/github/manifest.toml` → `source = { provider = "github" }`, no `setup = {kind="oauth"}`).
(The `ironclaw_reborn_webui_ingress` GitHub *sign-in* is a separate login system, out of scope here.)

- [x] Handshake — ⚪ N/A (no OAuth authorization-code flow for this credential).
- [x] **Manual-token submit validates format + stores under the credential owner.** Proven:
  `auth_product_contract/manual_token_contract.rs`; `composition/tests/manual_tokens.rs`.
- [ ] **A17 (optional) — submit-time network validation probe.** WAIVED: no validation-probe
  mechanism exists anywhere (github has none either — the brief's github/nearai asymmetry does not
  exist in live code); adding one is net-new infra, and A17 is explicitly Optional. `oauth/nearai` §.
- flow-lifecycle & engine-hardening: inherit §S.

## NEAR AI 🟢 (manual token / API key)

`assets/nearai-mcp/manifest.toml` → `source = { provider = "nearai" }`, manual token, no OAuth.

- [x] Handshake — ⚪ N/A.
- [x] **Manual-token submit + storage** (shared manual-token path). Proven as GitHub above.
- [ ] **A17 (optional) — submit-time key validation.** WAIVED — same reason as GitHub: no probe
  mechanism exists to reach parity with; Optional item.
- flow-lifecycle & engine-hardening: inherit §S.

## Telegram / web-access / acme ⚪ N/A

No OAuth credential recipe. `web-access` is an unauthenticated fetch surface
(`assets/web-access/manifest.toml`, no `runtime_credentials` OAuth source). `telegram` and `acme`
have no manifest in this tree. Nothing to verify.

---

## §S — Shared engine + flow controls

### Handshake controls (main-provenance — carried over, not re-verified on this rollup)

- [x] **H1 · PKCE S256, unconditional.** Front-channel `code_challenge` + `code_challenge_method=S256`;
  back-channel `code_verifier` always appended. No path can disable it. RFC 7636. _(Rollup:
  `exchange.rs` appends `code_verifier` when `recipe.pkce == PkceMode::S256`.)_
- [x] **H2 · CSRF `state` host-generated + constant-time compared.** `opaque_state_hash` bound into
  the flow, verified with `constant_time_eq` on prepare + claim (`ironclaw_auth/src/domain.rs`). RFC 6749 §10.12.
- [x] **H3 · `iss`/mix-up structurally mitigated.** Per-vendor callback path + vendor-bound state;
  claim rejects a provider mismatch. RFC 9207 (noted, not built — §5).
- [x] **H4 · Token endpoint HTTPS-only, private-IP denied, body capped.** _(Rollup: enforced by the
  engine's `RuntimeHttpEgress` network policy pinned to the recipe endpoint host + capped body.)_
- [x] **H5 · `redirect_uri` supplied host-side into the exchange body**, from client material,
  not echoed from the browser. _(Rollup: `exchange.rs::execute_oauth_exchange` builds `redirect_uri`
  from `callback_base.redirect_uri_for(vendor)`.)_

### Token-lifecycle controls (main-provenance — carried over)

- [x] **T1 · Access expiry from server `expires_in`; `0`/absent = non-expiring** (`store_token_pair`). RFC 6749 §5.1.
- [x] **T2 · Refresh rotation, crash-safe write order** (refresh-first, access-last). RFC 6749 §6.
  _(Rollup: `exchange.rs::store_token_pair`, refresh written before access.)_
- [x] **T3 · `invalid_grant` on refresh → `InvalidGrant` → account `Revoked`.** Rollup:
  `exchange.rs::execute_oauth_refresh` maps the vendor `invalid_grant` code to
  `AuthProductError::InvalidGrant`; production `credential.rs::report_terminal_refresh_status`
  maps `InvalidGrant` → `Revoked` (docstring credential.rs:804). RFC 6819.
- [x] **T4 · 5xx → `BackendUnavailable` (retryable); other 4xx → `TokenExchangeFailed`/`RefreshFailed`.**
- [x] **T5 · Failure-body redaction** — only the stable `error` code is extracted; raw body / tokens
  never logged (`exchange.rs::OAuthErrorResponseBody` + `log_vendor_error`). Proven:
  `serde_redaction_contract.rs`.

### Flow-lifecycle + engine-hardening items (flip only on a passing test)

- [x] **A1 · Supersede-on-start.** BUILT + TESTED. Supersession is
  `AuthFlowManager::create_flow`'s own contract: when the new flow's continuation is
  setup-class, `create_flow` cancels any prior non-terminal setup flow for the owner+provider
  inside the creation seam, under the same critical section as the insert (so two racing creates
  cannot both observe "no live predecessor"). The durable impl lists the **owner+surface+session
  flow root** via `flow_records_under_scope_root` (the durable flow path in `durable/paths.rs` is
  keyed by agent/project/surface/session — thread/mission/invocation-agnostic), the exact
  thread-less set; the in-memory fake matches the same owner-root granularity. Idempotent.
  RFC 9700 §4.7.1. (The earlier separate `cancel_superseded_setup_flows` seam that
  `start_setup_oauth_flow` called before `create_flow` was deleted as a strict-subset duplicate —
  `create_flow` already superseded the full setup class.) **Proven:**
  `oauth_flow_contract` supersede cases over the fake, and `durable::tests` over the durable store
  (real production code path over the FS backend).
- [x] **A2a · Projection honors `expires_at`.** BUILT + TESTED (folded — verbatim structure match).
  `AuthGateRecord::to_view(now)` returns not-live for a non-terminal flow past `expires_at`
  (`crates/ironclaw_product_workflow/src/auth_interaction/types.rs`), and
  `DefaultAuthInteractionService::list_pending` passes `chrono::Utc::now()`. RFC 6819 §5.1.5.3.
  **Proven:** `auth_interaction_contract::list_pending_auth_omits_flow_past_its_expiry`.
- [ ] **A2b · Background flow-expiry sweep** — DEFERRED FOLLOW-UP (bounded; not shipped here). A2a
  makes the read path correct and durable write/claim paths expire lazily, so no live flow ever
  mis-reads. The remaining gap (abandoned flows lingering as non-terminal rows) needs a periodic
  global driver. Not started; A1 supersede + A2a + lazy expiry cover the user-facing cases.
- [x] **A3 · Removal/disconnect cancels pending OAuth flows.** BUILT + TESTED. A provider-selected
  `cleanup_for_lifecycle` now cancels EVERY non-terminal flow for the credential-owner + provider, in
  both `FilesystemAuthProductServices` (`durable/cleanup.rs`, via the new cross-surface
  `lifecycle_flows_for_owner_provider` walk that enumerates every surface/session flow root — extracted
  alongside `flow_records_for_owner` into the shared `flow_records_for_resource_filtered`) and the
  `InMemoryAuthProductServices` fake (`fakes.rs`, `flow_matches_credential_owner`). **Owner decisions
  (2026-07-15):** cancel on **both** `Deactivate` and `Uninstall` (any provider-selected cleanup — the
  flow scope requires a provider anyway); cancel **all** non-terminal flow kinds — `SetupOnly` connect
  flows AND blocked-tool `TurnGateResume` gates — since any non-terminal flow can mint on a late
  callback (predicate = `&flow.provider == provider && !is_terminal_status(flow.status)`). **Shared-
  vendor safe by construction** (no shared-vendor logic added here): the production removal caller
  `revoke_exclusive_credentials` (`extension_lifecycle.rs`) only calls cleanup with a provider
  EXCLUSIVE to the removed extension (a provider still used by another installed extension is skipped);
  disconnect (`personal_credential_cleanup_request`) and post-activation-failure compensation likewise
  pass `provider: Some(..)` + `Uninstall`. Idempotent (a concurrently terminal flow is skipped, never
  an error). Consequence closed: after uninstall the pending flow is `Canceled`, so a late callback is
  rejected (`claim_oauth_callback` → `AuthProductError::Canceled`) and mints nothing. RFC 9700 §4.7.1 +
  RFC 7009 §1. **Proven:** fake-tier
  `cleanup_contract::uninstall_cancels_pending_flow_and_rejects_late_callback` +
  `cleanup_contract::cleanup_cancels_all_pending_flow_kinds_for_provider_on_deactivate`; durable-tier
  `product_auth::durable::tests::filesystem_cleanup_cancels_pending_flow_across_surfaces` (real FS path
  — a `Callback`-surface cleanup cancels a `Web`-surface popup flow, proving cross-surface enumeration;
  a different provider's flow survives; second cleanup idempotent). **Deliberately omitted:** main's
  `canceled_turn_gate_continuations` report field + parked-turn *notification* pipeline (the completed-
  but-unacked `TurnGateResume` re-enumeration main adds via `flow_requires_lifecycle_cleanup`) — this
  branch's `SecretCleanupReport` has no such field, removal already drains in-flight work before auth
  cleanup, and it is a separate turn-UX feature owned by the concurrent main-delta reconciliation. The
  predicate here is `!is_terminal_status` only.
- [x] **A6 · Scope downgrade/over-claim on the echoed-scope path.** BUILT + TESTED (folded onto the
  rollup engine). `extract_token_response` (`crates/ironclaw_auth/src/engine/exchange.rs`) now stores
  `granted ∩ requested` on the echoed-scope arm — dropping any scope the vendor granted beyond the
  request (stop over-claiming) and never widening to the full requested set — and emits a count-only
  guard `warn!` when the effective grant differs from requested. When the intersection is empty it
  falls through to the recipe's `MissingScopeBehavior` (shared helper `scopes_when_grant_absent`),
  exactly as an omitted scope would — `FallbackToRequested` preserved (RFC 6749 §3.3),
  `Reject` fails closed. Generic + spec-agnostic, no vendor branch. RFC 9700 §2.3. **Proven:**
  `auth_engine_contract::exchange_clamps_echoed_scopes_to_granted_intersect_requested` (vendor
  over-grants `chat:write` + omits requested `channels:read` → stored grant is exactly
  `["search:read"]`); the existing `pointer_extraction_reads_nested_fields_and_scope_fallback` was
  updated to request both echoed scopes so its multi-scope comma-normalization assertion survives the
  clamp.
- [ ] **A7 · Retire/harden the legacy browser-orchestrated OAuth path** — BACKLOG (main-provenance;
  not re-verified on the rollup). Deferred: touches the Python e2e suite. RFC 6749 §10.12.
- [ ] **A8 · Generic `token_response.success` predicate** (LOW, main-provenance). Standard shape
  rejects `{ok:false}` only because `access_token` is absent; build the generic field or WAIVE.
- [x] **A9 · Reject/remove `PkceMode::None`** — N/A: PKCE is unconditional S256 (see H1). On the
  rollup `PkceMode` exists (`S256`) but no path disables it. WAIVED as moot.
- [ ] **A10 · Durable PKCE verifier store for the setup path** (pre-HA) — BACKLOG (main-provenance).
- [ ] **A12 · Don't evict verifier on a `Completed` claim** — BACKLOG (main-provenance).
- [x] **A13 · Propagate swallowed secret-delete errors on removal** — main-provenance (the rollup's
  `durable/cleanup.rs` similarly propagates secret-delete failures as retryable; not re-proven here).
- [x] **A14 · InMemory fake fidelity — `Revoked` on `InvalidGrant`.** BUILT + TESTED (folded).
  Added the `Err(InvalidGrant) → Revoked` arm to the fake's `refresh_account`
  (`crates/ironclaw_auth/src/fakes.rs`), matching production
  `ProviderBackedCredentialAccountService::refresh_account` (credential.rs:804 maps
  `invalid_grant → Revoked`). **Proven:**
  `refresh_contract::fake_refresh_account_marks_account_revoked_on_invalid_grant` (drives the fake
  directly; asserts status `Revoked` + recovery `ReauthorizeRequired`).
- [x] **A15 · Keepalive enabled for non-`serve` callers** — ALREADY BEHAVES AS INTENDED
  (main-provenance, waived): enabled only for the `Serve` caller, inert otherwise.
- [ ] **A16 · DCR client re-register on `invalid_client`** — BACKLOG (main-provenance).
- ⏸ **A5 · Provider revocation on uninstall — DEFERRED (owner).** Local secret deletion suffices. Excluded from the count.

---

## Global acceptance

1. **Recipe-only (§0.5.1).** On this rollup the specificity scanner
   (`reborn_extension_specificity.rs`) DOES exist. The folded engine changes (A6 scope clamp, A1
   supersede) are generic mechanisms driven by recipe/scope data — no `if vendor == …` — so they
   introduce no new vendor literals. A6 lives entirely in `ironclaw_auth`'s generic `AuthEngine`.
2. **Done, not planned (§0.5.2).** Every `[x]` folded item ({A1, A2a, A6, A14}) is test-proven.
   **Both-DB caveat (honest):** the product-auth contract/durable suites run over the in-memory FS
   backend; the durable store tests run the real production code path over `InMemoryBackend`. The
   folded A1 durable test exercises the real `FilesystemAuthProductServices` path. A DB-backed auth
   harness (Postgres/libSQL `RootFilesystem`) remains a feasible follow-up, unchanged from main.
3. **Fixed on this rollup:** **A3** (removal/disconnect now cancels pending flows — see the §S entry;
   behavior added + tested at fake and durable tiers per the 2026-07-15 owner decisions, cross-surface
   enumeration proven; the `canceled_turn_gate_continuations` parked-turn notification is deliberately
   omitted, owned by the main-delta reconciliation).
   **Deferred:** A5 (`⏸`), A2b, A7, A10, A12, A16. **Waived/moot:** A9, A13, A15, A17. **Optional:** A8.
