# Audit V3 — Post-Fix Codebase Quality Review

Date: 2026-02-25
Branch: `feat/tool-transparency` (local working tree, post audit-v2 fixes)
Scope: Full paranoid-architect review of all files modified since `main`, including the original PR (#334) and all 20 audit-v2 fixes (C1–C5, I1–I9, D1–D5).

---

## Bug Fix Protocol

Same as audit-v2. For each issue below, follow this sequence strictly:

1. **Write regression test** — A failing test that demonstrates the bug before any fix
2. **Research** — Read the relevant code paths, understand root cause, identify all instances of the pattern
3. **Fix** — Apply the minimal correct fix
4. **Run regression test** — Confirm the new test passes
5. **Refactor if needed** — Clean up without changing behavior
6. **Run full tests** — `cargo test --all-features` to catch regressions
7. **Report** — Update the status column in this file

Status key: `[ ]` pending, `[~]` in progress, `[x]` done, `[-]` won't fix (with reason)

---

## Audit V2 Status

All 20 issues from audit-v2 are resolved: C1–C5 (5/5), I1–I9 (9/9), D1–D5 (5/5). See `.claude/docs/audit-v2.md` for details and resolutions.

---

## High Severity

### H1. UTF-8 byte-slice panic in NEAR AI error path `[x]`
- **File:** `src/llm/nearai_chat.rs:389`
- **Confidence:** 95
- **Impact:** Runtime panic (`byte index N is not a char boundary`) when the NEAR AI model-list endpoint returns a non-ASCII response and the code truncates with `&response_text[..response_text.len().min(300)]`. Byte indexing on `&str` is unsafe at arbitrary offsets. Reachable in production whenever the API returns error messages containing multi-byte characters.
- **Fix direction:** Replace byte-slice with `response_text.chars().take(300).collect::<String>()` or use `str::floor_char_boundary()` (nightly) / a manual char-boundary scan.
- **Resolution:** Replaced `&response_text[..response_text.len().min(300)]` with `let boundary = crate::util::floor_char_boundary(&response_text, 300); &response_text[..boundary]`. Uses the existing polyfill already used in shell.rs. Regression test `test_truncate_multibyte_response_no_panic` proves the old pattern panics on 4-byte emoji at byte 298–302 and the safe path works. Note: 6 more instances of the same unsafe pattern exist in `repl.rs`, `cli/config.rs`, `cli/tool.rs`, `cli/mcp.rs`, `wasm/host.rs`, `sandbox/container.rs` — not in scope for this PR but documented for future cleanup.

### H2. Raised hex leak threshold reduces detection of real secrets `[x]`
- **File:** `src/safety/leak_detector.rs:520`
- **Confidence:** 80
- **Impact:** The `high_entropy_hex` pattern threshold was raised from 64 to 128 chars to reduce false positives from CSS integrity hashes and webpack chunks. However, 64-char hex strings are common as HMAC signing secrets, webhook verification tokens, and generic API keys (any SHA-256-derived secret). The existing branded patterns (OpenAI `sk-proj-`, GitHub `ghp_`, AWS `AKIA`) don't cover unbranded hex secrets. The action was already `Warn` (mildest response), so the false-positive cost was low.
- **Fix direction:** Consider a compromise threshold (e.g., 96 chars) or a context-aware pattern that matches 64-char hex only when preceded by secret-context words (`secret|token|key|password|hmac|signing`). Alternatively, document the trade-off and accept the gap if web content false positives were genuinely disruptive in practice.
- **Resolution:** Reverted threshold from 128 back to 64 chars. The action is `Warn` (mildest), so false positives from CSS/webpack hashes only produce a log line — acceptable cost to catch unbranded 64-char hex secrets (HMAC keys, webhook tokens). Updated comment to document the trade-off. Test `test_high_entropy_hex_detects_64_char` verifies both detection at 64 and non-detection at 63.

---

## Medium Severity

### M1. PR description claims removed feature `[ ]`
- **File:** PR #334 description
- **Confidence:** 100
- **Impact:** The PR title says "add tool call audit trail" and the description's first bullet says "Persist tool call summaries (name, params, status, output size) to conversation DB". However, the audit-v2 fixes removed all audit trail code: `tool_summaries`, `sanitize_audit_field`, `extract_params_preview`, `AUDIT_FIELD_MAX_LEN`, the DB persistence call, and all 5 associated tests. The feature is gone; the description is misleading.
- **Fix direction:** Either update the PR title/description to reflect what actually ships, or re-add the audit trail feature. If re-adding, the original `sanitize_audit_field` had its own UTF-8 byte-slice bug (`&cleaned[..AUDIT_FIELD_MAX_LEN]`) that needs fixing first.

### M2. Silent data loss on malformed tool call arguments `[x]`
- **File:** `src/llm/nearai_chat.rs:513`
- **Confidence:** 90
- **Impact:** `serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Object(Default::default()))` silently replaces parse failures with `{}`. The tool executor receives empty params and fails with a confusing "missing required parameter" error. No log entry indicates the LLM returned unparseable JSON. Root-cause diagnosis is difficult.
- **Fix direction:** Add `tracing::warn!("Failed to parse tool call arguments for {}: {} (raw: {})", tc.function.name, err, &tc.function.arguments[..tc.function.arguments.len().min(200)])` before defaulting. (Note: the truncation in this warn also needs char-boundary safety — use the same fix as H1.)
- **Resolution:** Replaced `.unwrap_or()` with `.unwrap_or_else(|err| { ... })` that emits `tracing::warn!` with tool name, parse error, and raw arguments truncated to 200 bytes via `floor_char_boundary` (same polyfill as H1). Still defaults to `{}` so the tool executor can report a proper "missing parameter" error rather than panicking. Tests: `test_malformed_tool_call_arguments_defaults_to_empty_object` verifies fallback behavior, `test_malformed_tool_call_arguments_multibyte_truncation` verifies the truncation doesn't panic on multi-byte input.

### M3. `.expect()` in production startup path `[x]`
- **File:** `src/app.rs:610`
- **Confidence:** 100
- **Impact:** `SecretsCrypto::new(ephemeral_key).expect("ephemeral crypto")` violates the project's absolute policy of no `.unwrap()`/`.expect()` in production code. While the invariant (generated hex key) makes failure unlikely, the policy exists precisely to prevent "unlikely" panics from reaching production.
- **Fix direction:** Change to `SecretsCrypto::new(ephemeral_key).map_err(|e| anyhow::anyhow!("ephemeral crypto init failed: {e}"))?` or equivalent error propagation.
- **Resolution:** Replaced `.expect("ephemeral crypto")` with `.map_err(|e| anyhow::anyhow!("ephemeral crypto init failed: {e}"))?`. The containing function `build_all` already returns `Result<_, anyhow::Error>`, so the `?` propagates naturally. Verified no other `.expect()`/`.unwrap()` remain in `app.rs` production code.

---

## Low Severity

### L1. Full conversation content logged at DEBUG level `[x]`
- **File:** `src/llm/nearai_chat.rs:186,208`
- **Confidence:** 85
- **Impact:** Both the full HTTP request body (system prompt + conversation history + any PII) and response body are emitted via `tracing::debug!`. In centralized log collection environments (cloud logging, log aggregators), setting `RUST_LOG=debug` or `RUST_LOG=ironclaw=debug` exposes complete user conversations. The request body log is higher risk.
- **Fix direction:** Truncate logged content to first ~500 chars, or log only metadata (message count, model, token count). Alternatively, use a dedicated `tracing::trace!` level for full-body dumps and keep `debug!` for metadata-only.
- **Resolution:** Moved full request/response body logging from `debug!` to `trace!`. At `debug!` level, only metadata is logged: model name, message count, body size (request) and HTTP status, body size (response). Operators must now explicitly set `RUST_LOG=ironclaw::llm=trace` to see full bodies — an intentional, auditable decision.

### L2. Unbounded task-scoped HashMap in failover and smart routing `[x]`
- **File:** `src/llm/failover.rs` (`provider_for_task`), `src/llm/smart_routing.rs` (`routed_for_task`)
- **Confidence:** 70
- **Impact:** Both maintain `Mutex<HashMap<tokio::task::Id, _>>` where entries are inserted on `complete()` and removed on `take_bound_provider_for_current_task()` / `take_route_for_current_task()`. If a task panics or is cancelled between insert and remove, entries leak permanently. In a long-running agent, this could accumulate stale entries without bound.
- **Fix direction:** Either add a capacity check (log+evict when map exceeds N entries), add a periodic sweep, or use a guard-based RAII pattern that removes entries on drop.
- **Resolution:** Added `TASK_MAP_CAPACITY = 1000` constant to both `FailoverProvider` and `SmartRoutingProvider`. After each insert into `provider_for_task` / `routed_for_task`, if the map exceeds the threshold, all entries are evicted with a `tracing::warn!` log. Evicted tasks fall back to the global `last_used`/`last_routed` atomic (existing fallback behavior). Regression tests `provider_for_task_evicts_when_capacity_exceeded` and `routed_for_task_evicts_when_capacity_exceeded` verify the eviction logic.

### L3. Misleading `// SAFETY:` comment on safe code `[x]`
- **File:** `src/agent/agent_loop.rs:447-450`
- **Confidence:** 100
- **Impact:** Uses the `// SAFETY:` convention (standard for documenting `unsafe` block invariants) on ordinary safe code (`Arc::clone`). A safety auditor performing `grep -rn 'SAFETY:' src/` will stop to verify an invariant for a nonexistent `unsafe` block. Noise in security reviews.
- **Fix direction:** Rewrite as `// Note:` or remove the comment.
- **Resolution:** Replaced the two misleading `// Safety:` / `// SAFETY:` comments with a single plain comment: "Store engine reference for event trigger checking in the message loop below. This is just an Arc::clone; safe and cheap." Grep of other `// SAFETY:` comments confirmed this was the only false positive on purely safe code.

---

## Informational

### I1. Quality gate mutex-unwrap scan is partial `[-]`
- **File:** `scripts/quality-gate.sh:64-76`
- **Impact:** `grep -n '\.lock()\.unwrap()'` only catches single-line patterns. Multi-line lock-then-unwrap and mid-file `#[cfg(test)]` on non-module items are not caught.
- **Status:** Won't fix — the scan catches the common pattern; a complete lint would require a Rust AST parser. Documented here for awareness.

---

## Verified Correct (audit-v2 fixes reviewed)

The following audit-v2 fixes were reviewed in depth during this audit and confirmed correct:

| Fix | Verified Behavior |
|-----|-------------------|
| **D5** TOCTOU fix | `agent_context()` takes `&HashMap` ref; single `MutexGuard` covers lookup + insert. Structurally race-free. |
| **D1** Observer emission helpers | Clean wrappers, no logic duplication. |
| **C1** AgentEnd on compact-retry failure | Emitted inside `.map_err` closure before `?` propagation. |
| **C2** Separate LlmRequest/LlmResponse for retry | Timing reset, compacted message count, failure response for first call — all correct. |
| **C3** effective_model_name() | Request-scoped model resolution through SmartRouting/Failover chain. |
| **C4** Error-path LlmResponse emission | Both `ContextLengthExceeded` and general error paths emit LlmResponse + AgentEnd. |
| **C5** Observer flush/shutdown | Per-instance `flush()` and `shutdown()` on `Observer` trait, called from `app.rs` teardown. |
| **I1** Mutex poison recovery | `unwrap_or_else(\|e\| e.into_inner())` with warning log. |
| **I2** Orphan span drain | AgentEnd drains all remaining child spans with error status. |
| **I3** False compaction note | Only inserted when non-system messages were actually dropped. |
| **I4** Cache hit cost tracking | `cached = true` on cache hits, `Decimal::ZERO` cost. |
| **I5** Outbound event before hook | Hook fires before ToolCallStart emission (checked by grep). |
| **I9** Orphaned root spans | ChannelMessage and HeartbeatTick use `start_with_context` with agent parent. |
| **D2** Turn complete tool count | `runnable.len()` (executed tools) instead of `tool_calls.len()` (requested tools). |
| **D3** RecordingObserver wired into TestHarnessBuilder | `with_observer()` builder method, tests use it instead of post-build mutation. |
| **D4** Cargo.toml `[[test]]` for otel_e2e | `required-features = ["otel"]` skips compilation in default builds. |

---

## Summary

| Severity | Count | Status |
|----------|-------|--------|
| High | 2 | 2 done |
| Medium | 3 | 2 done, 1 pending |
| Low | 3 | 3 done |
| Informational | 1 | Won't fix |
| **Total actionable** | **8** | **7 done, 1 pending** |

All code-level findings are resolved. The sole remaining item is M1 (PR description claims removed feature) which is a PR metadata update, not a code fix.
