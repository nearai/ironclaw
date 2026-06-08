# Agent Map — ironclaw_reborn_openai_compat_storage

## Start Here

- Read `CLAUDE.md` first; it defines the storage boundary.
- Read `../ironclaw_reborn_openai_compat/CLAUDE.md` before changing ref or DTO
  semantics.

## What This Crate Owns

- Durable storage adapters for Reborn OpenAI-compatible public refs and
  idempotency mappings.
- Persistence behind the `OpenAiCompatRefStore` port.

## Do Not Move In Here

- OpenAI-compatible HTTP route handlers or listener binding.
- ProductWorkflow orchestration, route submission, or runtime dispatch.
- v1 gateway handlers, direct LLM proxying, or concrete channel behavior.

## Validation

- `cargo test -p ironclaw_reborn_openai_compat_storage`
- `cargo clippy -p ironclaw_reborn_openai_compat_storage --all-targets --all-features -- -D warnings`
- `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
