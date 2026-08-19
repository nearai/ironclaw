# ironclaw_auth Guardrails

**Gate-pinned:** `cargo test -p ironclaw_auth --test module_charter` reads the
"Sub-owner map" section below and separately pins the two-engine severance, so
the map is enforced, not prose — edit it with that suite in hand and do not
reflow the table. Orientation, measured surface/deps, and test commands
are in [`README.md`](./README.md); the family boundary is
[`../AGENTS.md`](../AGENTS.md). Sources of truth beyond the crate:
`docs/internal/reborn/contracts/auth-product.md` and PROPOSAL §6.4.8.

## Sub-owner map

PROPOSAL §6.4.8 asks for the **two-engine split (engine vs product_auth)** to
become "two chartered top-level modules". Both modules exist and are already
severed — neither names the other. What was missing is the charter, and
building it refuted the "two owners" framing: measured symbol-by-symbol,
**6 of the 11 shared top-level modules are named by _both_ engines**
(`credential`, `provider`, `oauth`, `scope`, `ids`, `error`), so charging them
to either engine would make one engine the owner of the other's dependencies.
There are **four** owners, not two. Each engine's own charter — what it owns
and what must never drift in — is in its `mod.rs` doc comment.

**This table is enforced.** `tests/module_charter.rs` asserts every `.rs` file
under `src/` appears in exactly one row and every path in a row exists, so the
map cannot rot in either direction, and it separately pins the severance the
two-engine split exists for: `engine` must not name `product_auth`, and
`product_auth` must not name `engine`.

| Sub-owner | Owns | Never contains | Files |
|---|---|---|---|
| `engine` | Every conversation with a vendor: authorize URLs, scope validation against the recipe ceiling, `oauth2_code`+PKCE, `api_key`+probe, RFC 7591 DCR, token exchange/refresh, the keepalive sweep and its leader lock, admission metadata, and the auth-account state machine | A vendor-conditional code path, or any durable product-auth lifecycle | `engine/mod.rs`, `engine/admission.rs`, `engine/dcr.rs`, `engine/exchange.rs`, `engine/http.rs`, `engine/keepalive.rs`, `account_state.rs` |
| `product-auth` | The durable, product-facing lifecycle: auth flows, credential accounts and their selection/recovery/refresh serialization, secure interactions and manual-token submission, ownership-aware cleanup, per-user delivery registrations, the filesystem-backed stores, the OAuth turn-gate, and the device-link step machine | A vendor handshake — that is `engine`, without exception | `product_auth/mod.rs`, `product_auth/api/mod.rs`, `product_auth/api/auth.rs`, `product_auth/api/auth/tests.rs`, `product_auth/credentials/mod.rs`, `product_auth/credentials/manual_token_flow.rs`, `product_auth/credentials/product_auth_refresh_lock.rs`, `product_auth/credentials/runtime_credentials.rs`, `product_auth/credentials/runtime_credentials/host_managed_fallback.rs`, `product_auth/credentials/runtime_credentials/tests.rs`, `product_auth/credentials/runtime_credentials/tests/duplicate_selection.rs`, `product_auth/device_link/mod.rs`, `product_auth/device_link/driver.rs`, `product_auth/device_link/driver/tests.rs`, `product_auth/durable/mod.rs`, `product_auth/durable/accounts.rs`, `product_auth/durable/cleanup.rs`, `product_auth/durable/domain.rs`, `product_auth/durable/flows.rs`, `product_auth/durable/interactions.rs`, `product_auth/durable/paths.rs`, `product_auth/durable/provider.rs`, `product_auth/durable/tests.rs`, `product_auth/oauth/mod.rs`, `product_auth/oauth/oauth_gate.rs`, `cleanup.rs`, `domain.rs`, `flow.rs`, `interaction.rs`, `product_prompt.rs`, `channel_connection.rs`, `delivery_registrations.rs` |
| `vocabulary` | What **both** engines stand on and neither owns: the crate's identifiers and hashes, the error taxonomy, the auth scope/surface pair, OAuth protocol types and PKCE helpers, the `AuthProviderClient` port, and credential-account types | Behavior either engine could own alone — if only one engine names it, it belongs to that engine | `lib.rs`, `ids.rs`, `error.rs`, `scope.rs`, `oauth.rs`, `provider.rs`, `credential.rs` |
| `test-support` | Test doubles and the cross-implementation conformance suite, including the published `test-support` feature downstream harnesses consume | Production behavior | `fakes.rs`, `fakes/device_link.rs`, `test_support.rs`, `test_support/conformance.rs` |

Three placement calls worth stating, because each is a file whose *location*
suggests one owner and whose *use* is another:

