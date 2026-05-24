# Agent Map - ironclaw_auth

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for dependencies and feature shape.
- Use `docs/reborn/contracts/auth-product.md` and issue #3289 / #3810 as the source of truth.

## What This Crate Owns

- Product-facing Reborn auth setup contracts: auth flows, secure manual-token interactions, credential accounts, provider exchange, continuations, and cleanup.
- Fake in-memory services for contract tests and downstream caller tests.
- Redacted DTOs safe for WebUI, CLI, chat, API, and projection rendering.

## Do Not Move In Here

- V1 route handlers, V1 pending maps, V1 extension manager authority, or V1 `SecretsStore` access.
- Durable encrypted secret storage, secret leases, raw HTTP clients, runtime credential injection, extension lifecycle mutation, or turn replay/resume.
- Raw OAuth codes, PKCE verifiers, access tokens, refresh tokens, backend provider bodies, host paths, or raw secret values in serializable records, errors, logs, docs, or projections. Tests may use sentinel values only to prove redaction.

## Validation

- Fast local check: `cargo test -p ironclaw_auth`
- Lint check: `cargo clippy -p ironclaw_auth --all-targets -- -D warnings`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`

## Agent Notes

- Behavior may be compatible with V1, but Reborn code paths must remain separate from V1 code paths.
- V1 behavior inventory is documentation and compatibility evidence only.
- Prefer caller/service-level tests when auth flows consume callback state, submit secrets, create accounts, emit continuations, or clean up grants.
