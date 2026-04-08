# LLM Module

Multi-provider LLM integration with circuit breaker, retry, failover, and response caching.

## File Map

| File | Role |
|------|------|
| `mod.rs` | Provider factory (`create_llm_provider`, `build_provider_chain`); `LlmBackend` enum |
| `config.rs` | LLM config types (`LlmConfig`, `RegistryProviderConfig`, `NearAiConfig`, `BedrockConfig`) |
| `error.rs` | `LlmError` enum used by all providers |
| `provider.rs` | `LlmProvider` trait, `ChatMessage`, `ToolCall`, `CompletionRequest`, `sanitize_tool_messages` |
| `nearai_chat.rs` | NEAR AI Chat Completions provider (dual auth: session token or API key) |
| `codex_auth.rs` | Reads Codex CLI `auth.json`, extracts tokens, refreshes ChatGPT OAuth access tokens |
| `codex_chatgpt.rs` | Custom Responses API provider for Codex ChatGPT backend (`/backend-api/codex`) |
| `openai_codex_provider.rs` | OpenAI Codex Responses API client (SSE streaming, JWT auth, subscription billing) |
| `openai_codex_session.rs` | OAuth 2.0 session manager for OpenAI Codex (device code flow, token persistence) |
| `token_refreshing.rs` | Token-refreshing `LlmProvider` decorator for OpenAI Codex (pre-emptive refresh, zero-cost billing) |
| `reasoning.rs` | `Reasoning` struct, `ReasoningContext`, `RespondResult`, `ActionPlan`, `ToolSelection`; thinking-tag stripping; `SILENT_REPLY_TOKEN` |
| `session.rs` | NEAR AI session token management with disk + DB persistence, OAuth login flow |
| `circuit_breaker.rs` | Circuit breaker: Closed → Open → HalfOpen state machine |
| `retry.rs` | Exponential backoff retry wrapper; `is_retryable()` classification |
| `failover.rs` | `FailoverProvider` — tries providers in order with per-provider cooldown |
| `response_cache.rs` | In-memory LLM response cache with TTL and LRU eviction (keyed by SHA-256) |
| `costs.rs` | Static per-model cost table (OpenAI, Anthropic, local/Ollama heuristics) |
| `rig_adapter.rs` | Adapter bridging rig-core `CompletionModel` → `LlmProvider`; used by OpenAI, Anthropic, Ollama, Tinfoil |
| `smart_routing.rs` | `SmartRoutingProvider` — 13-dimension complexity scorer routes cheap vs primary model |
| `recording.rs` | `RecordingLlm` — trace capture for E2E replay testing (`IRONCLAW_RECORD_TRACE`) |
| `bedrock.rs` | AWS Bedrock provider via native Converse API (feature-gated: `--features bedrock`) |

## Provider Selection

Set via `LLM_BACKEND` env var:

| Value | Provider | Key env vars |
|-------|----------|-------------|
| `nearai` (default) | NEAR AI Chat Completions | `NEARAI_SESSION_TOKEN` or `NEARAI_API_KEY` |
| `openai` | OpenAI | `OPENAI_API_KEY` |
| `anthropic` | Anthropic | `ANTHROPIC_API_KEY` |
| `github_copilot` | GitHub Copilot Chat API | `GITHUB_COPILOT_TOKEN`, `GITHUB_COPILOT_MODEL` |
| `ollama` | Ollama local | `OLLAMA_BASE_URL` |
| `openai_compatible` | Any OpenAI-compatible endpoint | `LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL` |
| `tinfoil` | Tinfoil TEE inference | `TINFOIL_API_KEY`, `TINFOIL_MODEL` |
| `bedrock` | AWS Bedrock (requires `--features bedrock`) | `BEDROCK_REGION`, `BEDROCK_MODEL`, `AWS_PROFILE` |
| `openai_codex` | OpenAI Codex (ChatGPT subscription) | `OPENAI_CODEX_MODEL`, `OPENAI_CODEX_CLIENT_ID` |