- **`account_state.rs` is `engine`, not `vocabulary`**, even though it sits at
  the crate root beside the shared types. Measured: `AuthAccountState` is named
  by `engine/` and by **zero** files in `product_auth/`, and `engine/mod.rs`'s
  own doc already claims "the auth-account state machine" as engine-owned. The
  other two symbols in the file (`AuthAccountLastError`,
  `project_auth_account_state`) are named by neither engine — they are public
  API consumed outside the crate.
- **`cleanup.rs`, `domain.rs`, `flow.rs` and `interaction.rs` are
  `product-auth`** despite living at the crate root: measured, every symbol
  either engine names in them is named by `product_auth/` only. They are the
  four files a later slice could physically `git mv` into `product_auth/`
  without touching the shared vocabulary; `domain.rs` would need a rename first
  because `product_auth/durable/domain.rs` already holds that name.
- **`credential.rs` is the one genuinely two-owner file.** Of its 25 exported
  symbols, 18 are `product_auth`-only and 6 are named by both engines —
  including `CredentialAccountService` and
  `ProviderBackedCredentialAccountService`, which `engine/keepalive.rs` drives
  for the refresh sweep. A file-granular map has to pick one, so it is charged
  to `vocabulary` (the shared half is what makes it un-movable), and splitting
  the service half out is owed work rather than a defect in this map.

## Guardrails

- Own product-facing auth vocabulary, durable filesystem-backed product-auth
  services, fake services, and the recipe-driven `AuthEngine`
  (extension-runtime workstream D): `oauth2_code` + PKCE, `api_key` + probe,
  RFC 7591 dynamic client registration, and the `AuthAccountState` machine.
  The engine executes `ironclaw_extension_contracts::recipe::VendorAuthRecipe` data only — never
  add a vendor-conditional code path here; a vendor difference belongs in
  recipe data or (as a last resort, with an ADR) a narrow declared quirk hook.
- Engine transport is the injected `RuntimeHttpEgress` port and token storage is the injected `ironclaw_secrets::SecretStore`; every vendor request pins a network policy to the recipe endpoint's host and caps the response body. Vendor response bodies are never logged, stored, or embedded in errors — only stable OAuth error codes are extracted.
- ~~Temporary exception: `loopback_oauth` contains the v1 fixed-port OAuth callback transport folded from `ironclaw_oauth`; do not add Reborn consumers, and delete it with v1.~~ **Struck 2026-08-04 (WS6): the module is gone.** PROPOSAL §6.4.8's "Deletes: `loopback_oauth` + its `urlencoding` dep" already landed — neither the file nor the dependency is in the tree. Do not re-add a fixed-port callback transport here; the callback arrives over the WebUI product-auth routes.
- Exception: `ProviderBackedCredentialAccountService` may live here because refresh serialization and status projection belong at the `CredentialAccountService` boundary, while raw provider/token material stays behind `AuthProviderClient` and secret boundaries.
- Keep Reborn auth code independent from V1 route handlers, V1 pending state,
  V1 extension manager authority, V1 secret-store implementation details,
  WebUI route serving, and host-runtime credential injection adapters.
- Serializable records may contain hashes, ids, handles, statuses, and redacted metadata. They must not contain raw OAuth state, PKCE verifiers, authorization codes, tokens, secret values, provider response bodies, backend internals, or host paths.
- Raw OAuth callback material may appear only in non-serializable one-shot inputs to provider exchange boundaries.
- Token refresh must go through `CredentialAccountService::refresh_account` and `AuthProviderClient::refresh_token`. Refresh requests/results stay behind host-mediated auth/provider boundaries, revalidate scope/provider/ownership/grants, and project recoverable failures as stable statuses rather than raw provider detail.
- `AuthFlowRecordSource` is the auth-owned read/list seam for product interaction read models. Composition crates may wire it, but should not define parallel auth-flow snapshot traits.
- Cleanup lifecycle handling must be ownership-aware and idempotent. Deactivate/uninstall may revoke extension-owned accounts or remove grants, but reusable/shared/system credentials must not be deleted by default; partial failures should surface stable quarantine categories only.
- Manual token values must move through `SecretString` inputs and must not appear in `Debug`, errors, projections, or docs. Tests may use sentinel values only to prove redaction.
- Credential recovery/account-selection projections must expose only stable status/reason categories and redacted authorized choices. Revalidate scope, provider, configured status, ownership, and grants when selecting a `CredentialAccountId`; ids are not authority by themselves.
- Use strong newtypes for auth-domain identifiers and hashes; deserialize through validation.
- Public wire enums must use stable snake_case serde names.
- Fakes should fail closed and model important state transitions closely enough that production consumers cannot depend on unsafe shortcuts.
