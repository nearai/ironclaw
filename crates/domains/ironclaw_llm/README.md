# ironclaw_llm

Multi-provider LLM integration: the `LlmProvider` contract, one concrete
adapter per model vendor, provider authentication/session handling, the
provider registry, reliability decorators (retry, circuit breaker, failover,
cache, hot-reload, smart routing), and trace recording. The family's
explicitly vendor-scoped provider cone — vendor SDKs and their auth flows are
isolated here so no non-LLM consumer inherits them.

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_llm` · **Manifest:** `crates/domains/ironclaw_llm/Cargo.toml`
- **Use this when:** adding a model provider, changing provider
  selection/reliability/caching, provider auth or session refresh, tool-schema
  normalization, or trace recording of model calls.
- **Don't use this when:** the credential is a *product* integration
  credential → `ironclaw_auth` (recipes); turn orchestration or prompt content
  → the loop/product tiers; pricing a run's usage →
  `ironclaw_common::llm_costs` (deliberately below this crate so surfaces can
  price without the provider cone).

## Public surface

- `LlmProvider` — the contract (`complete`, `complete_with_tools`, streaming
  variants, model metadata/cost hooks); `LlmError` taxonomy.
- Factory/assembly: `build_provider_chain`, `create_llm_provider`,
  `create_registry_provider`; `ProviderRegistry` / `ProviderProtocol`;
  `SwappableLlmProvider` + `LlmReloadHandle`.
- Decorators: `RetryProvider`, `CircuitBreakerProvider`, `FailoverProvider`,
  `CachedProvider`, `SmartRoutingProvider`, `TokenRefreshingProvider`.
- Providers: NEAR AI, OpenAI (+ Codex/ChatGPT), Anthropic (+ OAuth), Gemini
  OAuth, GitHub Copilot, AWS Bedrock (`--features bedrock`), rig-core-backed
  OpenAI-compatible endpoints.
- Recording: `RecordingLlm` (+ `trace_binding`); model catalogs
  (`models`, `vision_models`, `reasoning_models`, `image_models`).
- `test-support` feature: `testing::StubLlm`, fault injection,
  `provider_chain_over`.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_common`, `ironclaw_safety`.
- **Consumed by (6):** `ironclaw_composition`, `ironclaw_extension_host`,
  `ironclaw_loop_host`, `ironclaw_operator`, `ironclaw_stress`,
  `ironclaw_trace_commons` (recording vocabulary — the inventoried
  same-layer edge).

## Invariants

- **The sub-owner map in [`CONTRACT.md`](./CONTRACT.md) is enforced:** `cargo test
  -p ironclaw_llm --test module_charter` asserts every `src/**/*.rs` file has
  exactly one of the ten owners; a new file fails until charted.
- Vendor names are legal **here only** within the family —
  `reborn_extension_specificity.rs` carves this crate's `src/` out for
  LLM-vendor terms and scans everyone else.
- `complete_with_tools()` is never cached (tool calls can have side effects);
  decorator assembly happens only through the crate's own
  `apply_decorator_chain`.
- Authorization to call a model at all is a kernel decision made before
  dispatch reaches this crate — no kernel dependency, per the layer matrix.

## Tests

```bash
cargo test -p ironclaw_llm
cargo test -p ironclaw_llm --test module_charter   # the enforced sub-owner map
cargo clippy -p ironclaw_llm --all-targets --all-features -- -D warnings
```

## See also

- **The module spec is [`CONTRACT.md`](./CONTRACT.md)** (named in the root
  `AGENTS.md` Module Specs table): provider selection env vars, per-provider
  gotchas, decorator chain order, host trait surface. This README only
  orients — the spec is canonical.
- Orientation for agents: [`AGENTS.md`](./AGENTS.md) (pointer + reading aid).
- Family boundary: [`../AGENTS.md`](../AGENTS.md); design record:
  `families/domains.md`, PROPOSAL §6.4.13.
