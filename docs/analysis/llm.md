# IronClaw v0.11.1 — LLM Backend System Deep Dive

> **Scope:** `src/llm/`, `src/config/llm.rs`, `src/agent/cost_guard.rs`,
> `src/agent/context_monitor.rs`, `src/agent/compaction.rs`,
> `src/estimation/`, `src/evaluation/`, `src/observability/`

---

## Table of Contents

1. [Overview](#1-overview)
2. [Provider Trait and Core Types](#2-provider-trait-and-core-types)
3. [Supported Backends](#3-supported-backends)
4. [Reliability Wrapper Chain](#4-reliability-wrapper-chain)
5. [Smart Routing Provider](#5-smart-routing-provider)
6. [NEAR AI Provider — Consolidated Chat Completions](#6-near-ai-provider--consolidated-chat-completions)
7. [rig-core Adapter and Schema Normalization](#7-rig-core-adapter-and-schema-normalization)
8. [Session Management](#8-session-management)
9. [Configuration Resolution](#9-configuration-resolution)
10. [Cost Accounting and Guardrails](#10-cost-accounting-and-guardrails)
11. [Context Window Management](#11-context-window-management)
12. [Estimation, Evaluation, and Observability](#12-estimation-evaluation-and-observability)
13. [Extension Points](#13-extension-points)

---

## 1. Overview

IronClaw's LLM subsystem is structured as a layered stack with three tiers:

```
┌─────────────────────────────────────────────────────────┐
│  Reasoning  (src/llm/reasoning.rs)                      │
│  Orchestrates plan → select_tools → respond_with_tools  │
├─────────────────────────────────────────────────────────┤
│  Reliability wrappers (decorator chain)                 │
│  SmartRoutingProvider → RetryProvider →                 │
│  CircuitBreakerProvider → ResponseCacheProvider →       │
│  FailoverProvider → actual backend                      │
├─────────────────────────────────────────────────────────┤
│  Backend providers                                      │
│  NearAiChatProvider │ RigAdapter<M>                     │
│  (OpenAI, Anthropic, Ollama, OpenAI-compatible,         │
│   Tinfoil)                                              │
└─────────────────────────────────────────────────────────┘
```

All tiers implement the same `LlmProvider` trait (`src/llm/provider.rs`), so
the `Reasoning` orchestrator is blind to which backend or combination of
wrappers sits beneath it. Reliability behaviors compose by wrapping — adding
retry logic never requires modifying a provider implementation.

---

## 2. Provider Trait and Core Types

### 2.1 The `LlmProvider` Trait

Defined in `src/llm/provider.rs`. Every provider — native or wrapped — must
implement these methods:

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError>;
    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError>;
    fn model_name(&self) -> &str;
    fn cost_per_token(&self) -> Option<(Decimal, Decimal)>; // (input, output) per token
    fn model_metadata(&self) -> ModelMetadata;
    fn seed_response_chain(&self, response_id: String);     // NEAR AI only
    fn name(&self) -> &str;
}
```

The default `calculate_cost()` implementation multiplies token counts by the
rates from `cost_per_token()`:

```rust
fn calculate_cost(&self, input_tokens: u64, output_tokens: u64) -> Decimal {
    let (input_rate, output_rate) = self.cost_per_token().unwrap_or_default();
    input_rate * Decimal::from(input_tokens) + output_rate * Decimal::from(output_tokens)
}
```

### 2.2 Message and Completion Types

| Type | Purpose |
|------|---------|
| `ChatMessage` | Role-tagged message (`System`, `User`, `Assistant`, `Tool`) |
| `CompletionRequest` | Messages + max_tokens + temperature + stop_sequences |
| `CompletionResponse` | Content string + `FinishReason` + `TokenUsage` |
| `ToolCompletionRequest` | Extends `CompletionRequest` with `Vec<ToolDefinition>` |
| `ToolCompletionResponse` | Either `Text(String)` or `ToolCalls { calls, content }` |
| `ToolDefinition` | Name + description + JSON Schema parameters |
| `ToolCall` | ID + name + arguments (JSON `Value`) |
| `ToolResult` | ID + content (for feeding results back) |

### 2.3 Tool Message Sanitization

A key correctness concern: providers like Anthropic return `HTTP 400` when a
conversation references a `tool_call_id` that was never issued in the current
context (e.g., after failover to a different provider mid-session).

`sanitize_tool_messages()` in `provider.rs` scans the message list and rewrites
orphaned `role: Tool` messages as plain user messages:

```rust
// Before: Tool { tool_call_id: "abc", content: "result" }
// After:  User { content: "Tool result: result" }
```

This prevents HTTP 400 errors when switching providers mid-conversation.

---

## 3. Supported Backends

### 3.1 Backend Enum

`src/config/llm.rs` defines the `LlmBackend` enum, which is the entry point
for provider selection:

```rust
pub enum LlmBackend {
    #[default]
    NearAi,           // NEAR AI Chat Completions (dual auth: API key or session token)
    OpenAi,           // Direct OpenAI API
    Anthropic,        // Direct Anthropic API
    Ollama,           // Local Ollama instance
    OpenAiCompatible, // vLLM, LiteLLM, Together, OpenRouter, etc.
    Tinfoil,          // Private inference in hardware-attested TEEs
}
```

Parsing is case-insensitive and accepts aliases: `"claude"` maps to
`Anthropic`, `"near"` maps to `NearAi`, `"compatible"` maps to
`OpenAiCompatible`.

### 3.2 Factory in `src/llm/mod.rs`

`create_llm_provider(config)` matches on `LlmBackend` and constructs the
appropriate native provider, then wraps it in the reliability decorator chain
based on `NearAiConfig` settings (retry count, circuit breaker threshold,
cache TTL, failover cooldown).

Notable factory quirks:

- **OpenAI uses `CompletionsClient`** (not the Responses API client) to avoid
  a known `rig-core` panic when threading `call_id` values.
- **Tinfoil** always targets `https://inference.tinfoil.sh/v1` and supports
  only Chat Completions (no Responses API).
- **Ollama** defaults to `http://localhost:11434` and the `llama3` model.

### 3.3 Provider Matrix

| Backend | Auth | API Style | Tool Support | Cost Tracking |
|---------|------|-----------|--------------|---------------|
| NearAiChat (API key mode) | API key (`NEARAI_API_KEY`) | Chat Completions | Flattened to text | Via cost table |
| NearAiChat (session mode) | Session token (`SessionManager`) | Chat Completions | Flattened to text | Via cost table |
| OpenAI | API key | Chat Completions | Native (strict schema) | Via cost table |
| Anthropic | API key | Anthropic API | Native | Via cost table |
| Ollama | None | Chat Completions | Native | Zero cost (local) |
| OpenAiCompatible | Optional key | Chat Completions | Native | Via cost table |
| Tinfoil | API key | Chat Completions | Chat-format | Via cost table |

---

## 4. Reliability Wrapper Chain

The four reliability wrappers in `src/llm/` all implement `LlmProvider` and
accept `Arc<dyn LlmProvider>` as their inner provider. They compose cleanly:

```
RetryProvider(
  CircuitBreakerProvider(
    CachedProvider(
      FailoverProvider([primary, fallback])
    )
  )
)
```

### 4.1 Retry — `retry.rs`

**Retryable errors** (will retry): `RequestFailed`, `RateLimited`,
`InvalidResponse`, `SessionRenewalFailed`, `Http`, `Io`.

**Non-retryable errors** (fail immediately): `AuthFailed`, `SessionExpired`,
`ContextLengthExceeded`, `ModelNotAvailable`, `Json`.

**Backoff formula:**

```
delay = base_ms * 2^attempt * jitter_factor
base_ms = 1000ms
jitter_factor = random in [0.75, 1.25]  // +/-25%
minimum_delay = 100ms (floor)
```

For attempt 0: ~750–1250ms. Attempt 1: ~1500–2500ms. Attempt 2: ~3000–5000ms.

**Rate limit hint:** If the error is `RateLimited { retry_after: Some(dur) }`,
the retry provider sleeps exactly `dur` instead of computing backoff.

Default `max_retries` is 3, giving 4 total attempts (1 initial + 3 retries).

### 4.2 Circuit Breaker — `circuit_breaker.rs`

**State machine:**

```
Closed ──(threshold consecutive transient failures)──► Open
Open   ──(recovery_timeout elapsed)──────────────────► HalfOpen
HalfOpen ──(half_open_successes_needed successes)────► Closed
HalfOpen ──(any failure)─────────────────────────────► Open
```

**Key distinction from retry:** The circuit breaker includes `SessionExpired`
in its `is_transient()` check, while `retry.rs::is_retryable()` does not.
This means session expiry counts toward tripping the circuit breaker (signaling
persistent auth problems) but does not trigger individual retries.

**Default parameters:**

- `failure_threshold`: 5 consecutive transient failures
- `recovery_timeout`: 30 seconds
- `half_open_successes_needed`: 2 successful probes before fully closing

When Open, all requests are immediately rejected with `CircuitOpen` error
without touching the downstream provider.

### 4.3 Response Cache — `response_cache.rs`

`CachedProvider` intercepts `complete()` calls only. Tool calls via
`complete_with_tools()` are **never cached** because they may have side
effects.

**Cache key:** SHA-256 of `(model_name, messages_json, max_tokens, temperature,
stop_sequences)`. Any change to any field produces a different key.

**Eviction policy:** LRU by `last_accessed` timestamp. On each write, expired
entries (TTL exceeded) are pruned first, then the oldest entry is evicted if
at capacity.

**Default parameters:**

- `response_cache_enabled`: false (opt-in)
- `response_cache_ttl_secs`: 3600 (1 hour)
- `response_cache_max_entries`: 1000

### 4.4 Failover — `failover.rs`

`FailoverProvider` holds an ordered `Vec` of providers and tries each in
sequence when the active provider produces a retryable error.

**Cooldown mechanism:** Each provider tracks consecutive retryable failures
using lock-free atomics (`AtomicU32` failure count, `AtomicI64` cooldown
start time). A provider in cooldown is skipped. If all providers are in
cooldown, the one with the oldest cooldown start time is selected (never
blocks entirely).

**Per-task provider tracking:** Under concurrent requests, different tasks may
be using different providers in the failover chain simultaneously.
`provider_for_task: Mutex<HashMap<tokio::task::Id, usize>>` stores which
provider index each tokio task is currently using, so follow-up calls from
the same logical request stay on the same provider.

**Default parameters:**

- `failover_cooldown_secs`: 300 (5 minutes)
- `failover_cooldown_threshold`: 3 consecutive failures before cooldown

---

## 5. Smart Routing Provider

**`src/llm/smart_routing.rs`** (452 lines) — Added in v0.10.0

`SmartRoutingProvider` implements cost-optimized model selection by routing requests to cheap or primary models based on task complexity analysis.

The provider wraps two `LlmProvider` instances and implements the trait itself, fitting into the standard provider chain:

```
SmartRoutingProvider → RetryProvider → CircuitBreakerProvider → ResponseCacheProvider → FailoverProvider → actual backend
          ↓
   ┌──────┴──────┐
   ↓             ↓
Cheap Model  Primary Model
```

### 5.1 Task Complexity Classification

Every incoming request is classified by `classify_message()` into one of three tiers:

| Complexity | Criteria |
|------------|----------|
| `Simple` | Short queries ≤200 chars; single words; keywords: `list`, `show`, `what is`, `status`, `help`, `yes`, `no`, `ping` |
| `Moderate` | Medium length, ambiguous (falls between Simple and Complex) |
| `Complex` | Contains code blocks (` ``` `); keywords: `implement`, `refactor`, `analyze`, `debug`, `design`, `architecture`, `optimize`; or ≥1000 chars |

Additional rules:
- Very short messages (≤10 chars) → always `Simple`, regardless of content
- Tool use requests → always routed to primary model (reliable structured output required)

### 5.2 Routing

| Classification | Destination |
|---------------|-------------|
| `Simple` | Cheap model (`NEARAI_CHEAP_MODEL`, e.g., Claude Haiku) |
| `Complex` | Primary model (`NEARAI_MODEL`, e.g., Claude Sonnet/Opus) |
| `Moderate` | Cheap model, with cascade escalation if enabled |

### 5.3 Cascade Mode

Controlled by `SMART_ROUTING_CASCADE` (default: `true`).

When enabled, `Moderate` requests go to the cheap model first. If the cheap model returns an uncertain response, the request is automatically escalated to the primary model.

Uncertainty is detected by scanning the response for any of these phrases:
- `"I'm not sure"`
- `"I don't know"`
- `"I'm unable to"`
- `"I cannot"`
- `"I can't"`
- `"beyond my capabilities"`
- `"I need more context"`
- Empty response (zero-length content)

### 5.4 Configuration

```
SMART_ROUTING_CASCADE=true         # Enable cascade escalation (default)
NEARAI_CHEAP_MODEL=...             # Cheap model name for simple tasks
```

If `NEARAI_CHEAP_MODEL` is not set, smart routing is disabled and all requests go to the primary model.

### 5.5 Observable Statistics

Internal atomic counters expose routing decisions for observability:

| Counter | Description |
|---------|-------------|
| `total_requests` | All requests processed by the provider |
| `cheap_requests` | Requests routed to the cheap model |
| `primary_requests` | Requests routed to the primary model |
| `cascade_escalations` | Cheap-model responses escalated to primary due to uncertainty |

---

## 6. NEAR AI Provider — Consolidated Chat Completions (`src/llm/nearai_chat.rs`)

In v0.9.0, the separate `nearai.rs` (Responses API) was removed. Both auth modes are now unified in `nearai_chat.rs` using the Chat Completions API.

### 6.1 Auth Modes

| Mode | Trigger | Base URL | Token source |
|------|---------|----------|--------------|
| API key | `NEARAI_API_KEY` is set | `https://cloud-api.near.ai` (default) | API key as Bearer token |
| Session token | No `NEARAI_API_KEY` | `https://private.near.ai` (default) | `SessionManager` (auto-renew on 401) |

Both modes hit the same endpoint: `{NEARAI_BASE_URL}/v1/chat/completions`

The `NearAiChatProvider` struct holds: `client`, `config`, `session: Arc<SessionManager>`, `active_model: RwLock<String>`, `flatten_tool_messages: bool`. Auth is resolved via `resolve_bearer_token()`, which picks the API key or session token depending on which mode is active.

### 6.2 Tool Message Flattening

NEAR AI does not support `role: "tool"` messages. The provider rewrites them via `flatten_tool_messages()` before sending:

- Assistant messages with `tool_calls` → plain assistant text: `[Called tool \`name\` with arguments: {...}]`
- Tool result messages (`role: "tool"`) → user messages: `[Tool \`name\` returned: result]`

### 6.3 Session Token Renewal

On 401 response (session token mode only):

1. Check if body contains "session" + ("expired" | "invalid")
2. If yes → `LlmError::SessionExpired` → `session.handle_auth_failure()` → retry once
3. If no → `LlmError::AuthFailed`

Note: retries on other errors are handled by the outer `RetryProvider` wrapper, not here.

### 6.4 Model Listing

`GET /v1/models` — handles flexible response formats:

- `{models: [...]}` or `{data: [...]}` (OpenAI-style)
- Direct array `[...]`
- Each entry: tries `name`, `id`, `model`, `model_name`, `model_id`, nested `metadata.name`

### 6.5 Usage Parsing Resilience

Some providers omit `completion_tokens` from the usage block. The `parse_usage()` function handles this by computing `completion_tokens = total_tokens - prompt_tokens` when `completion_tokens` is missing.

---

## 7. rig-core Adapter and Schema Normalization

`src/llm/rig_adapter.rs` bridges `rig-core`'s `CompletionModel` trait to
IronClaw's `LlmProvider` trait. This adapter is used by OpenAI, Anthropic,
Ollama, OpenAI-compatible, and Tinfoil backends.

### 7.1 OpenAI Strict Mode Schema Normalization

OpenAI's strict mode requires JSON Schemas to be fully specified with no
ambiguity. `normalize_schema_strict()` transforms tool parameter schemas:

1. Sets `"additionalProperties": false` on all object types
2. Moves all properties to `"required"` array
3. Makes originally-optional properties nullable by wrapping their type in
   `["type", "null"]` via `make_nullable()`

This normalization happens recursively for nested object schemas. Without it,
OpenAI returns schema validation errors for tools with optional parameters.

### 7.2 Message Conversion

`convert_messages()` splits the flat `Vec<ChatMessage>` into rig-core's
expected structure:

- System role messages → `preamble` (rig-core uses this separately)
- All other messages → `history` (chronological order)

### 7.3 Tool Name and ID Normalization

**Tool name:** `normalize_tool_name()` strips the `proxy_` prefix that some
OpenAI-compatible proxies add to tool names (e.g., LiteLLM wrapping tools).
Without this, tool dispatch fails to find the registered tool.

**Tool call ID:** `normalized_tool_call_id()` generates a fallback ID
`generated_tool_call_{seed}` for empty or missing tool call IDs, which
some providers omit.

---

## 8. Session Management

`src/llm/session.rs` implements `SessionManager` for NEAR AI OAuth session
tokens.

### 8.1 Storage and Priority

Token resolution order (highest to lowest priority):

1. `NEARAI_SESSION_TOKEN` environment variable
2. Database settings table (`nearai.session_token`)
3. Disk file at `~/.ironclaw/session.json` (mode `0o600`)

### 8.2 Renewal and Thundering Herd Prevention

The session token is held in a `RwLock<SecretString>` for concurrent read
access. When renewal is needed, a `Mutex<()>` renewal lock serializes renewal
attempts. Only the first concurrent caller acquires the lock and performs
renewal; subsequent callers wait and then use the freshly renewed token.

This prevents the thundering herd problem where many simultaneous requests
all try to renew the token independently, causing multiple redundant OAuth
round trips.

### 8.3 OAuth Providers

Supports GitHub and Google OAuth flows for `private.near.ai` authentication.
For `cloud-api.near.ai`, API keys are used directly (saved to
`~/.ironclaw/.env` on first setup).

---

## 9. Configuration Resolution

`src/config/llm.rs` implements a three-tier priority system for all LLM
settings.

### 9.1 Priority Order

```
Environment variables  (highest — always override)
       ↓
Settings database      (per-user stored preferences)
       ↓
Compiled defaults      (lowest — always available)
```

This is implemented in `LlmConfig::resolve(settings: &Settings)`.

### 9.2 Provider-Specific Environment Variables

| Variable | Backend | Purpose |
|----------|---------|---------|
| `LLM_BACKEND` | All | Select backend (nearai/openai/anthropic/ollama/openai_compatible/tinfoil) |
| `NEARAI_SESSION_TOKEN` | NearAi | Optional env override for session token (used by session manager) |
| `NEARAI_API_KEY` | NearAi | API key; auto-selects ChatCompletions mode |
| `NEARAI_MODEL` | NearAi | Primary model name |
| `NEARAI_CHEAP_MODEL` | NearAi | Lightweight model for routing/heartbeat/evaluation |
| `NEARAI_FALLBACK_MODEL` | NearAi | Failover model |
| `NEARAI_MAX_RETRIES` | NearAi | Retry count (default: 3) |
| `CIRCUIT_BREAKER_THRESHOLD` | NearAi | Failures before open (None = disabled) |
| `CIRCUIT_BREAKER_RECOVERY_SECS` | NearAi | Recovery timeout (default: 30) |
| `RESPONSE_CACHE_ENABLED` | NearAi | Enable response cache (default: false) |
| `RESPONSE_CACHE_TTL_SECS` | NearAi | Cache TTL (default: 3600) |
| `RESPONSE_CACHE_MAX_ENTRIES` | NearAi | LRU capacity (default: 1000) |
| `LLM_FAILOVER_COOLDOWN_SECS` | NearAi | Provider cooldown (default: 300) |
| `LLM_FAILOVER_THRESHOLD` | NearAi | Failures before cooldown (default: 3) |
| `OPENAI_API_KEY` | OpenAi | Required |
| `OPENAI_MODEL` | OpenAi | Default: gpt-4o |
| `OPENAI_BASE_URL` | OpenAi | Optional proxy override |
| `ANTHROPIC_API_KEY` | Anthropic | Required |
| `ANTHROPIC_MODEL` | Anthropic | Default: claude-sonnet-4-20250514 |
| `OLLAMA_BASE_URL` | Ollama | Default: <http://localhost:11434> |
| `OLLAMA_MODEL` | Ollama | Default: llama3 |
| `LLM_BASE_URL` | OpenAiCompatible | Required (e.g. OpenRouter URL) |
| `LLM_API_KEY` | OpenAiCompatible | Optional |
| `LLM_MODEL` | OpenAiCompatible | Falls back to `selected_model` from DB |
| `LLM_EXTRA_HEADERS` | OpenAiCompatible | Comma-separated `Key:Value` pairs injected into every HTTP request (added v0.10.0). Example: `"HTTP-Referer:https://myapp.com,X-Title:MyApp"` |
| `TINFOIL_API_KEY` | Tinfoil | Required |
| `TINFOIL_MODEL` | Tinfoil | Default: kimi-k2-5 |

### 9.3 Cheap Model for Lightweight Tasks

`NEARAI_CHEAP_MODEL` names a second, lower-cost model used by
`create_cheap_llm_provider()` for operations that do not need the primary
model's full capability: heartbeat checks, intent routing, and LLM-based
job evaluation. When not set, it falls back to the primary model.

---

## 10. Cost Accounting and Guardrails

### 10.1 Cost Table — `src/llm/costs.rs`

`model_cost(model_id)` returns per-token pricing as `(input_rate, output_rate)`
in USD with `Decimal` precision. The function strips provider prefix from model
IDs (e.g., `openai/gpt-4o` → `gpt-4o`) before looking up the table.

Covered models include:

- OpenAI: GPT-3.5-turbo through GPT-5.3-codex, reasoning models (o1, o3, o4-mini)
- Anthropic: claude-haiku/sonnet/opus across all major versions
- Local models: zero cost via `is_local_model()` heuristic matching prefixes
  `llama`, `mistral`, `phi`, `gemma`, `qwen`, `deepseek`, `codellama`

Unknown models fall back to `default_cost()` which uses GPT-4o pricing, ensuring
cost tracking never panics but may slightly over-estimate for cheaper models.

### 10.2 CostGuard — `src/agent/cost_guard.rs`

`CostGuard` enforces two independent spending limits for autonomous/daemon modes:

**Daily budget:** Tracks cumulative USD spend via `Mutex<DailyCost>`. The
counter resets at UTC midnight. An `AtomicBool budget_exceeded` flag provides
a fast-path check that avoids acquiring the mutex on subsequent calls once
the budget is blown.

**Hourly rate limit:** Uses a `VecDeque<Instant>` sliding window of action
timestamps. Expired entries (older than 1 hour) are drained on each check.

**80% warning threshold:** A tracing `warn!` is emitted when daily spend
reaches 80% of the limit, before the hard block at 100%.

**Usage pattern:**

```rust
// BEFORE making an LLM call:
cost_guard.check_allowed().await?;  // Blocks if limit exceeded

// AFTER the call completes:
cost_guard.record_llm_call(model, input_tokens, output_tokens).await;
```

The separation of check and record means a single LLM call slot is evaluated
before commitment, but the cost is only counted after actual token consumption.

### 10.3 Gateway Status Popover (v0.10.0)

v0.10.0 added a gateway status popover in the UI that shows real-time token usage and estimated cost per session. The popover reads from the same counters updated by `CostGuard::record_llm_call()`, so the displayed figures are always consistent with the budget enforcement logic.

### 10.4 Reasoning Cost Integration

`Reasoning` (in `src/llm/reasoning.rs`) returns `TokenUsage` with every
`respond_with_tools()` call. The `agent/worker.rs` passes this to
`CostGuard::record_llm_call()` for budget tracking and to
`Estimator::record_actual()` for EMA learning.

---

## 11. Context Window Management

### 11.1 ContextMonitor — `src/agent/context_monitor.rs`

`ContextMonitor` tracks the size of the active conversation and triggers
compaction recommendations. Token estimation uses a word-count heuristic:

```
tokens = word_count * 1.3 + 4   // 1.3 tokens/word + 4 overhead for role+structure
```

This is intentionally approximate — exact tokenization would require shipping
a tokenizer per model. The conservative 1.3 multiplier slightly over-estimates,
which errs on the side of triggering compaction earlier.

**Default limits:**

- `context_limit`: 100,000 tokens
- `compaction_threshold`: 80% of limit (80,000 tokens)

**Strategy selection by fill level:**

| Fill % | Strategy |
|--------|---------|
| 80–85% | `MoveToWorkspace` (archive + keep 10 recent turns) |
| 85–95% | `Summarize` (LLM summary + keep 5 recent turns) |
| >95% | `Truncate` (drop oldest + keep 3 recent turns — no LLM call needed) |

`ContextBreakdown::analyze()` provides a per-role token breakdown
(system/user/assistant/tool) for debugging and monitoring.

### 11.2 ContextCompactor — `src/agent/compaction.rs`

`ContextCompactor` executes the strategy recommended by `ContextMonitor`.
It holds `Arc<dyn LlmProvider>` for the `Summarize` strategy.

**Summarize strategy:**

1. Collects turns to remove (all except the `keep_recent` most recent)
2. Calls the LLM with temperature 0.3, max 1024 tokens, requesting a bullet-
   point summary of key decisions, actions, and outcomes
3. Appends the summary to the workspace daily log at `daily/YYYY-MM-DD.md`
4. Truncates the thread via `thread.truncate_turns(keep_recent)`

**MoveToWorkspace strategy:**

1. Formats turns as structured markdown with turn numbers, user input, agent
   response, and tool names used
2. Appends raw content to the workspace daily log (no LLM call required)
3. Truncates to 10 recent turns

**Resilience:** Both workspace-writing strategies treat write failures as
non-fatal: a `tracing::warn!` is emitted and truncation still proceeds.
The agent never hangs on a failing workspace write.

**CompactionResult** reports `tokens_before`, `tokens_after`,
`turns_removed`, `summary_written`, and the generated `summary` text for
logging and observability.

---

## 12. Estimation, Evaluation, and Observability

### 12.1 Cost and Time Estimation — `src/estimation/`

`Estimator` combines three static estimators with an adaptive learner.

**Static estimates (per-tool lookup tables):**

| Tool category | Cost | Time |
|---------------|------|------|
| `http` | $0.0001 | 500ms |
| `echo`, `time`, `json` | $0.0000 | 10ms |
| LLM call | $0.01/1K tokens | ~50 tokens/second |

**`EstimationLearner` (EMA-based adaptive learning):**

- Tracks per-category `cost_factor` and `time_factor` (ratio of actual to
  estimated)
- Updates with Exponential Moving Average: `alpha = 0.1`
- Requires minimum 5 samples before adjusting estimates (cold-start guard)
- Confidence score: `0.5 + sample_factor * 0.3 + error_factor * 0.2`
  where `sample_factor` grows with sample count and `error_factor` decreases
  with prediction error
- Categories must be registered before use; unknown categories do not update

**`ValueEstimator`:**

- Target margin: 30% above cost
- Minimum acceptable margin: 10% above cost
- `is_profitable(cost, revenue)` → `revenue >= cost * 1.10`
- `calculate_margin(cost, revenue)` → `(revenue - cost) / revenue * 100`

### 12.2 Success Evaluation — `src/evaluation/`

`SuccessEvaluator` trait has two implementations:

**`RuleBasedEvaluator`:**

- Success rate threshold: 80% of tool actions must succeed
- Maximum tolerated failures: 3
- Critical error keywords: scans job output for patterns indicating hard failure
- Job state: `Completed` and `Submitted` states count as successful

**`LlmEvaluator`:**

- Sends a structured JSON prompt to the cheap LLM requesting a JSON response
  with fields `success: bool`, `confidence: f64`, and `reasoning: String`
- Falls back to `RuleBasedEvaluator` if JSON parsing fails

**`MetricsCollector`:**

- Per-tool `ToolMetrics`: call count, success/failure counts, average
  execution time, total cost
- Error categorization: timeout, rate_limit, auth, not_found, invalid_input,
  network, unknown
- `QualityMetrics`: overall success rate, average response time, total cost

### 12.3 Observability — `src/observability/`

The `Observer` trait provides a pluggable backend for event and metric recording:

```rust
pub trait Observer: Send + Sync {
    fn record_event(&self, event: ObserverEvent);
    fn record_metric(&self, metric: ObserverMetric);
    fn flush(&self);
    fn name(&self) -> &str;
}
```

**Events:** `AgentStart`, `LlmRequest`, `LlmResponse`, `ToolCallStart`,
`ToolCallEnd`, `TurnComplete`, `ChannelMessage`, `HeartbeatTick`,
`AgentEnd`, `Error`.

**Metrics:** `RequestLatency(Duration)`, `TokensUsed(u64)`,
`ActiveJobs(u64)`, `QueueDepth(u64)`.

**Backends:**

| Backend | Implementation | Overhead |
|---------|---------------|---------|
| `NoopObserver` | Empty inline methods | Zero |
| `LogObserver` | `tracing::info!` / `tracing::debug!` | Minimal |
| `MultiObserver` | Fans out to `Vec<Box<dyn Observer>>` | Per-backend |

`create_observer(config)` builds the right backend: `"log"` → `LogObserver`,
anything else (including `"none"`, `"noop"`, unknown strings) → `NoopObserver`.
The default backend is `"none"`.

Future backends (OpenTelemetry, Prometheus) can be added by implementing the
`Observer` trait and extending `create_observer()`.

---

## 13. Extension Points

### 13.1 Adding a New LLM Backend

1. Add a variant to `LlmBackend` in `src/config/llm.rs`
2. Add a config struct (e.g., `MyProviderConfig`) with auth and model fields
3. Add the config field to `LlmConfig` and populate it in `LlmConfig::resolve()`
4. Implement `LlmProvider` in `src/llm/my_provider.rs`
5. Add the match arm in `create_llm_provider()` in `src/llm/mod.rs`
6. Extend `FromStr` / `Display` for `LlmBackend`

The new provider immediately inherits the full reliability wrapper chain
(retry, circuit breaker, cache, failover) without any additional code.

### 13.2 Adding a New Observability Backend

1. Create `src/observability/my_backend.rs`
2. Implement `Observer` (four methods)
3. Add the match arm in `create_observer()` in `src/observability/mod.rs`
4. Expose via `pub use` in `src/observability/mod.rs`

### 13.3 Extending the Cost Table

Add entries to the match arm in `costs::model_cost()` in `src/llm/costs.rs`.
The function signature accepts any `&str` model ID and returns
`Option<(Decimal, Decimal)>`. Entries that return `None` fall through to
`default_cost()` (GPT-4o pricing).

For local models, extend the prefix list in `is_local_model()` — these return
zero cost without a table lookup.

### 13.4 Adding a Compaction Strategy

1. Add a variant to `CompactionStrategy` in `src/agent/context_monitor.rs`
2. Update `suggest_compaction()` fill-level thresholds as needed
3. Add the match arm in `ContextCompactor::compact()` in `src/agent/compaction.rs`
4. Implement the strategy method returning `CompactionPartial`

### 13.5 Custom Success Evaluation

Implement `SuccessEvaluator` in `src/evaluation/success.rs` and wire it into
the agent worker. The trait requires a single async method:

```rust
async fn evaluate(
    &self,
    job: &JobContext,
    actions: &[ActionRecord],
    output: &str,
) -> EvaluationResult;
```

`EvaluationResult` contains `success: bool`, `confidence: f64`, and
`reasoning: String`.

---

*Generated from IronClaw v0.11.1 source — `src/llm/`, `src/config/llm.rs`,
`src/agent/cost_guard.rs`, `src/agent/context_monitor.rs`,
`src/agent/compaction.rs`, `src/estimation/`, `src/evaluation/`,
`src/observability/`.*
