# Agent Map - ironclaw_auth

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for dependencies and feature shape.
- Use `docs/reborn/contracts/auth-product.md` and issues #3289 / #3810 / #3883 / #3884 as the source of truth.

## Module Charter — two engines, four owners

This crate is **two engines** (PROPOSAL §6.4.8), and they do not name each
other: `src/engine/` runs every conversation with a vendor, `src/product_auth/`
runs the durable product-facing lifecycle. Each module's `mod.rs` doc comment
carries its own charter — what it owns and what must never drift in — and
`CLAUDE.md`'s `## Sub-owner map` charts **every** `src/**/*.rs` file across
four owners: the two engines plus `vocabulary` (what both engines stand on and
neither owns) and `test-support`.

Both halves are enforced by `tests/module_charter.rs`: every file has exactly
one owner and every charted path exists, **and** `engine` must not name
`product_auth` nor `product_auth` name `engine`. Read the map before adding a
file — a new one fails the gate until it is given an owner, and a file only one
engine names belongs to that engine rather than to `vocabulary`.

## What This Crate Owns

- Product-facing Reborn auth setup contracts and implementations: auth flows,
  secure manual-token interactions, durable filesystem product-auth records,
  credential accounts, runtime selection/refresh, recovery/account-selection
  projections, provider exchange/refresh, continuations, recipes, fakes, and
  cleanup.
- ~~Temporary v1 loopback OAuth callback transport in `loopback_oauth`, re-exported through `oauth`, folded from `ironclaw_oauth` in W2.1 and deleted with v1.~~ **Struck 2026-08-04 (WS6): deleted.** Neither `loopback_oauth` nor its `urlencoding` dependency is in the tree; §6.4.8's delete clause already landed. Do not re-add a fixed-port callback transport.
- Fake in-memory services for contract tests and downstream caller tests.
- Redacted DTOs safe for WebUI, CLI, chat, API, and projection rendering.

## Do Not Move In Here

- New V1 route handlers, V1 pending maps, V1 extension manager authority, or V1 `SecretsStore` access. `loopback_oauth` is **deleted** (see above): do not re-add a fixed-port loopback callback transport, under that name or any other, and do not reintroduce `urlencoding` to serve one.
- Raw HTTP clients, host-runtime credential injection adapters, HTTP route
  serving, extension lifecycle mutation, or turn replay/resume. Durable
  product-auth records may live here; encrypted raw token material still stays
  behind `SecretStore` handles.
- Raw OAuth codes, PKCE verifiers, access tokens, refresh tokens, backend provider bodies, host paths, or raw secret values in serializable records, errors, logs, docs, or projections. Tests may use sentinel values only to prove redaction.

## Validation

- Fast local check: `cargo test -p ironclaw_auth`
- Lint check: `cargo clippy -p ironclaw_auth --all-targets -- -D warnings`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`

## Agent Notes

- Behavior may be compatible with V1, but Reborn code paths must remain separate from V1 code paths.
- V1 behavior inventory is documentation and compatibility evidence only.
- Prefer caller/service-level tests when auth flows consume callback state, submit secrets, create accounts, emit continuations, or clean up grants.
