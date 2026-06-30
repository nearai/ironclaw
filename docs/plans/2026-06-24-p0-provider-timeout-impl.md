# P0 Implementation Plan — Provider Timeout & `timeout < lease` Invariant

**Date:** 2026-06-24
**Branch:** `fix/reborn-p0-provider-timeout` (worktree `/Users/henry/Code/ironclaw-wt-p0`)
**Source triage:** `docs/plans/2026-06-24-reborn-runtime-wedge-triage.md`

## Problem (verified)

During the 2026-06-24 Reborn meltdown the NEAR AI client hung ~90s, runs failed
en masse with `failure_category=lease_expired`, and the runtime went silent for
~4 min before the gateway returned 502s. Two P0 root causes confirmed in code and
in the incident logs:

1. **NEAR AI reqwest client has no connect cap and a request timeout *longer*
   than the runner lease.** `crates/ironclaw_llm/src/nearai_chat.rs:192` builds the
   client with only `.timeout(request_timeout_secs)`. The production default for
   `request_timeout_secs` is **120** (`LlmConfig.request_timeout_secs`, default set
   in `resolution.rs:541`, `lib.rs:1229`, `models.rs:399`, `testing/mod.rs:45`).
   The runner lease is **90s** (`ironclaw_turns/src/memory/mod.rs:86`
   `DEFAULT_RUNNER_LEASE_TTL_SECONDS = 90`). Because `120 > 90`, the lease expires
   before the HTTP timeout can fire — the runner is killed mid-flight. There is no
   `.connect_timeout` and no `.tcp_keepalive`, so a cold/half-open socket can hang
   well past either bound.

2. **The PRIMARY assistant model call has no `tokio::time::timeout`.**
   `crates/ironclaw_turns/src/run_profile/model.rs:338`
   (`HostManagedLoopModelPort::stream_model`) awaits `self.gateway.stream_model(...)`
   unguarded. Only the compaction / system-inference path
   (`loop_support/src/system_inference.rs:159`) and the filesystem-apply path use a
   timeout. So a provider hang on the primary path is bounded only by the 90s
   lease — exactly the observed failure.

**Log evidence:** `failure_category=lease_expired` plus
`error=system inference timed out` in `ironclaw_reborn_composition::projection::turn_events`
across the 19:39–19:55 window, with 124 `502`s per log file.

## Constraints / non-goals

- `ironclaw_llm` must NOT depend on `ironclaw_turns` (confirmed: no such dep
  today). So the lease const cannot be imported into `ironclaw_llm`.
- This PR is **Phase 0 + Phase 1c** only (provider timeout + primary-call
  timeout + the invariant). Worker-pool parking (Wedge 2) and the remaining
  lock convoys (Wedge 3 / Phase 2) are explicitly out of scope.
- Match the existing error taxonomy; do not invent a parallel one.

## Design

### A. NEAR AI client hardening — `ironclaw_llm`

In `crates/ironclaw_llm/src/nearai_chat.rs::new_with_options` builder:

- Add `.connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))` — caps the
  TCP/TLS handshake so a cold/black-holed socket fails fast (10s).
- Add `.tcp_keepalive(Duration::from_secs(TCP_KEEPALIVE_SECS))` — keeps pooled
  sockets healthy and surfaces dead peers (30s).
- Keep `.timeout(request_timeout_secs)` (already present) — total-request cap.

These two new caps are provider-config-independent and always-on; they are local
named consts in `nearai_chat.rs`.

### B. Single source of truth for the request-timeout default — `ironclaw_llm`

Replace the four hardcoded `120` defaults with one public const:

```rust
/// Default per-request LLM HTTP timeout (seconds). Kept BELOW the Reborn
/// runner lease (see ironclaw_turns DEFAULT_RUNNER_LEASE_TTL_SECONDS = 90) so
/// the HTTP layer fails a hung request before the lease reclaims the runner.
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 60;
```

Used at `resolution.rs:541`, `lib.rs:1229`, `models.rs:399`, `testing/mod.rs:45`,
and as the value `nearai_chat.rs::new` passes (replacing the inline `120`). The
`LLM_REQUEST_TIMEOUT_SECS` env override path is unchanged.

Rationale for 60s: comfortably below the 90s lease with headroom for the
wrapper bound below, and below the wrapper so the precise provider error
surfaces first on the common path.

