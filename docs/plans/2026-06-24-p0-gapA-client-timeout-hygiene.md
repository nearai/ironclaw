# P0 Gap A — Cross-Provider LLM Client Timeout Hygiene

**Date:** 2026-06-24
**Branch:** `fix/reborn-p0-provider-timeout` (worktree `/Users/henry/Code/ironclaw-wt-p0`, PR #5204)
**Extends:** `docs/plans/2026-06-24-p0-provider-timeout-impl.md` (NEAR AI hardening + 75s primary-call wrapper, already committed at `8d4d8d47c`)

## Problem (verified in this worktree)

PR #5204 hardened only the **NEAR AI** reqwest client (`nearai_chat.rs:204`):
it added `.connect_timeout(10s)` + `.tcp_keepalive(30s)` and lowered the
request timeout to `DEFAULT_REQUEST_TIMEOUT_SECS = 60s`. Every **other**
production LLM HTTP client in `crates/ironclaw_llm/src/` still lacks a connect
timeout and TCP keepalive, and several have a total request timeout **above**
the 90s Reborn runner lease — the exact class of bug that wedged the runtime on
2026-06-24 (a half-open / cold socket hangs past the lease, the lease reclaims
the runner mid-flight before any HTTP timeout fires).

### Inventory (production reqwest `Client::builder()` / `Client::new()` sites)

Confirmed by reading every site. `openai::Client::builder()` /
`gemini::Client::builder()` etc. in `rig_adapter.rs` (incl. 2423/2442/2474/2499)
are **rig-core** builders, not reqwest, and the reqwest-shaped ones there are
test-only — out of scope.

| site | timeout today | connect | keepalive | pool_idle | note |
|---|---|---|---|---|---|
| `nearai_chat.rs:204` | 60s (const) | 10s | 30s | — | already hardened by #5204; reconcile onto shared path |
| `anthropic_oauth.rs:100` | **120s** hardcoded | — | — | — | LLM call; > lease |
| `github_copilot.rs:59` | `request_timeout_secs` | — | — | — | LLM call |
| `openai_codex_provider.rs:51` | `request_timeout_secs` | — | — | — | LLM call |
| `codex_chatgpt.rs:225` | `Client::new()` (per-req `.timeout`) | — | — | — | LLM call; client built with no caps |
| `rig_adapter.rs:113` | 30s | — | — | — | `/models` fetch; keeps `.redirect(none)` + `.resolve_to_addrs` |
| `gemini_oauth.rs:934` | **300s** hardcoded | — | — | — | LLM call; > lease |
| `gemini_oauth.rs:320` | 30s | — | — | — | OAuth credential-manager (auxiliary) |
| `openai_codex_session.rs:185` | 30s | — | — | — | session/token manager (auxiliary) |
| `session.rs:120` | 30s | — | — | — | NEAR session auth (auxiliary) |
| `transcription/openai.rs:23` | 120s | — | — | — | Whisper transcription (auxiliary, not a turn model call) |
| `transcription/chat_completions.rs:31` | 120s | — | — | — | transcription (auxiliary) |
| `auth.rs:287` | 15s | — | — | — | token exchange (auxiliary, short) |
| `lib.rs:338` (`build_http_client`) | **none** | — | — | — | `/models` probe builder; no timeout at all |

Verified facts:
- Runner lease `DEFAULT_RUNNER_LEASE_TTL_SECONDS = 90` (`ironclaw_turns/src/memory/mod.rs:92`, `pub(crate)`).
- `DEFAULT_REQUEST_TIMEOUT_SECS = 60` (`config.rs:269`).
- NEAR AI's `CONNECT_TIMEOUT_SECS`/`TCP_KEEPALIVE_SECS` are **module-private** consts in `nearai_chat.rs` — duplicating them across 4+ files is the "duplicate truth" smell to avoid.
- `ironclaw_llm` does **not** depend on `ironclaw_turns` (must stay so).
- No shared `hardened_client_builder` exists yet.

## Design

### One shared builder helper in `config.rs` (single source of truth)

Co-locate the connect/keepalive/pool-idle constants next to the existing
`DEFAULT_REQUEST_TIMEOUT_SECS`, and expose one factory every production client
funnels through:

```rust
/// Cap on the TCP/TLS handshake. A cold or black-holed socket fails fast
/// instead of hanging until the total request timeout.
pub const CONNECT_TIMEOUT_SECS: u64 = 10;
/// TCP keepalive probe interval so pooled sockets surface dead peers rather
/// than hanging on a half-open connection.
pub const TCP_KEEPALIVE_SECS: u64 = 30;
/// Max idle time a pooled connection is kept before being dropped. Bounds the
/// blast radius of a connection that silently went bad while idle. Set at the
/// lease boundary (90s) so an idle socket never outlives a runner lease.
pub const POOL_IDLE_TIMEOUT_SECS: u64 = 90;

/// Base reqwest builder with the standard timeout hygiene every LLM HTTP client
/// shares: total-request timeout, connect-handshake cap, TCP keepalive, and a
/// bounded idle-pool. Callers chain provider-specific options (.redirect,
/// .resolve_to_addrs, .default_headers, ...) onto the returned builder.
pub fn hardened_client_builder(request_timeout_secs: u64) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(request_timeout_secs))
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .tcp_keepalive(Duration::from_secs(TCP_KEEPALIVE_SECS))
        .pool_idle_timeout(Duration::from_secs(POOL_IDLE_TIMEOUT_SECS))
}
```

**Why a helper, not copy-pasted consts:** the four settings are one cohesive
policy. A returned `ClientBuilder` (not a built `Client`) lets each caller keep
the chained, site-specific options it already has (`rig_adapter`'s
`.redirect(Policy::none())` + `.resolve_to_addrs`, codex's `.default_headers`)
while guaranteeing the four hygiene settings are applied identically and can
only ever change in one place.

**Why `request_timeout_secs` stays a parameter (not folded into the helper):**
the request timeout is legitimately per-call. A turn model stream and a
one-shot OAuth token exchange should not share a 60s budget. The helper enforces
the *hygiene* (connect/keepalive/pool-idle) uniformly; the per-call total
timeout remains the caller's choice. This keeps auxiliary clients (auth token
exchange = 15s, OAuth credential manager = 30s) at their intentionally short
budgets while still gaining connect/keepalive/pool protection.

### Value justification

- `connect = 10s`: a TCP+TLS handshake to a healthy LLM endpoint completes in
  well under a second; 10s is generous slack for a slow-but-alive peer while
  still failing a black-holed socket an order of magnitude before the lease.
- `keepalive = 30s`: probes a pooled socket between requests so a peer that died
  while the connection sat idle is detected on the next use instead of hanging.
- `pool_idle = 90s`: drops idle pooled connections at the lease boundary so a
  silently-broken idle socket is never reused past a runner-lease lifetime;
  large enough to retain warm connections across back-to-back turns.

### Per-site changes

1. **`config.rs`** — add the three consts + `hardened_client_builder`. Bring
   `Duration` into scope.
2. **`nearai_chat.rs`** — delete the now-duplicate private `CONNECT_TIMEOUT_SECS`
   / `TCP_KEEPALIVE_SECS`; build via `config::hardened_client_builder(request_timeout_secs)`.
   (Now also gains `pool_idle_timeout`, which it previously lacked.) Keep its
   existing build-success + `< 90` tests.
3. **Primary LLM-call clients** route through the helper, preserving site options:
   - `anthropic_oauth.rs:100` — `120s` → `hardened_client_builder(DEFAULT_REQUEST_TIMEOUT_SECS)`
     (now 60s, below the lease).
   - `github_copilot.rs:59` — `hardened_client_builder(request_timeout_secs)`.
   - `openai_codex_provider.rs:51` — `hardened_client_builder(request_timeout_secs)`.
   - `codex_chatgpt.rs:225` (and the `#[cfg(test)] new`) — replace `Client::new()`
     with `hardened_client_builder(self.request_timeout/120).build()`; on build
     failure fall back to `Client::new()` so construction stays infallible
     (it currently can't error). Per-request `.timeout()` calls (302, 607) stay.
   - `gemini_oauth.rs:934` — `300s` → `hardened_client_builder(DEFAULT_REQUEST_TIMEOUT_SECS)`.
   - `rig_adapter.rs:113` — `hardened_client_builder(30)` then chain
     `.redirect(Policy::none())` and the conditional `.resolve_to_addrs`. The 30s
     `/models`-fetch budget is intentional and kept.
4. **Auxiliary clients** also route through the helper at their existing budgets
   (gains connect/keepalive/pool for free; request timeout unchanged):
   - `gemini_oauth.rs:320` (30s), `openai_codex_session.rs:185` (30s),
     `session.rs:120` (30s), `auth.rs:287` (15s),
     `transcription/openai.rs:23` (120s), `transcription/chat_completions.rs:31` (120s).
   Transcription stays at 120s: it is not a turn-model call and not gated by the
   runner lease; only its connect/keepalive/pool hygiene is the gap. (Noted, not
   lowered, to avoid changing transcription behavior in a P0 timeout-hygiene PR.)
5. **`lib.rs:338`** — `build_http_client` is fed a fresh `Client::builder()` for a
   `/models` probe with no timeout. Pass `hardened_client_builder(DEFAULT_REQUEST_TIMEOUT_SECS)`
   so the probe is bounded.

### Out of scope (unchanged)
- The 75s primary-call `tokio::time::timeout` wrapper in `ironclaw_turns`
  (already shipped in #5204) — untouched.
- Worker-pool parking / lock-convoy wedges (separate P1 work).

## Tests

- **`config.rs`** unit test: `hardened_client_builder(60).build()` succeeds, and
  `CONNECT_TIMEOUT_SECS`/`TCP_KEEPALIVE_SECS`/`POOL_IDLE_TIMEOUT_SECS`/
  `DEFAULT_REQUEST_TIMEOUT_SECS` are all `< 90` (reqwest exposes no builder-field
  readback, so assert on the consts + a successful build, matching the existing
  NEAR AI test convention).
- A test that every production provider that takes a config constructs
  successfully via the shared path (extend existing per-provider build smoke
  tests where present; keep NEAR AI's existing build test).
- Existing `ironclaw_turns` invariant tests stay green (no change there).

## Files touched
- `crates/ironclaw_llm/src/config.rs` — consts + helper + test.
- `crates/ironclaw_llm/src/nearai_chat.rs` — reconcile onto helper, drop dup consts.
- `anthropic_oauth.rs`, `github_copilot.rs`, `openai_codex_provider.rs`,
  `codex_chatgpt.rs`, `gemini_oauth.rs`, `rig_adapter.rs`,
  `openai_codex_session.rs`, `session.rs`, `auth.rs`,
  `transcription/openai.rs`, `transcription/chat_completions.rs`, `lib.rs`.
- `crates/ironclaw_llm/CLAUDE.md` — note the shared hardened-builder policy.