Codex auth reuse:
- Set `LLM_USE_CODEX_AUTH=true` to load credentials from `~/.codex/auth.json` (override with `CODEX_AUTH_PATH`).
- If Codex is logged in with API-key mode, IronClaw uses the standard OpenAI endpoint.
- If Codex is logged in with ChatGPT OAuth mode, IronClaw routes to the private `chatgpt.com/backend-api/codex` Responses API via `codex_chatgpt.rs`.
- ChatGPT mode supports one automatic 401 refresh using the refresh token persisted in `auth.json`.

## AWS Bedrock Provider

Uses the native Converse API via `aws-sdk-bedrockruntime` (`bedrock.rs`). Requires `--features bedrock` at build time — not in default features due to heavy AWS SDK dependencies.

**Auth:** Standard AWS credential chain — IAM credentials (`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`), SSO profiles (`AWS_PROFILE`), or instance roles. The SDK resolves auth automatically from the environment.

**Config:**
- `BEDROCK_REGION` — AWS region (default: `us-east-1`)
- `BEDROCK_MODEL` — Required model ID (e.g., `anthropic.claude-opus-4-6-v1`)
- `BEDROCK_CROSS_REGION` — Optional cross-region inference prefix (`us`, `eu`, `apac`, `global`)

## GitHub Copilot Provider Notes

`github_copilot` uses a dedicated `GithubCopilotProvider` (`github_copilot.rs`) with
direct HTTP via `reqwest::Client`. It cannot use `RigAdapter` because the Copilot API
requires a two-step authentication flow: a long-lived GitHub OAuth token is exchanged
for a short-lived Copilot session token via `api.github.com/copilot_internal/v2/token`.
The session token is cached and auto-refreshed before expiry by `CopilotTokenManager`
in `github_copilot_auth.rs`.

The API endpoint is `https://api.githubcopilot.com/chat/completions` (OpenAI Chat
Completions format). Token source: `GITHUB_COPILOT_TOKEN` env var, or the
`oauth_token` from your IDE sign-in flow (`~/.config/github-copilot/apps.json`).
The setup wizard supports GitHub device login or manual token paste.

**Known risk:** The device login flow uses the VS Code Copilot OAuth client ID
(`Iv1.b507a08c87ecfe98`) and injects VS Code identity headers (`User-Agent`,
`Editor-Version`, `Editor-Plugin-Version`, `Copilot-Integration-Id`). GitHub could
rotate this client ID at any time. If GitHub publishes an official third-party client
ID, migrate to it immediately. Advanced users can override headers via
`GITHUB_COPILOT_EXTRA_HEADERS`.

## NEAR AI Provider Gotchas

**Dual auth modes:**
- **Session token** (default): `NEARAI_SESSION_TOKEN=sess_...`, base URL = `https://private.near.ai`. Tokens are persisted to `~/.ironclaw/session.json` (mode 0600) and optionally to the DB `settings` table (`nearai.session_token`). On 401 responses where the body contains "session" + "expired"/"invalid", `NearAiChatProvider` calls `session.handle_auth_failure()` which triggers the interactive OAuth login flow and retries once. Plain `AuthFailed` 401s are not retried.
- **API key**: Set `NEARAI_API_KEY` (from `cloud.near.ai`), base URL defaults to `https://cloud-api.near.ai`. 401s with API key auth are immediately returned as `LlmError::AuthFailed` — no renewal.

**Session renewal is interactive:** When `SessionExpired` triggers renewal, it blocks and prompts the user in the terminal (GitHub/Google OAuth or manual API key entry). This is unsuitable for headless/hosted deployments — set `NEARAI_SESSION_TOKEN` env var instead.

**Tool message flattening:** NEAR AI's API doesn't support `role: "tool"` messages in the standard format. `nearai_chat.rs` defaults `flatten_tool_messages = true`, converting tool results to user messages with `[Tool result from <name>]: <content>` format. Use `NearAiChatProvider::new_with_flatten(..., false)` to disable for compliant endpoints.

