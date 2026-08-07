# ironclaw_auth

Product-facing authentication: the typed flow, interaction,
credential-account, recovery, provider-exchange, continuation, and cleanup
contracts, their durable filesystem-backed services, and the recipe-driven
`AuthEngine` that runs every conversation with a vendor. The family's
credential-custody domain — it holds token-*lifecycle* state, never raw secret
bytes (those stay behind `ironclaw_secrets` handles), and it never makes an
authorization decision.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_auth` · **Manifest:** `crates/domains/ironclaw_auth/Cargo.toml`
- **Use this when:** adding or changing a product auth flow, credential
  account behavior, manual-token or gated-OAuth interaction, keepalive/refresh,
  or an OAuth recipe execution concern.
- **Don't use this when:** the credential is a *model-provider* session →
  `ironclaw_llm` (auth-sessions owner); host/browser login → `ironclaw_webui`;
  serving HTTP routes or injecting credentials into tool runtimes → the
  product/kernel tiers; a vendor difference → recipe **data**
  (`ironclaw_extension_contracts::recipe::VendorAuthRecipe`), never a code
  branch here.

## Public surface

- The trait set: `AuthFlowManager`, `AuthInteractionService`,
  `CredentialAccountService` (+ `ProviderBackedCredentialAccountService`,
  `CredentialSetupService`), `SecretCleanupService`, `AuthProviderClient`
  (exchange/refresh), `ChannelConnectionService`.
- The engine: `AuthEngine`, `AuthRecipeResolver` (implemented by the extension
  host), OAuth admission metadata, DCR, keepalive sweep
  (`spawn_keepalive_sweep` + leader lock).
- Durable services: `FilesystemAuthProductServices`; the OAuth turn-gate
  (`OAuthGateFlowDriver`); runtime credential selection/refresh services.
- Redacted DTOs and validated newtypes (`ids`, `oauth`) safe for every product
  surface to render.
- `test-support` feature: `InMemoryAuthProductServices` fakes + the
  conformance suite (`test_support::conformance`) — gated out of release
  builds.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_common`, `ironclaw_event_log`,
  `ironclaw_extension_contracts` (recipe vocabulary),
  `ironclaw_filesystem`, `ironclaw_host_api`, `ironclaw_product_contracts`,
  `ironclaw_secrets`.
- **Consumed by (7):** `ironclaw` (CLI), `ironclaw_assistant`,
  `ironclaw_composition`, `ironclaw_extension_host`,
  `ironclaw_extension_manager`, `ironclaw_extension_support`,
  `ironclaw_webui`.

## Invariants

- **The sub-owner map in [`CLAUDE.md`](./CLAUDE.md) is enforced**, not
  documentation: `cargo test -p ironclaw_auth --test module_charter` asserts
  every `src/**/*.rs` file has exactly one of the four owners (`engine`,
  `product-auth`, `vocabulary`, `test-support`) and that the two engines never
  name each other.
- No raw OAuth codes, PKCE verifiers, tokens, provider bodies, host paths, or
  secret values in any serializable shape — the redaction rules in
  `CLAUDE.md`; tests use sentinels only to prove redaction.
- No turn-kernel dependency (the gate-prompt vocabulary arrives via
  `ironclaw_host_api`); the `BoundaryRule { crate_name: "ironclaw_auth" }` in
  `reborn_dependency_boundaries.rs` forbids kernel/loop/product/app crates and
  `ironclaw_llm` (model-provider sessions are a deliberately separate stack).
- Engine transport is the injected egress port with per-recipe host pinning
  and capped response bodies; vendor response bodies never reach logs, stores,
  or errors.

## Tests

```bash
cargo test -p ironclaw_auth
cargo test -p ironclaw_auth --test module_charter        # the enforced sub-owner map
cargo test -p ironclaw_auth --test auth_engine_contract
cargo test -p ironclaw_auth --test auth_product_contract
```

## See also

- Working rules and the enforced sub-owner map: [`CLAUDE.md`](./CLAUDE.md)
  (canonical crate guidance).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Contract doc: `docs/reborn/contracts/auth-product.md`; design record:
  `families/domains.md`, PROPOSAL §6.4.8.