### C. Primary-call timeout wrapper + invariant — `ironclaw_turns`

In `crates/ironclaw_turns/src/run_profile/model.rs`:

- Add module consts:
  ```rust
  /// Hard ceiling on a single primary assistant model call. MUST stay below
  /// the runner lease (DEFAULT_RUNNER_LEASE_TTL_SECONDS) so a hung provider is
  /// surfaced as a retryable error before the lease reclaims the runner.
  const PRIMARY_MODEL_CALL_TIMEOUT: Duration = Duration::from_secs(75);
  ```
- Wrap the `self.gateway.stream_model(...)` await (line ~338) in
  `tokio::time::timeout(PRIMARY_MODEL_CALL_TIMEOUT, ...)`. On `Elapsed`, build a
  `LoopModelGatewayError::new(AgentLoopHostErrorKind::Unavailable, "model gateway timed out")`
  and feed it through the existing `Err` arm so post-call accounting / milestone /
  `into_host_error()` all run exactly as for any other gateway failure (the RAII
  reservation guard stays armed across the wrapped await, so cancellation is
  already handled). `Unavailable` maps to `ModelErrorClass::Unavailable` in
  `agent_loop/executor/mapping.rs`, which the recovery strategy retries with
  backoff — the correct disposition for a transient timeout.

- Make `DEFAULT_RUNNER_LEASE_TTL_SECONDS` (`memory/mod.rs:86`) `pub(crate)` so the
  invariant test can reference it without re-declaring the literal.

- **Invariant test** (in `model.rs` tests, same crate so it can see both consts):
  ```rust
  #[test]
  fn primary_model_call_timeout_is_below_runner_lease() {
      assert!(
          PRIMARY_MODEL_CALL_TIMEOUT.as_secs()
              < crate::memory::DEFAULT_RUNNER_LEASE_TTL_SECONDS as u64,
          "primary model-call timeout must fire before the runner lease expires"
      );
  }
  ```

Why the wrapper lives in `ironclaw_turns` and not `ironclaw_llm`: the lease is a
turns concept, the wrapper is the *defense-in-depth* bound that applies to the
whole gateway (every provider, not just NEAR AI), and `ironclaw_turns` is the only
crate that can legitimately see the lease const. `ironclaw_llm`'s own 60s default
is the inner, provider-specific bound; the 75s wrapper catches anything the inner
bound misses (other providers, gateway-layer stalls) while still beating the 90s
lease.

Ordering invariant: `60s (HTTP) < 75s (wrapper) < 90s (lease)`.

## Tests

1. `ironclaw_llm` — client-builder config assertion: a unit test in
   `nearai_chat.rs` proving the provider builds with the new default and that
   `DEFAULT_REQUEST_TIMEOUT_SECS < 90`. (reqwest doesn't expose builder values for
   readback, so assert on the const + successful build, not on internal fields.)
2. `ironclaw_turns` — primary-call timeout behavior: a `#[tokio::test]` with a
   fake `LoopModelGateway` whose `stream_model` sleeps past the bound (use
   `tokio::time::pause`/`advance`), asserting the port returns
   `AgentLoopHostErrorKind::Unavailable`. A second case: a fast gateway returns
   `Ok` unaffected.
3. `ironclaw_turns` — the `timeout < lease` invariant test (above).

## Files touched

- `crates/ironclaw_llm/src/nearai_chat.rs` — builder caps, `DEFAULT_REQUEST_TIMEOUT_SECS`, `new` uses it, builder-config test.
- `crates/ironclaw_llm/src/lib.rs`, `resolution.rs`, `models.rs`, `testing/mod.rs` — replace `120` with the const.
- `crates/ironclaw_turns/src/run_profile/model.rs` — wrapper, const, timeout + invariant tests.
- `crates/ironclaw_turns/src/memory/mod.rs` — make lease const `pub(crate)`.

## Out of scope (deferred follow-ups)

- Wedge 2 (`spawn_blocking` / `block_in_place` worker-pool parking).
- Wedge 3 Phase 2 (lock convoys in `run_state`/`threads`/`resources`/`secrets`,
  and the `ironclaw_secrets` `Arc`→`Weak` map leak).
- Phase 3 trigger-poller tick-gap alerting.