**Pricing auto-fetch:** On startup, `NearAiChatProvider` fires a background task to fetch per-model pricing from `/v1/model/list`. If the fetch fails, it silently falls back to `costs::model_cost()` / `costs::default_cost()`. Pricing is stored in-memory only.

**HTTP request timeout:** The NEAR AI HTTP client has a 120-second timeout per request. Rate limit `Retry-After` headers are parsed (both delay-seconds and HTTP-date formats) and forwarded as `LlmError::RateLimited { retry_after }` for the `RetryProvider` to honor.

## Circuit Breaker

State machine in `circuit_breaker.rs`:
```
Closed (normal)
  → Open (after failure_threshold consecutive transient failures; default: 5)
    → HalfOpen (after recovery_timeout; default: 30s)
      → Closed (after half_open_successes_needed probe successes; default: 2)
      → Open (if any probe fails)
```

**Transient vs non-transient errors:** Only `RequestFailed`, `RateLimited`, `InvalidResponse`, `SessionExpired`, `SessionRenewalFailed`, `Http`, and `Io` count toward the threshold. `AuthFailed`, `ContextLengthExceeded`, `ModelNotAvailable`, and `Json` errors never trip the breaker — they indicate caller problems, not backend degradation.

Configure via `NearAiConfig` fields: `circuit_breaker_threshold` (None = disabled), `circuit_breaker_recovery_secs` (default: 30).

The circuit breaker wraps the entire provider chain. When open, it immediately returns `LlmError::RequestFailed` with a message including remaining cooldown seconds. The `FailoverProvider` sitting outside can then try a fallback model.

## Failover Chain

`FailoverProvider` in `failover.rs` wraps a list of `LlmProvider` instances. On a retryable error, it tries the next provider in the list. Providers that fail repeatedly enter a cooldown period and are skipped (unless all providers are in cooldown, in which case the least-recently-cooled one is tried).

**Cooldown defaults:** `failure_threshold = 3` consecutive retryable failures → cooldown for `cooldown_duration = 300s`. Configure via `NearAiConfig` fields: `failover_cooldown_secs`, `failover_cooldown_threshold`.

**Current wiring:** The failover is set up between primary model and `NEARAI_FALLBACK_MODEL` (a different model name on the same NEAR AI backend), not across different LLM provider types. Cross-provider failover (e.g., NEAR AI → Anthropic) requires manual construction.

## Retry

`RetryProvider` in `retry.rs` wraps any `LlmProvider` with exponential backoff. Retries on: `RequestFailed`, `RateLimited`, `InvalidResponse`, `SessionRenewalFailed`, `Http`, `Io`. Does **not** retry: `AuthFailed`, `SessionExpired`, `ContextLengthExceeded`, `ModelNotAvailable`, `Json`.

**Backoff schedule:** base 1s doubled per attempt with ±25% jitter, minimum floor 100ms. Attempt 0: ~1s, attempt 1: ~2s, attempt 2: ~4s. For `RateLimited`, uses the `retry_after` duration from the error (provider-supplied) instead of backoff.

Configure via `NearAiConfig.max_retries` (env: `NEARAI_MAX_RETRIES`; default: 3). Set to 0 to disable.

## LlmProvider Trait

