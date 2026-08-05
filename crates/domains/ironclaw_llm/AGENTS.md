# Agent Map — ironclaw_llm

## Start Here

- Read `CLAUDE.md` first; it contains detailed provider and reasoning notes.
- **`CLAUDE.md`'s `## Sub-owner map` is the authoritative file→owner
  assignment** and is enforced by `tests/module_charter.rs` (every `src/` file
  has exactly one owner; the table cannot rot in either direction). The
  informal buckets below are a reading aid for orientation only — when they
  disagree with the map, the map wins.
- Read `Cargo.toml` for provider dependencies and feature shape.
- Relevant files by area:
  - `provider.rs`, `registry.rs`, `runtime.rs`, `host.rs` — provider contracts and host seams.
  - `retry.rs`, `failover.rs`, `circuit_breaker.rs`, `response_cache.rs` — reliability and caching.
  - `tool_schema.rs`, `rig_adapter.rs`, `openai_codex_provider.rs` — tool-call/schema boundaries.
  - `reasoning.rs` — model-response text hygiene: `clean_response` plus
    textual tool-call recovery. **Not legacy and not dead** — the v1 engine
    half was deleted in #6964; what remains has five production call sites in
    `ironclaw_loop_host/src/model_gateway.rs`.
  - `*_oauth.rs`, `*_auth.rs`, `session.rs`, `token_refreshing.rs` — provider auth/session handling.

## What This Crate Owns

- Multi-provider LLM integration with retry, failover, circuit breaker, and response caching.
- `LlmProvider` trait, provider registry/chain construction, runtime adapters, tracing/recording, model/cost metadata.
- Provider-specific auth for NEAR AI, Anthropic/Gemini OAuth, GitHub Copilot, OpenAI Codex/ChatGPT, AWS Bedrock.
- Tool schema normalization and provider-specific tool-call compatibility.
- Transcription, image/vision model helpers, smart routing, and test fault-injection support.

## Do Not Move In Here

- Agent-loop/thread ownership or product workflow; those live in engine/turns/product crates.
- Tool execution side effects; `complete_with_tools()` must pass through cache and leave side effects to callers.
- Unredacted tokens, refresh tokens, request bodies, or provider internals in public errors/logs.
- Direct assumptions that one provider's model override/schema/auth behavior applies globally.

## Validation

- Fast local check: `cargo test -p ironclaw_llm`
- Lint check: `cargo clippy -p ironclaw_llm --all-targets --all-features -- -D warnings`
- Run caller tests when changing provider trait, tool schemas, reasoning output, auth refresh, or failover behavior.

## Agent Notes

- `RigAdapter` forwards per-request model overrides through rig-core's typed request model field; do not add a second `model` entry through flattened provider parameters.
- `complete_with_tools()` is never cached because tool calls can have side effects.
- OpenAI strict schema normalization rewrites optional object fields into required-nullable strict mode where required.
- 401 handling differs by auth mode; preserve provider-specific semantics from `CLAUDE.md`.
