# Staging CI Review Issues Triage

**Date:** 2026-03-18
**Branch:** staging (HEAD `b7a1edf`)
**Total open issues:** 50

---

## Batch 1 — Critical & 100-confidence issues

| # | Title | Severity | Verdict | File(s) | Action |
|---|-------|----------|---------|---------|--------|
| 1281 | Logic inversion in Telegram auto-verification | CRITICAL:100 | **FALSE POSITIVE** (closed) | `src/channels/web/server.rs` | Different handlers with intentional different SSE behavior |
| 908 | Missing consecutive_failures reset | CRITICAL:100 | **STALE** | `src/llm/circuit_breaker.rs` | Close — `record_success()` already resets to 0 |
| 1282 | Variable shadowing fallback notification | HIGH:100 | **STALE** | `src/agent/agent_loop.rs` | Close — fixed in commit `bcc38ce` |
| 1283 | Inconsistent fallback logic DRY | HIGH:75 | **STALE** | `src/agent/agent_loop.rs` | Close — fixed in commit `bcc38ce` |
| 1178 | Workflow linting bypass for test code | CRITICAL:75 | **FALSE POSITIVE** | `.github/workflows/code_style.yml` | Close — script reads full file, not hunk headers |

---

## Remaining Batches (queued)

### Batch 2 — Retry/DRY + CI workflow issues (completed)

| # | Title | Severity | Verdict | Action |
|---|-------|----------|---------|--------|
| 1288 | DRY violation: retry-after parsing | HIGH:95 | **LEGIT** | Fixed: extracted shared `parse_retry_after()` |
| 1289 | Semantic mismatch in RFC2822 test helpers | MEDIUM:85 | **DUPLICATE** (closed) | Duplicate of #1288 |
| 1290 | Unnecessary eager `chrono::Utc::now()` call | LOW:85 | **FALSE POSITIVE** (closed) | Already deferred inside successful parse branch |
| 963 | Logical equivalence bug in workflow conditions | HIGH:100 | **FALSE POSITIVE** (closed) | Refactored condition correctly handles `workflow_call` |
| 1280 | Flaky OAuth wildcard callback tests | Flaky | **LEGIT** | Fixed: added `tokio::sync::Mutex` for env var serialization |

### Batch 3 — Routine engine + notification routing
- #1365 — too_many_arguments on RoutineEngine::new()
- #1371 — Discovery schema regeneration on every tool_info call
- #1364 — Prompt injection via unescaped channel/user in lightweight routines
- #1284 — notification_target_for_channel() assumes channel owner

### Batch 4 — Telegram/Extension Manager webhook group
- #1247 — Synchronous 120-second blocking poll in HTTP handler
- #1248 — Hardcoded channel-specific logic violates architecture
- #1249 — Telegram-specific business logic bloats ExtensionManager
- #1250 — Response success/failure logic mismatch in chat auth
- #1251 — Channel-specific configuration mappings lack extensibility

### Batch 5 — HMAC/Auth/Security
- #1034 — Signature verification not constant-time
- #1035 — Incorrect order of operations in HMAC verification
- #1036 — Double opt-in lacks runtime validation consistency
- #1037 — API breaking change: auth() signature
- #1038 — CSP policy allows CDN scripts with risky fallback

### Batch 6 — Webhook handler + config
- #1039 — Per-request HTTP client creation in hot path
- #1040 — Complex nested auth logic in webhook_handler
- #1041 — Redundant JSON deserialization in webhook handler
- #1042 — Implicit state mutation in config conversion
- #1005 — Inconsistent double opt-in enforcement

### Batch 7 — Tool schema validation / WASM bounds
- #974 — Unbounded recursion in resolve_nested()
- #975 — Unbounded recursion in validate_tool_schema()
- #976 — Unbounded description string in CapabilitiesFile
- #977 — Unbounded parameters schema JSON
- #978 — Unnecessary clone of large JSON in hot path

### Batch 8 — Tool schema + config + security
- #979 — No size limits on JSON files read
- #980 — Misleading warning condition for missing parameters
- #988 — Hardcoded CLI_ENABLED env var in systemd template
- #990 — Configuration semantics unclear for daemon mode
- #1103 — SSRF risk via configurable embedding base URL

### Batch 9 — Agent loop / job worker
- #870 — Unbounded loop without cancellation token
- #871 — Stringly-typed unsupported parameter filtering
- #873 — RwLock overhead on hot path
- #892 — JobDelegate::check_signals() treats non-terminal as terminal
- #1252 — String concatenation in hot polling loop

### Batch 10 — Agent loop perf + CI scripts
- #893 — Unnecessary parameter cloning on every tool execution
- #894 — truncate_for_preview allocates for non-truncated strings
- #895 — Tool definitions fetched every iteration without caching
- #1179 — AWK state machine never resets between hunks
- #1180 — Code fence detection logic flawed in extract_suggestions()
- #1181 — Unsafe .unwrap() in production code manifest.rs