The full trait (all methods must be implemented or rely on defaults):

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    // Required
    fn model_name(&self) -> &str;
    fn cost_per_token(&self) -> (Decimal, Decimal);  // (input, output) per token
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;
    async fn complete_with_tools(&self, request: ToolCompletionRequest) -> Result<ToolCompletionResponse, LlmError>;

    // Optional (have defaults)
    async fn list_models(&self) -> Result<Vec<String>, LlmError> { Ok(vec![]) }
    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> { /* name only */ }
    fn effective_model_name(&self, requested_model: Option<&str>) -> String { /* uses active */ }
    fn active_model_name(&self) -> String { self.model_name().to_string() }
    fn set_model(&self, _model: &str) -> Result<(), LlmError> { /* Err: not supported */ }
    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal { /* uses cost_per_token */ }
}
```

Key notes:
- `model_name()` returns the configured model name; `active_model_name()` returns the currently active model (may differ if `set_model()` was called — only `NearAiChatProvider` supports this).
- `cost_per_token()` returns `(Decimal, Decimal)` using `rust_decimal`. Look up via `costs::model_cost()` in your constructor; fall back to `costs::default_cost()` for unknowns.
- `RigAdapter` ignores per-request model overrides (logs a warning). Only `NearAiChatProvider` supports per-request model overrides via `CompletionRequest::model`.
- `complete_with_tools()` is never cached (tool calls can have side effects) — `CachedProvider` always passes them through.

To add a new provider:
1. Create `src/llm/myprovider.rs` implementing `LlmProvider`
2. Add variant to `LlmBackend` in `mod.rs`
3. Wire into the factory match in `mod.rs`
4. Add env vars to `config/llm.rs` and `.env.example`

## Response Cache

`CachedProvider` in `response_cache.rs` caches `complete()` responses. `complete_with_tools()` is never cached (side effects). Cache key is SHA-256 of `(model_name, messages_json, max_tokens, temperature, stop_sequences)`. LRU eviction when `max_entries` is reached; TTL-based expiry on access.

**Defaults:** TTL = 1 hour, max entries = 1000. Configure via `NearAiConfig` fields: `response_cache_enabled` (env: `NEARAI_RESPONSE_CACHE_ENABLED`), `response_cache_ttl_secs`, `response_cache_max_entries`. Cache is in-memory only — evicted on restart.

## OpenAI-Compatible Custom Headers

Set `LLM_EXTRA_HEADERS=Key:Value,Key2:Value2` to inject headers into every request. Useful for OpenRouter attribution (`HTTP-Referer`, `X-Title`). Invalid header names/values are skipped with a warning (not a fatal error).

## OpenAI Codex Provider

Uses the Responses API at `chatgpt.com/backend-api/codex/responses` with ChatGPT subscription OAuth tokens (zero API cost — billing through subscription).

**Auth flow:** Device code OAuth via `auth.openai.com/api/accounts/deviceauth/*` endpoints. On first run, displays a code for the user to enter at a URL. Tokens are persisted to `~/.ironclaw/openai_codex_session.json` (mode 0600) and auto-refreshed before expiry.

**Provider chain:** `OpenAiCodexProvider` → `TokenRefreshingProvider` (pre-emptive refresh + retry on 401) → standard decorator chain. The `TokenRefreshingProvider` intercepts `AuthFailed`/`SessionExpired` errors, refreshes the OAuth token, and retries once.

**Key differences from other providers:**
- Uses Responses API (not Chat Completions) — SSE streaming with different event types
- System messages are sent as `instructions` field, not in `input` array
- Tool schemas are normalized via `normalize_schema_strict()` for OpenAI strict mode
- `cost_per_token()` returns `(0, 0)` — subscription-based billing
- `set_model()` returns error — model is fixed at construction time
- Image attachments are silently dropped with a warning log

**Env vars:** `OPENAI_CODEX_MODEL` (default: `gpt-5.3-codex`), `OPENAI_CODEX_CLIENT_ID`, `OPENAI_CODEX_AUTH_URL`, `OPENAI_CODEX_API_URL`.

## Provider Chain Construction

`build_provider_chain()` in `mod.rs` is the single source of truth for assembling decorators. It creates the base provider (dispatching to `create_openai_codex_provider()` for codex, `create_llm_provider()` for everything else), then applies all decorators inline:

```
Raw provider
  → RetryProvider           (per-provider backoff; wraps both primary and fallback)
  → SmartRoutingProvider    (cheap/primary split when NEARAI_CHEAP_MODEL is set)
  → FailoverProvider        (fallback model; only when NEARAI_FALLBACK_MODEL is set)
  → CircuitBreakerProvider  (fast-fail; only when NEARAI_CIRCUIT_BREAKER_THRESHOLD is set)
  → CachedProvider          (response cache; only when NEARAI_RESPONSE_CACHE_ENABLED=true)
  → RecordingLlm            (trace capture; only when IRONCLAW_RECORD_TRACE is set)
```

`build_provider_chain()` also returns a separate standalone cheap LLM provider (for heartbeat/evaluation tasks — not part of the decorator chain).

## reasoning.rs Contents

`reasoning.rs` does **not** contain an `IntentClassifier`. It contains:
- `Reasoning` struct — the main reasoning engine used by the agent worker; calls `complete_with_tools()` and handles tool dispatch
- `ReasoningContext` — carries messages, available tools, job description, and metadata into a reasoning call
- `RespondResult`, `ActionPlan`, `ToolSelection` — output types from the reasoning engine
- `TokenUsage` — input/output token counts
- `SILENT_REPLY_TOKEN` (`"NO_REPLY"`) and `is_silent_reply()` — used by the dispatcher to suppress empty responses in group chats
- Thinking-tag stripping — regex-based removal of `<thinking>`, `<reflection>`, `<scratchpad>`, `<|think|>`, `<final>`, etc. from model responses before returning to the user

## costs.rs Details

`costs.rs` provides a static lookup table (`model_cost(model_id)`) returning `(input_cost, output_cost)` per token as `rust_decimal::Decimal`. Provider prefixes like `"openai/gpt-4o"` are stripped before lookup. Returns `None` for unknown models — callers should fall back to `default_cost()` (roughly GPT-4o pricing). Local model heuristic (`is_local_model()`) returns zero cost for Ollama-style identifiers (llama*, mistral*, `:latest`, `:instruct`, etc.).

## rig_adapter.rs Details

`RigAdapter<M>` bridges any rig-core `CompletionModel` to `LlmProvider`. It is actively used in production for all non-NEAR AI providers (OpenAI, Anthropic, Ollama, Tinfoil, OpenAI-compatible). Key behaviors:
- **Per-request model overrides are silently ignored** (warning logged); the model is baked at construction time.
- **OpenAI strict-mode schema normalization** is applied to all tool definitions: `additionalProperties: false`, all properties added to `required`, optional fields made nullable via `"type": ["T", "null"]`. This happens transparently at the provider boundary.
- **System messages** are extracted into the rig-core `preamble` field (concatenated with newlines if multiple).
- **Tool call IDs** are generated (`generated_tool_call_{seed}`) if the provider returns empty/whitespace IDs.
- **Tool name normalization**: strips `proxy_` prefix if it matches a known tool (handles some proxy implementations).
- **OpenAI uses Chat Completions API** (`completions_api()`), not the newer Responses API — the Responses API path panics when tool results are sent back (rig-core doesn't thread `call_id` through `ToolCall`).

## Streaming Support

The `LlmProvider` trait has optional streaming methods:

```rust
async fn complete_streaming(&self, request, chunk_tx: StreamingChunkSender) -> Result<CompletionResponse, LlmError>;
async fn complete_with_tools_streaming(&self, request, chunk_tx: StreamingChunkSender) -> Result<ToolCompletionResponse, LlmError>;
```

`StreamingChunkSender` is `tokio::sync::mpsc::Sender<String>` — each `String` is one text delta.

**Default behavior:** Both methods fall back to the blocking `complete()` / `complete_with_tools()` and drop the sender (no chunks emitted). Existing providers that do not implement streaming continue to work unchanged.

**Wrapper override requirement:** Decorator wrappers (retry, circuit breaker, smart routing, failover, cache, recording, token refresh) **must override both streaming methods** — the trait default bypasses the wrapper's semantics. This is the same rule as for `complete()` / `complete_with_tools()` per `.claude/rules/review-discipline.md`.

**Currently implemented:** No provider implements native streaming yet. The `openai_compatible_streaming` backend (planned) will be the first.

## Trace Recording

Set `IRONCLAW_RECORD_TRACE=1` to enable live trace recording via `RecordingLlm`. Traces are JSON files containing: memory snapshot, HTTP exchanges from tools, and LLM steps (user inputs, text responses, tool call responses). Replay these in E2E tests via `TraceLlm`. Configure output path with `IRONCLAW_TRACE_OUTPUT` (default: `trace_{timestamp}.json`).
