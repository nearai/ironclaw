# Agent Map — ironclaw_reborn_openai_compat

## Start Here

- Read `CLAUDE.md` first; it defines the OpenAI-compatible Reborn boundary.
- Read `src/descriptors.rs` before changing routes or ingress policy.
- Read `src/error.rs` before changing any HTTP error shape.

## What This Crate Owns

- Reborn-native OpenAI-compatible HTTP route descriptors.
- Chat Completions and Responses API DTOs used by the migration slices.
- A sanitized OpenAI-compatible error envelope.
- Feature-gated fail-closed axum route fragments for host composition to mount.

## Do Not Move In Here

- Listener binding or `axum::serve`.
- v1 gateway handlers, `src/channels/web`, or direct LLM proxy behavior.
- Direct dispatcher, runtime, DB, secrets, network, or host-runtime access.
- Execution of client-supplied OpenAI tools as Reborn capabilities.

## Validation

- `cargo test -p ironclaw_reborn_openai_compat --features openai-compat-beta`
- `cargo clippy -p ironclaw_reborn_openai_compat --all-targets --all-features -- -D warnings`
- `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
