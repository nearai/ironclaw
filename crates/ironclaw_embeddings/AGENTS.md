# ironclaw_embeddings

Owns the shared embedding-provider trait, concrete provider impls, the LRU caching decorator, and the async factory used by everything in the workspace that needs vector embeddings.

## Responsibilities

- Define `EmbeddingProvider`, `EmbeddingError`, and the pure-data `EmbeddingsConfig` shape that callers fill from their own settings layer.
- House concrete provider impls: OpenAI / OpenAI-compatible, NEAR AI, Ollama, and AWS Bedrock (gated behind the `bedrock` cargo feature).
- Provide the `create_provider(config, deps)` async factory — the only supported way to construct a provider.
- Wrap an `Arc<dyn EmbeddingProvider>` in `CachedEmbeddingProvider` (LRU) via `EmbeddingCacheConfig` for callers that want hot-path caching.
- Run a baseline defense-in-depth URL check (`url_check::check_base_url`) inside the factory: reject non-http(s) schemes, unparseable URLs, and the AlwaysBlocked IP class (cloud-metadata `169.254.169.254`, link-local, multicast, `0.0.0.0`/`::`).
- Expose `default_dimension_for_model` so the binary can pick a dimension without hard-coding the table.

## Non-responsibilities

- Do not read `Settings`, env vars, or DB rows. The binary-side resolver at `src/config/embeddings.rs::resolve_embeddings_config` owns that — it produces an `EmbeddingsConfig` and hands it to the factory.
- Do not implement the full operator-tunable SSRF policy. `validate_operator_base_url` in `src/config/helpers.rs` is the policy-aware layer with allow/deny lists and DNS resolution; the crate's `url_check::check_base_url` is only the AlwaysBlocked-class floor.
- Do not decide cache size, whether to cache, or how to wire the provider into a workspace. Callers choose via `Workspace::with_embeddings_cached` / `with_embeddings_uncached`.
- Do not expose concrete provider constructors. `OpenAiEmbeddings`, `NearAiEmbeddings`, `OllamaEmbeddings`, `BedrockEmbeddings`, and `MockEmbeddings` stay crate-private; downstream code holds `Arc<dyn EmbeddingProvider>` only.
- Do not perform background work, retries, or circuit-breaking. Providers are thin clients; resilience belongs to the caller.

## Public surface

| Symbol | Use |
|--------|-----|
| `EmbeddingProvider` trait, `EmbeddingError` | Trait object + error returned by every provider |
| `create_provider(config, deps) -> Option<Arc<dyn EmbeddingProvider>>` | The factory. Returns `None` when embeddings are disabled or the resolved config can't yield a working provider |
| `ProviderDeps { session, bedrock_setup }` | Runtime wiring the factory needs that doesn't live in `EmbeddingsConfig` |
| `EmbeddingsConfig`, `DEFAULT_EMBEDDING_CACHE_SIZE`, `default_dimension_for_model` | Pure-data config + helpers consumed by the binary's resolver |
| `CachedEmbeddingProvider`, `EmbeddingCacheConfig` | LRU caching decorator |
| `BedrockEmbeddingSetup` | Compiled unconditionally so callers can build one without the `bedrock` feature; the underlying impl is `#[cfg(feature = "bedrock")]` |
| `MockEmbeddings` | Deterministic test double, gated behind the `testing` cargo feature |

## Safety rules

- Concrete provider constructors stay crate-private. New providers are reached only through `create_provider`; this is the single security boundary all callers must traverse.
- Any provider that takes a base URL MUST call `url_check::check_base_url` in its factory match arm before its constructor runs. The crate-level check is the AlwaysBlocked floor; do not omit it on the assumption that a downstream resolver will catch it — a caller constructing `EmbeddingsConfig` directly skips the resolver entirely.
- `max_input_length` measures bytes (matches `str::len()`), not characters. Keep the trait docs, each impl's inline comment, and any `text.len()` truncation logic consistent.
- `embed_batch` overrides on each provider must validate every input against `max_input_length()` before issuing the request — the caller-level `embed()` length check does not run for batches.
- `EmbeddingsConfig` is plain data with no construction-time validation of base URLs. The factory is the only enforcement point; do not move URL validation into `EmbeddingsConfig::new` (there isn't one) or onto `Deserialize`, or two construction paths will drift.
- The `testing` and `bedrock` features must be additive only — the default build must continue to compile and run without either.

## Where the binary plugs in

The wiring lives outside this crate:

- `src/config/embeddings.rs::resolve_embeddings_config` reads `Settings` + env vars, runs the operator-tunable SSRF policy, and returns an `EmbeddingsConfig`.
- `src/app.rs` (and `src/cli/mod.rs`) call `ironclaw_embeddings::create_provider(&cfg, ProviderDeps { session, bedrock_setup })` and attach the result to `Workspace` via `with_embeddings_cached`.
- `src/cli/doctor.rs::check_embeddings` reports configuration status and credential presence per provider.

When changing this crate's public surface, grep those three call sites first.
